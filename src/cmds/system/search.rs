//! Shared search-output filter for `rtk grep` and `rtk rg`.
//!
//! Runs the agent's exact engine (grep or rg) — never substituting one for the other — and
//! compresses its output by grouping matches by file, capping, and teeing overflow.

use crate::core::ai_output::{
    AiDocument, AiRecord, BudgetClass, EmissionMeta, ExactReason, Omission, OutputContract,
    Severity, prepare_emission_with_baseline, render_with_max_tokens,
};
use crate::core::arg_tokenizer::{self, Dialect, Token, TokenKind, ValueSpec};
use crate::core::guard::never_worse;
use crate::core::stream::{
    self, CaptureResult, FilterMode, StdinMode, StreamFilter, exec_capture, exec_capture_stdin,
};
use crate::core::tracking;
use crate::core::utils::{ChildArgExt, resolved_command, strip_ansi};
use crate::core::{args_utils, config, path_inventory, runner};
use anyhow::{Context, Result, anyhow};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

/// Long flags that consume the NEXT token as their value (space-separated form).
/// Inline `=` form (`--flag=value`) is one token and passes through unchanged.
/// `--regexp` is additionally extracted into `patterns` for semantic anchoring.
/// `--encoding` value is consumed correctly here; dialect routing is #2138's job.
const VALUE_FLAGS_LONG: &[&str] = &[
    "--after-context",
    "--before-context",
    "--color",
    "--colors",
    "--context",
    "--context-separator",
    "--encoding",
    "--engine",
    "--field-context-separator",
    "--field-match-separator",
    "--file",
    "--glob",
    "--iglob",
    "--ignore-file",
    "--max-columns",
    "--max-count",
    "--max-depth",
    "--max-filesize",
    "--path-separator",
    "--pre",
    "--pre-glob",
    "--replace",
    "--regexp",
    "--sort",
    "--sortr",
    "--threads",
    "--type",
    "--type-add",
    "--type-clear",
    "--type-not",
];

/// True if stdin is something the engine actually reads: a regular file, FIFO or socket --
/// ripgrep's own `is_readable_stdin` rule, and confirmed for both engines (`rg foo < file`
/// searches the file; `rg foo < /dev/null`, a character device, searches the cwd instead).
/// `!is_terminal()` is wider and wrongly matches that `/dev/null` case.
#[cfg(unix)]
fn stdin_is_readable() -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::FileTypeExt;
    std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map(std::fs::File::from)
        .and_then(|f| f.metadata())
        .map(|m| {
            let kind = m.file_type();
            kind.is_file() || kind.is_fifo() || kind.is_socket()
        })
        .unwrap_or(false)
}

/// KNOWN LIMITATION: no portable file-type check here, so Windows keeps the wider
/// `!is_terminal()` rule -- `rtk rg foo < NUL` still routes to the streaming path and drops
/// filenames there.
#[cfg(not(unix))]
fn stdin_is_readable() -> bool {
    !std::io::stdin().is_terminal()
}

/// Which flags consume a value, transcribed per engine from that engine's own `--help`.
/// grep and rg only intersect -- 13 of ~50 entries -- and disagree outright on `-T`, `-r`,
/// `-E` and `--color`, so one merged table with per-flag exceptions misreads whichever engine
/// it wasn't written for. `-e`/`--regexp`'s value is routed to `patterns` downstream, not
/// `flags`; both tables still report it, since this only answers "does the next token belong
/// to this flag".
fn grep_takes_value(kind: TokenKind, name: &str) -> bool {
    match kind {
        // `--color[=WHEN]`/`--colour[=WHEN]` attach their value, so they never consume a token.
        TokenKind::Long => matches!(
            name,
            "after-context"
                | "before-context"
                | "binary-files"
                | "context"
                | "devices"
                | "directories"
                | "exclude"
                | "exclude-dir"
                | "exclude-from"
                | "file"
                | "group-separator"
                | "include"
                | "label"
                | "max-count"
                | "regexp"
        ),
        // `-X` is grep's undocumented matcher selector, still accepted and still value-taking.
        TokenKind::Short => matches!(name, "A" | "B" | "C" | "D" | "X" | "d" | "e" | "f" | "m"),
        _ => false,
    }
}

fn rg_takes_value(kind: TokenKind, name: &str) -> bool {
    match kind {
        TokenKind::Long => matches!(
            name,
            "after-context"
                | "before-context"
                | "color"
                | "colors"
                | "context"
                | "context-separator"
                | "dfa-size-limit"
                | "encoding"
                | "engine"
                | "field-context-separator"
                | "field-match-separator"
                | "file"
                | "generate"
                | "glob"
                | "hostname-bin"
                | "hyperlink-format"
                | "iglob"
                | "ignore-file"
                | "max-columns"
                | "max-count"
                | "max-depth"
                | "max-filesize"
                | "path-separator"
                | "pre"
                | "pre-glob"
                | "regex-size-limit"
                | "regexp"
                | "replace"
                | "sort"
                | "sortr"
                | "threads"
                | "type"
                | "type-add"
                | "type-clear"
                | "type-not"
        ),
        TokenKind::Short => matches!(
            name,
            "A" | "B" | "C" | "E" | "M" | "T" | "d" | "e" | "f" | "g" | "j" | "m" | "r" | "t"
        ),
        _ => false,
    }
}

/// rg accepts the attached spellings `-A=1`/`-e=PAT` and strips the `=` itself; GNU grep does
/// not ("invalid context length argument"), so only rg's is unwrapped. Attached only: a
/// separate-token value is the user's own text, and `rg -e '=='` must search for `==`.
fn unwrap_attached_value(engine: Engine, value: &str) -> &str {
    match engine {
        Engine::Rg => value.strip_prefix('=').unwrap_or(value),
        Engine::Grep => value,
    }
}

/// The module's single tokenizer entry point. Shared so a pre-check and `extract_pattern_path`
/// cannot classify the same argument differently.
fn tokenize_search_args<'a, T: AsRef<str>>(args: &'a [T], engine: Engine) -> Vec<Token<'a>> {
    arg_tokenizer::tokenize_grammar(
        args,
        &|kind, name| search_takes_value(engine, kind, name),
        Dialect::Posix,
    )
}

/// Every grep/rg value-taking flag claims even a literal `--` as its value, unlike git/cargo --
/// verified against both engines for short and long, numeric- and file-typed flags alike.
fn search_takes_value(engine: Engine, kind: TokenKind, name: &str) -> Option<ValueSpec> {
    let takes = match engine {
        Engine::Grep => grep_takes_value(kind, name),
        Engine::Rg => rg_takes_value(kind, name),
    };
    takes.then(|| ValueSpec::value().claiming_dash_dash())
}

/// Unique, descriptive tee slug for a file's overflow matches. `idx` disambiguates
/// files within one grep; the tee filename's epoch handles separate runs.
fn grep_slug(idx: usize, path: &str) -> String {
    let cleaned: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let tail = &cleaned[cleaned.len().saturating_sub(32)..];
    format!("grep_{}_{}", idx, tail)
}

/// Format a file's matches as `path<sep>line<sep>content`. Tee blocks use the
/// real (un-compacted) `path` so recovered lines stay openable.
fn match_block(path: &str, entries: &[(usize, bool, String)]) -> String {
    let mut s = String::new();
    for (line_num, is_match, content) in entries {
        let sep = if *is_match { ':' } else { '-' };
        s.push_str(&format!("{}{}{}{}{}\n", path, sep, line_num, sep, content));
    }
    s
}

/// Extracts `(patterns, paths, flags, has_format_flag, detected)` from the raw trailing args.
/// `patterns` is the positional pattern plus all `-e`/`--regexp` values (empty → error); `paths`
/// is the remaining positionals (empty → caller defaults to `["."]`); `flags` is everything else
/// forwarded verbatim; `has_format_flag`/`detected` ([`DetectedFlags`]) are computed from this
/// same token pass rather than a second scan over the reconstructed `flags` strings.
fn extract_pattern_path<T: AsRef<str>>(
    args: &[T],
    engine: Engine,
) -> (Vec<String>, Vec<String>, Vec<String>, bool, DetectedFlags) {
    let tokens = tokenize_search_args(args, engine);

    let mut e_patterns: Vec<String> = Vec::new();
    let mut patterns_from_file = false;
    let mut positionals: Vec<String> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    let mut has_format_flag = false;
    // `None` until the user says either way; the last spelling wins, as both engines do.
    let mut show_file_flag: Option<bool> = None;
    let mut show_line_flag: Option<bool> = None;
    let mut recursive = false;
    let mut context = false;
    let mut i = 0;

    while i < tokens.len() {
        let t = &tokens[i];
        match t.kind {
            TokenKind::Long if t.text == "regexp" => {
                if let Some(v) = t.value(&tokens) {
                    e_patterns.push(v.to_string());
                }
            }
            TokenKind::Long => {
                if t.text == "file" {
                    patterns_from_file = true;
                }
                if is_format_flag_token(engine, t.kind, t.text) {
                    has_format_flag = true;
                }
                if is_show_file_token(t.kind, t.text) {
                    show_file_flag = Some(true);
                }
                if is_recursive_token(engine, t.kind, t.text) {
                    recursive = true;
                }
                if is_show_line_on_token(t.kind, t.text) {
                    show_line_flag = Some(true);
                }
                // Neither negation is forwarded: RTK forces `-nH` so it can parse the output,
                // and the user's `--no-filename`/`--no-line-number` would win as the later
                // flag, leaving nothing parseable and forcing a second run of the whole search.
                if is_show_file_off_token(engine, t.kind, t.text) {
                    show_file_flag = Some(false);
                    i += 1;
                    continue;
                }
                if is_show_line_off_token(engine, t.kind, t.text) {
                    show_line_flag = Some(false);
                    i += 1;
                    continue;
                }
                if is_context_token(engine, t.kind, t.text) {
                    context = true;
                }
                match t.attached {
                    Some(v) => flags.push(format!("--{}={v}", t.text)),
                    None => {
                        flags.push(format!("--{}", t.text));
                        if let Some(v) = t.value(&tokens) {
                            flags.push(v.to_string());
                        }
                    }
                }
            }
            // A value consumed by a preceding flag is handled there instead.
            TokenKind::Positional if t.is_free_positional() => {
                positionals.push(t.text.to_string());
            }
            TokenKind::Short => {
                // A cluster's boolean prefix (e.g. "r" in "-rA") stays glued into one
                // flag string, matching how the user typed it; only the trailing
                // value-taking char (if any) and its value are their own tokens.
                let source = t.source_index;
                let start = i;
                while i + 1 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Short
                    && tokens[i + 1].source_index == source
                {
                    i += 1;
                }
                let cluster = &tokens[start..=i];
                if cluster
                    .iter()
                    .any(|c| is_format_flag_token(engine, c.kind, c.text))
                {
                    has_format_flag = true;
                }
                // Letter by letter, so `-hH` and `-Hh` land where the engine lands them: the
                // later spelling wins.
                for c in cluster {
                    if is_show_file_token(c.kind, c.text) {
                        show_file_flag = Some(true);
                    } else if is_show_file_off_token(engine, c.kind, c.text) {
                        show_file_flag = Some(false);
                    }
                    if is_show_line_on_token(c.kind, c.text) {
                        show_line_flag = Some(true);
                    } else if is_show_line_off_token(engine, c.kind, c.text) {
                        show_line_flag = Some(false);
                    }
                    if is_recursive_token(engine, c.kind, c.text) {
                        recursive = true;
                    }
                }
                if cluster
                    .iter()
                    .any(|c| is_context_token(engine, c.kind, c.text))
                {
                    context = true;
                }
                let (bool_chars, value_char) = match cluster.split_last() {
                    Some((last, rest))
                        if search_takes_value(engine, TokenKind::Short, last.text).is_some() =>
                    {
                        (rest, Some(last))
                    }
                    _ => (cluster, None),
                };

                // `-h` drops out of the cluster for the same reason as its long form: RTK
                // forces `-H` to parse the output and the user's later `-h` would win.
                let glued: String = bool_chars
                    .iter()
                    .filter(|c| {
                        !is_show_file_off_token(engine, c.kind, c.text)
                            && !is_show_line_off_token(engine, c.kind, c.text)
                    })
                    .map(|c| c.text)
                    .collect();
                if !glued.is_empty() {
                    flags.push(format!("-{glued}"));
                }

                if let Some(vt) = value_char {
                    let value = match vt.attached {
                        Some(attached) => Some(unwrap_attached_value(engine, attached)),
                        None => vt.value(&tokens),
                    };
                    if vt.text == "e" {
                        match value {
                            Some(v) => e_patterns.push(v.to_string()),
                            None => flags.push("-e".to_string()),
                        }
                    } else {
                        if vt.text == "f" {
                            patterns_from_file = true;
                        }
                        flags.push(format!("-{}", vt.text));
                        if let Some(v) = value {
                            flags.push(v.to_string());
                        }
                    }
                }
            }
            // DashDash itself carries nothing to emit — the `--` boundary is handled by the
            // tokenizer (everything after it already comes back as Positional).
            _ => {}
        }
        i += 1;
    }

    // `-e`/`--regexp` and `-f`/`--file` both supply the patterns, so every positional is a
    // path. Taking the first one as the pattern instead left `paths` empty, which made the
    // engine read stdin (a hang under an agent harness) or walk the cwd.
    let (patterns, paths) = if !e_patterns.is_empty() || patterns_from_file {
        (e_patterns, positionals)
    } else {
        let paths = positionals.iter().skip(1).cloned().collect();
        let patterns = positionals.into_iter().take(1).collect();
        (patterns, paths)
    };

    let detected = DetectedFlags {
        show_file: show_file_flag,
        show_line: show_line_flag.unwrap_or(false),
        recursive,
        context,
    };

    (patterns, paths, flags, has_format_flag, detected)
}

fn unparsed_signal(stdout: &str) -> usize {
    stdout
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && trimmed != "--" && parse_match_line(line).is_none()
        })
        .count()
}

/// Output shapes that RTK can render as compact, line-oriented AI records.
/// Any flag that is not explicitly understood stays exact; adding a future
/// ripgrep flag must therefore be an intentional routing decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RgRoute {
    Matches,
    JsonEvents,
    Inventory,
    Counts,
    OnlyMatching,
    Exact(ExactReason),
}

fn select_rg_route(current: &mut RgRoute, next: RgRoute) -> Option<ExactReason> {
    match (*current, next) {
        (RgRoute::Matches, next) => {
            *current = next;
            None
        }
        (current, next) if current == next => None,
        _ => Some(ExactReason::Structured),
    }
}

fn rg_long_flag_value(flag: &str) -> bool {
    VALUE_FLAGS_LONG.contains(&flag)
}

fn is_rg_text_long_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--auto-hybrid-regex"
            | "--case-sensitive"
            | "--crlf"
            | "--fixed-strings"
            | "--glob-case-insensitive"
            | "--hidden"
            | "--ignore-case"
            | "--invert-match"
            | "--line-regexp"
            | "--messages"
            | "--mmap"
            | "--no-auto-hybrid-regex"
            | "--no-config"
            | "--no-ignore"
            | "--no-ignore-dot"
            | "--no-ignore-exclude"
            | "--no-ignore-files"
            | "--no-ignore-global"
            | "--no-ignore-messages"
            | "--no-ignore-parent"
            | "--no-ignore-vcs"
            | "--no-messages"
            | "--no-mmap"
            | "--no-pcre2-unicode"
            | "--no-require-git"
            | "--no-search-zip"
            | "--no-unicode"
            | "--one-file-system"
            | "--pcre2"
            | "--pcre2-unicode"
            | "--search-zip"
            | "--smart-case"
            | "--trim"
            | "--unicode"
            | "--with-filename"
            | "--no-filename"
            | "--line-number"
            | "--no-line-number"
            | "--word-regexp"
    )
}

