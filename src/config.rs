//! Formatter configuration.
//!
//! Construct a `Config` either via `Config::default()` (sensible
//! defaults: tabs, 4 wide, lowercase keywords) or by using the
//! `Config { ... }` literal directly. The CLI parses its flags into a
//! `Config` and the library API takes one by reference.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Tabs,
    Spaces,
    /// Don't touch leading whitespace at all.
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordCase {
    Lower,
    Upper,
    /// Don't touch keyword case at all.
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Indent character: tabs (default), spaces, or preserve.
    pub indent_style: IndentStyle,
    /// Width of one indent level. Used to emit `n` spaces per level
    /// when `indent_style == Spaces`. With `Tabs`, ignored at format
    /// time but still meaningful as a "visual width" hint that the
    /// editor integration may apply to its tabstop / shiftwidth.
    pub indent_width: usize,
    /// Case to apply to reserved words.
    pub keyword_case: KeywordCase,
    /// Whether to vertically align consecutive declaration lines
    /// (port lists, signal/variable/constant declarations) so that
    /// names, `:`, mode keywords, types, and trailing comments line
    /// up in columns. Default `true`.
    pub align_declarations: bool,
    /// Whether `when` branches inside a `case` statement add an
    /// extra indent level to their body. Default `true` (the more
    /// common modern style).
    pub case_branch_indent: bool,
    /// Whether to sort consecutive `use X.Y.Z;` clauses
    /// alphabetically. Default `true`.
    pub sort_use_clauses: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            indent_style: IndentStyle::Tabs,
            indent_width: 4,
            keyword_case: KeywordCase::Lower,
            align_declarations: true,
            case_branch_indent: true,
            sort_use_clauses: true,
        }
    }
}

impl Config {
    /// The string emitted for a single indent level.
    pub fn indent_unit(&self) -> String {
        match self.indent_style {
            IndentStyle::Tabs => "\t".to_string(),
            IndentStyle::Spaces => " ".repeat(self.indent_width),
            IndentStyle::Preserve => String::new(),
        }
    }
}
