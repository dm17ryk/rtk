//! Conservative display adapters for CMD built-ins.
//!
//! These adapters never synthesize CMD results. They recognize only stable,
//! English CMD layouts after the real built-in has completed; every other
//! layout returns `None` so the caller emits the original bytes unchanged.

use regex::Regex;

/// Whether `source` is a non-mutating display form that may be filtered.
pub fn is_display_form(command: &str, source: &str) -> bool {
    let arguments = source
        .trim_start()
        .strip_prefix('@')
        .unwrap_or(source.trim_start())
        .trim_start();
    let arguments = arguments
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest)
        .trim();

    match command.to_ascii_lowercase().as_str() {
        // `/B` is already a machine-friendly, newline-delimited listing.
        "dir" => !arguments
            .split_whitespace()
            .any(|argument| argument.eq_ignore_ascii_case("/b")),
        // `set NAME=VALUE`, `/a`, and `/p` can mutate state or prompt. Only
        // the no-argument and prefix-query forms are safe display commands.
        "set" => {
            !arguments.contains('=')
                && !arguments.starts_with("/a")
                && !arguments.starts_with("/A")
                && !arguments.starts_with("/p")
                && !arguments.starts_with("/P")
        }
        "help" | "assoc" | "ftype" => true,
        _ => false,
    }
}

/// Return a compact display only when the native layout is confidently known.
pub fn filter_display(command: &str, source: &str, stdout: &str) -> Option<String> {
    if !is_display_form(command, source) {
        return None;
    }

    match command.to_ascii_lowercase().as_str() {
        "dir" => filter_dir(stdout),
        "set" => filter_set(stdout),
        "help" => filter_help(stdout),
        "assoc" => filter_assignments(stdout, "[assoc]", |name| name.starts_with('.')),
        "ftype" => filter_assignments(stdout, "[ftype]", |name| {
            !name.is_empty() && !name.chars().any(char::is_whitespace)
        }),
        _ => None,
    }
}

fn filter_set(stdout: &str) -> Option<String> {
    let entries = parse_assignments(stdout, |name| !name.is_empty())?;
    if entries.len() <= 8 {
        return Some(format!("[set] {}", entries.join("; ")));
    }
    let shown = entries
        .iter()
        .take(4)
        .copied()
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "[set] {} vars: {shown}; … +{} more",
        entries.len(),
        entries.len() - 4
    ))
}

fn filter_assignments<F>(stdout: &str, label: &str, valid_name: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let entries = parse_assignments(stdout, valid_name)?;
    Some(format!("{label} {}", entries.join("; ")))
}

fn parse_assignments<F>(stdout: &str, valid_name: F) -> Option<Vec<&str>>
where
    F: Fn(&str) -> bool,
{
    let entries = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (name, value) = line.split_once('=')?;
            (valid_name(name) && !value.is_empty()).then_some(line.trim())
        })
        .collect::<Option<Vec<_>>>()?;
    (!entries.is_empty()).then_some(entries)
}

fn filter_help(stdout: &str) -> Option<String> {
    let lines = stdout.lines().map(str::trim_end).collect::<Vec<_>>();
    let description = lines
        .iter()
        .copied()
        .find(|line| is_english_sentence(line.trim()))?
        .trim();
    let usage = lines
        .iter()
        .copied()
        .find(|line| is_usage_line(line.trim()))?
        .trim();

    // A recognized English description, a CMD-shaped uppercase usage line,
    // and only indented detail makes the locale/layout contract explicit.
    lines
        .iter()
        .filter(|line| {
            !line.trim().is_empty() && line.trim() != description && line.trim() != usage
        })
        .all(|line| line.starts_with(' '))
        .then(|| format!("[help] {usage}\n{description}"))
}

fn is_english_sentence(line: &str) -> bool {
    line.ends_with('.')
        && line.is_ascii()
        && line
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
        && line.contains(' ')
}

fn is_usage_line(line: &str) -> bool {
    let command = line.split_whitespace().next().unwrap_or_default();
    !command.is_empty()
        && command
            .chars()
            .all(|character| character.is_ascii_uppercase())
        && line.is_ascii()
        && line.chars().any(|character| matches!(character, '[' | '<'))
}

fn filter_dir(stdout: &str) -> Option<String> {
    let entry =
        Regex::new(r"^\d{2}/\d{2}/\d{4}\s+\d{2}:\d{2}\s+(?:AM|PM)\s+(?:(<DIR>)\s+|(\d+)\s+)(.+)$")
            .expect("static directory entry regex");
    let file_total = Regex::new(r"^\s*\d+ File\(s\)").expect("static file total regex");
    let dir_total = Regex::new(r"^\s*\d+ Dir\(s\)").expect("static directory total regex");

    let mut output = Vec::new();
    let mut path = None;
    let mut entries = 0usize;
    let mut saw_footer = false;
    for line in stdout.lines().map(str::trim_end) {
        if line.trim().is_empty()
            || line.starts_with(" Volume in drive ")
            || line.starts_with(" Volume Serial Number is ")
        {
            continue;
        }
        if let Some(directory) = line.strip_prefix(" Directory of ") {
            path = Some(directory.trim());
            output.push(format!("[dir] {}", directory.trim()));
            continue;
        }
        if file_total.is_match(line) || dir_total.is_match(line) {
            saw_footer = true;
            continue;
        }
        let captures = entry.captures(line)?;
        let current_path = path?;
        let name = captures.get(3)?.as_str().trim();
        if name.is_empty() || current_path.is_empty() {
            return None;
        }
        let item = if captures.get(1).is_some() {
            format!("D {name}")
        } else {
            format!("F {} {name}", captures.get(2)?.as_str())
        };
        output.push(item);
        entries += 1;
    }

    (path.is_some() && saw_footer && entries > 0).then(|| {
        output.push(format!("{entries} entries"));
        output.join("\n")
    })
}
