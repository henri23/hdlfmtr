//! hdlfmtr — a fast VHDL formatter.
//!
//! Today this crate exposes a single best-effort `format` function that
//! runs targeted rewrite passes on a VHDL source string. Future versions
//! will grow into a full pretty-printer; the public API is intentionally
//! minimal so that the underlying implementation can be replaced
//! without breaking callers.

pub mod format;
pub mod lexer;

pub use format::{format, FormatError};
