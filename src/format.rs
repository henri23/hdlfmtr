//! Formatter pipeline.
//!
//! `format` runs each pass in order against the source, re-tokenizing
//! between passes that change byte content. Today the pipeline is:
//!
//!   1. process-label insertion (vsg `process_016` / `process_018`)
//!   2. keyword case folding
//!   3. indent normalization
//!   4. trailing-whitespace removal
//!   5. final-newline ensuring
//!
//! Each pass is independent and consumes the source as a string. The
//! re-tokenize cost is negligible at the file sizes we care about and
//! it keeps the architecture trivially understandable; we can swap to
//! a single shared token stream when profiling tells us to.

use crate::align::align_declarations;
use crate::config::{Config, KeywordCase};
use crate::indent::normalize_indent;
use crate::lexer::{Keyword, Token, TokenKind, tokenize};

/// Format a VHDL source string with the given config.
pub fn format(src: &str, cfg: &Config) -> Result<String, FormatError> {
    // Pass 1: insert labels.
    let after_labels = {
        let tokens = tokenize(src);
        insert_process_labels(src, &tokens)
    };
    // Pass 2: keyword case.
    let after_case = fold_keyword_case(&after_labels, cfg);
    // Pass 3: numeric literal cleanup (operates on token text).
    let after_numeric = format_numeric_literals(&after_case);
    // Pass 4: operator spacing normalization.
    let after_ops = normalize_operator_spacing(&after_numeric);
    // Pass 5: sort consecutive library and use clauses.
    let after_sort = if cfg.sort_use_clauses {
        let s = sort_library_clauses(&after_ops);
        sort_use_clauses(&s)
    } else {
        after_ops
    };
    // Pass 6: indent.
    let after_indent = normalize_indent(&after_sort, cfg);
    // Pass 7: vertical alignment of declaration lines.
    let after_align = if cfg.align_declarations {
        align_declarations(&after_indent)
    } else {
        after_indent
    };
    // Pass 8: insert stylish blank lines between top-level statements.
    let after_stylish = if cfg.stylish_blank_lines {
        insert_stylish_blank_lines(&after_align)
    } else {
        after_align
    };
    // Pass 9: collapse runs of 3+ blank lines.
    let after_collapse = collapse_blank_lines(&after_stylish);
    // Pass 10: trim trailing whitespace.
    let after_trim = trim_trailing_whitespace(&after_collapse);
    // Pass 11: ensure exactly one trailing newline.
    Ok(ensure_final_newline(&after_trim))
}

/// Errors returned from `format`. Currently uninhabited — every input
/// is best-effort and hdlfmtr never refuses to produce output. The type
/// exists so that fallible passes can be added without a public API
/// break.
#[derive(Debug)]
pub enum FormatError {}

impl std::fmt::Display for FormatError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for FormatError {}

