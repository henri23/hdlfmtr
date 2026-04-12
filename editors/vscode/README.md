# hdlfmtr for Visual Studio Code

A fast VHDL formatter and linter extension powered by
[hdlfmtr](https://github.com/henri23/hdlfmtr).

## Zero-install

The extension bundles a cross-platform **WebAssembly build** of the
hdlfmtr formatter. No Rust toolchain, no binary download, no PATH
configuration - install the extension and it works immediately on
Linux, macOS, and Windows.

## Features

- **Format on save** - registers as the default VHDL document
  formatter. Enable `"editor.formatOnSave": true` in your settings.
- **Lint diagnostics** - unused signals, missing sensitivity-list
  entries, latch detection, multiple drivers, and more show up in the
  Problems panel with gutter markers and squiggly underlines.
- **17-pass formatting pipeline** - process labels, keyword case,
  indent normalization, vertical alignment, operator spacing, comma
  spacing, use-clause sorting, blank-line management, and more.
- **Configurable** - all hdlfmtr options are exposed as VS Code
  settings under the `hdlfmtr.*` prefix.

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `hdlfmtr.indentStyle` | `"tabs"` | `"tabs"`, `"spaces"`, or `"preserve"` |
| `hdlfmtr.indentWidth` | `4` | Spaces per indent level or tabstop hint |
| `hdlfmtr.keywordCase` | `"lower"` | `"lower"`, `"upper"`, or `"preserve"` |
| `hdlfmtr.alignDeclarations` | `true` | Align port / signal / variable / port-map / aggregate lists |
| `hdlfmtr.caseBranchIndent` | `true` | Extra indent for case `when` bodies |
| `hdlfmtr.sortUseClauses` | `true` | Sort consecutive `use` clauses, group by library |
| `hdlfmtr.lint` | `true` | Show lint diagnostics in Problems panel |

## Lint checks

| Code | Description |
| --- | --- |
| `unused_001` | Signal declared but never read or written |
| `unused_002` | Signal written but never read |
| `unused_003` | Input port never read |
| `unused_004` | Output port never assigned |
| `sens_001` | Signal read in combinational process not in sensitivity list |
| `clock_001` | `rising_edge(X)` but X not in sensitivity list |
| `driver_001` | Signal driven by multiple processes |
| `latch_001` | Signal may infer a latch (no default assignment) |
| `reset_001` | Inconsistent reset polarity |

## Contributing

See the [main repository](https://github.com/henri23/hdlfmtr) for
build instructions and contribution guidelines.