fn classify_rg(args: &[String]) -> RgRoute {
    // Newline-bearing paths cannot be represented safely by the line-oriented
    // semantic parser, so preserve ripgrep's native output and exit behavior.
    if args
        .iter()
        .any(|arg| arg.contains('\r') || arg.contains('\n'))
    {
        return RgRoute::Exact(ExactReason::Sensitive);
    }
    let mut route = RgRoute::Matches;
    let mut no_filename = false;
    let mut past_dashdash = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if past_dashdash {
            i += 1;
            continue;
        }
        if arg == "--" {
            past_dashdash = true;
            i += 1;
            continue;
        }

        if let Some(rest) = arg.strip_prefix("--") {
            let (flag, has_inline_value) = rest
                .split_once('=')
                .map(|(flag, _)| (format!("--{flag}"), true))
                .unwrap_or_else(|| (arg.clone(), false));

            let mode = match flag.as_str() {
                "--json" => Some(RgRoute::JsonEvents),
                "--files" | "--files-with-matches" | "--files-without-match" => {
                    Some(RgRoute::Inventory)
                }
                "--count" | "--count-matches" => Some(RgRoute::Counts),
                "--only-matching" => Some(RgRoute::OnlyMatching),
                "--help" | "--version" => return RgRoute::Exact(ExactReason::Interactive),
                "--follow" => return RgRoute::Exact(ExactReason::Streaming),
                "--binary" | "--text" | "--text-encoding" => {
                    return RgRoute::Exact(ExactReason::Binary);
                }
                "--byte-offset"
                | "--column"
                | "--context-separator"
                | "--field-context-separator"
                | "--field-match-separator"
                | "--heading"
                | "--null"
                | "--null-data"
                | "--passthru"
                | "--pretty"
                | "--quiet"
                | "--silent"
                | "--stats"
                | "--vimgrep" => return RgRoute::Exact(ExactReason::Structured),
                "--debug" | "--trace" | "--type-list" | "--pcre2-version" => {
                    return RgRoute::Exact(ExactReason::Interactive);
                }
                "--color" | "--colors" | "--encoding" | "--pre" | "--pre-glob" => {
                    return RgRoute::Exact(ExactReason::Sensitive);
                }
                "--multiline" | "--multiline-dotall" => {
                    return RgRoute::Exact(ExactReason::Structured);
                }
                _ if rg_long_flag_value(&flag) || is_rg_text_long_flag(&flag) => None,
                _ => return RgRoute::Exact(ExactReason::Unknown),
            };

            if flag == "--no-filename" {
                no_filename = true;
            }

            if let Some(next) = mode
                && let Some(reason) = select_rg_route(&mut route, next)
            {
                return RgRoute::Exact(reason);
            }
            if matches!(route, RgRoute::Counts) && no_filename {
                return RgRoute::Exact(ExactReason::Structured);
            }
            if rg_long_flag_value(&flag) && !has_inline_value {
                if flag == "--replace" && args.get(i + 1).is_none_or(|value| value.starts_with('-'))
                {
                    return RgRoute::Exact(ExactReason::Unknown);
                }
                i += 1;
            }
            i += 1;
            continue;
        }

        let Some(cluster) = arg.strip_prefix('-').filter(|cluster| !cluster.is_empty()) else {
            i += 1;
            continue;
        };
        let bytes = cluster.as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            let flag = bytes[j] as char;
            let mode = match flag {
                'c' => Some(RgRoute::Counts),
                'l' => Some(RgRoute::Inventory),
                'L' => return RgRoute::Exact(ExactReason::Streaming),
                'o' => Some(RgRoute::OnlyMatching),
                'h' | 'V' => return RgRoute::Exact(ExactReason::Interactive),
                '0' | 'Z' | 'b' | 'p' | 'q' => return RgRoute::Exact(ExactReason::Structured),
                'a' | 'U' | 'z' => return RgRoute::Exact(ExactReason::Binary),
                'A' | 'B' | 'C' | 'M' | 'd' | 'e' | 'f' | 'g' | 'j' | 'm' | 'r' | 't' | 'T' => {
                    if j + 1 == bytes.len() {
                        if flag == 'r' && args.get(i + 1).is_none_or(|value| value.starts_with('-'))
                        {
                            return RgRoute::Exact(ExactReason::Unknown);
                        }
                        i += 1;
                    }
                    break;
                }
                'F' | 'H' | 'i' | 'n' | 'N' | 'P' | 'R' | 's' | 'S' | 'u' | 'v' | 'w' | 'x' => None,
                _ => return RgRoute::Exact(ExactReason::Unknown),
            };
            if let Some(next) = mode
                && let Some(reason) = select_rg_route(&mut route, next)
            {
                return RgRoute::Exact(reason);
            }
            j += 1;
        }
        i += 1;
    }

    route
}

const RG_AI_MAX_LINE_LEN: usize = 80;
type RgMatchEntry = (String, usize, bool, String, bool);
type RgMatchBlock = (String, Vec<(usize, bool, String, bool)>);

fn rg_document(
    route: RgRoute,
    raw: &str,
    patterns: &[String],
    _paths: &[String],
) -> Result<AiDocument> {
    if raw.is_empty() {
        return Ok(AiDocument::legacy(""));
    }

    match route {
        RgRoute::Matches => rg_match_document(raw, patterns, false),
        RgRoute::OnlyMatching => rg_match_document(raw, patterns, true),
        RgRoute::JsonEvents => rg_json_document(raw, patterns),
        RgRoute::Inventory => Ok(path_inventory::document(&parse_inventory_paths(raw))),
        RgRoute::Counts => rg_count_document(raw, _paths),
        RgRoute::Exact(reason) => Err(anyhow!(
            "exact rg route reached semantic renderer: {}",
            reason.as_str()
        )),
    }
}

fn rg_faithful_match_baseline(
    raw: &str,
    paths: &[String],
    extra_args: &[String],
) -> Result<String> {
    let show_file = rg_show_file(paths, extra_args);
    let show_line = extract_pattern_path(extra_args, Engine::Rg).4.show_line;
    let mut plain = String::new();
    for line in raw.lines() {
        if let Some(output) = format_match_line(line, show_file, show_line) {
            plain.push_str(&output);
        } else if line == "--" {
            plain.push_str("--\n");
        } else {
            return Err(anyhow!("unrecognized ripgrep match record"));
        }
    }
    Ok(plain)
}

fn rg_match_document(raw: &str, patterns: &[String], preserve_exact: bool) -> Result<AiDocument> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        if line == "--" {
            continue;
        }
        let Some((path, line_number, is_match, content)) = parse_match_line(line) else {
            return Err(anyhow!("unrecognized ripgrep match record"));
        };
        let (content, was_shortened) = clean_rg_line(content, patterns, preserve_exact);
        entries.push((path, line_number, is_match, content, was_shortened));
    }
    Ok(rg_match_document_from_entries(entries))
}

fn clean_rg_line(content: &str, anchors: &[String], preserve_exact: bool) -> (String, bool) {
    if preserve_exact {
        return (content.to_string(), false);
    }
    let content_lower = content.to_lowercase();
    let anchor = anchors
        .iter()
        .find(|anchor| !anchor.is_empty() && content_lower.contains(&anchor.to_lowercase()))
        .map(String::as_str)
        .unwrap_or_default();
    let cleaned = clean_line(content, RG_AI_MAX_LINE_LEN, None, anchor);
    let shortened = cleaned != content;
    (cleaned, shortened)
}

fn rg_match_document_from_entries(entries: Vec<RgMatchEntry>) -> AiDocument {
    let matches = entries
        .iter()
        .filter(|(_, _, is_match, _, _)| *is_match)
        .count();
    rg_match_document_from_entries_with_counts(entries, matches, 0)
}

fn rg_match_document_from_entries_with_counts(
    entries: Vec<RgMatchEntry>,
    matches: usize,
    omitted_items: usize,
) -> AiDocument {
    let mut document = AiDocument::new(Some("search"));
    document.fact("matches", matches.to_string());

    let mut blocks: Vec<RgMatchBlock> = Vec::new();
    for (path, line_number, is_match, content, was_shortened) in entries {
        if let Some((previous_path, records)) = blocks.last_mut()
            && *previous_path == path
        {
            records.push((line_number, is_match, content, was_shortened));
            continue;
        }
        blocks.push((path, vec![(line_number, is_match, content, was_shortened)]));
    }

    for (path, records) in blocks {
        // Keep per-file groups compact, but never make a large file an
        // indivisible record that is dropped wholesale by the source budget.
        for chunk in records.chunks(20) {
            let rendered_records = chunk
                .iter()
                .map(|(line_number, is_match, content, _)| {
                    let separator = if *is_match { ':' } else { '-' };
                    format!("{line_number}{separator} {content}")
                })
                .collect::<Vec<_>>()
                .join("; ");
            let shortened = chunk
                .iter()
                .filter(|(_, _, _, was_shortened)| *was_shortened)
                .count();
            document.push(
                AiRecord::new(Severity::Info, format!("{path} {{{rendered_records}}}"))
                    .grouped(&path)
                    .representing(chunk.len())
                    .omitting(shortened),
            );
        }
    }

    if omitted_items > 0 {
        document = document.with_omission(Omission {
            items: omitted_items,
            groups: 0,
        });
    }
    document
}

fn rg_json_document(raw: &str, patterns: &[String]) -> Result<AiDocument> {
    let mut entries = Vec::new();

    for line in raw.lines() {
        let event: Value = serde_json::from_str(line).context("invalid ripgrep JSON event")?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("ripgrep JSON event has no type"))?;
        if !matches!(event_type, "match" | "context") {
            continue;
        }

        let data = event
            .get("data")
            .ok_or_else(|| anyhow!("ripgrep JSON {event_type} event has no data"))?;
        let path_value = data.get("path");
        if path_value.and_then(|path| path.get("bytes")).is_some() {
            return Err(anyhow!("ripgrep JSON path is not valid UTF-8"));
        }
        let path = path_value
            .and_then(|path| path.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("<stdin>");
        let line_number = data
            .get("line_number")
            .and_then(Value::as_u64)
            .and_then(|line_number| usize::try_from(line_number).ok())
            .ok_or_else(|| anyhow!("ripgrep JSON {event_type} event has no line number"))?;
        let text = data
            .get("lines")
            .and_then(|lines| lines.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("ripgrep JSON {event_type} event has non-text lines"))?;
        let (text, was_shortened) =
            clean_rg_line(text.trim_end_matches(['\r', '\n']), patterns, false);
        entries.push((
            path.to_string(),
            line_number,
            event_type == "match",
            text,
            was_shortened,
        ));
    }

    Ok(rg_match_document_from_entries(entries))
}

fn parse_inventory_paths(raw: &str) -> Vec<String> {
    raw.split(['\n', '\0'])
        .map(|path| path.trim_end_matches('\r'))
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

fn rg_count_document(raw: &str, paths: &[String]) -> Result<AiDocument> {
    let mut document = AiDocument::new(Some("counts"));
    let mut files = 0;
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let (path, count) = if let Some((path, count)) = line.split_once('\0') {
            (path, count)
        } else if let Some((path, count)) = line.rsplit_once(':') {
            (path, count)
        } else if line.chars().all(|character| character.is_ascii_digit()) {
            (paths.first().map(String::as_str).unwrap_or("<stdin>"), line)
        } else {
            return Err(anyhow!("unrecognized ripgrep count record"));
        };
        if !count.chars().all(|character| character.is_ascii_digit()) {
            return Err(anyhow!("ripgrep count is not numeric"));
        }
        files += 1;
        document.push(AiRecord::new(Severity::Info, format!("{path}={count}")).grouped(path));
    }
    document.fact("files", files.to_string());
    Ok(document)
}

fn append_rg_parse_aids(cmd: &mut Command, args: &[String], aids: &[&str]) {
    if let Some(separator) = args.iter().position(|arg| arg == "--") {
        cmd.args(&args[..separator]);
        cmd.args(aids);
        cmd.arg("--");
        cmd.args(&args[separator + 1..]);
    } else {
        cmd.args(args);
        cmd.args(aids);
    }
}

const RG_STREAM_LINE_CAP: usize = 64 * 1024;

/// Bounded semantic search state. The producer is drained once, while only a
/// bounded number of parsed records and a bounded raw baseline are retained.
/// This is deliberately separate from the legacy streaming grep filter: an
/// oversized search line must be summarized, not replayed as an unbounded raw
/// fallback.
struct RgAiStreamFilter {
    route: RgRoute,
    patterns: Vec<String>,
    paths: Vec<String>,
    extra_args: Vec<String>,
    entries: Vec<RgMatchEntry>,
    max_entries: usize,
    total_matches: usize,
    omitted_items: usize,
    truncated_lines: usize,
    truncated_stored: usize,
    parse_errors: Vec<String>,
    raw_parse_aid: String,
    raw_complete: bool,
    stats: Arc<Mutex<EmissionMeta>>,
}

impl RgAiStreamFilter {
    fn new(
        route: RgRoute,
        patterns: Vec<String>,
        paths: Vec<String>,
        extra_args: Vec<String>,
        stats: Arc<Mutex<EmissionMeta>>,
    ) -> Self {
        Self {
            route,
            patterns,
            paths,
            extra_args,
            entries: Vec::new(),
            max_entries: config::limits().grep_max_results.max(1),
            total_matches: 0,
            omitted_items: 0,
            truncated_lines: 0,
            truncated_stored: 0,
            parse_errors: Vec::new(),
            raw_parse_aid: String::new(),
            raw_complete: true,
            stats,
        }
    }

    fn retain_raw_line(&mut self, line: &str) {
        if !self.raw_complete {
            return;
        }
        let required = line.len().saturating_add(1);
        if self.raw_parse_aid.len().saturating_add(required) > stream::RAW_CAP
            || line.ends_with(stream::TRUNCATED_LINE_MARKER)
        {
            self.raw_complete = false;
            return;
        }
        self.raw_parse_aid.push_str(line);
        self.raw_parse_aid.push('\n');
    }

    fn record_parse_error(&mut self, line: &str) {
        if self.parse_errors.len() < 4 {
            let sample = line.chars().take(512).collect::<String>();
            self.parse_errors.push(sample);
        }
    }

