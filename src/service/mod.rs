//! Integration-facing RTK services.

pub mod mcp;

use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use crate::core::config::Config;
use crate::discover::registry::rewrite_command;

static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:(?:token|password|secret|api[_-]?key|authorization)\s*[:=]\s*\S+(?:\s+\S+)?|bearer\s+\S+)"
    )
    .expect("secret redaction regex must compile")
});

/// Maximum command output returned by an integration request by default.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Maximum command runtime returned by an integration request by default.
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Serialize)]
pub struct RewriteResult {
    pub matched: bool,
    pub rewritten_command: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub rtk_args: Vec<String>,
    pub rewritten_command: Option<String>,
    pub filtered: bool,
    pub tee_path: Option<String>,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub saved_tokens: usize,
}

/// Rewrite a shell command using the same configuration as the hooks.
pub fn rewrite(raw_command: &str) -> RewriteResult {
    let (excluded, transparent_prefixes) = Config::load()
        .map(|config| {
            (
                config.hooks.exclude_commands,
                config.hooks.transparent_prefixes,
            )
        })
        .unwrap_or_default();

    let rewritten = rewrite_command(raw_command, &excluded, &transparent_prefixes);
    RewriteResult {
        matched: rewritten.is_some(),
        rewritten_command: rewritten.map(|command| redact_sensitive(&command)),
    }
}

/// Validate an integration working directory without executing anything.
pub fn validate_cwd(cwd: Option<&Path>) -> Result<Option<PathBuf>> {
    let Some(cwd) = cwd else {
        return Ok(None);
    };

    let absolute = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Failed to resolve current directory")?
            .join(cwd)
    };
    let canonical = absolute
        .canonicalize()
        .with_context(|| format!("Working directory does not exist: {}", absolute.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!(
            "Working directory is not a directory: {}",
            canonical.display()
        );
    }
    Ok(Some(canonical))
}

/// Execute an RTK command through the current binary using typed argv.
///
/// This deliberately uses a child-process boundary: every existing command
/// router and filter keeps its current stdout/stderr and exit-code behavior,
/// while integrations receive bounded captured output.
pub fn run_filtered(
    rtk_args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    max_output_bytes: usize,
    tee_on_failure: bool,
) -> Result<RunResult> {
    validate_rtk_args(rtk_args)?;
    let cwd = validate_cwd(cwd)?;
    let executable = std::env::current_exe().context("Failed to locate the RTK executable")?;

    // nosemgrep: dynamic-command-execution -- the executable comes only from
    // current_exe(), while validate_rtk_args rejects meta commands and malformed argv.
    let mut command = Command::new(executable);
    command
        .args(rtk_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !tee_on_failure {
        command.env("RTK_TEE", "0");
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    if debug_enabled() {
        eprintln!(
            "[rtk-debug] service.run_filtered decision=spawn args={} timeout_ms={} max_output_bytes={}",
            redact_sensitive(&rtk_args.join(" ")),
            timeout.as_millis(),
            max_output_bytes
        );
    }

    let child = command.spawn().context("Failed to spawn RTK command")?;
    let output = wait_with_timeout(child, timeout)?;
    let (stdout, stdout_truncated) = bounded_text(&output.stdout, max_output_bytes);
    let (stderr, stderr_truncated) = bounded_text(&output.stderr, max_output_bytes);
    let stdout = redact_sensitive(&stdout);
    let stderr = redact_sensitive(&stderr);
    let raw_command = rtk_args.join(" ");
    let rewritten = rewrite(&raw_command);
    let input_tokens = crate::core::tracking::estimate_tokens(&raw_command);
    let output_tokens = crate::core::tracking::estimate_tokens(&stdout);
    let tee_path = stdout
        .lines()
        .chain(stderr.lines())
        .find_map(|line| {
            line.strip_prefix("[full output: ")
                .and_then(|value| value.strip_suffix(']'))
        })
        .map(str::to_string);

    Ok(RunResult {
        exit_code: output.status.code().unwrap_or(1),
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
        rtk_args: rtk_args.iter().map(|arg| redact_sensitive(arg)).collect(),
        rewritten_command: rewritten
            .rewritten_command
            .map(|command| redact_sensitive(&command)),
        filtered: rewritten.matched,
        tee_path,
        input_tokens,
        output_tokens,
        saved_tokens: input_tokens.saturating_sub(output_tokens),
    })
}

fn validate_rtk_args(args: &[String]) -> Result<()> {
    let Some(first) = args.first() else {
        anyhow::bail!("rtk_args must contain at least one command");
    };
    if first.starts_with('-') {
        anyhow::bail!("rtk_args must begin with an RTK subcommand");
    }
    if matches!(
        first.as_str(),
        "mcp" | "hook" | "init" | "telemetry" | "run" | "proxy" | "pipe"
    ) {
        anyhow::bail!("rtk_args subcommand '{first}' is not supported through MCP execution");
    }
    if args.iter().any(|arg| arg.contains('\0')) {
        anyhow::bail!("rtk_args cannot contain NUL bytes");
    }
    Ok(())
}

fn bounded_text(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let limit = max_bytes.max(1);
    if bytes.len() <= limit {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    let mut end = limit;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    (
        format!(
            "{}\n[RTK:TRUNCATED] output exceeded {} bytes",
            String::from_utf8_lossy(&bytes[..end]),
            limit
        ),
        true,
    )
}

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output> {
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("Failed waiting for RTK command")? {
            let stdout = child
                .stdout
                .take()
                .map(|mut reader| {
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut reader, &mut bytes).map(|_| bytes)
                })
                .transpose()
                .context("Failed reading RTK stdout")?
                .unwrap_or_default();
            let stderr = child
                .stderr
                .take()
                .map(|mut reader| {
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut reader, &mut bytes).map(|_| bytes)
                })
                .transpose()
                .context("Failed reading RTK stderr")?
                .unwrap_or_default();
            return Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("RTK command timed out after {} ms", timeout.as_millis());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn debug_enabled() -> bool {
    matches!(
        std::env::var("RTK_DEBUG").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

/// Redact common secret-bearing argument and output forms before they cross an
/// integration boundary. The child process still receives the original argv.
pub fn redact_sensitive(value: &str) -> String {
    SECRET_RE.replace_all(value, "[REDACTED]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_meta_command_argv() {
        assert!(validate_rtk_args(&[]).is_err());
        assert!(validate_rtk_args(&["mcp".to_string()]).is_err());
        assert!(validate_rtk_args(&["git".to_string(), "status".to_string()]).is_ok());
    }

    #[test]
    fn bounded_text_marks_output_truncation() {
        let (text, truncated) = bounded_text(b"abcdef", 3);
        assert!(truncated);
        assert!(text.contains("[RTK:TRUNCATED]"));
    }

    #[test]
    fn redacts_common_secret_forms() {
        let value = redact_sensitive("token=abc password=xyz Authorization=Bearer secret");
        assert!(!value.contains("abc"));
        assert!(!value.contains("xyz"));
        assert!(!value.contains("secret"));
        assert!(value.contains("[REDACTED]"));
    }

    #[test]
    fn validate_cwd_rejects_files() {
        let file = std::env::current_exe().expect("current executable");
        assert!(validate_cwd(Some(&file)).is_err());
    }
}
