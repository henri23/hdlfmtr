//! Vertical alignment of consecutive declaration lines.
//!
//! Walks the source line-by-line, classifies each line as either an
//! "alignable" declaration (port, generic, signal, variable, constant,
//! record field) or not, then groups consecutive alignable lines that
//! share the same indent and the same shape (mode-bearing or not),
//! and rewrites the inter-token whitespace within each group so the
//! `name`, `:`, mode keyword, type, and trailing comment columns line
//! up vertically.
//!
//! The pass is a no-op for groups of size 1 — there's nothing to
//! align against. It runs *after* indent normalization, so it can
//! safely use the indent string as a grouping key.
//!
//! Groups are broken by:
//!   • blank lines,
//!   • non-alignable lines (statements, comments, `end` lines, parens),
//!   • a change of indent,
//!   • a change of shape (a mode-bearing port immediately followed by
//!     a mode-less signal does NOT get aligned together).

use crate::lexer::{Keyword, Token, TokenKind, tokenize};

pub fn align_declarations(src: &str) -> String {
    let tokens = tokenize(src);
    let lines = group_into_lines(&tokens);
    let parsed: Vec<Option<ParsedLine>> = lines
        .iter()
        .map(|line| parse_line(line, src))
        .collect();

    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some(start) = parsed[i].as_ref() {
            // Try to extend a group from i forward.
            let mut j = i + 1;
            while j < lines.len() {
                if let Some(p) = parsed[j].as_ref() {
                    if start.groups_with(p) {
                        j += 1;
                        continue;
                    }
                }
                break;
            }
            if j - i >= 2 {
                let group: Vec<&ParsedLine> =
                    (i..j).map(|k| parsed[k].as_ref().unwrap()).collect();
                let aligned = emit_aligned(&group);
                let trailing_newline_at_end = lines[j - 1]
                    .last()
                    .map(|t| t.kind == TokenKind::Newline)
                    .unwrap_or(false);
                for (k, line_text) in aligned.iter().enumerate() {
                    out.push_str(line_text);
                    // Preserve the trailing newline that the original
                    // line had (every line except possibly the very
                    // last in the file).
                    if k + 1 < aligned.len() || trailing_newline_at_end {
                        out.push('\n');
                    }
                }
                i = j;
                continue;
            }
        }
        // Single line: emit verbatim from the original tokens.
        for tok in lines[i] {
            out.push_str(tok.text(src));
        }
        i += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    /// `name : [mode] type` — port / signal / variable / constant declaration.
    Colon,
    /// `name => value` — component port map / generic map association.
    Arrow,
}

#[derive(Debug)]
struct ParsedLine {
    indent: String,
    name: String,
    separator: Sep,
    mode: Option<String>,
    type_part: String, // type+terminator for Colon, value+terminator for Arrow
    comment: Option<String>, // `-- ...` text, no leading whitespace
}

impl ParsedLine {
    fn has_mode(&self) -> bool {
        self.mode.is_some()
    }

    /// Two lines belong to the same alignment group iff they share
    /// indent, separator, and shape (mode-bearing or not).
    fn groups_with(&self, other: &Self) -> bool {
        self.indent == other.indent
            && self.separator == other.separator
            && self.has_mode() == other.has_mode()
    }
}

const MODE_KEYWORDS: &[&str] = &["in", "out", "inout", "buffer", "linkage", "view"];

