# hdlfmtr

A fast VHDL formatter for editors and CI.

`hdlfmtr` is a Rust-native command-line tool that reformats VHDL source.
It is designed to start in single-digit milliseconds, distribute as a
single static binary, and integrate cleanly with editor plugins that
follow the prettier-style stdin-to-stdout contract (conform.nvim,
null-ls, efm-langserver, ALE, VS Code formatters).

## Status — honest scope

`hdlfmtr` is young but already useful. The current pipeline runs the
following passes in order:

1. **Process-label insertion.** Finds unlabeled `process` statements
   and adds a synthetic `u_process_N` to both the head and the
   matching `end process`. This is the case VSG's `process_016` /
   `process_018` rules flag as errors but refuse to auto-fix.
2. **Keyword case folding.** Reserved words become lowercase by
   default; configurable to upper or preserve. Identifiers are never
   touched.
3. **Indent normalization.** Tabs by default, configurable to spaces.
   Stack-based depth tracking handles entity / architecture / package
   / process / function / procedure / if-then-elsif-else / for-loop /
   while-loop / case / record / block / generate, plus multi-line
   parentheses for port and generic lists.
4. **Trailing-whitespace removal.** Always on.
5. **Final-newline ensuring.** Always on; the file ends with exactly
   one `\n`.

What is intentionally **not yet** done: port-list colon alignment,
keyword-case-aware predefined identifier folding (`std_logic` etc),
case-statement `when` indenting, library and use-clause sorting,
Wadler-style line-width-aware pretty printing. These are on the
roadmap below.

Each release ships value at its current size — no six-month parser
refactor with nothing runnable in the middle.

## Installation

From source (Rust 1.85+):

```sh
cargo install --path .
```

A future release will publish prebuilt binaries on GitHub Releases and
a Mason installer for Neovim.

## Usage

As a stdin filter (the editor-integration mode):

```sh
cat module.vhd | hdlfmtr > module.formatted.vhd
```

On a file:

```sh
hdlfmtr module.vhd
```

Check mode — exits 1 if the input would be reformatted, prints nothing:

```sh
hdlfmtr --check module.vhd
```

### CLI flags

```
--indent-style <STYLE>     tabs (default) | spaces | preserve
--indent-width <N>         spaces per indent level (default 4); with
                           tabs, hint passed through to editor
                           integrations as a tabstop value
--keyword-case <CASE>      lower (default) | upper | preserve
--check                    exit 1 if reformatting would change input
--stdin-filepath <PATH>    path to use for diagnostics when reading
                           from stdin
```

## Editor integration

### Neovim — lazy.nvim (recommended)

`hdlfmtr` ships its own Neovim plugin layer, so you can install it as
a regular lazy.nvim spec and skip the manual cargo path wiring. The
`build` hook compiles the Rust binary at install time and the Lua
module knows where to find it afterwards.

```lua
{
  "henri23/hdlfmtr",
  build = "cargo build --release",
  ft = "vhdl",
  dependencies = { "stevearc/conform.nvim" }, -- optional
  config = function()
    require("hdlfmtr").setup({
      -- Formatter options (forwarded to the binary as CLI flags).
      indent_style = "tabs",   -- "tabs" | "spaces" | "preserve"
      indent_width = 4,        -- spaces per level, or tabstop hint
      keyword_case = "lower",  -- "lower" | "upper" | "preserve"

      -- Editor integration.
      register_with_conform = true,  -- register the formatter def
      auto_register_vhdl    = false, -- also add to formatters_by_ft.vhdl
      set_buffer_options    = true,  -- match nvim tabstop/shiftwidth
                                     -- to indent_width on vhdl buffers
    })
  end,
}
```

**Prerequisite:** a Rust toolchain (`cargo` on PATH). Prebuilt binaries
are on the roadmap; until then `cargo build --release` runs once at
install time. After install you also get:

- `:HdlFmt` — format the current buffer.
- `require("hdlfmtr").format_buffer(bufnr)` — programmatic API.
- `require("hdlfmtr").binary()` — absolute path to the bundled binary,
  in case you need it for tooling.

### Setup options

