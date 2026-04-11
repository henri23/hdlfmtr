--- hdlfmtr — fast VHDL formatter, Neovim integration.
---
--- This module is the entry point users `require("hdlfmtr")` from
--- their lazy.nvim config. It exposes:
---
---   • setup(opts)             — wire hdlfmtr into the editor
---   • format_buffer(bufnr)    — format a single buffer in place
---   • binary()                — absolute path to the bundled binary
---   • is_built()              — whether the binary exists on disk yet
---
--- The Rust binary is built by lazy.nvim's `build` hook
--- (`cargo build --release`) at install time and lives at
--- <plugin>/target/release/hdlfmtr (or .exe on Windows).

local M = {}

local DEFAULTS = {
  -- ── Editor integration ───────────────────────────────────────────
  -- When true, register `hdlfmtr` as a conform.nvim formatter, so
  -- users can reference it from their own `formatters_by_ft`. Has no
  -- effect if conform.nvim is not loaded.
  register_with_conform = true,

  -- When true, also prepend `hdlfmtr` to conform.formatters_by_ft.vhdl
  -- so format-on-save Just Works for vhdl buffers without further
  -- config. Defaults to false because users with an explicit conform
  -- config usually prefer to control ordering themselves.
  auto_register_vhdl = false,

  -- When true, the plugin sets `tabstop`, `shiftwidth`, `softtabstop`
  -- and `expandtab` on vhdl buffers to match the formatter config —
  -- so the visual indent in your editor matches the bytes hdlfmtr
  -- writes to disk. Disable if you manage these options yourself.
  set_buffer_options = true,

  -- ── Formatter options (forwarded as CLI flags) ───────────────────
  -- Indentation style: "tabs" | "spaces" | "preserve".
  indent_style = "tabs",

  -- Width of one indent level. With indent_style = "spaces" this is
  -- literally the number of spaces; with "tabs" it controls the
  -- buffer-local tabstop / shiftwidth set by `set_buffer_options`.
  indent_width = 4,

  -- Case to apply to reserved words: "lower" | "upper" | "preserve".
  keyword_case = "lower",

  -- Vertically align consecutive port / signal / generic / variable
  -- declarations so the names, `:`, modes, types and trailing comments
  -- line up in columns. Set false to disable.
  align_declarations = true,

  -- Indent `when` branch bodies one extra level inside case statements.
  -- Set false for the "flat" style where bodies sit at the same depth
  -- as the `when` keyword.
  case_branch_indent = true,

  -- Sort consecutive `use X.Y.Z;` clauses alphabetically. Set false to
  -- preserve the original order.
  sort_use_clauses = true,

  -- Insert one blank line between consecutive top-level statements
  -- inside an architecture body (process, labeled statement,
  -- component instantiation). Set false to disable.
  stylish_blank_lines = true,

  -- Run lint checks and surface their findings as nvim diagnostics
  -- (gutter signs, virtual text, ]d/[d navigation, quickfix). Set
  -- false to disable lint output entirely.
  lint = true,
}

--- Diagnostic namespace, lazily created. Each format_buffer call
--- replaces the diagnostics for the current buffer in this namespace.
local diagnostic_ns = nil
local function ns()
  if diagnostic_ns == nil then
    diagnostic_ns = vim.api.nvim_create_namespace("hdlfmtr")
  end
  return diagnostic_ns
end

local config = vim.deepcopy(DEFAULTS)

--- Resolve the directory containing this Lua file's grandparent —
--- i.e. the plugin install root, regardless of where lazy installed
--- it.
local function plugin_root()
  -- debug.getinfo gives us the absolute path to *this* file (with a
  -- leading `@` we strip). From `lua/hdlfmtr/init.lua` we go up three
  -- levels to reach the repo root.
  local source = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(source, ":h:h:h")
end

--- Absolute path to the hdlfmtr binary, accounting for Windows.
function M.binary()
  local exe = vim.fn.has("win32") == 1 and "hdlfmtr.exe" or "hdlfmtr"
  return plugin_root() .. "/target/release/" .. exe
end

