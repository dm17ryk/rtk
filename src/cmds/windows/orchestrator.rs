//! Public and hidden execution paths for CMD expressions.

use super::adapters;
use super::catalog::{builtins, validate_command_catalogs, AdapterStrategy};
use super::external_manifest::external_commands;
use super::parser::{parse_expression, OperatorKind};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::Command;

enum SegmentStdout {
    Native(Vec<u8>),
    Lossless(crate::core::tee::LosslessTeeCommit),
}

impl SegmentStdout {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Native(bytes) => bytes,
            Self::Lossless(commit) => commit.as_bytes(),
        }
    }

    #[cfg(debug_assertions)]
    fn lossless_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Native(_) => None,
            Self::Lossless(commit) => Some(commit.path()),
        }
    }
}

#[cfg(debug_assertions)]
fn observe_test_lossless_publication(stdout: &SegmentStdout) {
    let (Some(path), Ok(directory)) = (
        stdout.lossless_path(),
        std::env::var("RTK_TEST_TEE_PUBLICATION_DIR"),
    ) else {
        return;
    };
    if path.is_file() {
        let directory = std::path::Path::new(&directory);
        let marker = directory.join(format!("published-{}", std::process::id()));
        let _ = std::fs::write(marker, path.display().to_string());
        let release = directory.join("publication-release");
        while !release.exists() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// Internal subcommand used from a rewritten CMD segment.
pub const SEGMENT_RUNNER: &str = "__cmd-run";

/// The execution shape selected before starting `cmd.exe`.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Invoke CMD unchanged, for an interactive session or native `/K` mode.
    Passthrough(Vec<OsString>),
    /// Invoke a one-shot expression using the hardened default switches.
    Execute(String),
    /// Invoke independently supplied arguments through environment transport,
    /// enabling delayed expansion only in a nested execution command.
    Transport {
        expression: String,
        environment: Vec<(OsString, OsString)>,
    },
}

/// Classify public `rtk cmd` arguments without losing a single raw expression.
pub fn prepare_invocation(args: &[OsString], cmd_executable: &Path) -> Result<Invocation> {
    if args.is_empty() {
        return Ok(Invocation::Passthrough(Vec::new()));
    }

    let strings = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_owned)
                .context("rtk cmd accepts Unicode CMD arguments only")
        })
        .collect::<Result<Vec<_>>>()?;

    if strings[0].eq_ignore_ascii_case("/K") {
        return Ok(Invocation::Passthrough(args.to_vec()));
    }

    let expression_args = if strings[0].eq_ignore_ascii_case("/C") {
        &strings[1..]
    } else {
        &strings[..]
    };

    if expression_args.is_empty() {
        return Ok(Invocation::Execute(String::new()));
    }

    let invocation = if expression_args.len() == 1 {
        Invocation::Execute(expression_args[0].clone())
    } else {
        let environment = expression_args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                (
                    OsString::from(format!("RTK_CMD_ARG_{index}")),
                    OsString::from(argument),
                )
            })
            .collect::<Vec<_>>();
        let expression = expression_args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                if argument.is_empty() {
                    "\"\"".to_owned()
                } else {
                    format!("!RTK_CMD_ARG_{index}!")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Invocation::Transport {
            // The outer CMD parses this line with its default delayed expansion
            // disabled. The nested CMD enables it only after the literal
            // `!RTK_CMD_ARG_n!` tokens have crossed that parser boundary.
            expression: format!(
                "{} /D /S /V:ON /C {expression}",
                quote_cmd_path(&cmd_executable.to_string_lossy())
            ),
            environment,
        }
    };
    Ok(invocation)
}

/// Rewrite only cataloged, stateless query segments. Any opaque parser result
/// is binding and therefore executes byte-for-byte through the parent CMD.
#[cfg(test)]
pub fn rewrite_expression(source: &str, rtk_executable: &Path) -> String {
    rewrite_expression_for_terminal(source, rtk_executable, true)
}