/// Insert synthetic labels (`u_process_N : process ... end process u_process_N`)
/// for each process statement that lacks one.
///
/// This addresses vsg rules `process_016` ("Add label for process
/// statement") and `process_018` ("Add a label for the end process"),
/// both of which vsg reports as errors but does not auto-fix.
///
/// VHDL forbids nested `process` statements inside a process body, so
/// matching `end process` is a forward search for the literal token
/// pair `end ... process`. No depth tracking is required.
fn insert_process_labels(src: &str, tokens: &[Token]) -> String {
    let mut edits: Vec<Edit> = Vec::new();
    let mut next_id: usize = 1;
    // Build a set of identifiers already in the source so synthetic
    // labels can avoid colliding with user code.
    let existing_idents = collect_identifiers(tokens, src);

    for (i, tok) in tokens.iter().enumerate() {
        if tok.kind != TokenKind::Keyword(Keyword::Process) {
            continue;
        }

        // The `process` token inside `end process` is the closing
        // marker for an existing process, not the start of a new one.
        // Skip it — its head label (if needed) is handled when we
        // visit the corresponding opening `process`.
        let prev = previous_significant(tokens, i);
        if let Some(j) = prev
            && tokens[j].kind == TokenKind::Keyword(Keyword::End)
        {
            continue;
        }

        // Detect an existing label by walking back over the optional
        // `postponed` reserved word (`postponed_001` style) and then
        // checking for a `:` separator.
        let mut walk = prev;
        if let Some(j) = walk
            && tokens[j].kind == TokenKind::Reserved
            && tokens[j].text(src).eq_ignore_ascii_case("postponed")
        {
            walk = previous_significant(tokens, j);
        }
        let existing_label_text = walk
            .filter(|&j| tokens[j].kind == TokenKind::Colon)
            .and_then(|j| previous_significant(tokens, j))
            .filter(|&j| tokens[j].kind == TokenKind::Ident)
            .map(|j| tokens[j].text(src).to_string());

        // The label we will use on the *closing* `end process`. If
        // the user already labeled the process, use that label;
        // otherwise generate a fresh one.
        let label_for_end: Option<String> = if let Some(existing) = existing_label_text {
            // Process is already labeled — only the tail might need
            // patching (vsg `process_018` in isolation).
            Some(existing)
        } else {
            // Process is unlabeled — generate a unique name and
            // schedule both the head and tail edit.
            let name = next_unique_label(&mut next_id, &existing_idents);
            edits.push(Edit {
                at: tok.start,
                text: format!("{} : ", name),
            });
            Some(name)
        };

        if let Some(label) = label_for_end
            && let Some(end_label_at) = find_end_process_insertion_point(tokens, i, src, &label)
        {
            edits.push(Edit {
                at: end_label_at,
                text: format!(" {}", label),
            });
        }
    }

    apply_edits(src, edits)
}

/// Collect every identifier in the source so synthetic process labels
/// can avoid colliding with user-defined names.
fn collect_identifiers(tokens: &[Token], src: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for tok in tokens {
        if tok.kind == TokenKind::Ident {
            set.insert(tok.text(src).to_ascii_lowercase());
        }
    }
    set
}

/// Generate the next `u_process_N` name that does not collide with
/// any identifier currently in the source. Bumps `next_id` past the
/// last used value.
fn next_unique_label(
    next_id: &mut usize,
    existing: &std::collections::HashSet<String>,
) -> String {
    loop {
        let candidate = format!("u_process_{}", *next_id);
        *next_id += 1;
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
}

fn apply_edits(src: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| e.at);
    let mut out = String::with_capacity(src.len() + edits.len() * 16);
    let mut cursor = 0;
    for edit in edits {
        out.push_str(&src[cursor..edit.at]);
        out.push_str(&edit.text);
        cursor = edit.at;
    }
    out.push_str(&src[cursor..]);
    out
}

struct Edit {
    at: usize,
    text: String,
}

// ───────────────────────── case folding ─────────────────────────

/// Identifiers from VHDL's predefined packages and standard library
/// names. Folding their case alongside reserved words gives a
/// consistent look without forcing the user's own identifiers into a
/// canonical case.
const PREDEFINED_IDENTIFIERS: &[&str] = &[
    // STANDARD package
    "boolean",
    "bit",
    "bit_vector",
    "character",
    "string",
    "integer",
    "natural",
    "positive",
    "real",
    "time",
    "severity_level",
    "delay_length",
    "file_open_kind",
    "file_open_status",
    "line",
    "text",
    // STD_LOGIC_1164 package
    "std_logic",
    "std_logic_vector",
    "std_ulogic",
    "std_ulogic_vector",
    "x01",
    "x01z",
    "ux01",
    "ux01z",
    // NUMERIC_STD package
    "signed",
    "unsigned",
    // Common library names
    "ieee",
    "std",
    "work",
];

fn is_predefined_identifier(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PREDEFINED_IDENTIFIERS.contains(&lower.as_str())
}