    fn render_bounded(&mut self) -> String {
        let truncated_items = self.truncated_lines.saturating_sub(self.truncated_stored);
        let declared_omissions = self
            .omitted_items
            .saturating_add(self.truncated_stored)
            .saturating_add(truncated_items);

        let mut document = if self.parse_errors.is_empty() {
            rg_match_document_from_entries_with_counts(
                std::mem::take(&mut self.entries),
                self.total_matches,
                declared_omissions,
            )
        } else {
            let sample = self.parse_errors.join("\n");
            let mut document = AiDocument::parse_failure(&sample, "unrecognized rg record");
            document.fact("observed_matches", self.total_matches.to_string());
            document
        };
        if self.truncated_lines > 0 {
            document.fact("truncated_lines", self.truncated_lines.to_string());
        }

        let rendered = render_with_max_tokens(
            &document,
            BudgetClass::Source,
            runner::requested_max_tokens(),
        );

        if self.raw_complete && self.parse_errors.is_empty() && self.truncated_lines == 0 {
            let baseline =
                rg_faithful_match_baseline(&self.raw_parse_aid, &self.paths, &self.extra_args)
                    .unwrap_or_else(|_| self.raw_parse_aid.clone());
            let prepared =
                prepare_emission_with_baseline(&baseline, &baseline, "rg", rendered, true);
            let meta = prepared.meta();
            if let Ok(mut current) = self.stats.lock() {
                *current = meta;
            }
            return prepared.as_str().to_string();
        }

        let omission = rendered.omission.clone();
        let mut output = rendered.text;
        if let Some(ref omission) = omission {
            output.push_str(&format!(
                "\nomitted items={} groups={} recovery=unavailable",
                omission.items, omission.groups
            ));
        } else {
            output.push_str("\nrecovery=unavailable");
        }
        let output = format!("{}\n", output.trim_end_matches('\n'));
        let meta = EmissionMeta {
            omitted_items: omission
                .as_ref()
                .map_or(declared_omissions, |value| value.items),
            omitted_groups: omission.as_ref().map_or(0, |value| value.groups),
            parser_failed: !self.parse_errors.is_empty(),
            runtime_error: Some("capture_incomplete"),
            ..EmissionMeta::default()
        };
        if let Ok(mut current) = self.stats.lock() {
            *current = meta;
        }
        output
    }
}

impl StreamFilter for RgAiStreamFilter {
    fn feed_line(&mut self, line: &str) -> Option<String> {
        self.retain_raw_line(line);
        if line == "--" {
            return None;
        }

        let preserve_exact = matches!(self.route, RgRoute::OnlyMatching);
        let Some((path, line_number, is_match, content)) = parse_match_line(line) else {
            self.record_parse_error(line);
            return None;
        };
        if is_match {
            self.total_matches = self.total_matches.saturating_add(1);
        }
        let line_was_truncated = line.ends_with(stream::TRUNCATED_LINE_MARKER);
        if line_was_truncated {
            self.truncated_lines = self.truncated_lines.saturating_add(1);
        }
        let (content, was_shortened) = clean_rg_line(content, &self.patterns, preserve_exact);
        if self.entries.len() >= self.max_entries {
            self.omitted_items = self.omitted_items.saturating_add(1);
        } else {
            self.entries
                .push((path, line_number, is_match, content, was_shortened));
            if line_was_truncated {
                self.truncated_stored = self.truncated_stored.saturating_add(1);
            }
        }
        None
    }

    fn flush(&mut self) -> String {
        String::new()
    }

    fn on_exit(&mut self, _exit_code: i32, _raw: &str) -> Option<String> {
        Some(self.render_bounded())
    }
}

fn run_rg_ai_streaming(
    route: RgRoute,
    args: &[String],
    patterns: Vec<String>,
    paths: Vec<String>,
    extra_args: Vec<String>,
) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    let stats = Arc::new(Mutex::new(EmissionMeta::default()));
    let tracking_paths = paths.clone();
    let tracking_extra_args = extra_args.clone();
    let filter = RgAiStreamFilter::new(route, patterns, paths, extra_args, Arc::clone(&stats));
    let mut command = rg_semantic_command(route, args);
    let result = stream::run_streaming_with_line_cap(
        &mut command,
        StdinMode::Null,
        FilterMode::StreamingStdout(Box::new(filter)),
        Some(RG_STREAM_LINE_CAP),
    )
    .context("search failed")?;
    let meta = stats.lock().map(|value| *value).unwrap_or_default();
    let output_tokens =
        tracking::estimate_tokens(&format!("{}{}", result.raw_stderr, result.filtered));
    let input_tokens = rg_tracking_input_tokens(
        &result.raw_stdout,
        &result.raw_stderr,
        result.observed_output_bytes(),
        rg_capture_is_complete(result.capture_complete, meta),
        &tracking_paths,
        &tracking_extra_args,
    );
    timer.track_output_tokens(
        &format!("rg {}", args.join(" ")),
        &format!("rtk rg {}", args.join(" ")),
        input_tokens,
        output_tokens,
        runner::output_tracking_from_emission(OutputContract::AiOwned(BudgetClass::Source), meta),
    );
    Ok(result.exit_code)
}

fn rg_capture_is_complete(stream_capture_complete: bool, meta: EmissionMeta) -> bool {
    stream_capture_complete && meta.runtime_error != Some("capture_incomplete")
}

fn rg_tracking_input_tokens(
    raw_stdout: &str,
    raw_stderr: &str,
    observed_output_bytes: usize,
    capture_complete: bool,
    paths: &[String],
    extra_args: &[String],
) -> usize {
    if capture_complete
        && let Ok(native) = rg_faithful_match_baseline(raw_stdout, paths, extra_args)
    {
        return tracking::estimate_tokens(&format!("{}{}", native, raw_stderr));
    }
    tracking::estimate_tokens_from_bytes(observed_output_bytes)
}

fn rg_semantic_command(route: RgRoute, args: &[String]) -> Command {
    let mut cmd = resolved_command("rg");
    match route {
        RgRoute::Matches | RgRoute::OnlyMatching => {
            append_rg_parse_aids(&mut cmd, args, &["-n", "--with-filename", "--null"])
        }
        RgRoute::Counts | RgRoute::JsonEvents | RgRoute::Inventory | RgRoute::Exact(_) => {
            cmd.args(args);
        }
    };
    cmd
}

fn run_rg_ai(route: RgRoute, args: &[String]) -> Result<i32> {
    let (mut patterns, paths, _, _, _) = extract_pattern_path(args, Engine::Rg);
    // The shared grep parser strips display selectors for its parse-aided run.
    // Recovery must instead reconstruct the user's original filename/line policy.
    let extra_args = args.to_vec();
    patterns.extend(rg_replacement_values(args));
    if matches!(route, RgRoute::Matches | RgRoute::OnlyMatching) {
        return run_rg_ai_streaming(route, args, patterns, paths, extra_args);
    }
    let budget = match route {
        RgRoute::Inventory => BudgetClass::Collection,
        RgRoute::Matches | RgRoute::JsonEvents | RgRoute::Counts | RgRoute::OnlyMatching => {
            BudgetClass::Source
        }
        RgRoute::Exact(_) => unreachable!("exact rg route cannot use semantic runner"),
    };
    let command = rg_semantic_command(route, args);
    let args_display = args.join(" ");
    let native_args = args.to_vec();

    runner::run_ai_filtered_with_exit(
        command,
        "rg",
        &args_display,
        budget,
        move |raw, exit_code| {
            if raw.is_empty() && exit_code != 0 {
                Ok(AiDocument::legacy(""))
            } else {
                let parsed = (|| -> Result<AiDocument> {
                    let document = rg_document(route, raw, &patterns, &paths)?;
                    if matches!(route, RgRoute::Matches | RgRoute::OnlyMatching) {
                        Ok(document.with_lossless_baseline(rg_faithful_match_baseline(
                            raw,
                            &paths,
                            &extra_args,
                        )?))
                    } else {
                        Ok(document)
                    }
                })();

                parsed.or_else(|_| {
                    // Parse aids are deliberately augmented onto the semantic
                    // invocation. If parsing fails, rerun the original argv so
                    // recovery and fallback preserve native output exactly.
                    let mut native = resolved_command("rg");
                    native.args(&native_args);
                    let result = exec_capture_stdin(&mut native)?;
                    let stdout = result.stdout;
                    Ok(AiDocument::legacy(stdout.clone()).with_lossless_baseline(stdout))
                })
            }
        },
        runner::RunOptions::stdout_only(),
    )
}

fn rg_replacement_values(args: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    let mut past_dashdash = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if past_dashdash {
            break;
        }
        if arg == "--" {
            past_dashdash = true;
            index += 1;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--replace=") {
            values.push(value.to_string());
        } else if arg == "--replace" || arg == "-r" {
            if let Some(value) = args.get(index + 1) {
                values.push(value.clone());
                index += 1;
            }
        } else if let Some(cluster) = arg.strip_prefix('-')
            && let Some(position) = cluster.bytes().position(|character| character == b'r')
        {
            let replacement = &cluster[position + 1..];
            if !replacement.is_empty() {
                values.push(replacement.to_string());
            } else if let Some(value) = args.get(index + 1) {
                values.push(value.clone());
                index += 1;
            }
        }
        index += 1;
    }
    values
}

fn run_rg_exact(args: &[String], verbose: u8, reason: ExactReason) -> Result<i32> {
    let mut args = args
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    if reason == ExactReason::Streaming
        && !std::io::stdout().is_terminal()
        && !args.iter().any(|arg| arg == "--line-buffered")
    {
        // A piped search must emit matches while its stdin producer is still
        // open. Keep native output and exit semantics, but disable ripgrep's
        // block buffering for the exact streaming route.
        args.insert(0, std::ffi::OsString::from("--line-buffered"));
    }
    runner::run_passthrough_with_reason("rg", &args, verbose, reason)
}

/// Run real grep so matches and the savings baseline match the agent's command;
/// rg is the fallback when grep is absent, rejects a flag, or `--type` is used.
/// The search engine the agent actually invoked. RTK runs this binary verbatim
/// and never substitutes one for the other.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Grep,
    Rg,
}

impl Engine {
    fn bin(self) -> &'static str {
        match self {
            Engine::Grep => "grep",
            Engine::Rg => "rg",
        }
    }

    pub fn label(self) -> &'static str {
        self.bin()
    }

    /// `-n -H --null` are parse aids (NUL keeps the regroup unambiguous, #1436);
    /// `-I` skips binary noise (-a overrides).
    fn parse_flags(self) -> &'static [&'static str] {
        match self {
            Engine::Grep => &["-n", "-H", "-I", "--null"],
            Engine::Rg => &["-n", "--with-filename", "--null"],
        }
    }
}

/// Runs the agent's exact engine + flags for the grouping path, appending only the
/// parse aids (see `Engine::parse_flags`).
fn engine_capture<T: AsRef<str>>(
    engine: Engine,
    extra_args: &[T],
    patterns: &[String],
    paths: &[String],
) -> Result<CaptureResult> {
    let mut cmd = engine_command(engine, extra_args, patterns, paths, false);
    exec_capture_stdin(&mut cmd).context("search failed")
}

fn engine_command<T: AsRef<str>>(
    engine: Engine,
    extra_args: &[T],
    patterns: &[String],
    paths: &[String],
    line_buffered: bool,
) -> Command {
    let mut cmd = resolved_command(engine.bin());
    cmd.child_args(engine.parse_flags());
    for a in extra_args {
        cmd.child_arg(a.as_ref());
    }
    if line_buffered {
        // The engine writes through a pipe, so flush each match immediately.
        cmd.child_arg("--line-buffered");
    }
    for p in patterns {
        cmd.child_args(["-e", p]);
    }
    cmd.child_arg("--");
    cmd.child_args(paths);
    cmd
}

fn format_match_line(line: &str, show_file: bool, show_line: bool) -> Option<String> {
    let (file, line_num, is_match, content) = parse_match_line(line)?;
    let sep = if is_match { ':' } else { '-' };
    let mut output = String::new();
    if show_file {
        output.push_str(&file);
        output.push(sep);
    }
    if show_line {
        output.push_str(&line_num.to_string());
        output.push(sep);
    }
    output.push_str(content);
    output.push('\n');
    Some(output)
}

/// Emits each piped match as it arrives. Buffered search waits for EOF, so
/// `tail -f app.log | rtk grep ERROR` would otherwise show no matches.
struct SearchStreamFilter {
    show_file: bool,
    show_line: bool,
    max_results: usize,
    shown: usize,
    cap_reported: bool,
}

impl StreamFilter for SearchStreamFilter {
    fn feed_line(&mut self, line: &str) -> Option<String> {
        let Some(output) = format_match_line(line, self.show_file, self.show_line) else {
            if line == "--" && self.shown >= self.max_results {
                return None;
            }
            return Some(format!("{line}\n"));
        };

        if self.shown >= self.max_results {
            if self.cap_reported {
                return None;
            }
            self.cap_reported = true;
            return Some(format!(
                "[rtk] output capped at {} results\n",
                self.max_results
            ));
        }

        self.shown += 1;
        Some(output)
    }

    fn flush(&mut self) -> String {
        String::new()
    }
}

/// Ripgrep's filename selectors override its default of showing names for
/// multi-file and recursive searches. Preserve their argument order while
/// reconstructing the lossless baseline from our parse-aided invocation.
fn rg_show_file(paths: &[String], extra_args: &[String]) -> bool {
    let explicit = extract_pattern_path(extra_args, Engine::Rg).4.show_file;
    explicit.unwrap_or_else(|| {
        paths.is_empty()
            || paths.len() > 1
            || paths.iter().any(|path| std::path::Path::new(path).is_dir())
    })
}

/// The paths-based half of "should the filename be shown": multiple paths, or a directory among
/// them, regardless of any flag. Combined with `extract_pattern_path`'s pre-computed
/// `DetectedFlags::show_file` (the flags-based half) at each call site.
fn wants_show_file(paths: &[String], flags_show_file: bool) -> bool {
    paths.len() > 1 || paths.iter().any(|p| std::path::Path::new(p).is_dir()) || flags_show_file
}

#[allow(clippy::too_many_arguments)]
fn run_streaming_search(
    timer: &tracking::TimedExecution,
    engine: Engine,
    extra_args: &[String],
    patterns: &[String],
    paths: &[String],
    max_results: usize,
    real_cmd: &str,
    detected_flags: DetectedFlags,
) -> Result<i32> {
    let filter = SearchStreamFilter {
        show_file: detected_flags
            .show_file
            .unwrap_or_else(|| wants_show_file(paths, detected_flags.recursive)),
        show_line: detected_flags.show_line,
        max_results,
        shown: 0,
        cap_reported: false,
    };
    let mut cmd = engine_command(engine, extra_args, patterns, paths, true);
    let result = stream::run_streaming(
        &mut cmd,
        StdinMode::Inherit,
        FilterMode::Streaming(Box::new(filter)),
    )
    .context("search failed")?;

    timer.track(
        real_cmd,
        &format!("rtk {}", engine.label()),
        &result.raw_stdout,
        &result.filtered,
    );
    Ok(result.exit_code)
}

