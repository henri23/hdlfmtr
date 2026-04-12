# Changelog

All notable changes to `hdlfmtr` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-04-12

### Added
- **VS Code extension** with bundled WASM binary. No Rust toolchain
  required for VS Code users - the extension works cross-platform out
  of the box. Available on the Visual Studio Marketplace.
- **Vertical alignment of declaration lines.** Consecutive port,
  generic, signal, variable, constant, and record-field declarations
  aligned in columns. Also handles component port maps (`name => signal`),
  multi-line aggregates, and case `when` branches.
- **Predefined identifier folding.** `std_logic`, `std_logic_vector`,
  `signed`, `unsigned`, `integer`, `ieee`, and friends are folded
  alongside reserved words.
- **Use-clause sorting** with blank-line grouping between libraries.
- **Library clause sorting.**
- **Case-statement `when` indenting** (configurable).
- **Operator spacing** for `:`, `:=`, `<=`, `=>`, `/=`, `>=`, `**`.
- **Colon spacing** for label separators.
- **Comma spacing** normalization.
- **Keyword-paren spacing** - `process(clk)` -> `process (clk)`,
  same for `port`, `generic`, `port map`, `generic map`.
- **Paren padding removal** - strips whitespace inside `(...)`.
- **Numeric literal formatting** - underscore grouping, hex uppercase.
- **Concatenation continuation alignment** for `& foo` lines.
- **Conditional assignment alignment** for `when ... else` chains.
- **Single-line collapse** for trivially-empty constructs (opt-in).
- **Stylish blank line insertion** between top-level statements.
- **Blank-line collapse** (3+ -> 1).
- **9 lint checks** with Neovim `vim.diagnostic` and VS Code Problems
  panel integration: `unused_001`-`004`, `sens_001`, `clock_001`,
  `driver_001`, `latch_001`, `reset_001`.
- **Process label collision detection** - synthetic names skip
  existing identifiers.
- **`postponed process` recognition.**
- **`process_018` in isolation** - labeled process with unlabeled
  `end process` gets the label added.
- CLI flags: `--no-align`, `--no-case-branch-indent`, `--no-sort-uses`,
  `--no-stylish-blanks`, `--no-lint`, `--collapse-trivial`.
- Neovim plugin options for all new features.
- GitHub issue templates (bug report, feature request).

### Changed
- `format(src)` is now `format(src, &Config)`.
- Bumped version to 0.2.0.

## [0.1.0] - 2026-04-11

### Added
- Initial release.
- VHDL-2008 lexer with case-insensitive keyword recognition,
  string/comment/character-literal handling, and attribute-tick
  disambiguation.
- Process-label insertion (vsg `process_016` / `process_018`).
- Keyword case folding with lower / upper / preserve.
- Indent normalization with stack-based depth tracking.
- Trailing-whitespace removal and final-newline normalization.
- CLI: `hdlfmtr [FILE]`, `--check`, `--stdin-filepath`,
  `--indent-style`, `--indent-width`, `--keyword-case`.
- Neovim plugin (`lua/hdlfmtr/`) with conform.nvim auto-registration,
  buffer-option autocommand, `:HdlFmt` user command.
