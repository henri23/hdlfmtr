//! hdlfmtr CLI — a fast VHDL formatter.
//!
//! Compatible with editor integrations that follow the prettier-style
//! contract: read source from stdin (or a file), write formatted source
//! to stdout, exit non-zero on error. `--check` exits 1 without writing
//! when the input would be reformatted.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

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

    /// Path used for diagnostics when reading from stdin. Accepted for
    /// compatibility with conform.nvim and prettier-style integrations;
    /// currently informational only.
    #[arg(long, value_name = "PATH")]
    stdin_filepath: Option<PathBuf>,

    /// Check whether the input is already formatted. Exits 1 if it
    /// would be changed. Does not write any output.
    #[arg(long)]
    check: bool,
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

    let formatted = hdlfmtr::format(&source).context("formatting source")?;

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
