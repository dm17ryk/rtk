//! Public and hidden execution paths for CMD expressions.

use super::catalog::{builtins, CommandMode};
use super::parser::parse_expression;
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// Internal subcommand used from a rewritten CMD segment.
pub const SEGMENT_RUNNER: &str = "__cmd-run";

/// The execution shape selected before starting `cmd.exe`.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Invoke CMD unchanged, for an interactive session or native `/K` mode.
    Passthrough(Vec<OsString>),
    /// Invoke a one-shot expression using the hardened default switches.
    Execute(String),
}

/// Classify public `rtk cmd` arguments without losing a single raw expression.
pub fn prepare_invocation(args: &[OsString]) -> Result<Invocation> {
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

    let expression = if expression_args.len() == 1 {
        expression_args[0].clone()
    } else {
        expression_args
            .iter()
            .map(|argument| quote_cmd_argument(argument))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Ok(Invocation::Execute(expression))
}

/// Rewrite only cataloged, stateless query segments. Any opaque parser result
/// is binding and therefore executes byte-for-byte through the parent CMD.
pub fn rewrite_expression(source: &str, rtk_executable: &Path) -> String {
    let parsed = parse_expression(source);
    // Percent expansion happens once for a complete parent CMD line, before
    // stateful segments execute. Sending a later segment through a child CMD
    // would expand it at a different time, so variables fail open as a unit.
    if parsed.opaque_reason.is_some() || source.contains('%') {
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
        let eligible = catalog.iter().any(|entry| {
            entry.matches(command) && entry.mode == CommandMode::Query && entry.strategy.is_some()
        });
        if !eligible {
            continue;
        }

        let segment_end = parsed
            .operators
            .iter()
            .find(|operator| operator.span.start >= segment.span.end)
            .map(|operator| operator.span.start)
            .unwrap_or(source.len());
        let original = &source[segment.span.start..segment_end];
        let at_prefix = original.starts_with('@').then_some("@").unwrap_or("");
        let replacement = format!(
            "{at_prefix}{} {SEGMENT_RUNNER} --hex {}",
            quote_cmd_argument(&rtk_executable.to_string_lossy()),
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

    match prepare_invocation(args)? {
        Invocation::Passthrough(arguments) => execute_cmd(&arguments),
        Invocation::Execute(source) => {
            let executable =
                std::env::current_exe().context("Failed to resolve the current RTK executable")?;
            let expression = rewrite_expression(&source, &executable);
            execute_cmd(&[
                OsString::from("/D"),
                OsString::from("/S"),
                OsString::from("/C"),
                OsString::from(expression),
            ])
        }
    }
}

/// Execute one encoded source segment without rewriting it again.
pub fn run_segment(encoded: &str) -> Result<i32> {
    if !cfg!(windows) {
        bail!("rtk cmd is only supported on Windows 10 and 11");
    }
    let bytes = hex_decode(encoded)?;
    let source = String::from_utf8(bytes).context("Invalid UTF-8 CMD segment")?;
    execute_cmd(&[
        OsString::from("/D"),
        OsString::from("/S"),
        OsString::from("/C"),
        OsString::from(source),
    ])
}

fn execute_cmd(arguments: &[OsString]) -> Result<i32> {
    let cmd_executable = crate::core::utils::resolve_binary("cmd.exe")
        .context("Failed to resolve cmd.exe from PATH")?;
    let status = Command::new(cmd_executable)
        .args(arguments)
        .status()
        .context("Failed to execute cmd.exe")?;
    Ok(crate::core::utils::exit_code_from_status(&status, "cmd"))
}

fn quote_cmd_argument(argument: &str) -> String {
    if !argument.is_empty()
        && argument
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-.\\/:=%+@".contains(character))
    {
        return argument.to_owned();
    }
    format!("\"{}\"", argument.replace('"', "\\\""))
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
    if encoded.len() % 2 != 0 {
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