| Option | Default | Description |
| --- | --- | --- |
| `indent_style` | `"tabs"` | `"tabs"`, `"spaces"`, or `"preserve"` to skip the indent pass entirely. |
| `indent_width` | `4` | When `indent_style = "spaces"`, the number of spaces per level. When `"tabs"`, the tabstop / shiftwidth applied to vhdl buffers (cosmetic). |
| `keyword_case` | `"lower"` | `"lower"`, `"upper"`, or `"preserve"`. Identifiers are never touched. |
| `register_with_conform` | `true` | Register `hdlfmtr` as a conform.nvim formatter definition (no effect if conform is not loaded). |
| `auto_register_vhdl` | `false` | Also prepend `hdlfmtr` to `conform.formatters_by_ft.vhdl`. Set this if you want a true zero-config install. |
| `set_buffer_options` | `true` | On entering a vhdl buffer, set `tabstop`, `softtabstop`, `shiftwidth`, `expandtab` to match the formatter config. Disable if you manage these yourself. |

### Neovim — manual conform.nvim wiring

If you don't want the plugin layer and prefer to install `hdlfmtr` as
a system binary (`cargo install --path .`), point conform.nvim at it
directly:

```lua
require("conform").setup({
  formatters_by_ft = {
    vhdl = { "hdlfmtr" },
  },
  format_on_save = {
    timeout_ms = 5000,
    lsp_format = "fallback",
  },
})

require("conform").formatters.hdlfmtr = {
  command = "hdlfmtr",
  args = { "--stdin-filepath", "$FILENAME" },
  stdin = true,
}
```

### Composing with vsg

While hdlfmtr's pretty-printer is still being built, you may want to
run both: hdlfmtr first to fix the labels vsg refuses to fix, then vsg
for its full set of style rewrites. With conform.nvim:

```lua
formatters_by_ft = {
  vhdl = { "hdlfmtr", "vsg" },
}
```

This pipes the file through hdlfmtr first, then vsg. Because hdlfmtr's
output satisfies vsg's `process_016` and `process_018`, vsg no longer
bails out and writes its formatted result back as expected.

## Why a new formatter

VHDL tooling is sparse. The de-facto VHDL formatter is `vsg`
(VHDL Style Guide), which is Python-based, has a slow cold-start, and
refuses to write output when it sees rules it cannot auto-fix — even
if those rules are trivially fixable in principle. The result is that
editor format-on-save flows time out or silently drop fixes.

hdlfmtr exists to provide:

1. **Editor-grade startup time.** A Rust binary starts in
   single-digit milliseconds. There is no Python import cost, no node
   process, no JIT warmup. Format-on-save will not time out.
2. **A robust write contract.** hdlfmtr always produces output. If a
   pass cannot improve the source, the source is returned unchanged.
   It does not "succeed" by writing nothing.
3. **A single static binary.** Drop it in `~/.local/bin`, distribute
   it via Mason, brew, AUR, or vendor it into a CI image. No runtime
   to install.

## Roadmap

The plan is to grow each pass on the same token stream, so that the
public `format` API stays stable as the implementation deepens.

- [x] **Lexer.** Complete VHDL-2008 token stream with case-insensitive
      keyword recognition, comment and string handling, character
      literal vs. attribute-tick disambiguation, bit-string and based
      literals.
- [x] **Process-label insertion** (`process_016`, `process_018`).
- [x] **Indent normalization.** Tab-default, spaces-configurable.
      Stack-based depth tracking across structural blocks and
      multi-line parens.
- [x] **Keyword case folding.** Lower / upper / preserve. Identifiers
      untouched.
- [x] **Trailing-whitespace removal & final-newline.**
- [ ] **Case-statement `when` indenting.** Today `when` lines render at
      the case body depth without an extra level for branch bodies.
- [ ] **Predefined identifier folding** (`std_logic`, `bit_vector`,
      etc) so a mixed-case file ends up consistently lowercased.
- [ ] **Port and signal-list alignment.** The single biggest source of
      churn in hand-edited VHDL.
- [ ] **Comment reflow** for line and block comments.
- [ ] **Library and use-clause sorting.**
- [ ] **Full parser + Wadler-style pretty-printer.** When the rewrite
      passes have outgrown what a token-stream rewriter can do
      reliably, switch the engine.
- [ ] **Configuration file** (`hdlfmtr.toml`).
- [ ] **Range formatting** for editor "format selection" commands.

## License

Licensed under either of

- Apache License, Version 2.0 (LICENSE-APACHE)
- MIT license (LICENSE-MIT)

at your option.
