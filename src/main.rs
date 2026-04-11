//! hdlfmtr CLI — a fast VHDL formatter.
//!
//! Compatible with editor integrations that follow the prettier-style
//! contract: read source from stdin (or a file), write formatted
//! source to stdout, exit non-zero on error. `--check` exits 1 without
//! writing when the input would be reformatted.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use hdlfmtr::{Config, IndentStyle, KeywordCase};

#[derive(Parser, Debug)]
#[command(
    name = "hdlfmtr",
    version,
    about = "A fast VHDL formatter",
    long_about = None,
)]
struct Cli {
    /// File to format. If omitted, hdlfmtr reads from stdin.
    file: Option<PathBuf>,

    /// Path used for diagnostics when reading from stdin. Accepted
    /// for compatibility with conform.nvim and prettier-style
    /// integrations; currently informational only.
    #[arg(long, value_name = "PATH")]
    stdin_filepath: Option<PathBuf>,

    /// Indentation style.
    #[arg(long, value_enum, default_value_t = IndentStyleArg::Tabs)]
    indent_style: IndentStyleArg,

    /// Width of one indent level. Used to emit `n` spaces per level
    /// when --indent-style=spaces. With tabs, the value is still
    /// passed through to editor integrations as a tabstop hint.
    #[arg(long, default_value_t = 4)]
    indent_width: usize,

    /// Case to apply to reserved words.
    #[arg(long, value_enum, default_value_t = KeywordCaseArg::Lower)]
    keyword_case: KeywordCaseArg,

    /// Disable vertical alignment of consecutive declaration lines
    /// (port lists, signal/variable/constant declarations).
    #[arg(long)]
    no_align: bool,

    /// Disable extra indenting of `when` branch bodies inside case
    /// statements.
    #[arg(long)]
    no_case_branch_indent: bool,

    /// Disable alphabetical sorting of consecutive `use X.Y.Z;`
    /// clauses.
    #[arg(long)]
    no_sort_uses: bool,

    /// Disable lint checks. Lints emit JSONL diagnostics to stderr
    /// (one per line) for editor integrations to consume.
    #[arg(long)]
    no_lint: bool,

    /// Disable insertion of one blank line between consecutive
    /// top-level statements inside an architecture body.
    #[arg(long)]
    no_stylish_blanks: bool,

    /// Collapse trivially-empty entity / package / architecture
    /// constructs onto a single line. Off by default.
    #[arg(long)]
    collapse_trivial: bool,

    /// Check whether the input is already formatted. Exits 1 if it
    /// would be changed. Does not write any output.
    #[arg(long)]
    check: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lower")]
enum IndentStyleArg {
    Tabs,
    Spaces,
    Preserve,
}

impl From<IndentStyleArg> for IndentStyle {
    fn from(v: IndentStyleArg) -> Self {
        match v {
            IndentStyleArg::Tabs => IndentStyle::Tabs,
            IndentStyleArg::Spaces => IndentStyle::Spaces,
            IndentStyleArg::Preserve => IndentStyle::Preserve,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lower")]
enum KeywordCaseArg {
    Lower,
    Upper,
    Preserve,
}

impl From<KeywordCaseArg> for KeywordCase {
    fn from(v: KeywordCaseArg) -> Self {
        match v {
            KeywordCaseArg::Lower => KeywordCase::Lower,
            KeywordCaseArg::Upper => KeywordCase::Upper,
            KeywordCaseArg::Preserve => KeywordCase::Preserve,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("hdlfmtr: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    let cfg = Config {
        indent_style: cli.indent_style.into(),
        indent_width: cli.indent_width,
        keyword_case: cli.keyword_case.into(),
        align_declarations: !cli.no_align,
        case_branch_indent: !cli.no_case_branch_indent,
        sort_use_clauses: !cli.no_sort_uses,
        stylish_blank_lines: !cli.no_stylish_blanks,
        collapse_trivial: cli.collapse_trivial,
    };

    let source = match &cli.file {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?,
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("reading stdin")?;
            buf
        }
    };

    let formatted = hdlfmtr::format(&source, &cfg).context("formatting source")?;

    // Lint runs on the formatted output so positions reported in
    // diagnostics line up with what the user sees on disk after the
    // formatter writes.
    if !cli.no_lint {
        let diagnostics = hdlfmtr::lint(&formatted, &cfg);
        if !diagnostics.is_empty() {
            let stderr = io::stderr();
            let mut err = stderr.lock();
            for d in &diagnostics {
                writeln!(err, "{}", d.to_jsonl()).context("writing diagnostics")?;
            }
        }
    }

    if cli.check {
        return Ok(if formatted == source {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        });
    }

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    handle
        .write_all(formatted.as_bytes())
        .context("writing stdout")?;
    Ok(ExitCode::SUCCESS)
}