--- True iff the bundled binary exists and is executable.
function M.is_built()
  return vim.fn.executable(M.binary()) == 1
end

local function notify_not_built()
  vim.notify(
    "hdlfmtr: binary not found at "
      .. M.binary()
      .. "\nrun `cargo build --release` from the plugin directory.",
    vim.log.levels.ERROR
  )
end

--- Build the CLI argument list from the current config. Used by both
--- the conform.nvim integration and the standalone format_buffer
--- code path so they always agree on flags.
local function build_args(filename)
  local args = {}
  if filename and filename ~= "" then
    table.insert(args, "--stdin-filepath")
    table.insert(args, filename)
  end
  table.insert(args, "--indent-style")
  table.insert(args, config.indent_style)
  table.insert(args, "--indent-width")
  table.insert(args, tostring(config.indent_width))
  table.insert(args, "--keyword-case")
  table.insert(args, config.keyword_case)
  if config.align_declarations == false then
    table.insert(args, "--no-align")
  end
  if config.case_branch_indent == false then
    table.insert(args, "--no-case-branch-indent")
  end
  if config.sort_use_clauses == false then
    table.insert(args, "--no-sort-uses")
  end
  if config.stylish_blank_lines == false then
    table.insert(args, "--no-stylish-blanks")
  end
  if config.lint == false then
    table.insert(args, "--no-lint")
  end
  return args
end

--- Severity strings emitted by the binary, mapped to nvim's enum.
local SEVERITY = {
  error = vim.diagnostic.severity.ERROR,
  warning = vim.diagnostic.severity.WARN,
  info = vim.diagnostic.severity.INFO,
  hint = vim.diagnostic.severity.HINT,
}

--- Decode a single JSON-encoded string body. Handles the escape
--- sequences hdlfmtr's hand-rolled encoder produces (no need to be
--- a full JSON parser - the binary controls both ends).
local function unescape_json_string(s)
  s = s:gsub("\\u(%x%x%x%x)", function(hex)
    return vim.fn.nr2char(tonumber(hex, 16))
  end)
  s = s:gsub("\\n", "\n")
  s = s:gsub("\\r", "\r")
  s = s:gsub("\\t", "\t")
  s = s:gsub('\\"', '"')
  s = s:gsub("\\\\", "\\")
  return s
end

--- Parse one JSONL diagnostic line into a table. Returns nil if the
--- line doesn't look like a diagnostic.
local function parse_diagnostic(line)
  if line == nil or line == "" then
    return nil
  end
  -- Field extraction is regex-based; the encoder is fixed-shape so
  -- this is safe and avoids pulling in a JSON dependency.
  local function num(field)
    return tonumber(line:match('"' .. field .. '":(%d+)'))
  end
  local function str(field)
    local m = line:match('"' .. field .. '":"(.-[^\\])"')
    if m == nil then
      m = line:match('"' .. field .. '":""')
      if m ~= nil then
        return ""
      end
    end
    if m == nil then
      return nil
    end
    return unescape_json_string(m)
  end
  local lnum = num("line")
  local col = num("col")
  local end_lnum = num("end_line")
  local end_col = num("end_col")
  local severity = str("severity")
  local code = str("code")
  local message = str("message")
  if lnum == nil or col == nil or message == nil then
    return nil
  end
  return {
    -- nvim is 0-indexed, hdlfmtr emits 1-indexed.
    lnum = lnum - 1,
    col = col - 1,
    end_lnum = (end_lnum or lnum) - 1,
    end_col = (end_col or col) - 1,
    severity = SEVERITY[severity or "warning"] or vim.diagnostic.severity.WARN,
    source = "hdlfmtr",
    code = code,
    message = message or "",
  }
end

--- Parse a stderr blob from the binary into a list of diagnostics.
local function parse_diagnostics_blob(blob)
  if blob == nil or blob == "" then
    return {}
  end
  local out = {}
  for line in blob:gmatch("[^\n]+") do
    local d = parse_diagnostic(line)
    if d ~= nil then
      table.insert(out, d)
    end
  end
  return out
end