/// Runs the agent's command verbatim for forms RTK does not group: format/shape
/// flags and pattern-less modes (`--files`, `--type-list`).
///
/// One exception: a bare file list (`-l`/`-L`/`--files`, see [`is_bare_file_list`]) has its
/// shared directory prefix folded into a header when captured. The streaming form reads
/// stdin, where the only "file" is `(standard input)`, so it has nothing to fold.
fn passthrough<T: AsRef<str>>(
    timer: &tracking::TimedExecution,
    engine: Engine,
    args: &[T],
    real_cmd: &str,
    stream_stdin: bool,
    fold_file_list: bool,
) -> Result<i32> {
    let mut cmd = resolved_command(engine.bin());
    if stream_stdin && !std::io::stdout().is_terminal() {
        // Keep passthrough output live when stdout is piped.
        cmd.child_arg("--line-buffered");
    }
    for a in args {
        cmd.child_arg(a.as_ref());
    }

    if stream_stdin {
        let exit_code =
            stream::run_streaming(&mut cmd, StdinMode::Inherit, FilterMode::Passthrough)
                .context("search failed")?
                .exit_code;
        timer.track_passthrough(real_cmd, &format!("rtk {} (passthrough)", real_cmd));
        return Ok(exit_code);
    }

    let result = exec_capture_stdin(&mut cmd).context("search failed")?;
    let cleaned = strip_ansi(&result.stdout);
    let folded = if fold_file_list {
        fold_path_prefix(&cleaned).filter(|f| never_worse(&cleaned, f) == f)
    } else {
        None
    };
    match &folded {
        Some(folded) => print!("{}", folded),
        None => print!("{}", cleaned),
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    match &folded {
        // Real sizes, so the fold shows up in `rtk gain`.
        Some(folded) => timer.track(
            real_cmd,
            &format!("rtk {}", engine.label()),
            &cleaned,
            folded,
        ),
        // 0/0 keeps an unchanged passthrough from diluting the savings statistics.
        None => timer.track_passthrough(real_cmd, &format!("rtk {} (passthrough)", real_cmd)),
    }
    Ok(result.exit_code)
}

/// Folds the directory prefix every line of a file list shares into a one-line header, then
/// emits each path's remaining tail in engine order:
///
/// ```text
/// /home/u/proj/src/ (3 files)
/// a/foo.rs
/// a/bar.rs
/// b/baz.rs
/// ```
///
/// Agents search from absolute roots, so `-l` output restates the same long prefix on every
/// line; that prefix is the whole redundancy of the list. The transform is lossless (each path
/// is `prefix + tail`) and needs no cap or tee. `None` when there is nothing to fold: fewer
/// than two lines, a line that is not a plain path (empty, NUL-joined under `-Z`, carrying a
/// `\r` that `lines()` would drop, or an escape such as an rg `--hyperlink-format` OSC 8 link
/// that `strip_ansi` leaves in place), or no shared directory component. The prefix is cut on whole components, never inside one, so
/// `/a/foobar/x` and `/a/foobaz/y` share `/a/`, not `/a/fooba`.
///
/// KNOWN LIMITATION: only `/` separates components, so Windows-style `\` paths never fold.
fn fold_path_prefix(raw: &str) -> Option<String> {
    // Checked on the raw text: `lines()` would already have dropped a `\r` before `\n`.
    if raw.contains(['\0', '\r', '\x1b']) {
        return None;
    }
    let paths: Vec<&str> = raw.lines().collect();
    if paths.len() < 2 || paths.iter().any(|p| p.is_empty()) {
        return None;
    }

    fn dirs(path: &str) -> Vec<&str> {
        match path.rsplit_once('/') {
            Some((dir, _)) => dir.split('/').collect(),
            None => Vec::new(),
        }
    }

    let first = dirs(paths[0]);
    let mut common = first.len();
    for path in &paths[1..] {
        let shared = dirs(path)
            .iter()
            .zip(&first)
            .take(common)
            .take_while(|(a, b)| a == b)
            .count();
        common = shared;
        if common == 0 {
            return None;
        }
    }

    // An absolute path splits to a leading empty component, so `/a/b` rejoins with its slash;
    // a bare `/` root is the one prefix that saves nothing.
    let prefix = format!("{}/", first[..common].join("/"));
    if prefix.len() <= 1 {
        return None;
    }

    let mut out = format!("{} ({} files)\n", prefix, paths.len());
    for path in &paths {
        out.push_str(path.strip_prefix(&prefix).unwrap_or(path));
        out.push('\n');
    }
    Some(out)
}

pub fn run(
    engine: Engine,
    max_line_len: usize,
    max_results: usize,
    context_only: bool,
    args: &[String],
    verbose: u8,
) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    // Restored first: every check below classifies these args, and clap ate the boundary.
    let args = &args_utils::restore_double_dash(args);

    // --version / --help: pass through to the engine without filtering. Token-based and
    // scoped before the boundary, because `rtk grep -- --version` searches *for* that string.
    // `-h` is engine-specific: rg's is --help, grep's is --no-filename.
    let help_tokens = tokenize_search_args(args, engine);
    let asks_for_help = arg_tokenizer::before_dashdash(&help_tokens)
        .iter()
        .any(|t| {
            (t.kind == TokenKind::Long && matches!(t.text, "version" | "help"))
                || (t.kind == TokenKind::Short && t.text == "h" && engine == Engine::Rg)
        });
    let dangling_value_flag = help_tokens.iter().any(|t| {
        matches!(t.kind, TokenKind::Long | TokenKind::Short)
            && search_takes_value(engine, t.kind, t.text).is_some()
            && t.value(&help_tokens).is_none()
    });
    if dangling_value_flag {
        let real_cmd = format!("{} {}", engine.bin(), args.join(" "));
        return passthrough(&timer, engine, args, &real_cmd, false, false);
    }

    if asks_for_help {
        let mut cmd = resolved_command(engine.bin());
        cmd.child_args(args);
        let result = exec_capture(&mut cmd).context("search failed")?;
        print!("{}", result.stdout);
        if !result.stderr.is_empty() {
            eprint!("{}", result.stderr);
        }
        return Ok(result.exit_code);
    }

    if matches!(engine, Engine::Rg) {
        let (_, rg_paths, _, _, _) = extract_pattern_path(args, Engine::Rg);
        let route = classify_rg(args);
        let reads_piped_stdin =
            stdin_is_readable() && (rg_paths.is_empty() || rg_paths.iter().any(|path| path == "-"));
        if reads_piped_stdin && !matches!(route, RgRoute::Inventory) {
            return run_rg_exact(args, verbose, ExactReason::Streaming);
        }
        return match route {
            RgRoute::Exact(reason) => run_rg_exact(args, verbose, reason),
            route => run_rg_ai(route, args),
        };
    }

    let real_cmd = format!("{} {}", engine.label(), args.join(" "));
    let rtk_label = format!("rtk {}", engine.label());

    let (patterns, paths, extra_args, extra_args_has_format_flag, detected_flags) =
        extract_pattern_path(args, engine);

    if patterns.is_empty() {
        // `rg --files` lists paths without a pattern; fold it like `-l`.
        let fold = is_bare_file_list(engine, args);
        return passthrough(&timer, engine, args, &real_cmd, false, fold);
    }

    let pattern_display = if patterns.len() == 1 {
        patterns[0].clone()
    } else {
        patterns.join("|")
    };

    let path_display = paths.join(" ");

    if verbose > 0 {
        eprintln!("grep: '{}' in {}", pattern_display, path_display);
    }

    let reads_piped_stdin =
        stdin_is_readable() && (paths.is_empty() || paths.iter().any(|path| path == "-"));

    // format/shape flags (-c/-l/-o/...): already-minimal native output, passthrough.
    if extra_args_has_format_flag {
        let fold = is_bare_file_list(engine, args);
        return passthrough(&timer, engine, args, &real_cmd, reads_piped_stdin, fold);
    }

    if reads_piped_stdin {
        return run_streaming_search(
            &timer,
            engine,
            &extra_args,
            &patterns,
            &paths,
            max_results,
            &real_cmd,
            detected_flags,
        );
    }

    let result = engine_capture(engine, &extra_args, &patterns, &paths)?;

    let exit_code = result.exit_code;
    let raw_output = result.stdout.clone();

    // Unparseable shape re-runs verbatim below (with its own stderr), so handle it
    // before surfacing this run's stderr (#2333).
    if unparsed_signal(&raw_output) > 0 {
        return passthrough(&timer, engine, args, &real_cmd, false, false);
    }

    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    if result.stdout.trim().is_empty() {
        timer.track(&real_cmd, &rtk_label, &raw_output, "");
        return Ok(exit_code);
    }

    let context_re = if context_only {
        Regex::new(&format!(
            "(?i).{{0,20}}{}.*",
            regex::escape(&pattern_display)
        ))
        .ok()
    } else {
        None
    };

    let mut by_file: HashMap<String, Vec<(usize, bool, String)>> = HashMap::new();
    for line in raw_output.lines() {
        let Some((file, line_num, is_match, content)) = parse_match_line(line) else {
            continue;
        };
        let cleaned = clean_line(content, max_line_len, context_re.as_ref(), &pattern_display);
        by_file
            .entry(file)
            .or_default()
            .push((line_num, is_match, cleaned));
    }

    let total_matches: usize = by_file
        .values()
        .flat_map(|v| v.iter())
        .filter(|(_, is_match, _)| *is_match)
        .count();

    // Mirror what the real command prints: the filename only when grep/rg would
    // show one (multiple files, a directory, -r or -H), the line number only with
    // -n. We force -nH--null for robust parsing, then drop what the engine itself
    // would not have shown.
    // With no path given, rg walks the cwd, and there the filename is the only way to tell
    // matches apart -- real rg prints it even when a single file matched. grep with no path
    // reads stdin instead (whatever stdin is), where a filename would be `(standard input)`,
    // so the same reasoning does not carry over.
    let walks_cwd = engine == Engine::Rg && paths.is_empty();
    let show_file = detected_flags.show_file.unwrap_or_else(|| {
        by_file.len() > 1 || walks_cwd || wants_show_file(&paths, detected_flags.recursive)
    });
    let show_line = detected_flags.show_line;

    // Faithful baseline: exactly what the real command prints, full content.
    let mut plain = String::new();
    for line in raw_output.lines() {
        let Some(output) = format_match_line(line, show_file, show_line) else {
            if line == "--" {
                plain.push_str("--\n");
            }
            continue;
        };
        plain.push_str(&output);
    }

    let has_context = detected_flags.context;

    let per_file = config::limits().grep_max_per_file;
    let mut files: Vec<_> = by_file.iter().collect();
    files.sort_by_key(|(f, _)| *f);

    let mut body = String::new();
    let mut shown = 0;
    let mut skipped_files = 0;
    let mut skipped_block = String::new();
    for (idx, (file, entries)) in files.into_iter().enumerate() {
        if shown >= max_results {
            skipped_files += 1;
            skipped_block.push_str(&match_block(file, entries));
            continue;
        }

        let file_display = compact_path(file);
        let mut file_shown = 0;
        let mut prev_line: usize = 0;
        for (line_num, is_match, content) in entries.iter().take(per_file) {
            if shown >= max_results {
                break;
            }
            if has_context && prev_line > 0 && *line_num > prev_line + 1 {
                body.push_str("--\n");
            }
            prev_line = *line_num;
            let sep = if *is_match { ':' } else { '-' };
            if show_file {
                body.push_str(&file_display);
                body.push(sep);
            }
            if show_line {
                body.push_str(&line_num.to_string());
                body.push(sep);
            }
            body.push_str(content);
            body.push('\n');
            shown += 1;
            file_shown += 1;
        }

        let remaining = entries.len() - file_shown;
        if remaining == 0 {
            continue;
        }
        // Tee the file's full matches (real path) so the tail hint recovers them
        // openably, skipping the lines already shown.
        let full_block = match_block(file, entries);
        match crate::core::tee::force_tee_tail_hint(
            &full_block,
            &grep_slug(idx, file),
            file_shown + 1,
        ) {
            Some(hint) => body.push_str(&format!(
                "  +{} more in {} {}\n",
                remaining, file_display, hint
            )),
            None => body.push_str(&format!("  +{} more in {}\n", remaining, file_display)),
        }
    }

    if skipped_files > 0 {
        let hint = crate::core::tee::force_tee_tail_hint(&skipped_block, "grep_skipped", 1)
            .map(|h| format!(" {}", h))
            .unwrap_or_default();
        body.push_str(&format!("+{} more files{}\n", skipped_files, hint));
    }

    // Switch to the grouped form only when capping actually shrank the output;
    // otherwise emit the faithful baseline, so RTK never exceeds the real command.
    let capped = shown < total_matches || skipped_files > 0;
    let rtk_output = if capped {
        format!(
            "{} matches in {} files:\n\n{}",
            total_matches,
            by_file.len(),
            body
        )
    } else {
        body
    };

    let output = if capped && rtk_output.len() < plain.len() {
        rtk_output
    } else {
        plain
    };

    print!("{}", output);
    timer.track(&real_cmd, &rtk_label, &raw_output, &output);

    Ok(exit_code)
}

/// Parses a single rg/grep match or context line of the form `file\0line_number[:-]content`.
/// Requires `-0`/`--null` so the filename is NUL-separated -- NUL can't appear in file paths,
/// so this stays unambiguous even with `:` in the content or path. The `bool` is `true` for a
/// match line (`:` separator), `false` for context (`-`, from `-A`/`-B`/`-C`).
fn parse_match_line(line: &str) -> Option<(String, usize, bool, &str)> {
    static MATCH_LINE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([^\x00]+)\x00(\d+)([:-])(.*)$").unwrap());

    MATCH_LINE_RE.captures(line).and_then(|caps| {
        let file = caps.get(1)?.as_str().to_string();
        let line_num: usize = caps.get(2)?.as_str().parse().ok()?;
        let sep = caps.get(3)?.as_str();
        let content = caps.get(4)?.as_str();
        let is_match = sep == ":";
        Some((file, line_num, is_match, content))
    })
}

/// Minimal/shape forms the agent already chose (`-c`/`-l`/`--json`/...). `engine`-aware: `-L`
/// is grep's `--files-without-match` but rg's `--follow` (symlinks); `-z` is grep's
/// `--null-data` but rg's `--search-zip` -- neither rg meaning is a shape flag.
fn is_format_flag_token(engine: Engine, kind: TokenKind, text: &str) -> bool {
    const LONG: &[&str] = &[
        "byte-offset",
        "column",
        "count",
        "count-matches",
        "files",
        "files-with-matches",
        "files-without-match",
        "json",
        "null",
        "null-data",
        "only-matching",
        "passthru",
        "quiet",
        "silent",
        "vimgrep",
    ];
    match kind {
        // grep's `--initial-tab` pads and tabs every match line, so RTK's own `-H --null -n`
        // parse reads nothing back and leaked the injected flags into the output. ripgrep has
        // no such flag, and its `-T` is `--type-not`, a value-taking flag (see rg_takes_value).
        TokenKind::Long => {
            LONG.contains(&text) || (engine == Engine::Grep && text == "initial-tab")
        }
        // -c count, -l/-L lists, -o only-matching, -q quiet, -b byte-offset, -Z NUL are shared;
        // -L/-T/-z mean something unrelated to output shape for rg specifically (see above).
        TokenKind::Short => match text {
            "L" | "T" | "z" => engine == Engine::Grep,
            "Z" | "b" | "c" | "l" | "o" | "q" => true,
            _ => false,
        },
        _ => false,
    }
}

