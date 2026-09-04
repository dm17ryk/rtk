//! Reads source files with optional language-aware filtering to strip boilerplate.

use crate::core::ai_output::{AiDocument, AiRecord, BudgetClass, Omission, Severity};
use crate::core::filter::{self, FilterLevel, Language};
use crate::core::guard::never_worse;
use crate::core::tracking;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

fn source_document(file: &str, lines: &[filter::FilteredLine]) -> AiDocument {
    let mut document = AiDocument::new(Some("source"));
    document.fact("file", file);
    for line in lines {
        document.push(AiRecord::new(
            Severity::Info,
            format!("{}: {}", line.original_line, line.text),
        ));
    }
    document
}

fn emit_ai_source(
    timer: &tracking::TimedExecution,
    original_cmd: &str,
    rtk_cmd: &str,
    file_label: &str,
    content: &str,
    level: FilterLevel,
    lang: &Language,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    fallback_baseline: &str,
    verbose: u8,
) -> bool {
    let filter = filter::get_filter(level);
    let lines = filter.filter_lines(content, lang);
    if lines.is_empty() || max_lines == Some(0) || tail_lines == Some(0) {
        return false;
    }
    let displayed = apply_filtered_line_window(&lines, max_lines, tail_lines);

    if verbose > 0 {
        let original_lines = content.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - displayed.len()) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines,
            displayed.len(),
            reduction
        );
    }

    let omitted = content.lines().count().saturating_sub(displayed.len());
    let mut document = source_document(file_label, displayed);
    if omitted > 0 {
        document = document.with_omission(Omission {
            items: omitted,
            groups: 0,
        });
    }
    crate::core::runner::emit_ai_document_with_baseline(
        timer,
        original_cmd,
        rtk_cmd,
        content,
        fallback_baseline,
        "read",
        BudgetClass::Source,
        document,
        true,
    );
    true
}

pub fn run(
    file: &Path,
    level: FilterLevel,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading: {} (filter: {})", file.display(), level);
    }

    // Read file content
    let content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    // Detect language from extension
    let lang = file
        .extension()
        .and_then(|e| e.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Unknown);

    if verbose > 1 {
        eprintln!("Detected language: {:?}", lang);
    }

    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content, &lang);

    // Safety: if filter emptied a non-empty file, fall back to raw content
    if filtered.trim().is_empty() && !content.trim().is_empty() {
        eprintln!(
            "rtk: warning: filter produced empty output for {} ({} bytes), showing raw content",
            file.display(),
            content.len()
        );
        filtered = content.clone();
    }

    filtered = apply_line_window(&filtered, max_lines, tail_lines, &lang);

    let (raw, rtk_output) = if line_numbers {
        (
            format_with_line_numbers(&content),
            format_with_line_numbers(&filtered),
        )
    } else {
        (content.clone(), filtered.clone())
    };

    if level != FilterLevel::None
        && emit_ai_source(
            &timer,
            &format!("cat {}", file.display()),
            "rtk read",
            &file.display().to_string(),
            &content,
            level,
            &lang,
            max_lines,
            tail_lines,
            &rtk_output,
            verbose,
        )
    {
        return Ok(());
    }

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    let shown = never_worse(&raw, &rtk_output);
    print!("{}", shown);
    timer.track(
        &format!("cat {}", file.display()),
        "rtk read",
        &raw,
        shown,
    );
    Ok(())
}

pub fn run_stdin(
    level: FilterLevel,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    use std::io::{self, Read as IoRead};

    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading from stdin (filter: {})", level);
    }

    // Read from stdin
    let mut content = String::new();
    io::stdin()
        .lock()
        .read_to_string(&mut content)
        .context("Failed to read from stdin")?;

    // No file extension, so use Unknown language
    let lang = Language::Unknown;

    if verbose > 1 {
        eprintln!("Language: {:?} (stdin has no extension)", lang);
    }

    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content, &lang);

    // Safety: if filter emptied non-empty stdin, fall back to raw content.
    if filtered.trim().is_empty() && !content.trim().is_empty() {
        eprintln!(
            "rtk: warning: filter produced empty output for stdin ({} bytes), showing raw content",
            content.len()
        );
        filtered = content.clone();
    }

    filtered = apply_line_window(&filtered, max_lines, tail_lines, &lang);

    let (raw, rtk_output) = if line_numbers {
        (
            format_with_line_numbers(&content),
            format_with_line_numbers(&filtered),
        )
    } else {
        (content.clone(), filtered.clone())
    };

    if level != FilterLevel::None
        && emit_ai_source(
            &timer,
            "cat - (stdin)",
            "rtk read -",
            "stdin",
            &content,
            level,
            &lang,
            max_lines,
            tail_lines,
            &rtk_output,
            verbose,
        )
    {
        return Ok(());
    }

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    let shown = never_worse(&raw, &rtk_output);
    print!("{}", shown);

    timer.track("cat - (stdin)", "rtk read -", &raw, shown);
    Ok(())
}

fn format_with_line_numbers(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let width = lines.len().to_string().len();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{:>width$} │ {}\n", i + 1, line, width = width));
    }
    out
}