--- Format a single buffer in place. Cursor and view are preserved.
--- Returns true on success, false otherwise.
function M.format_buffer(bufnr)
  bufnr = bufnr or 0
  if vim.fn.has("nvim-0.10") == 0 then
    vim.notify(
      "hdlfmtr requires Neovim 0.10 or newer (vim.system unavailable)",
      vim.log.levels.ERROR
    )
    return false
  end
  if not M.is_built() then
    notify_not_built()
    return false
  end

  local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
  local input = table.concat(lines, "\n") .. "\n"
  local filename = vim.api.nvim_buf_get_name(bufnr)

  local cmd = { M.binary() }
  for _, a in ipairs(build_args(filename)) do
    table.insert(cmd, a)
  end

  local result = vim.system(cmd, { stdin = input, text = true }):wait()

  if result.code ~= 0 then
    local msg = result.stderr ~= nil and result.stderr ~= "" and result.stderr
      or ("exit code " .. tostring(result.code))
    vim.notify("hdlfmtr: " .. msg, vim.log.levels.ERROR)
    return false
  end

  -- Parse diagnostics from stderr (the binary writes one JSON per
  -- line). Always replace the previous set, even if empty - that
  -- way fixing a warning clears it from the gutter immediately.
  if config.lint ~= false then
    local diagnostics = parse_diagnostics_blob(result.stderr or "")
    vim.diagnostic.set(ns(), bufnr, diagnostics)
  end

  -- Preserve cursor + window view across the buffer rewrite.
  local view = vim.api.nvim_win_call(0, function()
    return vim.fn.winsaveview()
  end)

  local out = result.stdout or ""
  -- Strip the trailing newline so vim.split doesn't introduce a
  -- phantom blank line at end-of-buffer.
  if out:sub(-1) == "\n" then
    out = out:sub(1, -2)
  end
  local out_lines = vim.split(out, "\n", { plain = true })

  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, out_lines)
  vim.api.nvim_win_call(0, function()
    vim.fn.winrestview(view)
  end)
  return true
end

--- Register hdlfmtr with conform.nvim if it is loaded.
local function register_with_conform()
  local ok, conform = pcall(require, "conform")
  if not ok then
    return
  end
  conform.formatters = conform.formatters or {}
  conform.formatters.hdlfmtr = {
    command = M.binary(),
    -- Function form so the args reflect the current config every
    -- time conform invokes the formatter, even if the user calls
    -- setup() again later with different options.
    args = function()
      return build_args("$FILENAME")
    end,
    stdin = true,
  }
  if config.auto_register_vhdl then
    conform.formatters_by_ft = conform.formatters_by_ft or {}
    local vhdl = conform.formatters_by_ft.vhdl or {}
    for _, name in ipairs(vhdl) do
      if name == "hdlfmtr" then
        return
      end
    end
    table.insert(vhdl, 1, "hdlfmtr")
    conform.formatters_by_ft.vhdl = vhdl
  end
end

--- Apply hdlfmtr's indent settings to vhdl buffers as they're opened.
--- Disabled by setting `set_buffer_options = false`.
local function install_buffer_option_autocmd()
  local group = vim.api.nvim_create_augroup("hdlfmtr.buffer_options", { clear = true })
  vim.api.nvim_create_autocmd("FileType", {
    group = group,
    pattern = "vhdl",
    callback = function(args)
      local bo = vim.bo[args.buf]
      bo.tabstop = config.indent_width
      bo.softtabstop = config.indent_width
      bo.shiftwidth = config.indent_width
      if config.indent_style == "spaces" then
        bo.expandtab = true
      elseif config.indent_style == "tabs" then
        bo.expandtab = false
      end
      -- "preserve" leaves expandtab alone.
    end,
  })
end

--- One-time setup. Idempotent — calling twice is harmless.
function M.setup(opts)
  config = vim.tbl_deep_extend("force", DEFAULTS, opts or {})

  if config.register_with_conform then
    register_with_conform()
  end
  if config.set_buffer_options then
    install_buffer_option_autocmd()
  end
end

return M
