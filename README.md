<p align="center">
  <img src="images/logo.png" alt="hdlfmtr logo" width="200">
</p>

# hdlfmtr

A fast VHDL formatter and linter for editors and CI.

`hdlfmtr` is a Rust-native command-line tool that reformats VHDL source
and catches common hardware-design bugs. It starts in single-digit
milliseconds, distributes as a single static binary (or a bundled WASM
module for VS Code), and integrates with Neovim (lazy.nvim + conform.nvim)
and Visual Studio Code out of the box.

## Status

The formatter pipeline runs the following passes in order:

1. **Process-label insertion** - finds unlabeled `process` statements
   and adds a synthetic `u_process_N` to both the head and the matching
   `end process`. Handles `postponed process`, `process_018` in
   isolation, and collision-safe synthetic names.
2. **Keyword case folding** - reserved words and common predefined
   identifiers (`std_logic`, `bit_vector`, `ieee`, ...) become
   lowercase by default; configurable to upper or preserve.
3. **Numeric literal cleanup** - underscore grouping for long integers
   (`1000000` -> `1_000_000`), uppercase hex in bit-string literals.
4. **Operator spacing** - one space around `:`, `:=`, `<=`, `=>`,
   `/=`, `>=`, `**`.
5. **Comma spacing** - no space before `,`, one space after.
6. **Keyword-paren spacing** - `process(clk)` -> `process (clk)`,
   same for `port`, `generic`, `port map`, `generic map`.
7. **Paren padding removal** - strips whitespace immediately inside
   `(...)` on single-line parens.
8. **Library and use-clause sorting** - alphabetical, with blank-line
   grouping between different libraries.
9. **Indent normalization** - tabs by default, spaces configurable.
   Stack-based depth tracking across all structural VHDL blocks and
   multi-line parentheses, including case-statement `when` branches.
10. **Vertical alignment** - port / signal / variable / constant /
    generic / record-field declarations, component port maps
    (`name => signal`), aggregates, and case `when` branches.
11. **Concatenation continuation alignment** - `& foo` lines aligned.
12. **Conditional assignment alignment** - `when ... else` chains
    aligned vertically.
13. **Trivial construct collapse** (off by default) - empty
    `entity X is end X;` collapsed to one line.
14. **Stylish blank lines** - one blank line between consecutive
    top-level statements inside architecture bodies.
15. **Blank-line collapse** - 3+ consecutive blank lines reduced to 1.
16. **Trailing-whitespace removal.**
17. **Final-newline ensuring.**

**Lint checks** (emitted as JSONL diagnostics to stderr, integrated
with Neovim's `vim.diagnostic` and VS Code's Problems panel):

- `unused_001` / `unused_002` - signal never read or written
- `unused_003` / `unused_004` - input port never read / output never assigned
- `sens_001` - signal read in combinational process not in sensitivity list
- `clock_001` - `rising_edge(X)` but X not in sensitivity list
- `driver_001` - signal driven by multiple processes
- `latch_001` - signal may infer a latch (no default assignment)
- `reset_001` - inconsistent reset polarity

## Installation

### From source (Rust 1.85+)

```sh
cargo install --path .
```

### Neovim (lazy.nvim)

```lua
{
  "henri23/hdlfmtr",
  build = "cargo build --release",
  ft = "vhdl",
  dependencies = { "stevearc/conform.nvim" }, -- optional
  config = function()
    require("hdlfmtr").setup()
  end,
}
```

Requires a Rust toolchain (`cargo` on PATH). The build hook runs once
at install time.

### VS Code

