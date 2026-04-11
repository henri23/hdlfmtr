# Changelog

All notable changes to `hdlfmtr` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Vertical alignment of declaration lines.** Consecutive port,
  generic, signal, variable, constant, and record-field declarations
  with the same indent and shape (mode-bearing or not) now have their
  names, `:` separators, mode keywords, types, and trailing comments
  aligned in columns. Disable with `--no-align` (CLI) or
  `align_declarations = false` (lazy.nvim setup).
- **Predefined identifier folding.** The case-folding pass now also
  folds the case of `std_logic`, `std_logic_vector`, `signed`,
  `unsigned`, `integer`, `natural`, `positive`, `real`, `time`,
  `boolean`, `bit`, `bit_vector`, `character`, `string`, `ieee`,
  `std`, and `work`. User-defined identifiers are still untouched.
- **Use-clause sorting.** Consecutive `use X.Y.Z;` lines are sorted
  alphabetically. Disable with `--no-sort-uses` or
  `sort_use_clauses = false`. Library clauses are not reordered.
- **Case-statement `when` indenting.** `when` branches inside a
  `case` statement now indent their bodies one extra level. Disable
  with `--no-case-branch-indent` or `case_branch_indent = false`.
- **Process label collision detection.** Synthetic `u_process_N`
  labels skip any name already used as an identifier elsewhere in
  the source.
- **`postponed process` recognition.** A labeled `postponed process`
  is correctly detected as already-labeled.
- **`process_018` in isolation.** A labeled process whose
  `end process` doesn't repeat the label still gets the label
  inserted on the closing line.

### Changed
- `format(src)` is now `format(src, &Config)`. The configuration
  struct gained `align_declarations`, `case_branch_indent`, and
  `sort_use_clauses` fields.
- The CLI gained `--no-align`, `--no-case-branch-indent`, and
  `--no-sort-uses` flags.
- The lazy.nvim plugin's `setup()` accepts the corresponding options
  and forwards them as CLI flags.

## [0.1.0] — 2026-04-11

### Added
- Initial release.
- VHDL-2008 lexer with case-insensitive keyword recognition,
  string/comment/character-literal handling, and attribute-tick
  disambiguation.
- Process-label insertion (vsg `process_016` / `process_018`).
- Keyword case folding with lower / upper / preserve.
- Indent normalization with stack-based depth tracking, supporting
  entity / architecture / package / process / function / procedure /
  if-then-elsif-else / for-loop / while-loop / case / record /
  block / generate constructs and multi-line parentheses.
- Trailing-whitespace removal and final-newline normalization.
- CLI: `hdlfmtr [FILE]`, `--check`, `--stdin-filepath`,
  `--indent-style`, `--indent-width`, `--keyword-case`.
- Neovim plugin (`lua/hdlfmtr/`) with conform.nvim auto-registration,
  buffer-option autocommand, `:HdlFmt` user command, and a
  programmatic `format_buffer` API.
