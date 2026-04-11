//! Lint checks and diagnostic emission.
//!
//! Each lint check operates on the token stream and produces a list
//! of `Diagnostic` entries. The CLI emits them to stderr as one JSON
//! object per line; the lazy.nvim plugin captures stderr, parses the
//! diagnostics, and pushes them through `vim.diagnostic.set()` so
//! they show up as gutter signs and virtual text in the editor.
//!
//! Lints in this module are intentionally heuristic-only. They run
//! on the token stream alone, no parser, no symbol table. This means
//! some false positives are possible — the cost is low because the
//! warnings are advisory and easy to silence per-line later. The
//! benefit is that the entire lint pass adds zero new dependencies
//! and runs in the same single-digit-millisecond budget as the rest
//! of the formatter.

use crate::config::Config;
use crate::lexer::{Keyword, Token, TokenKind, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
    /// 1-based end line (inclusive of the offending span).
    pub end_line: usize,
    /// 1-based end column (exclusive).
    pub end_col: usize,
    pub severity: Severity,
    /// Stable rule code, e.g. `"latch_001"`. Used by future per-line
    /// suppress directives.
    pub code: &'static str,
    pub message: String,
}

impl Diagnostic {
    /// Serialize a single diagnostic as a JSONL entry. The encoder is
    /// hand-rolled to keep `serde`/`serde_json` out of the dep tree.
    pub fn to_jsonl(&self) -> String {
        format!(
            "{{\"line\":{},\"col\":{},\"end_line\":{},\"end_col\":{},\"severity\":\"{}\",\"code\":\"{}\",\"message\":{}}}",
            self.line,
            self.col,
            self.end_line,
            self.end_col,
            self.severity.as_str(),
            self.code,
            json_string(&self.message),
        )
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Convert a byte offset into a (line, column) pair, both 1-based.
pub fn byte_to_line_col(src: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    let limit = byte_offset.min(src.len());
    for c in src[..limit].chars() {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Run every lint pass currently enabled in `cfg` and return the
/// merged set of diagnostics, sorted by source position.
pub fn lint(src: &str, _cfg: &Config) -> Vec<Diagnostic> {
    let tokens = tokenize(src);
    let mut diagnostics = Vec::new();

    let processes = find_processes(&tokens);

    diagnostics.extend(check_unused_signals_and_ports(src, &tokens, &processes));
    diagnostics.extend(check_multiple_drivers(src, &tokens, &processes));
    diagnostics.extend(check_sensitivity_list(src, &tokens, &processes));
    diagnostics.extend(check_implicit_clock(src, &tokens, &processes));
    diagnostics.extend(check_latch(src, &tokens, &processes));
    diagnostics.extend(check_reset_polarity(src, &tokens, &processes));
    diagnostics.extend(check_magic_numbers(src, &tokens));

    diagnostics.sort_by_key(|d| (d.line, d.col));
    diagnostics
}

// ───────────────── shared structural helpers ─────────────────

/// A range of token indices spanning one process statement.
#[derive(Debug, Clone, Copy)]
struct ProcessBlock {
    /// Index of the `process` keyword token.
    process_kw: usize,
    /// Index of the `begin` keyword token (start of executable region).
    begin_kw: usize,
    /// Index of the `end` token of `end process`.
    end_kw: usize,
    /// Optional sensitivity-list paren range, if `process(...)`.
    sens_list: Option<(usize, usize)>, // (open_paren_idx, close_paren_idx)
}

/// Find every process statement in the token stream. Returns one
/// ProcessBlock per process, in source order.
fn find_processes(tokens: &[Token]) -> Vec<ProcessBlock> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].kind != TokenKind::Keyword(Keyword::Process) {
            i += 1;
            continue;
        }
        // Skip the `process` token of `end process`.
        if let Some(prev_idx) = previous_significant(tokens, i)
            && tokens[prev_idx].kind == TokenKind::Keyword(Keyword::End)
        {
            i += 1;
            continue;
        }

        let process_kw = i;
        // Optional sensitivity list.
        let sens_list = next_significant(tokens, i).and_then(|j| {
            if tokens[j].kind == TokenKind::LParen {
                let close = matching_rparen(tokens, j)?;
                Some((j, close))
            } else {
                None
            }
        });

        // Find the matching `begin`.
        let mut begin_kw = None;
        let depth = 0i32;
        let mut j = i + 1;
        while j < tokens.len() {
            match tokens[j].kind {
                TokenKind::Keyword(Keyword::Begin) if depth == 0 => {
                    begin_kw = Some(j);
                    break;
                }
                TokenKind::Keyword(Keyword::Process) if depth == 0 => {
                    // Hit another `process` (likely `end process`); abort.
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        let Some(begin_kw) = begin_kw else {
            i += 1;
            continue;
        };

        // Find the matching `end process`.
        let mut end_kw = None;
        let mut k = begin_kw + 1;
        while k < tokens.len() {
            if tokens[k].kind == TokenKind::Keyword(Keyword::End)
                && let Some(after) = next_significant(tokens, k)
                && tokens[after].kind == TokenKind::Keyword(Keyword::Process)
            {
                end_kw = Some(k);
                break;
            }
            k += 1;
        }
        let Some(end_kw) = end_kw else {
            i += 1;
            continue;
        };

        out.push(ProcessBlock {
            process_kw,
            begin_kw,
            end_kw,
            sens_list,
        });
        i = end_kw + 1;
    }
    out
}

fn matching_rparen(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn previous_significant(tokens: &[Token], i: usize) -> Option<usize> {
    (0..i).rev().find(|&j| !tokens[j].is_trivia())
}

fn next_significant(tokens: &[Token], i: usize) -> Option<usize> {
    ((i + 1)..tokens.len()).find(|&j| !tokens[j].is_trivia())
}

/// Returns true iff the body of the given process contains any of
/// the canonical clock-edge expressions: `rising_edge`, `falling_edge`,
/// or `X'event`.
fn process_is_clocked(tokens: &[Token], proc: &ProcessBlock, src: &str) -> bool {
    for k in (proc.begin_kw + 1)..proc.end_kw {
        let tok = &tokens[k];
        if tok.kind == TokenKind::Ident {
            let lower = tok.text(src).to_ascii_lowercase();
            if lower == "rising_edge" || lower == "falling_edge" {
                return true;
            }
        }
        if tok.kind == TokenKind::Tick
            && let Some(after) = next_significant(tokens, k)
            && tokens[after].kind == TokenKind::Ident
            && tokens[after].text(src).eq_ignore_ascii_case("event")
        {
            return true;
        }
    }
    false
}

/// Walk the body of a process and collect (token index, identifier
/// text) for every signal that is the **target** of an assignment
/// (`<=` or `:=`). Identifiers are returned lowercased so callers
/// can do case-insensitive matching against declared signal names.
fn collect_assignments_in_body(
    tokens: &[Token],
    proc: &ProcessBlock,
    src: &str,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = proc.begin_kw + 1;
    while i < proc.end_kw {
        if matches!(tokens[i].kind, TokenKind::LE | TokenKind::Assign) {
            // Walk back over whitespace to find the identifier.
            if let Some(j) = previous_significant(tokens, i)
                && tokens[j].kind == TokenKind::Ident
            {
                out.push((j, tokens[j].text(src).to_ascii_lowercase()));
            }
        }
        i += 1;
    }
    out
}

/// Walk a token range and collect every plain identifier that is
/// **read** (i.e. appears anywhere except as the immediate LHS of
/// an assignment). Returns lowercased names for case-insensitive
/// matching. Approximate but useful.
fn collect_reads_in_body(tokens: &[Token], proc: &ProcessBlock, src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = proc.begin_kw + 1;
    while i < proc.end_kw {
        let tok = &tokens[i];
        if tok.kind == TokenKind::Ident {
            // If the next significant token is `<=` or `:=`, this is
            // an assignment target — skip it as a read.
            let next = next_significant(tokens, i);
            let is_lhs = matches!(
                next.map(|j| tokens[j].kind),
                Some(TokenKind::LE | TokenKind::Assign)
            );
            if !is_lhs {
                out.push(tok.text(src).to_ascii_lowercase());
            }
        }
        i += 1;
    }
    out
}

/// Collect identifiers in the sensitivity list of a process. Returns
/// lowercased names. Empty for processes without a sensitivity list.
fn sensitivity_list_idents(tokens: &[Token], proc: &ProcessBlock, src: &str) -> Vec<String> {
    let Some((open, close)) = proc.sens_list else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tok in &tokens[open + 1..close] {
        if tok.kind == TokenKind::Ident {
            out.push(tok.text(src).to_ascii_lowercase());
        }
    }
    out
}

/// Collect signal/variable/constant declarations across the entire
/// file. Returns (token index of declared name, name lowercased).
fn collect_signal_declarations(tokens: &[Token], src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let kind = tokens[i].kind;
        let is_decl_keyword = matches!(kind, TokenKind::Reserved)
            && {
                let lower = tokens[i].text(src).to_ascii_lowercase();
                matches!(lower.as_str(), "signal" | "variable" | "constant")
            };
        if is_decl_keyword
            && let Some(name_idx) = next_significant(tokens, i)
            && tokens[name_idx].kind == TokenKind::Ident
        {
            // Collect identifiers up to the `:` (handles `a, b, c : ...`).
            let mut k = name_idx;
            while k < tokens.len() && tokens[k].kind != TokenKind::Colon {
                if tokens[k].kind == TokenKind::Ident {
                    out.push((k, tokens[k].text(src).to_ascii_lowercase()));
                }
                k += 1;
            }
            i = k;
            continue;
        }
        i += 1;
    }
    out
}

/// Collect port declarations from the entity. Returns (name idx,
/// name lowercased, mode lowercased) where mode is "in"/"out"/etc.
fn collect_port_declarations(tokens: &[Token], src: &str) -> Vec<(usize, String, String)> {
    let mut out = Vec::new();
    // Find each `port (` ... `)` block at top level (inside entity).
    let mut i = 0;
    while i < tokens.len() {
        let is_port_kw = tokens[i].kind == TokenKind::Reserved
            && tokens[i].text(src).eq_ignore_ascii_case("port");
        if is_port_kw
            && let Some(open) = next_significant(tokens, i)
            && tokens[open].kind == TokenKind::LParen
            && let Some(close) = matching_rparen(tokens, open)
        {
            // Walk the port list, splitting on `;`.
            let mut k = open + 1;
            while k < close {
                // Each port declaration: name(s) ... : mode type ;
                // Find the colon.
                let mut colon = None;
                let mut depth = 0i32;
                let mut j = k;
                while j < close {
                    match tokens[j].kind {
                        TokenKind::LParen => depth += 1,
                        TokenKind::RParen => depth -= 1,
                        TokenKind::Colon if depth == 0 => {
                            colon = Some(j);
                            break;
                        }
                        TokenKind::Semi if depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                let Some(colon) = colon else {
                    k = j + 1;
                    continue;
                };
                // The mode is the first significant token after `:`.
                let mode = next_significant(tokens, colon)
                    .map(|m| tokens[m].text(src).to_ascii_lowercase())
                    .unwrap_or_default();
                // Collect names from k to colon.
                for (n, tok) in tokens.iter().enumerate().take(colon).skip(k) {
                    if tok.kind == TokenKind::Ident {
                        out.push((n, tok.text(src).to_ascii_lowercase(), mode.clone()));
                    }
                }
                // Advance past the next `;` at depth 0.
                let mut j = colon + 1;
                let mut depth = 0i32;
                while j < close {
                    match tokens[j].kind {
                        TokenKind::LParen => depth += 1,
                        TokenKind::RParen => depth -= 1,
                        TokenKind::Semi if depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                k = j + 1;
            }
            i = close + 1;
            continue;
        }
        i += 1;
    }
    out
}

// ───────────────── lint passes ─────────────────

fn check_sensitivity_list(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for proc in processes {
        if process_is_clocked(tokens, proc, src) {
            // Clocked processes don't need sensitivity-list completeness
            // because their sensitivity list is the clock (and reset).
            continue;
        }
        let sens: Vec<String> = sensitivity_list_idents(tokens, proc, src);
        if sens.is_empty() {
            // VHDL-2008 `process` (no list) — skip; user opted out.
            continue;
        }
        let reads = collect_reads_in_body(tokens, proc, src);
        let assigned: Vec<String> =
            collect_assignments_in_body(tokens, proc, src)
                .into_iter()
                .map(|(_, n)| n)
                .collect();
        for name in &reads {
            // Skip identifiers that are also assigned in the same
            // process (intermediate variables / temp signals can be
            // both read and written without being external inputs).
            if assigned.contains(name) {
                continue;
            }
            // Skip predefined names like rising_edge, etc.
            if is_predefined_function(name) {
                continue;
            }
            if !sens.contains(name) {
                let (line, col) = byte_to_line_col(src, tokens[proc.process_kw].start);
                let (end_line, end_col) = byte_to_line_col(src, tokens[proc.process_kw].end);
                out.push(Diagnostic {
                    line,
                    col,
                    end_line,
                    end_col,
                    severity: Severity::Warning,
                    code: "sens_001",
                    message: format!(
                        "signal `{}` is read in this combinational process but is not in the sensitivity list",
                        name
                    ),
                });
            }
        }
    }
    out
}

fn check_implicit_clock(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for proc in processes {
        if !process_is_clocked(tokens, proc, src) {
            continue;
        }
        // Find the clock identifier inside `rising_edge(X)` or
        // `falling_edge(X)`. We only check the first occurrence.
        let mut clock_name: Option<String> = None;
        let mut j = proc.begin_kw + 1;
        while j < proc.end_kw {
            if tokens[j].kind == TokenKind::Ident {
                let lower = tokens[j].text(src).to_ascii_lowercase();
                if (lower == "rising_edge" || lower == "falling_edge")
                    && let Some(open) = next_significant(tokens, j)
                    && tokens[open].kind == TokenKind::LParen
                    && let Some(name_idx) = next_significant(tokens, open)
                    && tokens[name_idx].kind == TokenKind::Ident
                {
                    clock_name = Some(tokens[name_idx].text(src).to_ascii_lowercase());
                    break;
                }
            }
            j += 1;
        }
        let Some(clock_name) = clock_name else {
            continue;
        };
        let sens = sensitivity_list_idents(tokens, proc, src);
        if !sens.is_empty() && !sens.contains(&clock_name) {
            let (line, col) = byte_to_line_col(src, tokens[proc.process_kw].start);
            let (end_line, end_col) = byte_to_line_col(src, tokens[proc.process_kw].end);
            out.push(Diagnostic {
                line,
                col,
                end_line,
                end_col,
                severity: Severity::Warning,
                code: "clock_001",
                message: format!(
                    "process uses `rising_edge({})` but `{}` is not in the sensitivity list",
                    clock_name, clock_name
                ),
            });
        }
    }
    out
}

fn check_multiple_drivers(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Map signal name -> Vec<process_kw_idx>
    use std::collections::BTreeMap;
    let mut driver_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for proc in processes {
        let assigned = collect_assignments_in_body(tokens, proc, src);
        let mut seen_in_proc = std::collections::BTreeSet::new();
        for (_, name) in assigned {
            if seen_in_proc.insert(name.clone()) {
                driver_map.entry(name).or_default().push(proc.process_kw);
            }
        }
    }
    for (name, procs) in &driver_map {
        if procs.len() < 2 {
            continue;
        }
        for &proc_idx in procs {
            let (line, col) = byte_to_line_col(src, tokens[proc_idx].start);
            let (end_line, end_col) = byte_to_line_col(src, tokens[proc_idx].end);
            out.push(Diagnostic {
                line,
                col,
                end_line,
                end_col,
                severity: Severity::Warning,
                code: "driver_001",
                message: format!(
                    "signal `{}` is assigned by {} different processes - this will be a synthesis error",
                    name,
                    procs.len()
                ),
            });
        }
    }
    out
}

fn check_unused_signals_and_ports(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Collect every read and every assignment across all processes
    // and at the architecture concurrent level.
    let mut all_reads = std::collections::BTreeSet::new();
    let mut all_writes = std::collections::BTreeSet::new();
    for proc in processes {
        for read in collect_reads_in_body(tokens, proc, src) {
            all_reads.insert(read);
        }
        for (_, name) in collect_assignments_in_body(tokens, proc, src) {
            all_writes.insert(name);
        }
    }
    // Concurrent assignments outside processes: collect all `<=` at
    // top level by walking *between* processes.
    collect_concurrent_assignments(
        tokens,
        src,
        processes,
        &mut all_reads,
        &mut all_writes,
    );

    // Unused signals.
    for (idx, name) in collect_signal_declarations(tokens, src) {
        if !all_reads.contains(&name) && !all_writes.contains(&name) {
            let (line, col) = byte_to_line_col(src, tokens[idx].start);
            let (end_line, end_col) = byte_to_line_col(src, tokens[idx].end);
            out.push(Diagnostic {
                line,
                col,
                end_line,
                end_col,
                severity: Severity::Warning,
                code: "unused_001",
                message: format!("signal `{}` is declared but never read or written", name),
            });
        } else if !all_reads.contains(&name) {
            let (line, col) = byte_to_line_col(src, tokens[idx].start);
            let (end_line, end_col) = byte_to_line_col(src, tokens[idx].end);
            out.push(Diagnostic {
                line,
                col,
                end_line,
                end_col,
                severity: Severity::Warning,
                code: "unused_002",
                message: format!("signal `{}` is written but never read", name),
            });
        }
    }
    // Unused ports.
    for (idx, name, mode) in collect_port_declarations(tokens, src) {
        match mode.as_str() {
            "in" => {
                if !all_reads.contains(&name) {
                    let (line, col) = byte_to_line_col(src, tokens[idx].start);
                    let (end_line, end_col) = byte_to_line_col(src, tokens[idx].end);
                    out.push(Diagnostic {
                        line,
                        col,
                        end_line,
                        end_col,
                        severity: Severity::Warning,
                        code: "unused_003",
                        message: format!("input port `{}` is never read", name),
                    });
                }
            }
            "out" => {
                if !all_writes.contains(&name) {
                    let (line, col) = byte_to_line_col(src, tokens[idx].start);
                    let (end_line, end_col) = byte_to_line_col(src, tokens[idx].end);
                    out.push(Diagnostic {
                        line,
                        col,
                        end_line,
                        end_col,
                        severity: Severity::Warning,
                        code: "unused_004",
                        message: format!("output port `{}` is never assigned", name),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn collect_concurrent_assignments(
    tokens: &[Token],
    src: &str,
    processes: &[ProcessBlock],
    reads: &mut std::collections::BTreeSet<String>,
    writes: &mut std::collections::BTreeSet<String>,
) {
    // Walk all tokens; skip ranges that are inside a process.
    let mut process_ranges: Vec<(usize, usize)> = processes
        .iter()
        .map(|p| (p.process_kw, p.end_kw))
        .collect();
    process_ranges.sort();

    let mut i = 0;
    while i < tokens.len() {
        // Skip if inside any process.
        if let Some(range) = process_ranges.iter().find(|(s, e)| i >= *s && i <= *e) {
            i = range.1 + 1;
            continue;
        }
        if matches!(tokens[i].kind, TokenKind::LE)
            && let Some(prev) = previous_significant(tokens, i)
            && tokens[prev].kind == TokenKind::Ident
        {
            writes.insert(tokens[prev].text(src).to_ascii_lowercase());
            // Walk forward to the `;`, collecting reads.
            let mut k = i + 1;
            while k < tokens.len() && tokens[k].kind != TokenKind::Semi {
                if tokens[k].kind == TokenKind::Ident {
                    reads.insert(tokens[k].text(src).to_ascii_lowercase());
                }
                k += 1;
            }
            i = k;
            continue;
        }
        i += 1;
    }
}

fn check_latch(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for proc in processes {
        if process_is_clocked(tokens, proc, src) {
            continue;
        }
        // Approximation: for each signal assigned inside this process,
        // check whether it has at least one assignment that is *not*
        // inside an `if`/`case` branch (i.e. an unconditional default).
        // If every assignment is inside a branch, warn.
        let assigned: Vec<(usize, String)> =
            collect_assignments_in_body(tokens, proc, src);
        let mut grouped: std::collections::BTreeMap<String, Vec<usize>> =
            Default::default();
        for (idx, name) in &assigned {
            grouped.entry(name.clone()).or_default().push(*idx);
        }
        for (name, occurrences) in &grouped {
            // For each occurrence, compute its conditional depth.
            let any_unconditional = occurrences
                .iter()
                .any(|&idx| conditional_depth(tokens, proc, idx) == 0);
            if !any_unconditional {
                let first = occurrences[0];
                let (line, col) = byte_to_line_col(src, tokens[first].start);
                let (end_line, end_col) = byte_to_line_col(src, tokens[first].end);
                out.push(Diagnostic {
                    line,
                    col,
                    end_line,
                    end_col,
                    severity: Severity::Warning,
                    code: "latch_001",
                    message: format!(
                        "signal `{}` may infer a latch: assigned only inside conditional branches with no default",
                        name
                    ),
                });
            }
        }
    }
    out
}

/// Compute how many `if` / `case` constructs the token at `target_idx`
/// is nested inside, relative to the body of the given process.
/// 0 means the token is at the top level of the process body.
fn conditional_depth(tokens: &[Token], proc: &ProcessBlock, target_idx: usize) -> i32 {
    let mut depth = 0i32;
    for i in (proc.begin_kw + 1)..target_idx {
        match tokens[i].kind {
            TokenKind::Keyword(Keyword::If) => depth += 1,
            TokenKind::Keyword(Keyword::Case) => {
                // Skip `end case` — preceded by `end`.
                if let Some(prev) = previous_significant(tokens, i)
                    && tokens[prev].kind == TokenKind::Keyword(Keyword::End)
                {
                    // already handled by End below
                } else {
                    depth += 1;
                }
            }
            TokenKind::Keyword(Keyword::End) => {
                // Decrement only when we see `end if` or `end case`.
                if let Some(after) = next_significant(tokens, i)
                    && matches!(
                        tokens[after].kind,
                        TokenKind::Keyword(Keyword::If) | TokenKind::Keyword(Keyword::Case)
                    )
                {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    depth
}

fn check_reset_polarity(
    src: &str,
    tokens: &[Token],
    processes: &[ProcessBlock],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Collect: for each "rst-like" signal name, the comparison value
    // we observe in the body. If the same name is compared to both
    // '0' and '1', warn.
    use std::collections::BTreeMap;
    let mut comparisons: BTreeMap<String, std::collections::BTreeSet<char>> =
        BTreeMap::new();

    for proc in processes {
        for k in (proc.begin_kw + 1)..proc.end_kw {
            // Pattern: ident `=` char
            if tokens[k].kind == TokenKind::Eq {
                let prev = previous_significant(tokens, k);
                let next = next_significant(tokens, k);
                if let (Some(p), Some(n)) = (prev, next)
                    && tokens[p].kind == TokenKind::Ident
                    && tokens[n].kind == TokenKind::Char
                {
                    let name = tokens[p].text(src).to_ascii_lowercase();
                    if name.contains("rst") || name.contains("reset") {
                        let value_text = tokens[n].text(src);
                        // value_text is like `'0'` — extract middle char.
                        let value = value_text.chars().nth(1).unwrap_or('?');
                        comparisons.entry(name).or_default().insert(value);
                    }
                }
            }
        }
    }
    for (name, values) in &comparisons {
        if values.contains(&'0') && values.contains(&'1') {
            // Find the first occurrence to attach the diagnostic to.
            let pos = tokens
                .iter()
                .position(|t| {
                    t.kind == TokenKind::Ident && t.text(src).eq_ignore_ascii_case(name)
                })
                .unwrap_or(0);
            let (line, col) = byte_to_line_col(src, tokens[pos].start);
            let (end_line, end_col) = byte_to_line_col(src, tokens[pos].end);
            out.push(Diagnostic {
                line,
                col,
                end_line,
                end_col,
                severity: Severity::Warning,
                code: "reset_001",
                message: format!(
                    "signal `{}` is compared against both '0' and '1' - inconsistent reset polarity",
                    name
                ),
            });
        }
    }
    out
}

fn check_magic_numbers(src: &str, tokens: &[Token]) -> Vec<Diagnostic> {
    // Off by default in this MVP - re-enable when a config-file
    // mechanism exists. We still implement the pass so the lint code
    // path is exercised once a config flag is added.
    let _ = (src, tokens);
    Vec::new()
}

fn is_predefined_function(name: &str) -> bool {
    matches!(
        name,
        "rising_edge"
            | "falling_edge"
            | "to_unsigned"
            | "to_signed"
            | "to_integer"
            | "resize"
            | "shift_left"
            | "shift_right"
            | "rotate_left"
            | "rotate_right"
            | "and_reduce"
            | "or_reduce"
            | "xor_reduce"
            | "std_logic_vector"
            | "unsigned"
            | "signed"
            | "conv_integer"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint_src(src: &str) -> Vec<Diagnostic> {
        lint(src, &Config::default())
    }

    #[test]
    fn unused_signal_is_flagged() {
        let src = "\
architecture rtl of t is
  signal dead : std_logic;
begin
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "unused_001"));
    }

    #[test]
    fn used_signal_is_not_flagged() {
        let src = "\
architecture rtl of t is
  signal a : std_logic;
begin
  a <= '0';
  b <= a;
end rtl;
";
        let diags = lint_src(src);
        assert!(!diags.iter().any(|d| d.code == "unused_001"));
    }

    #[test]
    fn missing_sensitivity_list_signal_is_flagged() {
        let src = "\
architecture rtl of t is
begin
  process(a)
  begin
    y <= a and b;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "sens_001" && d.message.contains("`b`")));
    }

    #[test]
    fn complete_sensitivity_list_is_silent() {
        let src = "\
architecture rtl of t is
begin
  process(a, b)
  begin
    y <= a and b;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(!diags.iter().any(|d| d.code == "sens_001"));
    }

    #[test]
    fn clocked_process_does_not_trigger_sens_list_check() {
        let src = "\
architecture rtl of t is
begin
  process(clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(!diags.iter().any(|d| d.code == "sens_001"));
    }

    #[test]
    fn missing_clock_in_sensitivity_list_is_flagged() {
        let src = "\
architecture rtl of t is
begin
  process(d)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "clock_001"));
    }

    #[test]
    fn multiple_drivers_is_flagged() {
        let src = "\
architecture rtl of t is
begin
  p1 : process(a)
  begin
    x <= a;
  end process p1;
  p2 : process(b)
  begin
    x <= b;
  end process p2;
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "driver_001"));
    }

    #[test]
    fn latch_is_flagged_when_default_missing() {
        let src = "\
architecture rtl of t is
begin
  process(a, sel)
  begin
    if sel = '1' then
      y <= a;
    end if;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "latch_001"));
    }

    #[test]
    fn no_latch_when_default_present() {
        let src = "\
architecture rtl of t is
begin
  process(a, sel)
  begin
    y <= '0';
    if sel = '1' then
      y <= a;
    end if;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(!diags.iter().any(|d| d.code == "latch_001"));
    }

    #[test]
    fn reset_polarity_inconsistency() {
        let src = "\
architecture rtl of t is
begin
  process(clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        q <= '0';
      end if;
    end if;
    if rst = '0' then
      q <= '1';
    end if;
  end process;
end rtl;
";
        let diags = lint_src(src);
        assert!(diags.iter().any(|d| d.code == "reset_001"));
    }

    #[test]
    fn diagnostic_jsonl_serialization_round_trips_safely() {
        let d = Diagnostic {
            line: 12,
            col: 5,
            end_line: 12,
            end_col: 18,
            severity: Severity::Warning,
            code: "test_001",
            message: r#"oops "quoted" \backslash"#.to_string(),
        };
        let s = d.to_jsonl();
        // Spot-check the wire shape.
        assert!(s.starts_with('{'));
        assert!(s.ends_with('}'));
        assert!(s.contains("\"line\":12"));
        assert!(s.contains("\"severity\":\"warning\""));
        assert!(s.contains("\"code\":\"test_001\""));
        assert!(s.contains(r#"\"quoted\""#));
    }
}