/// Rewrite the bytes of every Keyword(_) and Reserved token — and
/// every identifier in `PREDEFINED_IDENTIFIERS` — to match
/// `cfg.keyword_case`. User-defined identifiers are never touched.
fn fold_keyword_case(src: &str, cfg: &Config) -> String {
    if cfg.keyword_case == KeywordCase::Preserve {
        return src.to_string();
    }
    let tokens = tokenize(src);
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0;
    for tok in &tokens {
        let foldable = match tok.kind {
            TokenKind::Keyword(_) | TokenKind::Reserved => true,
            TokenKind::Ident => is_predefined_identifier(tok.text(src)),
            _ => false,
        };
        if foldable {
            out.push_str(&src[cursor..tok.start]);
            let original = tok.text(src);
            match cfg.keyword_case {
                KeywordCase::Lower => {
                    for b in original.bytes() {
                        out.push(b.to_ascii_lowercase() as char);
                    }
                }
                KeywordCase::Upper => {
                    for b in original.bytes() {
                        out.push(b.to_ascii_uppercase() as char);
                    }
                }
                KeywordCase::Preserve => unreachable!(),
            }
            cursor = tok.end;
        }
    }
    out.push_str(&src[cursor..]);
    out
}

// ───────────────────────── use clause sorting ─────────────────────────

/// Sort consecutive `use X.Y.Z;` clauses alphabetically.
///
/// A "run" of use clauses is a maximal sequence of lines, each of
/// which is a `use ... ;` statement, with no blank lines, comments,
/// or other statements between them. Library clauses are *not*
/// reordered — they have semantic precedence over the `use` clauses
/// they enable.
fn sort_use_clauses(src: &str) -> String {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_use_clause(line) {
            let mut j = i + 1;
            while j < lines.len() && is_use_clause(lines[j]) {
                j += 1;
            }
            if j - i >= 2 {
                let mut group: Vec<&str> = lines[i..j].to_vec();
                // Sort by the trimmed lowercase content so case
                // doesn't influence ordering.
                group.sort_by_key(|s| s.trim().to_ascii_lowercase());
                for s in &group {
                    out.push_str(s);
                }
                i = j;
                continue;
            }
        }
        out.push_str(line);
        i += 1;
    }
    out
}

fn is_use_clause(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.len() < 4 {
        return false;
    }
    trimmed.as_bytes()[..4]
        .eq_ignore_ascii_case(b"use ")
}

fn is_library_clause(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.len() < 8 {
        return false;
    }
    trimmed.as_bytes()[..8]
        .eq_ignore_ascii_case(b"library ")
}

/// Sort consecutive `library X;` clauses alphabetically. Same shape
/// as `sort_use_clauses` but for the other top-of-file directive.
fn sort_library_clauses(src: &str) -> String {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < lines.len() {
        if is_library_clause(lines[i]) {
            let mut j = i + 1;
            while j < lines.len() && is_library_clause(lines[j]) {
                j += 1;
            }
            if j - i >= 2 {
                let mut group: Vec<&str> = lines[i..j].to_vec();
                group.sort_by_key(|s| s.trim().to_ascii_lowercase());
                for s in &group {
                    out.push_str(s);
                }
                i = j;
                continue;
            }
        }
        out.push_str(lines[i]);
        i += 1;
    }
    out
}

// ───────────────────────── operator spacing ─────────────────────────

/// Normalize the spacing around binary operators so each operator
/// has exactly one space on either side. Preserves all other source
/// content (including indent, content of strings/comments, and the
/// internal layout of expressions).
///
/// Operators handled: `<=`, `:=`, `=>`, `=`, `/=`, `<`, `>`, `<=`,
/// `>=`, `**`. Unary `-` and `+` are intentionally left alone — we
/// can't reliably tell unary from binary without a parser.
fn normalize_operator_spacing(src: &str) -> String {
    let tokens = tokenize(src);
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        let is_op = matches!(
            tok.kind,
            TokenKind::Assign
                | TokenKind::Arrow
                | TokenKind::LE
                | TokenKind::GE
                | TokenKind::NEq
                | TokenKind::Power
        );
        if !is_op {
            continue;
        }
        // Don't normalize on a line that begins (after indent) with
        // the operator — this can happen with multi-line continuations
        // we don't yet support; leave them alone.
        if i == 0 {
            continue;
        }

        // Look at the previous and next non-trivia tokens. If either
        // is missing, skip.
        let prev = previous_significant_idx(&tokens, i);
        let next = next_significant_idx(&tokens, i);
        let (Some(prev), Some(next)) = (prev, next) else {
            continue;
        };

        // Only normalize if the previous and next tokens are on the
        // same line as the operator. Cross-line wrapping can have
        // intentional layout we shouldn't disturb.
        let same_line_left = !any_newline_between(&tokens, prev, i);
        let same_line_right = !any_newline_between(&tokens, i, next);
        if !(same_line_left && same_line_right) {
            continue;
        }

        // Emit src up to the end of the previous significant token,
        // then ` <op> `, then advance the cursor to the start of the
        // next significant token. This collapses any whitespace runs
        // around the operator into single spaces.
        out.push_str(&src[cursor..tokens[prev].end]);
        out.push(' ');
        out.push_str(tok.text(src));
        out.push(' ');
        cursor = tokens[next].start;
    }
    out.push_str(&src[cursor..]);
    out
}