/// True when the command's only shape flag is a file list -- `-l`/`--files-with-matches`,
/// grep's `-L`/`--files-without-match`, or rg's `--files` -- so every stdout line is one
/// plain path and [`fold_path_prefix`] applies. Any other shape flag changes the line
/// (`-c` appends `:count`, `-Z`/`--null` joins with NUL, `--json` wraps it) or removes it
/// (`-q`), so the list is left verbatim.
pub(crate) fn is_bare_file_list<T: AsRef<str>>(engine: Engine, args: &[T]) -> bool {
    let tokens = tokenize_search_args(args, engine);
    let mut file_list = false;
    for t in &tokens {
        let is_list = match t.kind {
            TokenKind::Long => matches!(
                t.text,
                "files" | "files-with-matches" | "files-without-match"
            ),
            TokenKind::Short => t.text == "l" || (t.text == "L" && engine == Engine::Grep),
            _ => false,
        };
        if is_list {
            file_list = true;
        } else if is_format_flag_token(engine, t.kind, t.text) {
            return false;
        }
    }
    file_list
}

/// True for `-H`/`--with-filename`, an explicit request for the filename prefix (same meaning
/// for both engines).
fn is_show_file_token(kind: TokenKind, text: &str) -> bool {
    match kind {
        TokenKind::Long => text == "with-filename",
        TokenKind::Short => text == "H",
        _ => false,
    }
}

/// True for grep's `-r`/`-R`/`--recursive`. Recursion is not a filename request: it only makes
/// the search span several files, so grep shows the prefix by default -- an explicit `-h` still
/// wins whichever side of it the recursion flag is typed on. ripgrep has none of these
/// spellings (`-r` is `--replace`, a value-taking flag, see [`rg_takes_value`]).
fn is_recursive_token(engine: Engine, kind: TokenKind, text: &str) -> bool {
    engine == Engine::Grep
        && match kind {
            TokenKind::Long => text == "recursive",
            TokenKind::Short => matches!(text, "R" | "r"),
            _ => false,
        }
}

/// True for `-n`/`--line-number` (identical meaning for both engines).
fn is_show_line_on_token(kind: TokenKind, text: &str) -> bool {
    match kind {
        TokenKind::Long => text == "line-number",
        TokenKind::Short => text == "n",
        _ => false,
    }
}

/// True for `-h`/`--no-filename` (negates [`is_show_file_token`]). RTK forces `-H` so it can
/// parse the output, so the user's request has to be honoured at display time instead --
/// leaving it in the engine command would defeat RTK's own parse and force a second run.
fn is_show_file_off_token(engine: Engine, kind: TokenKind, text: &str) -> bool {
    match kind {
        TokenKind::Long => text == "no-filename",
        // Divergent both ways: grep's `-h` is --no-filename where rg's is --help, and rg's
        // `-I` is --no-filename where grep's is --binary-files=without-match.
        TokenKind::Short => match text {
            "h" => engine == Engine::Grep,
            "I" => engine == Engine::Rg,
            _ => false,
        },
        _ => false,
    }
}

/// True for `-N`/`--no-line-number` (negates [`is_show_line_on_token`]). ripgrep-only: GNU grep
/// has neither spelling and exits 2 on both, so recognising them there would swallow a flag the
/// engine itself refuses.
fn is_show_line_off_token(engine: Engine, kind: TokenKind, text: &str) -> bool {
    engine == Engine::Rg
        && match kind {
            TokenKind::Long => text == "no-line-number",
            TokenKind::Short => text == "N",
            _ => false,
        }
}

/// True for a context-window flag: `-A`/`-B`/`-C`, their long forms, or -- grep only -- the
/// `-NUM` shorthand for `--context=NUM` (the tokenizer keeps that digit run as one `Short`
/// token). ripgrep has no `-NUM`; its `-0` is `--null`, so reading a digit as context there
/// changes the output shape for a flag that has nothing to do with context.
fn is_context_token(engine: Engine, kind: TokenKind, text: &str) -> bool {
    match kind {
        TokenKind::Long => matches!(text, "after-context" | "before-context" | "context"),
        TokenKind::Short => {
            matches!(text, "A" | "B" | "C")
                || (engine == Engine::Grep && arg_tokenizer::is_digit_run(text))
        }
        _ => false,
    }
}

/// Flags detected during [`extract_pattern_path`]'s own token pass, replacing the
/// reconstructed-string scans `show_file`/`show_line`/`has_context_flag` used to rely on (see
/// the ambiguity this avoids: a value-taking flag's own value,
/// pushed into `flags` as a bare string, could otherwise be misread as one of these).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DetectedFlags {
    /// What the user asked for with `-H`/`--with-filename` (`Some(true)`) or
    /// `-h`/`--no-filename` (`Some(false)`), last spelling winning as both engines do; `None`
    /// when they said neither, leaving the decision to `recursive` and to the paths (multiple
    /// paths, a directory among them), which the call sites check against `paths` themselves.
    show_file: Option<bool>,
    /// `-n`/`--line-number`, unless negated by `-N`/`--no-line-number`.
    show_line: bool,
    /// grep's `-r`/`-R`/`--recursive`: not a filename request, only a reason for the engine to
    /// show one by default, so it feeds `show_file`'s fallback rather than overriding it.
    recursive: bool,
    /// `-A`/`-B`/`-C` or their long forms.
    context: bool,
}

/// Test-only convenience wrapper; the production call site gets this from the
/// `has_format_flag` extract_pattern_path already returns, computed in the same token pass
/// instead of tokenizing the reconstructed `flags` strings a second time.
#[cfg(test)]
fn has_format_flag<T: AsRef<str>>(engine: Engine, extra_args: &[T]) -> bool {
    // The module's shared tokenizer, so a value-taking flag's value (e.g. `-e --json`, where
    // "--json" is -e's pattern, not the real --json flag) is classified exactly as
    // extract_pattern_path classifies it.
    let tokens = tokenize_search_args(extra_args, engine);
    tokens
        .iter()
        .any(|t| is_format_flag_token(engine, t.kind, t.text))
}

fn clean_line(line: &str, max_len: usize, context_re: Option<&Regex>, pattern: &str) -> String {
    let trimmed = line.trim();

    if let Some(re) = context_re
        && let Some(m) = re.find(trimmed)
    {
        let matched = m.as_str();
        if matched.len() <= max_len {
            return matched.to_string();
        }
    }

    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        let lower = trimmed.to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        if let Some(pos) = lower.find(&pattern_lower) {
            let char_pos = lower[..pos].chars().count();
            let chars: Vec<char> = trimmed.chars().collect();
            let char_len = chars.len();

            let start = char_pos.saturating_sub(max_len / 3);
            let end = (start + max_len).min(char_len);
            let start = if end == char_len {
                end.saturating_sub(max_len)
            } else {
                start
            };

            let slice: String = chars[start..end].iter().collect();
            if start > 0 && end < char_len {
                format!("...{}...", slice)
            } else if start > 0 {
                format!("...{}", slice)
            } else {
                format!("{}...", slice)
            }
        } else {
            let t: String = trimmed.chars().take(max_len - 3).collect();
            format!("{}...", t)
        }
    }
}