/// Rewrite only when stdout is attached to a terminal. Redirected or piped
/// output is machine-consumed data and therefore retains the exact CMD path.
pub fn rewrite_expression_for_terminal(
    source: &str,
    rtk_executable: &Path,
    stdout_is_terminal: bool,
) -> String {
    if !stdout_is_terminal {
        return source.to_owned();
    }
    let parsed = parse_expression(source);
    // Percent expansion happens once for a complete parent CMD line, before
    // stateful segments execute. Sending a later segment through a child CMD
    // would expand it at a different time, so variables fail open as a unit.
    if parsed.opaque_reason.is_some()
        || source.contains('%')
        || parsed
            .operators
            .iter()
            .any(|operator| operator.kind == OperatorKind::RedirectInput)
    {
        return source.to_owned();
    }

    let catalog = builtins();
    let mut rewritten = source.to_owned();
    for segment in parsed.segments.iter().rev() {
        let command = &source[segment.command_span.start..segment.command_span.end];
        let command = command
            .strip_prefix('"')
            .and_then(|name| name.strip_suffix('"'))
            .unwrap_or(command);
        let segment_end = parsed
            .operators
            .iter()
            .find(|operator| operator.span.start >= segment.span.end)
            .map(|operator| operator.span.start)
            .unwrap_or(source.len());
        let original = &source[segment.span.start..segment_end];
        let eligible = catalog
            .iter()
            .find(|entry| entry.matches(command))
            .is_some_and(|entry| match entry.strategy {
                Some(AdapterStrategy::Structured { adapter }) => {
                    adapters::is_display_form(adapter, original)
                }
                Some(AdapterStrategy::Identity { .. }) | None => false,
            });
        if !eligible {
            continue;
        }

        let at_prefix = if original.starts_with('@') { "@" } else { "" };
        let replacement = format!(
            "{at_prefix}{} {SEGMENT_RUNNER} --hex {}",
            quote_cmd_path(&rtk_executable.to_string_lossy()),
            hex_encode(original.as_bytes())
        );
        rewritten.replace_range(segment.span.start..segment.span.end, &replacement);
    }
    rewritten
}

/// Run the public route. This path intentionally does not track the compound
/// expression: hidden runners are the sole future accounting boundary.
pub fn run(args: &[OsString]) -> Result<i32> {
    if !cfg!(windows) {
        bail!("rtk cmd is only supported on Windows 10 and 11");
    }
    validate_checked_in_catalogs()?;

    let cmd_executable = resolve_cmd_executable()?;
    match prepare_invocation(args, &cmd_executable)? {
        Invocation::Passthrough(arguments) => execute_cmd(&cmd_executable, &arguments, &[]),
        Invocation::Execute(source) => {
            let executable =
                std::env::current_exe().context("Failed to resolve the current RTK executable")?;
            let expression =
                rewrite_expression_for_terminal(&source, &executable, io::stdout().is_terminal());
            execute_cmd(
                &cmd_executable,
                &[
                    OsString::from("/D"),
                    OsString::from("/S"),
                    OsString::from("/C"),
                    OsString::from(expression),
                ],
                &[],
            )
        }
        Invocation::Transport {
            expression,
            environment,
        } => execute_cmd(
            &cmd_executable,
            &[
                OsString::from("/D"),
                OsString::from("/S"),
                OsString::from("/C"),
                OsString::from(expression),
            ],
            &environment,
        ),
    }
}

/// Execute one encoded source segment without rewriting it again.
pub fn run_segment(encoded: &str) -> Result<i32> {
    if !cfg!(windows) {
        bail!("rtk cmd is only supported on Windows 10 and 11");
    }
    validate_checked_in_catalogs()?;
    let bytes = hex_decode(encoded)?;
    let source = String::from_utf8(bytes).context("Invalid UTF-8 CMD segment")?;
    let cmd_executable = resolve_cmd_executable()?;
    let output = Command::new(&cmd_executable)
        .args(["/D", "/S", "/C", &source])
        .output()
        .context("Failed to execute CMD segment")?;
    let exit_code = crate::core::utils::exit_code_from_status(&output.status, "cmd");

    let stdout = if output.status.success() {
        render_segment_stdout(&source, &output.stdout)
    } else {
        SegmentStdout::Native(output.stdout)
    };
    io::stdout()
        .write_all(stdout.as_bytes())
        .context("Failed to write CMD stdout")?;
    #[cfg(debug_assertions)]
    observe_test_lossless_publication(&stdout);
    io::stderr()
        .write_all(&output.stderr)
        .context("Failed to write CMD stderr")?;
    Ok(exit_code)
}