fn previous_significant_idx(tokens: &[Token], i: usize) -> Option<usize> {
    (0..i).rev().find(|&j| !tokens[j].is_trivia())
}

fn next_significant_idx(tokens: &[Token], i: usize) -> Option<usize> {
    ((i + 1)..tokens.len()).find(|&j| !tokens[j].is_trivia())
}

fn any_newline_between(tokens: &[Token], a: usize, b: usize) -> bool {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    tokens[lo + 1..hi]
        .iter()
        .any(|t| t.kind == TokenKind::Newline)
}

// ───────────────────────── stylish blank lines ─────────────────────────

/// Insert one blank line between consecutive process / labeled
/// statements inside an architecture body, if they're not already
/// separated by one. Conservative pass: never inserts more than one
/// blank, never touches anything that already has spacing.
fn insert_stylish_blank_lines(src: &str) -> String {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut out = String::with_capacity(src.len() + 64);
    let mut i = 0;
    while i < lines.len() {
        out.push_str(lines[i]);
        if line_ends_top_level_statement(lines[i]) {
            // Look ahead to the next non-blank, non-comment line.
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim();
                if t.is_empty() {
                    break; // already separated by a blank
                }
                if t.starts_with("--") {
                    j += 1;
                    continue; // comments stay attached to the next stmt
                }
                break;
            }
            if j < lines.len() && !lines[j].trim().is_empty() {
                let next = lines[j];
                if line_starts_top_level_statement(next) {
                    out.push('\n');
                }
            }
        }
        i += 1;
    }
    out
}

/// True iff a line is the end of a top-level statement we want to
/// separate from the next: `end process [label];`,
/// `end generate [label];`, `end block [label];`, or a single-line
/// concurrent assignment ending in `;`.
fn line_ends_top_level_statement(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("end process")
        || lower.starts_with("end generate")
        || lower.starts_with("end block")
    {
        return true;
    }
    // We deliberately don't fire on `end if` / `end loop` / `end case`
    // because those are *inside* statements, not between them.
    false
}

/// True iff a line begins a new top-level statement that should be
/// preceded by a blank line: a `process` declaration, a labeled
/// statement (`label : process` / `label : <component>`), or a
/// labeled generate.
fn line_starts_top_level_statement(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("process") || lower.starts_with("postponed process") {
        return true;
    }
    // `<label> : <something>` where <something> is process / postponed /
    // a component name (we can't tell components from other types
    // without a symbol table, but we can use the heuristic that any
    // labeled statement at the architecture top level is "blockish"
    // enough to deserve separation).
    if let Some(colon) = trimmed.find(':') {
        let after = trimmed[colon + 1..].trim_start().to_ascii_lowercase();
        if !after.is_empty() && !after.starts_with('=') {
            return true;
        }
    }
    false
}

// ───────────────────────── blank-line collapse ─────────────────────────

/// Collapse runs of three or more consecutive blank lines into one.
/// Single and double blank lines are preserved.
fn collapse_blank_lines(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut blank_run: usize = 0;
    for line in src.split_inclusive('\n') {
        let is_blank = line.trim().is_empty();
        if is_blank {
            blank_run += 1;
            if blank_run <= 1 {
                out.push_str(line);
            }
        } else {
            blank_run = 0;
            out.push_str(line);
        }
    }
    out
}

// ───────────────────────── numeric literal formatting ─────────────────────────