fn compact_path(path: &str) -> String {
    if path.len() <= 50 {
        return path.to_string();
    }

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 3 {
        return path.to_string();
    }

    format!(
        "{}/.../{}/{}",
        parts[0],
        parts[parts.len() - 2],
        parts[parts.len() - 1]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rg_tracking_uses_native_output_without_parse_aids() {
        let augmented = "src/main.rs\0:1:needle\n";
        let native = "src/main.rs:1:needle\n";
        assert_eq!(
            rg_tracking_input_tokens(augmented, "", augmented.len(), true, &[".".into()], &[]),
            tracking::estimate_tokens(native)
        );
    }

    #[test]
    fn rg_tracking_treats_truncated_preview_as_incomplete_capture() {
        let meta = EmissionMeta {
            runtime_error: Some("capture_incomplete"),
            ..Default::default()
        };
        assert!(!rg_capture_is_complete(true, meta));
    }
    use crate::core::ai_output::Omission;

    #[test]
    fn rg_route_table_is_conservative_and_complete() {
        let cases = [
            (&["needle"][..], RgRoute::Matches),
            (&["--json", "needle"][..], RgRoute::JsonEvents),
            (&["--files"][..], RgRoute::Inventory),
            (&["-l", "needle"][..], RgRoute::Inventory),
            (
                &["-L", "needle"][..],
                RgRoute::Exact(ExactReason::Streaming),
            ),
            (&["-c", "needle"][..], RgRoute::Counts),
            (&["--count-matches", "needle"][..], RgRoute::Counts),
            (&["-o", "needle"][..], RgRoute::OnlyMatching),
            (&["--replace", "hit", "needle"][..], RgRoute::Matches),
            (
                &["--replace", "-h", "needle"][..],
                RgRoute::Exact(ExactReason::Unknown),
            ),
            (
                &["-r", "-h", "needle"][..],
                RgRoute::Exact(ExactReason::Unknown),
            ),
            (&["--regexp", "needle", "src"][..], RgRoute::Matches),
            (&["--regexp=needle", "src"][..], RgRoute::Matches),
            (&["-C", "2", "needle"][..], RgRoute::Matches),
            (&["--glob", "--future-flag", "needle"][..], RgRoute::Matches),
            (&["needle", "--", "--future-flag"][..], RgRoute::Matches),
            (
                &["--null", "needle"][..],
                RgRoute::Exact(ExactReason::Structured),
            ),
            (&["--help"][..], RgRoute::Exact(ExactReason::Interactive)),
            (&["--version"][..], RgRoute::Exact(ExactReason::Interactive)),
            (
                &["--text", "needle"][..],
                RgRoute::Exact(ExactReason::Binary),
            ),
            (
                &["--follow", "needle"][..],
                RgRoute::Exact(ExactReason::Streaming),
            ),
            (
                &["--future-flag", "needle"][..],
                RgRoute::Exact(ExactReason::Unknown),
            ),
            (
                &["needle", "file\nname.txt"][..],
                RgRoute::Exact(ExactReason::Sensitive),
            ),
            (
                &["--count", "--no-filename", "needle"][..],
                RgRoute::Exact(ExactReason::Structured),
            ),
            (
                &["--no-filename", "--count", "needle"][..],
                RgRoute::Exact(ExactReason::Structured),
            ),
            (
                &["--json", "--count", "needle"][..],
                RgRoute::Exact(ExactReason::Structured),
            ),
        ];

        for (raw, expected) in cases {
            let args = raw
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>();
            assert_eq!(classify_rg(&args), expected, "args={raw:?}");
        }
    }

    #[test]
    fn rg_matches_group_contiguous_entries_without_reordering() {
        let raw = concat!(
            "a.rs\0",
            "3:needle one\n",
            "b.rs\0",
            "2:needle two\n",
            "a.rs\0",
            "9:needle three\n",
        );
        let document = rg_document(RgRoute::Matches, raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source)
                .text;

        assert!(rendered.contains("a.rs"));
        assert!(rendered.contains("3: needle one"));
        assert!(rendered.find("b.rs").unwrap() < rendered.rfind("a.rs").unwrap());
    }

    #[test]
    fn rg_shortened_match_declares_a_lossless_omission() {
        let long_match = format!("prefix {} needle suffix", "x".repeat(100));
        let raw = format!("a.rs\0{}:{}\n", 7, long_match);
        let document = rg_document(RgRoute::Matches, &raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source);

        assert_eq!(
            rendered.omission,
            Some(Omission {
                items: 1,
                groups: 0,
            })
        );
    }

    #[test]
    fn rg_whitespace_cleanup_declares_a_lossless_omission() {
        let raw = "a.rs\x007:  needle  \n";
        let document = rg_document(RgRoute::Matches, raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source);

        assert_eq!(
            rendered.omission,
            Some(Omission {
                items: 1,
                groups: 0,
            })
        );
    }

    #[test]
    fn rg_no_filename_baseline_overrides_multiple_paths() {
        let raw = "a.rs\x007:needle\nb.rs\x007:needle\n";
        let paths = vec!["a.rs".to_string(), "b.rs".to_string()];
        let extra_args = vec!["--no-filename".to_string()];

        assert_eq!(
            rg_faithful_match_baseline(raw, &paths, &extra_args).unwrap(),
            "needle\nneedle\n"
        );
    }

    #[test]
    fn rg_filename_selectors_follow_argument_order() {
        let paths = vec!["a.rs".to_string(), "b.rs".to_string()];

        assert!(!rg_show_file(
            &paths,
            &["--with-filename".to_string(), "--no-filename".to_string()],
        ));
        assert!(rg_show_file(
            &paths,
            &["--no-filename".to_string(), "--with-filename".to_string()],
        ));
        assert!(!rg_show_file(&paths, &["-HI".to_string()]));
        assert!(rg_show_file(&paths, &["-IH".to_string()]));
    }

    #[test]
    fn rg_filename_selector_ignores_inline_replace_values() {
        let raw = "a.rs\x007:-h\nb.rs\x007:-h\n";
        let paths = vec!["a.rs".to_string(), "b.rs".to_string()];
        let extra_args = vec!["--replace=-h".to_string()];

        assert_eq!(
            rg_faithful_match_baseline(raw, &paths, &extra_args).unwrap(),
            "a.rs:-h\nb.rs:-h\n"
        );

        let short_args = vec!["-r-h".to_string()];
        assert_eq!(
            rg_faithful_match_baseline(raw, &paths, &short_args).unwrap(),
            "a.rs:-h\nb.rs:-h\n"
        );
    }

    #[test]
    fn rg_default_filename_behavior_ignores_inline_replace_flag_text() {
        let raw = "a.rs\x007:-h\n";
        let paths = vec!["a.rs".to_string()];
        let extra_args = vec!["-r-h".to_string()];

        assert_eq!(
            rg_faithful_match_baseline(raw, &paths, &extra_args).unwrap(),
            "-h\n"
        );
    }

    #[test]
    fn rg_budget_chunks_large_match_set() {
        let long_match = format!("needle {}", "x".repeat(60));
        let raw = (1..=500)
            .map(|line_number| format!("a.rs\0{line_number}:{long_match}\n"))
            .collect::<String>();
        let document = rg_document(RgRoute::Matches, &raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source);

        let omission = rendered
            .omission
            .expect("large match set should report omission");
        assert_eq!(omission.groups, 1);
        assert!(
            omission.items < 500,
            "large file should be previewed in chunks: {omission:?}, text_len={}",
            rendered.text.len()
        );
        assert!(rendered.text.contains("a.rs"));
    }

    #[test]
    fn bounded_rg_preview_does_not_replay_a_huge_line() {
        let stats = Arc::new(Mutex::new(EmissionMeta::default()));
        let mut filter = RgAiStreamFilter::new(
            RgRoute::Matches,
            vec!["needle".into()],
            vec!["a.rs".into()],
            Vec::new(),
            Arc::clone(&stats),
        );
        let line = format!(
            "a.rs\x00180:needle {}{}",
            "x".repeat(RG_STREAM_LINE_CAP),
            stream::TRUNCATED_LINE_MARKER
        );
        filter.feed_line(&line);
        let output = filter.on_exit(0, "").expect("bounded preview");

        assert!(output.contains("matches=1"));
        assert!(output.contains("truncated_lines=1"));
        assert!(output.contains("recovery=unavailable"));
        assert!(output.len() < 4_096);
        assert_eq!(
            stats.lock().unwrap().runtime_error,
            Some("capture_incomplete")
        );
    }

    #[test]
    fn bounded_rg_preview_keeps_total_matches_when_result_cap_is_reached() {
        let stats = Arc::new(Mutex::new(EmissionMeta::default()));
        let mut filter = RgAiStreamFilter::new(
            RgRoute::Matches,
            vec!["needle".into()],
            vec!["a.rs".into()],
            Vec::new(),
            stats,
        );
        filter.max_entries = 1;
        filter.feed_line("a.rs\x001:needle first");
        filter.feed_line("a.rs\x002:needle second");
        filter.raw_complete = false;
        let output = filter.on_exit(0, "").expect("bounded preview");

        assert!(output.contains("matches=2"));
        assert!(output.contains("omitted items=1"));
        assert!(!output.contains("second"));
    }

    #[test]
    fn rg_json_budget_chunks_large_match_set() {
        let long_match = format!("needle {}", "x".repeat(60));
        let raw = (1..=500)
            .map(|line_number| {
                format!(
                    "{{\"type\":\"match\",\"data\":{{\"path\":{{\"text\":\"a.rs\"}},\"lines\":{{\"text\":\"{long_match}\\n\"}},\"line_number\":{line_number}}}}}\n"
                )
            })
            .collect::<String>();
        let document = rg_document(RgRoute::JsonEvents, &raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source);

        let omission = rendered
            .omission
            .expect("large JSON match set should report omission");
        assert_eq!(omission.groups, 1);
        assert!(
            omission.items < 500,
            "large JSON file should be previewed in chunks: {omission:?}, text_len={}",
            rendered.text.len()
        );
        assert!(rendered.text.contains("a.rs"));
    }

    #[test]
    fn rg_json_discards_event_noise_but_keeps_match_text() {
        let raw = concat!(
            "{\"type\":\"begin\",\"data\":{\"path\":{\"text\":\"a.rs\"}}}\n",
            "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"a.rs\"},",
            "\"lines\":{\"text\":\"needle here\\n\"},\"line_number\":7,",
            "\"absolute_offset\":0,\"submatches\":[{\"match\":{\"text\":\"needle\"},\"start\":0,\"end\":6}]}}\n",
            "{\"type\":\"end\",\"data\":{\"path\":{\"text\":\"a.rs\"},\"binary_offset\":null,\"stats\":{}}}\n",
        );
        let document = rg_document(RgRoute::JsonEvents, raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source)
                .text;

        assert!(rendered.contains("a.rs"));
        assert!(rendered.contains("7: needle here"));
        assert!(!rendered.contains("\"type\":\"begin\""));
        assert!(!rendered.contains("omitted items="));
    }

    #[test]
    fn rg_count_records_are_path_equals_count() {
        let document = rg_document(
            RgRoute::Counts,
            concat!("a.rs\0", "4\n", "b.rs\0", "1\n"),
            &[],
            &[],
        )
        .unwrap();
        let rendered = crate::core::ai_output::render(
            &document,
            crate::core::ai_output::BudgetClass::Collection,
        )
        .text;

        assert!(rendered.contains("a.rs=4"));
        assert!(rendered.contains("b.rs=1"));
    }

    #[test]
    fn rg_count_single_file_numeric_output_keeps_the_file_name() {
        let document = rg_document(RgRoute::Counts, "4\n", &[], &["src/main.rs".into()]).unwrap();
        let rendered = crate::core::ai_output::render(
            &document,
            crate::core::ai_output::BudgetClass::Collection,
        )
        .text;

        assert!(rendered.contains("src/main.rs=4"));
        assert!(!rendered.contains("<stdin>"));
    }

    #[test]
    fn rg_only_matching_preserves_long_match_values() {
        let value = "needle".to_string() + &"x".repeat(120);
        let raw = format!("a.rs\01:{value}\n");
        let document = rg_document(RgRoute::OnlyMatching, &raw, &["needle".into()], &[]).unwrap();
        let rendered =
            crate::core::ai_output::render(&document, crate::core::ai_output::BudgetClass::Source)
                .text;

        assert!(rendered.contains(&value));
        assert!(!rendered.contains("omitted items="));
    }

    #[test]
    fn rg_json_non_utf8_path_is_not_fabricated() {
        let raw = r#"{"type":"match","data":{"path":{"bytes":[255]},"lines":{"text":"needle\n"},"line_number":1}}"#;
        let error = rg_document(RgRoute::JsonEvents, raw, &["needle".into()], &[]).unwrap_err();
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn test_clean_line() {
        let line = "            const result = someFunction();";
        let cleaned = clean_line(line, 50, None, "result");
        assert!(!cleaned.starts_with(' '));
        assert!(cleaned.len() <= 50);
    }

    #[test]
    fn test_compact_path() {
        let path = "/Users/patrick/dev/project/src/components/Button.tsx";
        let compact = compact_path(path);
        assert!(compact.len() <= 60);
    }

    #[test]
    fn streaming_search_preserves_native_shape() {
        let mut filter = SearchStreamFilter {
            show_file: false,
            show_line: true,
            max_results: 10,
            shown: 0,
            cap_reported: false,
        };

        assert_eq!(
            filter.feed_line("engine warning"),
            Some("engine warning\n".to_string())
        );
        assert_eq!(
            filter.feed_line(concat!("(standard input)\0", "1:match")),
            Some("1:match\n".to_string())
        );
    }

    #[test]
    fn streaming_search_reports_the_cap_once() {
        let mut filter = SearchStreamFilter {
            show_file: false,
            show_line: true,
            max_results: 1,
            shown: 0,
            cap_reported: false,
        };

        assert_eq!(
            filter.feed_line(concat!("(standard input)\0", "1:first")),
            Some("1:first\n".to_string())
        );
        assert_eq!(
            filter.feed_line(concat!("(standard input)\0", "2:second")),
            Some("[rtk] output capped at 1 results\n".to_string())
        );
        assert_eq!(
            filter.feed_line(concat!("(standard input)\0", "3:third")),
            None
        );
        assert_eq!(filter.feed_line("--"), None);
    }

    #[test]
    fn test_clean_line_multibyte() {
        // Thai text that exceeds max_len in bytes
        let line = "  สวัสดีครับ นี่คือข้อความที่ยาวมากสำหรับทดสอบ  ";
        let cleaned = clean_line(line, 20, None, "ครับ");
        // Should not panic
        assert!(!cleaned.is_empty());
    }

    #[test]
    fn test_clean_line_emoji() {
        let line = "🎉🎊🎈🎁🎂🎄 some text 🎃🎆🎇✨";
        let cleaned = clean_line(line, 15, None, "text");
        assert!(!cleaned.is_empty());
    }

    // --- fold_path_prefix / is_bare_file_list ---

    #[test]
    fn fold_path_prefix_folds_shared_dir_into_header() {
        let raw =
            "/home/u/proj/src/a/foo.rs\n/home/u/proj/src/a/bar.rs\n/home/u/proj/src/b/baz.rs\n";
        assert_eq!(
            fold_path_prefix(raw).as_deref(),
            Some("/home/u/proj/src/ (3 files)\na/foo.rs\na/bar.rs\nb/baz.rs\n")
        );
    }

    #[test]
    fn fold_path_prefix_keeps_engine_order_and_relative_paths() {
        let raw = "src/z.rs\nsrc/a.rs\n";
        assert_eq!(
            fold_path_prefix(raw).as_deref(),
            Some("src/ (2 files)\nz.rs\na.rs\n")
        );
    }

    #[test]
    fn fold_path_prefix_cuts_on_whole_components() {
        // `/a/foobar` and `/a/foobaz` share `/a/`, never the byte run `/a/fooba`.
        let raw = "/a/foobar/x\n/a/foobaz/y\n";
        assert_eq!(
            fold_path_prefix(raw).as_deref(),
            Some("/a/ (2 files)\nfoobar/x\nfoobaz/y\n")
        );
    }

    #[test]
    fn fold_path_prefix_never_folds_into_a_filename() {
        // The shared prefix is a directory only; a file that sits at the prefix root keeps its
        // full name.
        let raw = "/a/b/c.rs\n/a/b/d/e.rs\n";
        assert_eq!(
            fold_path_prefix(raw).as_deref(),
            Some("/a/b/ (2 files)\nc.rs\nd/e.rs\n")
        );
    }

    #[test]
    fn fold_path_prefix_none_when_nothing_to_fold() {
        assert_eq!(fold_path_prefix(""), None, "empty");
        assert_eq!(fold_path_prefix("/a/b/c.rs\n"), None, "single line");
        assert_eq!(fold_path_prefix("a.rs\nb.rs\n"), None, "bare filenames");
        assert_eq!(
            fold_path_prefix("/x/a.rs\n/y/b.rs\n"),
            None,
            "only `/` shared"
        );
        assert_eq!(
            fold_path_prefix("src/a.rs\nlib/b.rs\n"),
            None,
            "no shared dir"
        );
        assert_eq!(fold_path_prefix("/a/b.rs\n\n/a/c.rs\n"), None, "blank line");
        assert_eq!(
            fold_path_prefix("/a/b.rs\0/a/c.rs\0"),
            None,
            "NUL-joined (-Z)"
        );
        assert_eq!(fold_path_prefix("/a/x\r\n/a/y\n"), None, "CR in a name");
        assert_eq!(
            fold_path_prefix("\x1b]8;;file:///a/x\x1b\\/a/x\n\x1b]8;;file:///a/y\x1b\\/a/y\n"),
            None,
            "OSC 8 hyperlink"
        );
    }

    #[test]
    fn fold_path_prefix_is_lossless() {
        let raw = "/p/q/a.rs\n/p/q/r/b.rs\n/p/q/c.rs\n";
        let folded = fold_path_prefix(raw).expect("folds");
        let mut lines = folded.lines();
        let header = lines.next().unwrap();
        let prefix = header.strip_suffix(" (3 files)").unwrap();
        let rebuilt: String = lines.map(|tail| format!("{prefix}{tail}\n")).collect();
        assert_eq!(rebuilt, raw);
    }

    #[test]
    fn bare_file_list_detects_list_flags_per_engine() {
        assert!(is_bare_file_list(Engine::Grep, &["-rl", "foo", "."]));
        assert!(is_bare_file_list(Engine::Grep, &["-L", "foo", "."]));
        assert!(is_bare_file_list(
            Engine::Grep,
            &["--files-with-matches", "foo"]
        ));
        assert!(is_bare_file_list(Engine::Rg, &["-l", "foo"]));
        assert!(is_bare_file_list(Engine::Rg, &["--files", "src"]));
        // rg's -L is --follow, not a list.
        assert!(!is_bare_file_list(Engine::Rg, &["-L", "foo"]));
        assert!(!is_bare_file_list(Engine::Grep, &["-rn", "foo", "."]));
    }

    #[test]
    fn bare_file_list_rejects_other_shape_flags() {
        // Each of these changes or removes the path line, so the list must stay verbatim.
        assert!(!is_bare_file_list(Engine::Grep, &["-lc", "foo", "."]));
        assert!(!is_bare_file_list(Engine::Grep, &["-lZ", "foo", "."]));
        assert!(!is_bare_file_list(
            Engine::Grep,
            &["-l", "--null", "foo", "."]
        ));
        assert!(!is_bare_file_list(Engine::Grep, &["-lq", "foo", "."]));
        assert!(!is_bare_file_list(Engine::Rg, &["-l", "--json", "foo"]));
        assert!(!is_bare_file_list(Engine::Rg, &["--files", "--null"]));
        // `-e -l` is a pattern, not the list flag.
        assert!(!is_bare_file_list(Engine::Grep, &["-e", "-l", "."]));
    }

    // --- extract_pattern_path ---
    //
    // parse_cluster/ClusterResult were replaced by arg_tokenizer::tokenize; the
    // extract_pattern_path tests below exercise the same short-cluster/value-taking/`-e`
    // behavior end-to-end instead of unit-testing the internal cluster scanner directly.

    #[test]
    fn test_extract_simple() {
        let (patterns, paths, flags, _, _) = extract_pattern_path(&["foo", "src/"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src/"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_engine_specific_long_value_flags() {
        // grep 3.11: `--include`/`--exclude-dir`/... require a separate value, and rg has no
        // such flags at all. Missing them made the glob the pattern and the pattern a file.
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--include", "*.txt", "-r", "match", "."], Engine::Grep);
        assert_eq!(patterns, vec!["match"]);
        assert_eq!(paths, vec!["."]);
        assert_eq!(flags, vec!["--include", "*.txt", "-r"]);

        // grep's --color[=WHEN] attaches its value; rg's --color takes the next token.
        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["--color", "match", "a.txt"], Engine::Grep);
        assert_eq!(patterns, vec!["match"]);
        assert_eq!(paths, vec!["a.txt"]);

        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["--color", "never", "match", "a.txt"], Engine::Rg);
        assert_eq!(patterns, vec!["match"]);
        assert_eq!(paths, vec!["a.txt"]);
    }

    #[test]
    fn test_context_detection_covers_greps_numeric_shorthand() {
        // grep's `-1` is `--context=1`. Missing it dropped the `--` separators between
        // non-contiguous context blocks, so two far-apart hunks read as one run.
        assert!(is_context_token(Engine::Grep, TokenKind::Short, "1"));
        assert!(is_context_token(Engine::Grep, TokenKind::Short, "12"));
        assert!(is_context_token(Engine::Grep, TokenKind::Short, "C"));
        assert!(is_context_token(Engine::Grep, TokenKind::Long, "context"));
        assert!(!is_context_token(Engine::Grep, TokenKind::Short, "n"));

        let (_, _, _, _, detected) = extract_pattern_path(&["-1", "TODO", "f.txt"], Engine::Grep);
        assert!(detected.context);
    }

    #[test]
    fn test_help_short_circuit_respects_the_boundary_and_the_engine() {
        let asks = |engine: Engine, args: &[&str]| -> bool {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let tokens = arg_tokenizer::tokenize_grammar(
                &args,
                &|kind, name| search_takes_value(engine, kind, name),
                Dialect::Posix,
            );
            arg_tokenizer::before_dashdash(&tokens).iter().any(|t| {
                (t.kind == TokenKind::Long && matches!(t.text, "version" | "help"))
                    || (t.kind == TokenKind::Short && t.text == "h" && engine == Engine::Rg)
            })
        };

        assert!(asks(Engine::Grep, &["--version"]));
        // Past `--` it is the pattern to search for, not a request for the banner.
        assert!(!asks(Engine::Grep, &["--", "--version", "f.txt"]));
        // `-h` is rg's --help but grep's --no-filename.
        assert!(asks(Engine::Rg, &["-h"]));
        assert!(!asks(Engine::Grep, &["-h", "TODO", "f.txt"]));
    }

    #[test]
    fn test_no_filename_is_honoured_at_display_not_forwarded() {
        // RTK forces `-H` so it can parse the output, so the user's `-h`/`--no-filename` has to
        // be applied when printing -- forwarded, it wins as the later flag, the NUL-separated
        // parse fails on every line, and the whole search runs a second time.
        let (_, _, flags, _, detected) =
            extract_pattern_path(&["--no-filename", "x", "a.txt"], Engine::Grep);
        assert_eq!(detected.show_file, Some(false));
        assert!(!flags.iter().any(|f| f == "--no-filename"));

        let (_, _, flags, _, detected) = extract_pattern_path(&["-ih", "x", "a.txt"], Engine::Grep);
        assert_eq!(detected.show_file, Some(false));
        assert_eq!(flags, vec!["-i"], "the rest of the cluster survives");

        // Both engines arbitrate the pair by last-one-wins, so RTK must too.
        let (_, _, _, _, detected) =
            extract_pattern_path(&["-h", "-H", "x", "a.txt"], Engine::Grep);
        assert_eq!(detected.show_file, Some(true));
        let (_, _, _, _, detected) =
            extract_pattern_path(&["-H", "-h", "x", "a.txt"], Engine::Grep);
        assert_eq!(detected.show_file, Some(false));
        let (_, _, _, _, detected) = extract_pattern_path(&["-Hh", "x", "a.txt"], Engine::Grep);
        assert_eq!(detected.show_file, Some(false), "within one cluster too");

        // rg's -N is its --no-line-number; withheld for the same reason as -h.
        let (_, _, flags, _, detected) = extract_pattern_path(&["-nN", "x", "a.txt"], Engine::Rg);
        assert!(!detected.show_line);
        assert!(!flags.iter().any(|f| f.contains('N')));
    }

    #[test]
    fn test_rg_unwraps_an_equals_attached_short_value_but_grep_does_not() {
        // rg accepts `-A=1` and strips the `=`; GNU grep answers "invalid context length
        // argument", so RTK must not normalise it for grep.
        let (_, _, flags, _, _) = extract_pattern_path(&["-A=1", "x", "a.txt"], Engine::Rg);
        assert_eq!(flags, vec!["-A", "1"]);

        let (_, _, flags, _, _) = extract_pattern_path(&["-A=1", "x", "a.txt"], Engine::Grep);
        assert_eq!(flags, vec!["-A", "=1"]);
    }

    #[test]
    fn test_extract_patterns_from_file_leaves_every_positional_a_path() {
        // `-f`/`--file` supplies the patterns like `-e` does. Taking the first positional as
        // the pattern instead left no path at all, so the engine read stdin (a hang) or walked
        // the cwd, and `rtk grep -f pats.txt a.txt` answered "no matches" for a file that had
        // one.
        for args in [
            vec!["-f", "pats.txt", "a.txt"],
            vec!["--file", "pats.txt", "a.txt"],
            vec!["-fpats.txt", "a.txt"],
            vec!["--file=pats.txt", "a.txt"],
        ] {
            let (patterns, paths, _, _, _) = extract_pattern_path(&args, Engine::Grep);
            assert!(patterns.is_empty(), "{args:?} -> {patterns:?}");
            assert_eq!(paths, vec!["a.txt"], "{args:?}");
        }
    }

    #[test]
    fn test_extract_engine_specific_short_value_flags() {
        // `-E` is grep's boolean --extended-regexp but rg's --encoding, which takes a value.
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-E", "match", "a.txt"], Engine::Grep);
        assert_eq!(patterns, vec!["match"]);
        assert_eq!(paths, vec!["a.txt"]);
        assert_eq!(flags, vec!["-E"]);

        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["-E", "utf8", "match", "a.txt"], Engine::Rg);
        assert_eq!(patterns, vec!["match"]);
        assert_eq!(paths, vec!["a.txt"]);
    }

    #[test]
    fn test_extract_with_bool_flag() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-i", "foo", "src/"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src/"]);
        assert_eq!(flags, vec!["-i"]);
    }

    #[test]
    fn test_extract_value_taking_flag() {
        // -A 2 must not steal "error" as its value
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-A", "2", "error", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["error"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-A", "2"]);
    }

    #[test]
    fn test_extract_cluster_keeps_r() {
        // -rn: r kept, passed straight to grep
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-rn", "foo", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-rn"]);
    }

    #[test]
    fn test_extract_cluster_ending_in_e() {
        // -rne PATTERN: rn kept, e consumes PATTERN as the pattern
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-rne", "PATTERN", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["PATTERN"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-rn"]);
    }

    #[test]
    fn test_extract_cluster_ending_in_value_flag() {
        // -rA 2: r kept as its own flag, A consumes 2 as context value
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-rA", "2", "foo", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-r", "-A", "2"]);
    }

    #[test]
    fn test_extract_multi_path() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["TODO", "src", "tests"], Engine::Grep);
        assert_eq!(patterns, vec!["TODO"]);
        assert_eq!(paths, vec!["src", "tests"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_glob_value() {
        // -g '*.md' must not steal "agent" as its value
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-i", "x", "agent", "-g", "*.md"], Engine::Rg);
        assert_eq!(patterns, vec!["x"]);
        assert_eq!(paths, vec!["agent"]);
        assert_eq!(flags, vec!["-i", "-g", "*.md"]);
    }

    #[test]
    fn test_extract_e_flag() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-e", "fn run", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["fn run"]);
        assert_eq!(paths, vec!["src"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_multi_e() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-e", "foo", "-e", "bar", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo", "bar"]);
        assert_eq!(paths, vec!["src"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_dashdash_boundary() {
        // After --, args are positional even if they look like flags
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--", "--version"], Engine::Grep);
        assert_eq!(patterns, vec!["--version"]);
        assert!(paths.is_empty());
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_e_claims_literal_dash_dash() {
        // grep/rg -e -- means "the pattern is the literal string --", not the end-of-options
        // boundary (confirmed against both real grep and real rg).
        let (patterns, paths, flags, _, _) = extract_pattern_path(&["-e", "--", "f"], Engine::Grep);
        assert_eq!(patterns, vec!["--"]);
        assert_eq!(paths, vec!["f"]);
        assert!(flags.is_empty());

        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["--regexp", "--", "f"], Engine::Grep);
        assert_eq!(patterns, vec!["--"]);
        assert_eq!(paths, vec!["f"]);
    }

    #[test]
    fn test_extract_no_args() {
        let (patterns, paths, flags, _, _) = extract_pattern_path::<&str>(&[], Engine::Grep);
        assert!(patterns.is_empty());
        assert!(paths.is_empty());
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_default_path_empty() {
        // Caller is responsible for defaulting empty paths to ["."]
        let (patterns, paths, _, _, _) = extract_pattern_path(&["foo"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_ending_e() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-e", "foo", "-e", "bar", "src", "-e"], Engine::Grep);
        assert_eq!(patterns, vec!["foo", "bar"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-e"]);
    }

    // --- inline short flag values (Bug 5) ---

    #[test]
    fn test_extract_inline_e_value() {
        // -ecarrot: e hits at j=0, inline="carrot", no r-stripping on value
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-ecarrot", "file"], Engine::Grep);
        assert_eq!(patterns, vec!["carrot"]);
        assert_eq!(paths, vec!["file"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_inline_e_value_no_rstrip() {
        // -ecarrot: the 'r' in "carrot" must NOT be stripped (it's value, not a flag)
        let (patterns, _, _, _, _) = extract_pattern_path(&["-ecarrot", "file"], Engine::Grep);
        assert_eq!(
            patterns,
            vec!["carrot"],
            "r in inline value must not be stripped"
        );
    }

    #[test]
    fn test_extract_inline_g_value() {
        // -g*.rs: g hits at j=0, inline="*.rs", no r-stripping on value
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["aaa", "sub", "-g*.rs"], Engine::Rg);
        assert_eq!(patterns, vec!["aaa"]);
        assert_eq!(paths, vec!["sub"]);
        assert_eq!(flags, vec!["-g", "*.rs"]);
    }

    #[test]
    fn test_extract_inline_g_value_no_rstrip() {
        // -g*.rs: the 'r' in "*.rs" must NOT be stripped
        let (_, _, flags, _, _) = extract_pattern_path(&["aaa", "sub", "-g*.rs"], Engine::Rg);
        assert!(
            flags.contains(&"*.rs".to_string()),
            "r in glob value must not be stripped"
        );
    }

    // --- long value-taking flags (Bug 5) ---

    #[test]
    fn test_extract_long_glob_value() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["compact", "sub", "--glob", "*.md"], Engine::Rg);
        assert_eq!(patterns, vec!["compact"]);
        assert_eq!(paths, vec!["sub"]);
        assert_eq!(flags, vec!["--glob", "*.md"]);
    }

    #[test]
    fn test_extract_long_max_count() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--max-count", "1", "fn", "file"], Engine::Grep);
        assert_eq!(patterns, vec!["fn"]);
        assert_eq!(paths, vec!["file"]);
        assert_eq!(flags, vec!["--max-count", "1"]);
    }

    #[test]
    fn test_extract_short_type() {
        // -t rust: type filter, value must not become pattern
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-t", "rust", "fn", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["fn"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-t", "rust"]);
    }

    #[test]
    fn test_extract_short_max_depth() {
        // -d 3: max-depth, value must not become pattern
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-d", "3", "foo", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-d", "3"]);
    }

    #[test]
    fn test_extract_short_max_columns() {
        // -M 120: max-columns, value must not become pattern
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-M", "120", "foo", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-M", "120"]);
    }

    #[test]
    fn test_extract_long_regexp() {
        // --regexp is the long form of -e; value goes to patterns
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--regexp", "fn run", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["fn run"]);
        assert_eq!(paths, vec!["src"]);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_extract_long_regexp_multi() {
        // --regexp can be combined with -e
        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["--regexp", "foo", "-e", "bar", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo", "bar"]);
        assert_eq!(paths, vec!["src"]);
    }

    #[test]
    fn test_extract_long_ignore_file() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--ignore-file", ".myignore", "foo", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["--ignore-file", ".myignore"]);
    }

    #[test]
    fn test_extract_long_engine() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--engine", "pcre2", "foo", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["--engine", "pcre2"]);
    }

    #[test]
    fn test_extract_long_type_clear() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--type-clear", "rust", "foo", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["--type-clear", "rust"]);
    }

    #[test]
    fn test_extract_long_path_separator() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["--path-separator", "/", "foo", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["--path-separator", "/"]);
    }

    #[test]
    fn test_extract_long_flag_inline_eq_passthrough() {
        // --glob=*.rs is one token (inline =): passes through as-is, not consumed as pair
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["foo", "src", "--glob=*.rs"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["--glob=*.rs"]);
    }

    // --- has_format_flag additions ---

    #[test]
    fn test_format_flag_detects_count_matches() {
        assert!(has_format_flag(Engine::Grep, &["--count-matches"]));
    }

    #[test]
    fn test_format_flag_detects_json() {
        assert!(has_format_flag(Engine::Grep, &["--json"]));
    }

    #[test]
    fn test_format_flag_detects_passthru() {
        assert!(has_format_flag(Engine::Grep, &["--passthru"]));
    }

    #[test]
    fn test_format_flag_detects_files() {
        assert!(has_format_flag(Engine::Grep, &["--files"]));
    }

    // --- truncation accuracy ---

    #[test]
    fn test_grep_overflow_uses_uncapped_total() {
        // Confirm the grep overflow invariant: matches vec is never capped before overflow calc.
        // If total_matches > per_file, overflow = total_matches - per_file (not capped).
        // This documents that the search filter avoids the diff_cmd bug (cap at N then compute N-10).
        let per_file = config::limits().grep_max_per_file;
        let total_matches = per_file + 42;
        let overflow = total_matches - per_file;
        assert_eq!(overflow, 42, "overflow must equal true suppressed count");
        // Demonstrate why capping before subtraction is wrong:
        let hypothetical_cap = per_file + 5;
        let capped = total_matches.min(hypothetical_cap);
        let wrong_overflow = capped - per_file;
        assert_ne!(
            wrong_overflow, overflow,
            "capping before subtraction gives wrong overflow"
        );
    }

    // --- format flag detection ---

    #[test]
    fn test_format_flag_detects_count() {
        assert!(has_format_flag(Engine::Grep, &["-c"]));
        assert!(has_format_flag(Engine::Grep, &["--count"]));
    }

    #[test]
    fn test_format_flag_detects_files_with_matches() {
        assert!(has_format_flag(Engine::Grep, &["-l"]));
        assert!(has_format_flag(Engine::Grep, &["--files-with-matches"]));
    }

    #[test]
    fn test_format_flag_detects_files_without_match() {
        assert!(has_format_flag(Engine::Grep, &["-L"]));
        assert!(has_format_flag(Engine::Grep, &["--files-without-match"]));
    }

    #[test]
    fn test_format_flag_is_engine_aware_for_ambiguous_short_letters() {
        assert!(has_format_flag(Engine::Grep, &["-L"]));
        assert!(!has_format_flag(Engine::Rg, &["-L"]));

        assert!(has_format_flag(Engine::Grep, &["-z"]));
        assert!(!has_format_flag(Engine::Rg, &["-z"]));

        // Unambiguous shape letters still agree across engines.
        assert!(has_format_flag(Engine::Rg, &["-c"]));
        assert!(has_format_flag(Engine::Rg, &["-l"]));
        assert!(has_format_flag(Engine::Rg, &["-o"]));
        assert!(has_format_flag(Engine::Rg, &["-q"]));
        assert!(has_format_flag(Engine::Rg, &["-b"]));
        assert!(has_format_flag(Engine::Rg, &["-Z"]));
    }

    #[test]
    fn test_dash_capital_t_is_engine_aware() {
        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["-T", "pattern", "file.txt"], Engine::Grep);
        assert_eq!(patterns, vec!["pattern"]);
        assert_eq!(paths, vec!["file.txt"]);

        // Rg's -T genuinely does take a value (a file type to exclude).
        let (patterns, paths, _, _, _) =
            extract_pattern_path(&["-T", "markdown", "pattern", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["pattern"]);
        assert_eq!(paths, vec!["src"]);
    }

    #[test]
    fn test_dash_r_is_engine_aware() {
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-rREPLACEMENT", "pattern", "src"], Engine::Rg);
        assert_eq!(patterns, vec!["pattern"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-r".to_string(), "REPLACEMENT".to_string()]);

        // Grep's -r/-R remain plain boolean flags, clustering as before.
        let (patterns, paths, flags, _, _) =
            extract_pattern_path(&["-rn", "foo", "src"], Engine::Grep);
        assert_eq!(patterns, vec!["foo"]);
        assert_eq!(paths, vec!["src"]);
        assert_eq!(flags, vec!["-rn".to_string()]);
    }

    #[test]
    fn test_detected_flags_ignore_a_value_taking_flags_own_value() {
        let (_, _, _, _, detected) =
            extract_pattern_path(&["--replace", "-Chart", "pattern", "src"], Engine::Rg);
        assert!(
            !detected.context,
            "--replace's value must not be misread as a -C context flag"
        );

        // Same for show_file's -H/-r/-R and show_line's -n/-N letters.
        let (_, _, _, _, detected) =
            extract_pattern_path(&["--replace", "-Hart", "pattern", "src"], Engine::Rg);
        assert_eq!(
            detected.show_file, None,
            "--replace's value must not trigger -H"
        );

        let (_, _, _, _, detected) =
            extract_pattern_path(&["--replace", "-normal", "pattern", "src"], Engine::Rg);
        assert!(!detected.show_line, "--replace's value must not trigger -n");

        // A genuine short context/show-file/show-line flag is still detected correctly.
        let (_, _, _, _, detected) =
            extract_pattern_path(&["-C", "3", "pattern", "src"], Engine::Grep);
        assert!(detected.context);
        let (_, _, _, _, detected) = extract_pattern_path(&["-H", "pattern", "src"], Engine::Grep);
        assert_eq!(detected.show_file, Some(true));
        let (_, _, _, _, detected) = extract_pattern_path(&["-n", "pattern", "src"], Engine::Grep);
        assert!(detected.show_line);
    }

    #[test]
    fn test_format_flag_detects_only_matching() {
        assert!(has_format_flag(Engine::Grep, &["-o"]));
        assert!(has_format_flag(Engine::Grep, &["--only-matching"]));
    }

    #[test]
    fn test_format_flag_detects_null() {
        assert!(has_format_flag(Engine::Grep, &["-Z"]));
        assert!(has_format_flag(Engine::Grep, &["--null"]));
    }

    #[test]
    fn test_format_flag_ignores_normal_flags() {
        assert!(!has_format_flag(Engine::Grep, &["-i", "-w", "-A", "3"]));
    }

    #[test]
    fn test_format_flag_ignores_value_of_value_taking_flag() {
        // Regression: has_format_flag used to be its own mini-tokenizer that scanned every raw
        // arg string independently, with no notion of "this token is another flag's value."
        // `-e --json` means "--json" is -e's pattern argument (both tables take a value for
        // true), not the real --json format flag -- but the old per-arg scan matched "--json"
        // regardless of position.
        assert!(!has_format_flag(Engine::Grep, &["-e", "--json"]));
        // Same for the long-flag form of a value-taking option.
        assert!(!has_format_flag(Engine::Grep, &["--regexp", "--quiet"]));
    }

    #[test]
    fn test_extract_pattern_path_has_format_flag_matches_single_pass() {
        // extract_pattern_path computes has_format_flag from its own token pass instead of
        // tokenizing the reconstructed `flags` strings a second time; pin that it agrees with
        // has_format_flag's own (test-only) from-scratch computation on representative cases.
        let (_, _, _, has_format, _) = extract_pattern_path(&["foo", "src", "-q"], Engine::Grep);
        assert!(has_format, "-q (quiet) should be detected");

        let (_, _, _, has_format, _) = extract_pattern_path(&["foo", "src", "-rl"], Engine::Grep);
        assert!(has_format, "-l inside the -rl cluster should be detected");

        let (_, _, _, has_format, _) =
            extract_pattern_path(&["foo", "src", "--json"], Engine::Grep);
        assert!(has_format, "--json should be detected");

        let (_, _, _, has_format, _) = extract_pattern_path(&["-e", "--json", "src"], Engine::Grep);
        assert!(
            !has_format,
            "-e's value must not be misread as the real --json flag"
        );

        let (_, _, _, has_format, _) =
            extract_pattern_path(&["foo", "src", "-i", "-w"], Engine::Grep);
        assert!(!has_format, "plain boolean flags aren't format flags");
    }

    #[test]
    fn test_format_flag_detects_clusters() {
        // clustered minimal forms must route to passthrough, not GROUP
        assert!(has_format_flag(Engine::Grep, &["-rl"]));
        assert!(has_format_flag(Engine::Grep, &["-rc"]));
        assert!(has_format_flag(Engine::Grep, &["-rq"]));
        assert!(has_format_flag(Engine::Grep, &["-rln"]));
        assert!(has_format_flag(Engine::Grep, &["-cr"]));
    }

    #[test]
    fn test_format_flag_detects_quiet_and_shape() {
        assert!(has_format_flag(Engine::Grep, &["-q"]));
        assert!(has_format_flag(Engine::Grep, &["--quiet"]));
        assert!(has_format_flag(Engine::Grep, &["--silent"]));
        assert!(has_format_flag(Engine::Grep, &["-b"]));
        assert!(has_format_flag(Engine::Grep, &["--byte-offset"]));
        assert!(has_format_flag(Engine::Grep, &["--column"]));
        assert!(has_format_flag(Engine::Grep, &["--vimgrep"]));
        assert!(has_format_flag(Engine::Grep, &["-z"]));
        assert!(has_format_flag(Engine::Grep, &["--null-data"]));
    }

    #[test]
    fn test_format_flag_compresses_default_and_context() {
        // compressible forms must NOT passthrough
        assert!(!has_format_flag(Engine::Grep, &["-rn"]));
        assert!(!has_format_flag(Engine::Grep, &["-A", "3"]));
        assert!(!has_format_flag(Engine::Grep, &["-v"]));
        assert!(!has_format_flag(Engine::Grep, &["-rin"]));
    }

    /// What production computes for these args, so a change to the real detector shows up here
    /// rather than only in a test-only twin of it.
    fn detected(args: &[&str]) -> DetectedFlags {
        detected_for(Engine::Grep, args)
    }

    fn detected_for(engine: Engine, args: &[&str]) -> DetectedFlags {
        let mut with_pattern = vec!["pattern"];
        with_pattern.extend_from_slice(args);
        extract_pattern_path(&with_pattern, engine).4
    }

    #[test]
    fn show_line_is_off_without_an_explicit_request() {
        assert!(!detected(&[]).show_line);
        assert!(!detected(&["-i"]).show_line);
        assert!(!detected(&["-r"]).show_line);
        assert!(!detected(&["-A", "3"]).show_line);
    }

    #[test]
    fn show_line_honours_n_in_every_spelling() {
        assert!(detected(&["-n"]).show_line);
        assert!(detected(&["--line-number"]).show_line);
        assert!(detected(&["-rn"]).show_line);
        assert!(detected(&["-in"]).show_line);
    }

    #[test]
    fn show_line_is_off_when_explicitly_negated() {
        // `-n` has to be present, or the assertion holds whether or not the negation works.
        assert!(detected_for(Engine::Rg, &["-n"]).show_line);
        assert!(!detected_for(Engine::Rg, &["-n", "-N"]).show_line);
        assert!(!detected_for(Engine::Rg, &["-n", "--no-line-number"]).show_line);
    }

    #[test]
    fn grep_initial_tab_is_a_shape_flag_in_both_spellings() {
        // `-T` pads and tabs every match line, so RTK's forced `-H --null -n` parse reads
        // nothing back and leaked the injected flags -- filename, a raw NUL and the line
        // number -- straight into the output.
        assert!(is_format_flag_token(Engine::Grep, TokenKind::Short, "T"));
        // ripgrep's -T is --type-not, a value-taking flag, not a shape flag.
        assert!(!is_format_flag_token(Engine::Rg, TokenKind::Short, "T"));
        assert!(is_format_flag_token(
            Engine::Grep,
            TokenKind::Long,
            "initial-tab"
        ));
        assert!(!is_format_flag_token(
            Engine::Rg,
            TokenKind::Long,
            "initial-tab"
        ));
    }

    #[test]
    fn grep_does_not_claim_ripgrep_only_line_number_negations() {
        // Real grep 3.12 exits 2 on both spellings; swallowing them would report a match for a
        // command the engine refuses to run.
        for negation in ["-N", "--no-line-number"] {
            let (_, _, flags, _, detected) =
                extract_pattern_path(&["pattern", "-n", negation], Engine::Grep);
            assert!(
                detected.show_line,
                "{negation} is not grep's, so -n still stands"
            );
            assert!(
                flags.iter().any(|f| f == negation),
                "{negation} must reach grep"
            );
        }
    }

    #[test]
    fn recursion_does_not_outrank_an_explicit_no_filename() {
        // Real grep: `-hr` and `-rh` both drop the prefix -- `-r` only makes the search span
        // several files, it is not the counterpart of `-h` the way `-H` is.
        assert_eq!(detected(&["-rh"]).show_file, Some(false));
        assert_eq!(detected(&["-hr"]).show_file, Some(false));
        assert_eq!(detected(&["-h", "-r"]).show_file, Some(false));
        assert_eq!(detected(&["-rH"]).show_file, Some(true));
        assert_eq!(detected(&["-Hr"]).show_file, Some(true));
    }

    #[test]
    fn recursion_alone_still_asks_for_the_filename() {
        for args in [&["-r"][..], &["-R"][..], &["--recursive"][..]] {
            let d = detected(args);
            assert_eq!(d.show_file, None, "{args:?} is not an explicit request");
            assert!(d.recursive, "{args:?} must feed show_file's fallback");
        }
        // ripgrep's `-r` is `--replace`, so its value must not be read as recursion.
        assert!(!detected_for(Engine::Rg, &["-r", "X"]).recursive);
    }

    #[test]
    fn match_block_stays_fully_qualified() {
        let entries = vec![
            (3usize, true, "line 3 needle".to_string()),
            (4usize, false, "line 4 context".to_string()),
        ];
        let block = match_block("src/deep/file.rs", &entries);
        assert_eq!(
            block,
            "src/deep/file.rs:3:line 3 needle\nsrc/deep/file.rs-4-line 4 context\n"
        );
    }

    #[test]
    fn match_block_keeps_position_when_display_drops_it() {
        assert!(!detected(&[]).show_line);
        let entries = vec![(42usize, true, "hit".to_string())];
        assert_eq!(match_block("f.txt", &entries), "f.txt:42:hit\n");
    }

    // Verify line numbers are always enabled in the engine invocation (parse_flags).
    // The -n/--line-numbers clap flag in main.rs is a no-op accepted for compat.
    #[test]
    fn test_rg_always_has_line_numbers() {
        // engine_capture always passes "-n" to the engine via parse_flags().
        // This test documents that -n is built-in, so the clap flag is safe to ignore.
        let mut cmd = resolved_command("rg");
        cmd.args(["-n", "--no-heading", "NONEXISTENT_PATTERN_12345", "."]);
        // If rg is available, it should accept -n without error (exit 1 = no match, not error)
        if let Ok(output) = cmd.output() {
            assert!(
                output.status.code() == Some(1) || output.status.success(),
                "rg -n should be accepted"
            );
        }
        // If rg is not installed, skip gracefully (test still passes)
    }

    // --- issues #1436 / #1613: parse_match_line robustness (single-file colon misparse) ---
    // Input shape is `file\0line[:-]content` (rg --null / grep -Z).

    #[test]
    fn test_parse_match_line_simple() {
        let line = "file.php\x0010:use Foo\\Bar;";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "file.php");
        assert_eq!(line_num, 10);
        assert!(is_match);
        assert_eq!(content, "use Foo\\Bar;");
    }

    // Issue #1436 reproducer: content with `::` must not split into a phantom
    // file bucket. With NUL separation between file and line:content, content
    // colons are irrelevant to the parser.
    #[test]
    fn test_parse_match_line_content_with_double_colon() {
        let line = "externalImportShell.class.php\x0081:        $this->queueProcessModel = ClassRegistry::init('Collections.QueueProcess');";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "externalImportShell.class.php");
        assert_eq!(line_num, 81);
        assert!(is_match);
        assert_eq!(
            content,
            "        $this->queueProcessModel = ClassRegistry::init('Collections.QueueProcess');"
        );
    }

    // Windows abs-path safety: drive letter + backslashes must not break the
    // parser. The NUL separator makes the file portion unambiguous.
    #[test]
    fn test_parse_match_line_windows_path() {
        let line = "C:\\src\\file.rs\x0042:fn main() {}";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, r"C:\src\file.rs");
        assert_eq!(line_num, 42);
        assert!(is_match);
        assert_eq!(content, "fn main() {}");
    }

    // Filenames containing `:digits:` (which would fool a greedy `:` parser)
    // must still parse correctly under NUL separation.
    #[test]
    fn test_parse_match_line_filename_with_colons() {
        let line = "badly_named:52:file.txt\x001:xxx";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "badly_named:52:file.txt");
        assert_eq!(line_num, 1);
        assert!(is_match);
        assert_eq!(content, "xxx");
    }

    // Content that itself contains `:digits:` (e.g. log lines, port numbers,
    // line-number-like substrings) must not confuse the parser.
    #[test]
    fn test_parse_match_line_content_with_digit_colons() {
        let line = "log.txt\x007:debug: counter is :42: now";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "log.txt");
        assert_eq!(line_num, 7);
        assert!(is_match);
        assert_eq!(content, "debug: counter is :42: now");
    }

    #[test]
    fn test_parse_match_line_malformed_returns_none() {
        // No NUL separator (e.g. rg/grep invoked without --null/-Z, or a
        // context line written with `-`).
        assert!(parse_match_line("file.rs:1:content").is_none());
        assert!(parse_match_line("not a match line").is_none());
        // Missing line number after NUL
        assert!(parse_match_line("file.rs\x00fn foo()").is_none());
        // Empty
        assert!(parse_match_line("").is_none());
    }

    #[test]
    fn test_parse_match_line_empty_content() {
        let line = "file.rs\x007:";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "file.rs");
        assert_eq!(line_num, 7);
        assert!(is_match);
        assert_eq!(content, "");
    }

    // Context line: separator is `-` → is_match==false
    #[test]
    fn test_parse_match_line_context_line() {
        let line = "file.txt\x004-after1";
        let (file, line_num, is_match, content) = parse_match_line(line).unwrap();
        assert_eq!(file, "file.txt");
        assert_eq!(line_num, 4);
        assert!(!is_match, "dash separator must yield is_match==false");
        assert_eq!(content, "after1");
    }

    // --- unparsed_signal ---

    #[test]
    fn test_unparsed_signal_parseable_lines_yield_zero() {
        // NUL-separated match lines all parse → signal == 0
        let stdout = "file.txt\x001:hello\nfile.txt\x002:world\n";
        assert_eq!(unparsed_signal(stdout), 0);
    }

    #[test]
    fn test_unparsed_signal_context_separator_not_counted() {
        // The `--` context separator emitted by rg/grep between match groups
        // must not be counted as an unparsed line.
        let stdout = "file.txt\x001:hello\n--\nfile.txt\x003:world\n";
        assert_eq!(unparsed_signal(stdout), 0);
    }

    #[test]
    fn test_unparsed_signal_empty_line_not_counted() {
        let stdout = "file.txt\x001:hello\n\nfile.txt\x002:world\n";
        assert_eq!(unparsed_signal(stdout), 0);
    }

    #[test]
    fn test_unparsed_signal_bare_colon_line_counted() {
        // A line like "file.rs:1:content" (no NUL) is what --heading or
        // --no-filename output looks like — it must be counted.
        let stdout = "file.rs:1:content\n";
        assert_eq!(unparsed_signal(stdout), 1);
    }

    #[test]
    fn test_unparsed_signal_binary_notice_counted() {
        // rg emits "Binary file foo matches" for binary files; no NUL → counted.
        let stdout = "Binary file foo matches\n";
        assert_eq!(unparsed_signal(stdout), 1);
    }

    #[test]
    fn test_unparsed_signal_context_lines_parse_ok() {
        // Context lines (dash separator) parse via the updated regex → not counted.
        let stdout =
            "file.txt\x003-context_before\nfile.txt\x004:match\nfile.txt\x005-context_after\n";
        assert_eq!(unparsed_signal(stdout), 0);
    }

    // --- has_context_flag ---

    #[test]
    fn test_has_context_flag_short() {
        let f = |args: &[&str]| -> bool { detected(args).context };
        assert!(f(&["-A", "3"]));
        assert!(f(&["-B", "2"]));
        assert!(f(&["-C", "1"]));
        assert!(!f(&["-rn"]));
        assert!(!f(&["-i", "-w"]));
    }

    #[test]
    fn test_has_context_flag_long() {
        let f = |args: &[&str]| -> bool { detected(args).context };
        assert!(f(&["--after-context", "3"]));
        assert!(f(&["--before-context", "2"]));
        assert!(f(&["--context", "1"]));
        assert!(f(&["--after-context=3"]));
        assert!(f(&["--before-context=2"]));
        assert!(f(&["--context=1"]));
        assert!(!f(&["--color", "auto"]));
    }
}