fn parse_line(line: &[Token], src: &str) -> Option<ParsedLine> {
    // Skip leading whitespace.
    let mut idx = 0;
    while idx < line.len() && line[idx].kind == TokenKind::Whitespace {
        idx += 1;
    }
    let indent_end = idx;
    let indent: String = line[..indent_end]
        .iter()
        .map(|t| t.text(src))
        .collect();

    // Find the first `:` or `=>` not inside parens. Whichever comes
    // first determines the line shape (declaration vs port-map). If
    // we hit a newline first, it's not an alignable line.
    let mut paren_depth = 0i32;
    let mut sep_idx: Option<(usize, Sep)> = None;
    let mut k = idx;
    while k < line.len() {
        match line[k].kind {
            TokenKind::Newline => break,
            TokenKind::LParen => paren_depth += 1,
            TokenKind::RParen => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return None;
                }
            }
            TokenKind::Colon if paren_depth == 0 => {
                sep_idx = Some((k, Sep::Colon));
                break;
            }
            TokenKind::Arrow if paren_depth == 0 => {
                sep_idx = Some((k, Sep::Arrow));
                break;
            }
            // Lines that contain `:=` before any `:` aren't declarations.
            TokenKind::Assign if paren_depth == 0 => return None,
            _ => {}
        }
        k += 1;
    }
    let (sep_pos, separator) = sep_idx?;

    // Name = trimmed text between indent_end and sep_pos.
    let name: String = line[indent_end..sep_pos]
        .iter()
        .map(|t| t.text(src))
        .collect::<String>()
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    // Reject lines whose "name" looks like a label (label : process,
    // label : block, label : if-generate, etc.) - those are statements,
    // not declarations. Only relevant for Colon-separator lines; arrow
    // lines can never be statement labels.
    if separator == Sep::Colon
        && let Some(after) = next_significant_token(line, sep_pos + 1)
    {
        let lower = after.text(src).to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "process"
                | "block"
                | "for"
                | "if"
                | "while"
                | "case"
                | "loop"
                | "postponed"
        ) {
            return None;
        }
    }

    // After the separator, find the mode (only valid for Colon).
    let mut k = sep_pos + 1;
    while k < line.len() && line[k].kind == TokenKind::Whitespace {
        k += 1;
    }
    // Only consider mode when it is not followed by a semicolon (special case for view decls)
    let mode = if separator == Sep::Colon
        && k < line.len()
        && is_mode_keyword(&line[k], src)
        && next_significant_token(line, k + 1).is_none_or(|tok| tok.kind != TokenKind::Semi)
    {
        let m = line[k].text(src).to_string();
        k += 1;
        Some(m)
    } else {
        None
    };
    while k < line.len() && line[k].kind == TokenKind::Whitespace {
        k += 1;
    }
    let type_start = k;

    // Find a trailing line comment, if any.
    let comment_start: Option<usize> = line[type_start..]
        .iter()
        .position(|t| t.kind == TokenKind::LineComment)
        .map(|p| p + type_start);

    let pre_comment_end = comment_start.unwrap_or(line.len());
    let mut type_end = pre_comment_end;
    while type_end > type_start
        && matches!(
            line[type_end - 1].kind,
            TokenKind::Whitespace | TokenKind::Newline
        )
    {
        type_end -= 1;
    }
    if type_end == type_start {
        return None;
    }
    let type_part: String = line[type_start..type_end]
        .iter()
        .map(|t| t.text(src))
        .collect::<String>()
        .trim_end()
        .to_string();
    if type_part.is_empty() {
        return None;
    }

    let comment = comment_start.map(|c| {
        line[c..]
            .iter()
            .take_while(|t| t.kind != TokenKind::Newline)
            .map(|t| t.text(src))
            .collect::<String>()
            .trim_end()
            .to_string()
    });

    Some(ParsedLine {
        indent,
        name,
        separator,
        mode,
        type_part,
        comment,
    })
}

fn is_mode_keyword(tok: &Token, src: &str) -> bool {
    if !matches!(tok.kind, TokenKind::Reserved | TokenKind::Keyword(Keyword::View) | TokenKind::Ident) {
        return false;
    }
    let lower = tok.text(src).to_ascii_lowercase();
    MODE_KEYWORDS.contains(&lower.as_str())
}

fn next_significant_token<'a>(line: &'a [Token], from: usize) -> Option<&'a Token> {
    for tok in &line[from..] {
        if !tok.is_trivia() {
            return Some(tok);
        }
    }
    None
}

