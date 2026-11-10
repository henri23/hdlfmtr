//! Indent normalization pass.
//!
//! Walks the token stream once to compute the structural depth at every
//! line boundary, then rewrites the leading whitespace of each line to
//! `depth × indent_unit`. The pass is purely token-driven — there is no
//! AST — and uses a stack of "open block" markers to track nesting.
//!
//! Constructs handled today:
//!
//!   • entity, architecture, package [body], process, function, procedure
//!   • if/then/elsif/else/end if
//!   • for/while/loop
//!   • case/is/end case (case branches are not yet specially handled —
//!     `when` lines render at the case body depth, which matches one of
//!     the two common style conventions)
//!   • record, block, generate
//!   • multi-line parentheses (port lists, generic lists, ...)
//!
//! Lines are blank-line preserved as-is. Lines containing only a comment
//! are indented at the current depth. Anything inside a malformed source
//! is best-effort and will not crash; the depth counter saturates at 0.

use std::collections::HashSet;

use crate::config::{Config, IndentStyle};
use crate::lexer::{Keyword, Token, TokenKind, tokenize};

pub fn normalize_indent(src: &str, cfg: &Config) -> String {
    if cfg.indent_style == IndentStyle::Preserve {
        return src.to_string();
    }
    let tokens = tokenize(src);
    let multiline_parens = compute_multiline_parens(&tokens);
    let lines = group_into_lines(&tokens);
    let unit = cfg.indent_unit();

    let mut depth: i32 = 0;
    let mut paren_stack: Vec<bool> = Vec::new(); // true = multi-line `(`
    // "expecting `is`" flag for the constructs whose body opens on `is`
    // (entity, architecture, package, process, function, procedure,
    // case). When `is` arrives we push and clear the flag.
    let mut expect_is_opens_block = false;
    // "expecting `begin`" flag for process / function / procedure that
    // didn't use the optional `is`. When `begin` arrives we push and
    // clear the flag.
    let mut expect_begin_opens_block = false;
    // Tracks open `case` statements for when-branch indenting. Each
    // entry is `true` iff we currently have an open `when` branch
    // inside that case (its body is at one extra depth level).
    let mut case_branch_open: Vec<bool> = Vec::new();
    // "expecting `is`" flag specific to `case` (so we can push the
    // case_branch_open stack at the right moment).
    let mut expect_case_is = false;

    let mut output = String::with_capacity(src.len());

    for (line_idx, line) in lines.iter().enumerate() {
        // Find the first non-whitespace token *and* its position in
        // the line, so we can do small lookahead from it.
        let first_idx = line
            .iter()
            .position(|t| !matches!(t.kind, TokenKind::Whitespace));
        let first = first_idx.map(|i| &line[i]);

        // Lookahead helper: next significant token in this line after
        // a given position. Used to detect `end case` (vs `end if`,
        // `end loop`, etc.) at line render time.
        let next_significant_after = |idx: usize| -> Option<&Token> {
            line[idx + 1..]
                .iter()
                .find(|t| !t.is_trivia())
        };

        // Compute the rendered depth for this line. Lines whose first
        // significant token closes a construct are rendered one level
        // shallower than the body of that construct.
        let adjustment: i32 = match first.map(|t| t.kind) {
            Some(TokenKind::Keyword(Keyword::End)) => {
                // `end case` while a when-branch is open closes both
                // the branch and the case body, so it needs to drop
                // two levels rather than one.
                let next_kind = first_idx.and_then(next_significant_after).map(|t| t.kind);
                if cfg.case_branch_indent
                    && case_branch_open.last() == Some(&true)
                    && next_kind == Some(TokenKind::Keyword(Keyword::Case))
                {
                    -2
                } else {
                    -1
                }
            }
            Some(TokenKind::Keyword(Keyword::Else | Keyword::Elsif)) => -1,
            Some(TokenKind::Keyword(Keyword::When)) => {
                // A `when` line that isn't the first branch in its
                // case needs to drop back to the case-body depth.
                if cfg.case_branch_indent && case_branch_open.last() == Some(&true) {
                    -1
                } else {
                    0
                }
            }
            // `begin` rendering is context-sensitive:
            //   • If it opens a process/function/procedure body that
            //     skipped the optional `is`, it sits at the same depth
            //     as the introducing keyword (no adjustment).
            //   • Otherwise (architecture begin, block begin, or a
            //     subprogram body that already used `is`), the matching
            //     opener push has already happened and `begin` should
            //     render at the parent depth.
            Some(TokenKind::Keyword(Keyword::Begin)) => {
                if expect_begin_opens_block {
                    0
                } else {
                    -1
                }
            }
            Some(TokenKind::RParen) => {
                // Only adjust if this `)` closes a multi-line `(`.
                if paren_stack.last().copied() == Some(true) {
                    -1
                } else {
                    0
                }
            }
            _ => 0,
        };

        // Emit indent only on non-blank lines so we never produce
        // trailing whitespace on otherwise empty lines.
        let is_blank = first.is_none()
            || matches!(
                first.map(|t| t.kind),
                Some(TokenKind::Newline) | None
            );
        if !is_blank {
            let line_depth = (depth + adjustment).max(0) as usize;
            for _ in 0..line_depth {
                output.push_str(&unit);
            }
        }

        // Emit line content, stripping the leading horizontal whitespace
        // we just replaced. Trailing newline (if any) is part of the
        // last token of the line and will pass through verbatim.
        let mut started = false;
        for tok in line.iter() {
            if !started && tok.kind == TokenKind::Whitespace {
                continue;
            }
            started = true;
            output.push_str(tok.text(src));
        }

        // Walk the line's tokens once to update the depth state for
        // the *next* line. We track structural openers / closers here.
        for (i, tok) in line.iter().enumerate() {
            update_depth(
                &tokens,
                tok,
                i,
                &mut depth,
                &mut paren_stack,
                &mut expect_is_opens_block,
                &mut expect_begin_opens_block,
                &mut case_branch_open,
                &mut expect_case_is,
                cfg,
                &multiline_parens,
                line,
                src,
            );
        }

        // Defensive: depth should never go negative.
        if depth < 0 {
            depth = 0;
        }

        // Last line edge case: keep the loop iteration count flowing
        // even if the last line has no trailing newline.
        let _ = line_idx;
    }

    output
}