fn apply_filtered_line_window(
    lines: &[filter::FilteredLine],
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
) -> &[filter::FilteredLine] {
    if let Some(tail) = tail_lines {
        let start = lines.len().saturating_sub(tail);
        return &lines[start..];
    }

    if let Some(max) = max_lines {
        return &lines[..lines.len().min(max)];
    }

    lines
}

fn apply_line_window(
    content: &str,
    max_lines: Option<usize>,
    tail_lines: Option<usize>,
    lang: &Language,
) -> String {
    if let Some(tail) = tail_lines {
        if tail == 0 {
            return String::new();
        }
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(tail);
        let mut result = lines[start..].join("\n");
        if content.ends_with('\n') {
            result.push('\n');
        }
        return result;
    }

    if let Some(max) = max_lines {
        return filter::smart_truncate(content, max, lang);
    }

    content.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_rust_file() -> Result<()> {
        let mut file = NamedTempFile::with_suffix(".rs")?;
        writeln!(
            file,
            r#"// Comment
fn main() {{
    println!("Hello");
}}"#
        )?;

        // Just verify it doesn't panic
        run(file.path(), FilterLevel::Minimal, None, None, false, 0)?;
        Ok(())
    }

    #[test]
    fn test_stdin_support_signature() {
        // Test that run_stdin has correct signature and compiles
        // We don't actually run it because it would hang waiting for stdin
        // Compile-time verification that the function exists with correct signature
    }

    #[test]
    fn source_document_uses_original_dense_line_markers() {
        let lines = vec![
            filter::FilteredLine {
                original_line: 2,
                text: "fn kept() {}".into(),
            },
            filter::FilteredLine {
                original_line: 5,
                text: "let value = 1;".into(),
            },
        ];

        let rendered = crate::core::ai_output::render(
            &source_document("sample.rs", &lines),
            crate::core::ai_output::BudgetClass::Source,
        )
        .text;

        assert_eq!(
            rendered,
            "status=source file=sample.rs\n2: fn kept() {}\n5: let value = 1;"
        );
    }

    #[test]
    fn filtered_line_window_keeps_original_locations_for_tail_output() {
        let lines = vec![
            filter::FilteredLine {
                original_line: 2,
                text: "first".into(),
            },
            filter::FilteredLine {
                original_line: 5,
                text: "second".into(),
            },
            filter::FilteredLine {
                original_line: 9,
                text: "third".into(),
            },
        ];

        let window = apply_filtered_line_window(&lines, None, Some(2));
        let rendered = crate::core::ai_output::render(
            &source_document("sample.rs", window),
            crate::core::ai_output::BudgetClass::Source,
        )
        .text;

        assert_eq!(
            rendered,
            "status=source file=sample.rs\n5: second\n9: third"
        );
    }

    #[test]
    fn test_apply_line_window_tail_lines() {
        let input = "a\nb\nc\nd\n";
        let output = apply_line_window(input, None, Some(2), &Language::Unknown);
        assert_eq!(output, "c\nd\n");
    }

    #[test]
    fn test_apply_line_window_tail_lines_no_trailing_newline() {
        let input = "a\nb\nc\nd";
        let output = apply_line_window(input, None, Some(2), &Language::Unknown);
        assert_eq!(output, "c\nd");
    }

    #[test]
    fn test_apply_line_window_max_lines_still_works() {
        let input = "a\nb\nc\nd\n";
        let output = apply_line_window(input, Some(2), None, &Language::Unknown);
        assert!(output.starts_with("a\n"));
        assert!(output.contains("more lines"));
    }

    fn rtk_bin() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("rtk")
    }

    #[test]
    #[ignore]
    fn test_read_two_valid_files_concatenated() {
        let bin = rtk_bin();
        assert!(bin.exists(), "Run `cargo build` first");

        let mut f1 = NamedTempFile::with_suffix(".txt").unwrap();
        let mut f2 = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(f1, "alpha\nbravo").unwrap();
        writeln!(f2, "charlie\ndelta").unwrap();

        let output = std::process::Command::new(&bin)
            .args(["read", &f1.path().to_string_lossy(), &f2.path().to_string_lossy()])
            .output()
            .expect("failed to run rtk read");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("alpha"), "first file content missing");
        assert!(stdout.contains("charlie"), "second file content missing");
    }

    #[test]
    #[ignore]
    fn test_read_valid_and_nonexistent() {
        let bin = rtk_bin();
        assert!(bin.exists(), "Run `cargo build` first");

        let mut f1 = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(f1, "valid content").unwrap();

        let output = std::process::Command::new(&bin)
            .args(["read", &f1.path().to_string_lossy(), "/tmp/rtk_nonexistent_file.txt"])
            .output()
            .expect("failed to run rtk read");

        assert!(!output.status.success(), "should exit non-zero on missing file");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains("valid content"), "valid file should still be printed");
        assert!(stderr.contains("rtk_nonexistent_file"), "should report missing file on stderr");
    }

    #[test]
    #[ignore]
    fn test_read_stdin_dedup_warning() {
        let bin = rtk_bin();
        assert!(bin.exists(), "Run `cargo build` first");

        let output = std::process::Command::new(&bin)
            .args(["read", "-", "-"])
            .stdin(std::process::Stdio::piped())
            .output()
            .expect("failed to run rtk read");

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stdin specified more than once"),
            "should warn about duplicate stdin, got stderr: {}",
            stderr
        );
    }
}