fn validate_checked_in_catalogs() -> Result<()> {
    validate_command_catalogs(&builtins(), &external_commands())
        .map_err(|error| anyhow::anyhow!("invalid checked-in CMD catalog: {error}"))
}

/// Filter only a successful, UTF-8, cataloged structured display. Every
/// rejected layout, non-text output, identity adapter, and failed command is
/// emitted byte-for-byte by `run_segment`.
fn render_segment_stdout(source: &str, stdout: &[u8]) -> SegmentStdout {
    let Some((command, entry)) = source_command_and_catalog_entry(source) else {
        return SegmentStdout::Native(stdout.to_vec());
    };
    let Some(AdapterStrategy::Structured { adapter }) = entry.strategy else {
        return SegmentStdout::Native(stdout.to_vec());
    };
    let Ok(raw) = std::str::from_utf8(stdout) else {
        return SegmentStdout::Native(stdout.to_vec());
    };
    let Some(filtered) = adapters::filter_display(adapter, source, raw) else {
        return SegmentStdout::Native(stdout.to_vec());
    };
    if filtered == raw || !should_attempt_lossy_output(raw, &filtered) {
        return SegmentStdout::Native(stdout.to_vec());
    }

    // A lossy display is never emitted unless the full native stdout has a
    // recoverable tee artifact. The guard also includes the hint itself.
    let label = format!("cmd-{command}");
    let Some(reservation) = crate::core::tee::reserve_lossless_tee(raw, &label) else {
        return SegmentStdout::Native(stdout.to_vec());
    };
    crate::core::tee::commit_lossless_if_better(raw, &filtered, reservation).map_or_else(
        || SegmentStdout::Native(stdout.to_vec()),
        SegmentStdout::Lossless,
    )
}

fn should_attempt_lossy_output(raw: &str, filtered: &str) -> bool {
    crate::core::guard::never_worse(raw, filtered) != raw
}

fn source_command_and_catalog_entry(
    source: &str,
) -> Option<(&str, super::catalog::BuiltinCommand)> {
    let parsed = parse_expression(source);
    if parsed.opaque_reason.is_some() || parsed.segments.len() != 1 {
        return None;
    }
    let segment = parsed.segments.first()?;
    let command = &source[segment.command_span.start..segment.command_span.end];
    let command = command
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(command);
    builtins()
        .into_iter()
        .find(|entry| entry.matches(command))
        .map(|entry| (command, entry))
}

fn resolve_cmd_executable() -> Result<std::path::PathBuf> {
    crate::core::utils::resolve_binary("cmd.exe").context("Failed to resolve cmd.exe from PATH")
}

fn execute_cmd(
    cmd_executable: &Path,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
) -> Result<i32> {
    let status = Command::new(cmd_executable)
        .args(arguments)
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .status()
        .context("Failed to execute cmd.exe")?;
    Ok(crate::core::utils::exit_code_from_status(&status, "cmd"))
}

fn quote_cmd_path(path: &str) -> String {
    if path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_-.\\/:=+@".contains(character))
    {
        return path.to_owned();
    }
    format!("\"{}\"", path.replace('"', "^\""))
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(encoded: &str) -> Result<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        bail!("CMD segment encoding must contain an even number of hex digits");
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_value(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => bail!("CMD segment encoding contains a non-hex digit"),
    }
}

#[cfg(test)]
mod output_tests {
    use super::should_attempt_lossy_output;

    #[test]
    fn never_worse_is_checked_before_creating_a_lossy_recovery_artifact() {
        assert!(!should_attempt_lossy_output(
            "raw",
            "a much longer filtered display"
        ));
        assert!(should_attempt_lossy_output(
            &"raw output ".repeat(40),
            "summary"
        ));
    }
}
