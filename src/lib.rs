//! hdlfmtr — a fast VHDL formatter.
//!
//! The crate exposes a `format` function that runs a fixed pipeline
//! of token-stream rewrite passes (label insertion, keyword case
//! folding, indent normalization, whitespace housekeeping). All passes
//! are driven from a single `Config`. The public API is intentionally
//! minimal so the underlying implementation can be replaced without
//! breaking callers.

pub mod config;
pub mod format;
pub mod indent;
pub mod lexer;

pub use config::{Config, IndentStyle, KeywordCase};
pub use format::{FormatError, format};