Install from the
[Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=HenriHaka.hdlfmtr)
or download the `.vsix` from
[GitHub Releases](https://github.com/henri23/hdlfmtr/releases).

The VS Code extension bundles a cross-platform WASM build of hdlfmtr -
**no Rust toolchain or binary install required**. It just works.

## Usage

As a stdin filter (the editor-integration mode):

```sh
cat module.vhd | hdlfmtr > module.formatted.vhd
```

On a file:

```sh
hdlfmtr module.vhd
```

Check mode - exits 1 if the input would be reformatted, prints nothing:

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
--no-align                 disable vertical alignment of declarations
--no-case-branch-indent    disable extra indent for case `when` bodies
--no-sort-uses             disable sorting of consecutive `use` clauses
--no-stylish-blanks        disable blank-line insertion between processes
--no-lint                  disable lint diagnostics on stderr
--collapse-trivial         collapse empty entity/package/architecture
--check                    exit 1 if reformatting would change input
--stdin-filepath <PATH>    path for diagnostics when reading from stdin
```

## Editor integration

### Neovim - lazy.nvim (recommended)

`hdlfmtr` ships its own Neovim plugin layer. The `build` hook compiles
the Rust binary at install time and the Lua module knows where to find
it afterwards.

```lua
{
  "henri23/hdlfmtr",
  build = "cargo build --release",
  ft = "vhdl",
  dependencies = { "stevearc/conform.nvim" }, -- optional
  config = function()
    require("hdlfmtr").setup({
      -- Formatter options (forwarded to the binary as CLI flags).
      indent_style       = "tabs",  -- "tabs" | "spaces" | "preserve"
      indent_width       = 4,       -- spaces per level, or tabstop hint
      keyword_case       = "lower", -- "lower" | "upper" | "preserve"
      align_declarations = true,    -- vertically align port/signal lists
      case_branch_indent = true,    -- indent case `when` branch bodies
      sort_use_clauses   = true,    -- sort consecutive `use` clauses
      stylish_blank_lines = true,   -- blank lines between processes
      lint               = true,    -- emit diagnostics to vim.diagnostic

      -- Editor integration.
      register_with_conform = true,  -- register the formatter def
      auto_register_vhdl    = false, -- also add to formatters_by_ft.vhdl
      set_buffer_options    = true,  -- match nvim tabstop/shiftwidth
    })
  end,
}
```

After install you get:

- `:HdlFmt` - format the current buffer
- `require("hdlfmtr").format_buffer(bufnr)` - programmatic API
- `require("hdlfmtr").binary()` - absolute path to the bundled binary
- Lint diagnostics in the gutter / virtual text / `]d` / `[d` navigation

### Setup options

| Option | Default | Description |
| --- | --- | --- |
| `indent_style` | `"tabs"` | `"tabs"`, `"spaces"`, or `"preserve"` |
| `indent_width` | `4` | Spaces per level, or tabstop hint for tabs |
| `keyword_case` | `"lower"` | `"lower"`, `"upper"`, or `"preserve"`. Predefined identifiers (`std_logic`, `ieee`, ...) are also folded. |
| `align_declarations` | `true` | Align port / signal / variable / constant / generic / record-field / port-map / aggregate lines |
| `case_branch_indent` | `true` | Extra indent for case `when` branch bodies |
| `sort_use_clauses` | `true` | Sort consecutive `use` clauses, group by library |
| `stylish_blank_lines` | `true` | Insert blank line between top-level statements |
| `lint` | `true` | Run lint checks and push diagnostics to `vim.diagnostic` |
| `register_with_conform` | `true` | Register `hdlfmtr` as a conform.nvim formatter definition |
| `auto_register_vhdl` | `false` | Also prepend to `conform.formatters_by_ft.vhdl` |
| `set_buffer_options` | `true` | Set buffer-local `tabstop` / `shiftwidth` / `expandtab` on vhdl files |

### VS Code

The VS Code extension bundles a WASM build of the formatter - no
external binary required. Install from the Marketplace or a `.vsix`
file, and it works immediately.

Settings are exposed under the `hdlfmtr.*` prefix in VS Code settings.
See `editors/vscode/README.md` for details.

### Neovim - manual conform.nvim wiring

If you prefer to install `hdlfmtr` as a system binary and wire it
manually:

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

## Why a new formatter

VHDL tooling is sparse. The de-facto VHDL formatter (`vsg` / VHDL
Style Guide) is Python-based, has a slow cold-start, and refuses to
write output when it sees rules it cannot auto-fix. The result is that
editor format-on-save flows time out or silently drop fixes.

hdlfmtr exists to provide:

1. **Editor-grade startup time.** ~2 ms per invocation. No Python, no
   Node, no JIT warmup.
2. **A robust write contract.** Always produces output. Never
   "succeeds" by writing nothing.
3. **Cross-platform distribution.** Single static binary for CLI
   users, WASM bundle for VS Code, cargo build for Neovim.
4. **Integrated linting.** Catches real hardware bugs (latches,
   sensitivity-list mismatches, multiple drivers, unused signals)
   alongside formatting, surfaced in-editor as diagnostics.

## Roadmap

- [x] Lexer (VHDL-2008 token stream)
- [x] Process-label insertion (with collision detection, postponed, process_018)
- [x] Indent normalization (tabs/spaces, case-when branches)
- [x] Keyword + predefined identifier case folding
- [x] Operator / colon / comma / keyword-paren spacing
- [x] Paren padding removal
- [x] Vertical alignment (ports, signals, generics, records, port maps, aggregates, when branches)
- [x] Concatenation and conditional-assignment alignment
- [x] Library + use-clause sorting with grouping
- [x] Numeric literal formatting
- [x] Stylish blank lines + blank-line collapse
- [x] Trailing-whitespace removal + final-newline
- [x] 9 lint checks with Neovim + VS Code diagnostic integration
- [x] VS Code extension with bundled WASM (no external binary needed)
- [ ] Per-line lint suppression (`-- hdlfmtr: ignore <code>`)
- [ ] Continuation indentation for multi-line expressions
- [ ] Comment reflow
- [ ] Configuration file (`hdlfmtr.toml`)
- [ ] Range formatting
- [ ] Prebuilt release binaries via GitHub Releases
- [ ] Mason package recipe

## License

Licensed under either of

- Apache License, Version 2.0 (LICENSE-APACHE)
- MIT license (LICENSE-MIT)

at your option.