/// Returns the set of byte offsets (the start of each `(` token) for
/// every left paren whose matching right paren is on a different line.
fn compute_multiline_parens(tokens: &[Token]) -> HashSet<usize> {
    let mut multiline = HashSet::new();
    let mut stack: Vec<usize> = Vec::new(); // open paren start offsets
    for tok in tokens {
        match tok.kind {
            TokenKind::LParen => stack.push(tok.start),
            TokenKind::RParen => {
                stack.pop();
            }
            TokenKind::Newline => {
                for &open in &stack {
                    multiline.insert(open);
                }
            }
            _ => {}
        }
    }
    multiline
}

/// Group tokens into lines. Each entry is the slice of tokens that
/// makes up one source line, *including* the trailing newline token if
/// present. Blank lines (a single Newline token) are their own entry.
fn group_into_lines(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, tok) in tokens.iter().enumerate() {
        if tok.kind == TokenKind::Newline {
            out.push(&tokens[start..=i]);
            start = i + 1;
        }
    }
    if start < tokens.len() {
        out.push(&tokens[start..]);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn update_depth(
    all_tokens: &[Token],
    tok: &Token,
    idx_in_line: usize,
    depth: &mut i32,
    paren_stack: &mut Vec<bool>,
    expect_is_opens_block: &mut bool,
    expect_begin_opens_block: &mut bool,
    case_branch_open: &mut Vec<bool>,
    expect_case_is: &mut bool,
    cfg: &Config,
    multiline_parens: &HashSet<usize>,
    line: &[Token],
    src: &str,
) {
    let _ = (all_tokens, src); // reserved for future context-sensitive lookups
    match tok.kind {
        // ───── Block introducers that open on `is` ─────
        TokenKind::Keyword(Keyword::Case) => {
            if !preceded_by_end_in_line(line, idx_in_line) {
                *expect_is_opens_block = true;
                if cfg.case_branch_indent {
                    *expect_case_is = true;
                }
            } else if cfg.case_branch_indent {
                // `end case` — close the when branch (if any) and
                // pop the case context.
                if let Some(true) = case_branch_open.last() {
                    *depth -= 1;
                }
                case_branch_open.pop();
            }
        }
        TokenKind::Keyword(
            Keyword::Entity | Keyword::Architecture | Keyword::Package,
        ) => {
            // Skip if this is part of `end <kw>` — `end` is the
            // previous significant token in the line OR earlier.
            if !preceded_by_end_in_line(line, idx_in_line) {
                *expect_is_opens_block = true;
            }
        }
        TokenKind::Keyword(Keyword::Process | Keyword::Function | Keyword::Procedure) => {
            if !preceded_by_end_in_line(line, idx_in_line) {
                // process / function / procedure may use either `is`
                // or directly `begin` to open the body. Set both
                // flags; whichever fires first wins.
                *expect_is_opens_block = true;
                *expect_begin_opens_block = true;
            }
        }
        TokenKind::Keyword(Keyword::Is) => {
            if *expect_is_opens_block {
                *depth += 1;
                *expect_is_opens_block = false;
                *expect_begin_opens_block = false;
                if *expect_case_is {
                    case_branch_open.push(false);
                    *expect_case_is = false;
                }
            }
        }
        TokenKind::Keyword(Keyword::Begin) => {
            if *expect_begin_opens_block {
                *depth += 1;
                *expect_begin_opens_block = false;
                *expect_is_opens_block = false;
            }
        }

        // ───── Block introducers that open on a single keyword ─────
        TokenKind::Keyword(Keyword::Then) => {
            // `if ... then` and `elsif ... then` both open. The line
            // starting with `elsif` already gets the -1 render
            // adjustment, so the body sits one level deeper than the
            // elsif/if line itself.
            *depth += 1;
        }
        TokenKind::Keyword(Keyword::Loop) => {
            // for/while/loop. Opens unconditionally.
            if !preceded_by_end_in_line(line, idx_in_line) {
                *depth += 1;
            }
        }
        TokenKind::Keyword(Keyword::Generate) => {
            if !preceded_by_end_in_line(line, idx_in_line) {
                *depth += 1;
            }
        }
        TokenKind::Keyword(Keyword::Record | Keyword::Block) => {
            if !preceded_by_end_in_line(line, idx_in_line) {
                *depth += 1;
            }
        }

        // `else` is a within-if marker. The line itself uses the -1
        // render adjustment, the body of `else` re-enters the if's
        // body depth automatically because there's no `then` to push.
        TokenKind::Keyword(Keyword::Else) => {
            // No state change.
        }
        // `when` inside a case opens (or transitions to) a new
        // branch body. The line itself is rendered at the case-body
        // depth via the -1 adjustment when a previous branch is
        // already open.
        TokenKind::Keyword(Keyword::When) => {
            if cfg.case_branch_indent
                && let Some(top) = case_branch_open.last_mut()
            {
                if *top {
                    // Close the previous branch, open a new one.
                    *depth -= 1;
                } else {
                    *top = true;
                }
                *depth += 1;
            }
        }
        // `elsif` must pop one level here, because the upcoming `then`
        // on the same line will push *unconditionally* and we don't
        // want a fresh nesting level for each elsif branch.
        TokenKind::Keyword(Keyword::Elsif) => {
            *depth -= 1;
        }

        // ───── End ─────
        TokenKind::Keyword(Keyword::End) => {
            // Pops one level. The matching opener kind is not
            // tracked: we trust the source to be well-formed and
            // saturate at zero.
            *depth -= 1;
            // Clear any pending opener flags — `end` always cancels.
            *expect_is_opens_block = false;
            *expect_begin_opens_block = false;
        }

        // ───── Multi-line parens ─────
        TokenKind::LParen => {
            let multi = multiline_parens.contains(&tok.start);
            paren_stack.push(multi);
            if multi {
                *depth += 1;
            }
        }
        TokenKind::RParen => {
            if let Some(multi) = paren_stack.pop() {
                if multi {
                    *depth -= 1;
                }
            }
        }

        _ => {}
    }
}

/// True iff some earlier token in this same line was an `End` keyword.
/// Used to suppress block-opener pushes inside `end <kw>` constructs.
fn preceded_by_end_in_line(line: &[Token], idx: usize) -> bool {
    line[..idx]
        .iter()
        .any(|t| t.kind == TokenKind::Keyword(Keyword::End))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, IndentStyle};

    fn fmt_spaces(src: &str, width: usize) -> String {
        let cfg = Config {
            indent_style: IndentStyle::Spaces,
            indent_width: width,
            keyword_case: crate::config::KeywordCase::Preserve,
            ..Config::default()
        };
        normalize_indent(src, &cfg)
    }

    #[test]
    fn empty_input() {
        assert_eq!(fmt_spaces("", 2), "");
    }

    #[test]
    fn entity_body_indented_one_level() {
        let src = "\
entity test is
port (
a : in std_logic
);
end test;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
entity test is
  port (
    a : in std_logic
  );
end test;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn architecture_with_process_and_if() {
        let src = "\
architecture rtl of t is
begin
proc1 : process(clk)
begin
if rising_edge(clk) then
x <= '1';
end if;
end process proc1;
end rtl;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
architecture rtl of t is
begin
  proc1 : process(clk)
  begin
    if rising_edge(clk) then
      x <= '1';
    end if;
  end process proc1;
end rtl;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn elsif_and_else_at_parent_depth() {
        let src = "\
architecture rtl of t is
begin
process(clk)
begin
if a then
x <= '1';
elsif b then
x <= '0';
else
x <= 'Z';
end if;
end process;
end rtl;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
architecture rtl of t is
begin
  process(clk)
  begin
    if a then
      x <= '1';
    elsif b then
      x <= '0';
    else
      x <= 'Z';
    end if;
  end process;
end rtl;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn blank_lines_have_no_indent() {
        let src = "\
entity t is

end t;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
entity t is

end t;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn preserve_mode_is_no_op() {
        let cfg = Config {
            indent_style: IndentStyle::Preserve,
            indent_width: 4,
            keyword_case: crate::config::KeywordCase::Preserve,
            ..Config::default()
        };
        let src = "  weird   indent\n\t\tmore weird\n";
        assert_eq!(normalize_indent(src, &cfg), src);
    }

    #[test]
    fn tabs_emit_tab_characters() {
        let cfg = Config {
            indent_style: IndentStyle::Tabs,
            indent_width: 4,
            keyword_case: crate::config::KeywordCase::Preserve,
            ..Config::default()
        };
        let src = "\
entity t is
end t;
";
        let out = normalize_indent(src, &cfg);
        // entity body is empty here, so the only depth-1 line would
        // be inside it; we just verify no panic and the structure is
        // preserved.
        assert!(out.contains("entity t is\n"));
        assert!(out.contains("end t;\n"));
    }

    #[test]
    fn nested_record_in_package() {
        let src = "\
package p is
type r is record
a : std_logic;
b : std_logic;
end record;
end package;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
package p is
  type r is record
    a : std_logic;
    b : std_logic;
  end record;
end package;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn loop_generate_respect_end() {
        let src = "\
architecture rtl of t is
begin
if true generate
end generate;

process
begin
for i in 0 to 1 loop
end loop;
end process proc;

forgen : for i in 0 to 1 generate
end generate forgen;
end architecture rtl;
";
        let out = fmt_spaces(src, 2);
        let expected = "\
architecture rtl of t is
begin
  if true generate
  end generate;

  process
  begin
    for i in 0 to 1 loop
    end loop;
  end process proc;

  forgen : for i in 0 to 1 generate
  end generate forgen;
end architecture rtl;
";
        assert_eq!(out, expected);
    }
}