/// Two related normalizations on numeric literals:
///
///   • Insert `_` group separators in long decimal integers
///     (`1000000` -> `1_000_000`).
///   • Normalize the case of hex bit-string literals (`x"ff"` ->
///     `x"FF"` by default).
///
/// Floating-point and based literals are intentionally left alone -
/// they're already varied enough that re-grouping them is risky.
fn format_numeric_literals(src: &str) -> String {
    let tokens = tokenize(src);
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for tok in &tokens {
        match tok.kind {
            TokenKind::Number => {
                let text = tok.text(src);
                // Skip based literals (anything containing #) and
                // anything with a decimal point or exponent — leave
                // those alone for now.
                if text.contains(['#', '.', 'e', 'E']) {
                    continue;
                }
                // Plain integer: re-group with underscores every 3
                // digits from the right, but only if it's at least
                // 5 digits long.
                let digits: String = text.chars().filter(|c| c != &'_').collect();
                if digits.len() < 5 {
                    continue;
                }
                let regrouped = group_with_underscores(&digits);
                if regrouped == text {
                    continue;
                }
                out.push_str(&src[cursor..tok.start]);
                out.push_str(&regrouped);
                cursor = tok.end;
            }
            TokenKind::BitString => {
                // Normalize hex case to uppercase.
                let text = tok.text(src);
                // The prefix is one or two letters then `"`.
                let prefix_end = text.find('"').unwrap_or(0);
                let prefix = &text[..prefix_end];
                let prefix_upper = prefix.to_ascii_uppercase();
                // Only touch hex prefixes (`X`, `UX`, `SX`).
                if !matches!(prefix_upper.as_str(), "X" | "UX" | "SX") {
                    continue;
                }
                let body_end = text.rfind('"').unwrap_or(text.len());
                let body = &text[prefix_end + 1..body_end];
                let body_upper = body.to_ascii_uppercase();
                let normalized = format!("{}\"{}\"", prefix, body_upper);
                if normalized == text {
                    continue;
                }
                out.push_str(&src[cursor..tok.start]);
                out.push_str(&normalized);
                cursor = tok.end;
            }
            _ => {}
        }
    }
    out.push_str(&src[cursor..]);
    out
}

fn group_with_underscores(digits: &str) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::with_capacity(chars.len() + chars.len() / 3);
    let len = chars.len();
    for (i, c) in chars.iter().enumerate() {
        let from_end = len - i;
        if i > 0 && from_end % 3 == 0 {
            out.push('_');
        }
        out.push(*c);
    }
    out
}

// ───────────────────── whitespace housekeeping ─────────────────────

/// Strip trailing horizontal whitespace from every line. Newlines and
/// blank lines are preserved.
fn trim_trailing_whitespace(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        // Separate the line body from its trailing newline (if any).
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let trimmed = body.trim_end_matches([' ', '\t']);
        out.push_str(trimmed);
        out.push_str(nl);
    }
    out
}

/// Ensure the source ends with exactly one newline. Empty input stays
/// empty (no spurious newline added).
fn ensure_final_newline(src: &str) -> String {
    if src.is_empty() {
        return String::new();
    }
    let trimmed = src.trim_end_matches('\n');
    let mut out = String::with_capacity(trimmed.len() + 1);
    out.push_str(trimmed);
    out.push('\n');
    out
}

// ───────────────────────── helpers ─────────────────────────

/// Index of the previous non-trivia token before `i`, if any.
fn previous_significant(tokens: &[Token], i: usize) -> Option<usize> {
    (0..i).rev().find(|&j| !tokens[j].is_trivia())
}

/// Index of the next non-trivia token after `i`, if any.
fn next_significant(tokens: &[Token], i: usize) -> Option<usize> {
    ((i + 1)..tokens.len()).find(|&j| !tokens[j].is_trivia())
}

