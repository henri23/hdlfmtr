# hdlfmtr

A fast VHDL formatter for editors and CI.

`hdlfmtr` is a Rust-native command-line tool that reformats VHDL source.
It is designed to start in single-digit milliseconds, distribute as a
single static binary, and integrate cleanly with editor plugins that
follow the prettier-style stdin-to-stdout contract (conform.nvim,
null-ls, efm-langserver, ALE, VS Code formatters).

## Status — honest scope

`hdlfmtr` is brand new. Today it does **one** thing well, and complements
existing VHDL tooling rather than replacing it:

- **Inserts placeholder labels on unlabeled `process` statements.**
  This is the case that VSG's `process_016` and `process_018` rules
  flag as errors but refuse to auto-fix, which causes editor
  format-on-save flows to silently drop the formatter's output. hdlfmtr
  handles it correctly: it finds each unlabeled process, generates a
  fresh `u_process_N` name, and inserts it on both the head and the
  matching `end process`.

Everything else in the file is preserved byte-for-byte.

The roadmap below lists the passes that come next. The intent is for
hdlfmtr to grow into a full pretty-printer, but each release should
deliver real value at its current size — no six-month parser refactor
with nothing runnable in the middle.

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
      -- Set true to also prepend hdlfmtr to conform's
      -- formatters_by_ft.vhdl, so format-on-save Just Works without
      -- further config. Defaults to false because chaining with vsg
      -- (see below) usually wants explicit ordering.
      auto_register_vhdl = false,
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
- [ ] **Indent normalization.** Two-space indentation as the default,
      configurable.
- [ ] **Keyword case folding.** Honor a project-wide convention
      (lower / upper / preserve).
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