fn emit_aligned(group: &[&ParsedLine]) -> Vec<String> {
    let name_width = group.iter().map(|l| l.name.len()).max().unwrap_or(0);
    let has_mode = group.iter().any(|l| l.mode.is_some());
    let mode_width = group
        .iter()
        .filter_map(|l| l.mode.as_ref().map(|m| m.len()))
        .max()
        .unwrap_or(0);
    let has_comment = group.iter().any(|l| l.comment.is_some());
    let type_width = group.iter().map(|l| l.type_part.len()).max().unwrap_or(0);

    group
        .iter()
        .map(|line| {
            let mut s = String::new();
            s.push_str(&line.indent);
            s.push_str(&line.name);
            for _ in line.name.len()..name_width {
                s.push(' ');
            }
            // Separator: ` : ` for declarations, ` => ` for port maps
            // and case branches.
            match line.separator {
                Sep::Colon => s.push_str(" : "),
                Sep::Arrow => s.push_str(" => "),
            }
            if has_mode {
                let m = line.mode.as_deref().unwrap_or("");
                s.push_str(m);
                for _ in m.len()..mode_width {
                    s.push(' ');
                }
                s.push(' ');
            }
            s.push_str(&line.type_part);
            if has_comment && line.comment.is_some() {
                for _ in line.type_part.len()..type_width {
                    s.push(' ');
                }
                s.push_str("  ");
                s.push_str(line.comment.as_deref().unwrap());
            }
            s
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_a_simple_port_list() {
        let src = "\
entity t is
\tport (
\t\ttest_in : in std_logic;
\t\tclk : in std_logic;
\t\ttest_out : out std_logic
\t);
end t;
";
        let out = align_declarations(src);
        let expected = "\
entity t is
\tport (
\t\ttest_in  : in  std_logic;
\t\tclk      : in  std_logic;
\t\ttest_out : out std_logic
\t);
end t;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_with_inout_widening_the_mode_column() {
        let src = "\
\ttest_in : in std_logic;
\tclk : in std_logic;
\tdata : inout std_logic_vector(7 downto 0);
\ttest_out : out std_logic
";
        let out = align_declarations(src);
        let expected = "\
\ttest_in  : in    std_logic;
\tclk      : in    std_logic;
\tdata     : inout std_logic_vector(7 downto 0);
\ttest_out : out   std_logic
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_signal_declarations_without_modes() {
        let src = "\
\tsignal a : std_logic;
\tsignal bb : std_logic_vector(7 downto 0);
\tsignal ccc : integer;
";
        let out = align_declarations(src);
        let expected = "\
\tsignal a   : std_logic;
\tsignal bb  : std_logic_vector(7 downto 0);
\tsignal ccc : integer;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn does_not_mix_modes_and_no_modes_in_one_group() {
        let src = "\
\ta : in std_logic;
\tb : std_logic
";
        let out = align_declarations(src);
        // The two lines have different shapes (one with mode, one
        // without) so they're each their own group of size 1, which
        // means no alignment, which means the original whitespace is
        // preserved verbatim.
        assert_eq!(out, src);
    }

    #[test]
    fn does_not_align_across_blank_line() {
        let src = "\
\ta : in std_logic;
\tbb : in std_logic;

\tc : in std_logic;
\tdddd : in std_logic;
";
        let out = align_declarations(src);
        let expected = "\
\ta  : in std_logic;
\tbb : in std_logic;

\tc    : in std_logic;
\tdddd : in std_logic;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_trailing_comments() {
        let src = "\
\tclk : in std_logic; -- system clock
\tdata : in std_logic_vector(7 downto 0); -- input bus
\tena : in std_logic; -- enable
";
        let out = align_declarations(src);
        let expected = "\
\tclk  : in std_logic;                     -- system clock
\tdata : in std_logic_vector(7 downto 0);  -- input bus
\tena  : in std_logic;                     -- enable
";
        assert_eq!(out, expected);
    }

    #[test]
    fn does_not_touch_a_lone_declaration() {
        let src = "\tonly : in std_logic;\n";
        let out = align_declarations(src);
        assert_eq!(out, src);
    }

    #[test]
    fn label_assignment_is_not_a_declaration() {
        // `proc1 : process(clk)` is a labeled statement, NOT a port
        // declaration. The alignment pass must reject it.
        let src = "\tproc1 : process(clk)\n";
        let out = align_declarations(src);
        assert_eq!(out, src);
    }

    #[test]
    fn aligns_a_port_map() {
        let src = "\
\tu1 : my_block
\t\tport map (
\t\t\tclk => sys_clk,
\t\t\trst_n => sys_rst_n,
\t\t\tdata_in => bus_word
\t\t);
";
        let out = align_declarations(src);
        let expected = "\
\tu1 : my_block
\t\tport map (
\t\t\tclk     => sys_clk,
\t\t\trst_n   => sys_rst_n,
\t\t\tdata_in => bus_word
\t\t);
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_case_when_branches() {
        let src = "\
\twhen '0' => y <= '1';
\twhen '1' => y <= '0';
\twhen others => y <= 'Z';
";
        let out = align_declarations(src);
        let expected = "\
\twhen '0'    => y <= '1';
\twhen '1'    => y <= '0';
\twhen others => y <= 'Z';
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_a_generic_list() {
        let src = "\
\tgeneric (
\t\tWIDTH : integer := 8;
\t\tDEPTH : integer := 256;
\t\tINIT_VALUE : std_logic_vector := (others => '0')
\t);
";
        let out = align_declarations(src);
        let expected = "\
\tgeneric (
\t\tWIDTH      : integer := 8;
\t\tDEPTH      : integer := 256;
\t\tINIT_VALUE : std_logic_vector := (others => '0')
\t);
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_record_fields() {
        let src = "\
\ttype packet_t is record
\t\thdr : std_logic_vector(7 downto 0);
\t\tpayload : std_logic_vector(63 downto 0);
\t\tcrc : std_logic_vector(15 downto 0);
\tend record;
";
        let out = align_declarations(src);
        let expected = "\
\ttype packet_t is record
\t\thdr     : std_logic_vector(7 downto 0);
\t\tpayload : std_logic_vector(63 downto 0);
\t\tcrc     : std_logic_vector(15 downto 0);
\tend record;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_multi_line_aggregate() {
        let src = "\
\tconstant init : data_t := (
\t\tfield_a => '0',
\t\tfield_bb => '1',
\t\tfield_ccc => 'Z'
\t);
";
        let out = align_declarations(src);
        let expected = "\
\tconstant init : data_t := (
\t\tfield_a   => '0',
\t\tfield_bb  => '1',
\t\tfield_ccc => 'Z'
\t);
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_function_parameter_list() {
        let src = "\
\tfunction make_packet (
\t\thdr : std_logic_vector;
\t\tpayload : std_logic_vector;
\t\tcrc : std_logic_vector
\t) return packet_t is
";
        let out = align_declarations(src);
        let expected = "\
\tfunction make_packet (
\t\thdr     : std_logic_vector;
\t\tpayload : std_logic_vector;
\t\tcrc     : std_logic_vector
\t) return packet_t is
";
        assert_eq!(out, expected);
    }

    #[test]
    fn declarations_and_port_maps_do_not_share_a_group() {
        let src = "\
\ta : in std_logic;
\tb => sys_b
";
        let out = align_declarations(src);
        // Different separators -> two groups of size 1 -> no alignment.
        assert_eq!(out, src);
    }

    #[test]
    fn multiple_groups_in_a_row_via_indent_change() {
        let src = "\
\ta : in std_logic;
\tb : in std_logic;
\t\tx : in std_logic;
\t\tyy : in std_logic;
";
        let out = align_declarations(src);
        // Two groups: depth 1 (a/b) and depth 2 (x/yy).
        let expected = "\
\ta : in std_logic;
\tb : in std_logic;
\t\tx  : in std_logic;
\t\tyy : in std_logic;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn view_is_a_mode() {
        let src = "\
\ti : in std_logic;
\to : out std_logic;
\tio : inout std_logic;
\tslave : view slave;
\tmaster : view master;
";
        let out = align_declarations(src);
        // Views align with other modes in the same group
        let expected = "\
\ti      : in    std_logic;
\to      : out   std_logic;
\tio     : inout std_logic;
\tslave  : view  slave;
\tmaster : view  master;
";
        assert_eq!(out, expected);
    }

    #[test]
    fn aligns_view_declaration() {
        let src = "\
\tview v of r is
\t\tinput : in;
\t\toutput : out;
\t\tio : inout;
\tend view;
";
        let out = align_declarations(src);
        // Modes followed by semicolon do not trigger semicolon alignment
        let expected = "\
\tview v of r is
\t\tinput  : in;
\t\toutput : out;
\t\tio     : inout;
\tend view;
";
        assert_eq!(out, expected);
    }
}