/// Given a `process` keyword at `proc_idx` and the label that the
/// process *should* carry on its closing `end process`, return the
/// byte offset at which to insert that label — or `None` if no
/// insertion is needed (the existing label already matches, or the
/// matching `end process` cannot be found).
fn find_end_process_insertion_point(
    tokens: &[Token],
    proc_idx: usize,
    src: &str,
    expected_label: &str,
) -> Option<usize> {
    let mut i = proc_idx + 1;
    while i < tokens.len() {
        if tokens[i].kind == TokenKind::Keyword(Keyword::End) {
            let after_end = next_significant(tokens, i)?;
            if tokens[after_end].kind == TokenKind::Keyword(Keyword::Process) {
                let after_process = next_significant(tokens, after_end)?;
                return match tokens[after_process].kind {
                    // `end process ;` — insert the label between
                    // `process` and `;`.
                    TokenKind::Semi => Some(tokens[after_end].end),
                    TokenKind::Ident => {
                        let existing = tokens[after_process].text(src);
                        if existing.eq_ignore_ascii_case(expected_label) {
                            // Label already present and matches — nothing to do.
                            None
                        } else {
                            // Label is wrong (e.g. someone renamed the
                            // process). The safe thing is to leave it
                            // alone — overwriting could destroy info.
                            None
                        }
                    }
                    // Anything else is malformed; bail safely.
                    _ => None,
                };
            }
            // `end if`, `end loop`, `end case`, ... — keep scanning.
            i = after_end;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{IndentStyle, KeywordCase};

    /// Config that disables every pass except label insertion. Used by
    /// the existing label-pass tests so changes to indent / case
    /// passes can never break them.
    fn labels_only() -> Config {
        Config {
            indent_style: IndentStyle::Preserve,
            indent_width: 4,
            keyword_case: KeywordCase::Preserve,
            align_declarations: false,
            case_branch_indent: false,
            sort_use_clauses: false,
            stylish_blank_lines: false,
        }
    }

    #[test]
    fn unlabeled_process_gets_label() {
        let src = "\
architecture rtl of t is
begin
  process(clk)
  begin
    if rising_edge(clk) then
    end if;
  end process;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert!(out.contains("u_process_1 : process"), "head label missing:\n{out}");
        assert!(
            out.contains("end process u_process_1;"),
            "tail label missing:\n{out}"
        );
    }

    #[test]
    fn labeled_process_is_left_alone() {
        let src = "\
architecture rtl of t is
begin
  my_proc : process(clk)
  begin
  end process my_proc;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn multiple_unlabeled_processes_get_unique_labels() {
        let src = "\
architecture rtl of t is
begin
  process(a)
  begin
  end process;
  process(b)
  begin
  end process;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert!(out.contains("u_process_1 : process"));
        assert!(out.contains("u_process_2 : process"));
        assert!(out.contains("end process u_process_1;"));
        assert!(out.contains("end process u_process_2;"));
    }

    #[test]
    fn case_insensitive_keywords() {
        let src = "\
architecture rtl of t is
begin
  PROCESS(clk)
  BEGIN
  END PROCESS;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert!(out.contains("u_process_1 : PROCESS"));
        assert!(out.contains("END PROCESS u_process_1;"));
    }

    #[test]
    fn comment_containing_process_keyword_is_ignored() {
        let src = "\
architecture rtl of t is
begin
  -- this process is unlabeled, but the word here doesn't count
  process(clk)
  begin
  end process;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        // Two occurrences total: one at head, one at tail. Three would
        // mean we mistakenly labeled the comment.
        assert_eq!(out.matches("u_process_1").count(), 2, "got:\n{out}");
    }

    #[test]
    fn string_containing_process_keyword_is_ignored() {
        let src = "\
architecture rtl of t is
begin
  process(clk)
  begin
    report \"the process did not start\";
  end process;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert_eq!(out.matches("u_process_1").count(), 2, "got:\n{out}");
    }

    #[test]
    fn nested_if_inside_process_does_not_confuse_end_matching() {
        let src = "\
architecture rtl of t is
begin
  process(clk)
  begin
    if a then
      if b then
      end if;
    end if;
  end process;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert!(out.contains("end process u_process_1;"));
        // The inner `end if`s must remain untouched.
        assert_eq!(out.matches("end if;").count(), 2);
    }

    #[test]
    fn already_correct_file_round_trips() {
        let src = "\
library IEEE;
use IEEE.STD_LOGIC_1164.ALL;

entity t is
end t;

architecture rtl of t is
begin
  sync : process(clk)
  begin
  end process sync;
end rtl;
";
        let out = format(src, &labels_only()).unwrap();
        assert_eq!(out, src);
    }
}
