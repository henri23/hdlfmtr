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
    // Pass 3: indent.
    let after_indent = normalize_indent(&after_case, cfg);
    // Pass 4: trim trailing whitespace.
    let after_trim = trim_trailing_whitespace(&after_indent);
    // Pass 5: ensure exactly one trailing newline.
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

    for (i, tok) in tokens.iter().enumerate() {
        if tok.kind != TokenKind::Keyword(Keyword::Process) {
            continue;
        }
        let prev = previous_significant(tokens, i);

        // The `process` token inside `end process` is the closing
        // marker for an existing process, not the start of a new one.
        // Skip it — its head label (if needed) is handled when we
        // visit the corresponding opening `process`.
        if let Some(j) = prev
            && tokens[j].kind == TokenKind::Keyword(Keyword::End)
        {
            continue;
        }

        // A process is already labeled iff its previous significant
        // token is `:`. We treat the optional `postponed` modifier as
        // transparent: if present, the previous significant token is
        // the `postponed` reserved word and we still (correctly)
        // treat the process as unlabeled.
        let already_labeled = prev
            .map(|j| tokens[j].kind == TokenKind::Colon)
            .unwrap_or(false);
        if already_labeled {
            continue;
        }

        let name = format!("u_process_{}", next_id);
        next_id += 1;

        // Head: insert `<name> : ` immediately before the `process`
        // keyword. Using the keyword's start preserves any leading
        // whitespace exactly.
        edits.push(Edit {
            at: tok.start,
            text: format!("{} : ", name),
        });

        // Tail: walk forward to `end process`, and insert the same
        // name after the `process` keyword unless there's already a
        // label there.
        if let Some(end_label_at) = find_end_process_insertion_point(tokens, i) {
            edits.push(Edit {
                at: end_label_at,
                text: format!(" {}", name),
            });
        }
    }

    apply_edits(src, edits)
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

/// Rewrite the bytes of every Keyword(_) and Reserved token to match
/// `cfg.keyword_case`. Identifiers are never touched.
fn fold_keyword_case(src: &str, cfg: &Config) -> String {
    if cfg.keyword_case == KeywordCase::Preserve {
        return src.to_string();
    }
    let tokens = tokenize(src);
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0;
    for tok in &tokens {
        if matches!(tok.kind, TokenKind::Keyword(_) | TokenKind::Reserved) {
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

/// Given a `process` keyword at `proc_idx`, return the byte offset at
/// which to insert the closing label, or `None` if a label is already
/// present (or if the matching `end process` cannot be found).
fn find_end_process_insertion_point(tokens: &[Token], proc_idx: usize) -> Option<usize> {
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
                    // `end process some_label ;` — already labeled.
                    TokenKind::Ident => None,
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
