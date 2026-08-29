//! Public and hidden execution paths for CMD expressions.

use super::adapters;
use super::catalog::{builtins, AdapterStrategy, BuiltinCommand};
use super::external_manifest::classify_external;
use super::parser::{parse_expression, OperatorKind};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static HIDDEN_TRANSPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
const CMD_COMMAND_LINE_UTF16_LIMIT: usize = 8191;

/// The execution shape selected before starting `cmd.exe`.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Invoke CMD unchanged, for an interactive session or native `/K` mode.
    Passthrough(Vec<OsString>),
    /// Invoke a one-shot expression using the hardened default switches.
    Execute(String),
    /// Invoke a resolved external executable with the original argument
    /// vector. This is the lossless route for multi-argument calls whose
    /// quoting cannot be represented safely as CMD source.
    Direct(Vec<OsString>),
    /// Invoke a CMD-safe expression reconstructed from independent arguments.
    Reconstructed(String),
    /// Carry argument data that cannot be reconstructed as CMD source through
    /// delayed expansion, capture it in non-environment FOR variables, then
    /// clear the transport keys before the requested command starts.
    HiddenTransport {
        expression: String,
        environment: Vec<(OsString, OsString)>,
    },
}

/// Recognition is intentionally separate from rewrite eligibility: an external
/// name is known to this snapshot but still stays on the native raw route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandRecognition {
    Builtin(AdapterStrategy),
    ExternalRaw,
    Unknown,
}

#[cfg(test)]
pub(crate) fn recognize_command(name: &str) -> CommandRecognition {
    let catalog = builtins();
    recognize_command_in(name, &catalog)
}

fn recognize_command_in(name: &str, catalog: &[BuiltinCommand]) -> CommandRecognition {
    if let Some(entry) = catalog.iter().find(|entry| entry.matches(name)) {
        return entry
            .strategy
            .map(CommandRecognition::Builtin)
            .unwrap_or(CommandRecognition::Unknown);
    }
    if classify_external(name).is_some() {
        CommandRecognition::ExternalRaw
    } else {
        CommandRecognition::Unknown
    }
}

/// Classify public `rtk cmd` arguments without losing a single raw expression.
pub fn prepare_invocation(args: &[OsString], _cmd_executable: &Path) -> Result<Invocation> {
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

    let echo_arguments = expression_args
        .first()
        .is_some_and(|command| command.eq_ignore_ascii_case("echo"));
    let cmd_host = expression_args.first().is_some_and(|command| {
        command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe")
    });
    let nested_source_index = cmd_host
        .then(|| {
            expression_args
                .iter()
                .position(|argument| argument.eq_ignore_ascii_case("/c"))
                .filter(|index| {
                    expression_args.get(index + 1).is_some_and(|command| {
                        let name = command.split_whitespace().next().unwrap_or(command);
                        builtins().iter().any(|entry| entry.matches(name))
                    })
                })
                .map(|index| index + 1)
        })
        .flatten();
    // Newlines cannot be represented in a CMD source expression without
    // changing its command boundaries, so carry those values through the
    // delayed-expansion transport. Ordinary quoted arguments are safe to
    // reconstruct below (literal quotes use the Windows argv escape form),
    // and must not take the FOR path where embedded quotes can change its
    // iteration cardinality.
    let needs_hidden_transport = expression_args.iter().any(|argument| {
        argument.contains(['\r', '\n']) || (!echo_arguments && argument.contains('%'))
    });

    let invocation = if expression_args.len() == 1 {
        Invocation::Execute(expression_args[0].clone())
    } else if should_direct_external(expression_args) {
        Invocation::Direct(expression_args.iter().map(OsString::from).collect())
    } else if needs_hidden_transport {
        prepare_hidden_transport(expression_args)?
    } else {
        let expression = expression_args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                escape_cmd_argument(
                    argument,
                    index > 0
                        && (echo_arguments
                            || nested_source_index
                                .is_some_and(|source_index| index >= source_index)),
                )
            })
            .collect::<Result<Vec<_>>>()?
            .join(" ");
        Invocation::Reconstructed(expression)
    };
    Ok(invocation)
}

fn prepare_hidden_transport(arguments: &[String]) -> Result<Invocation> {
    prepare_hidden_transport_with_key_check(arguments, |key| std::env::var_os(key).is_some())
}

fn should_direct_external(arguments: &[String]) -> bool {
    let Some(command) = arguments.first() else {
        return false;
    };
    if builtins().iter().any(|entry| entry.matches(command))
        || command.eq_ignore_ascii_case("cmd")
        || command.eq_ignore_ascii_case("cmd.exe")
    {
        return false;
    }
    let extension = Path::new(command)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("bat" | "cmd")) {
        return false;
    }
    crate::core::utils::resolve_binary(command).is_ok()
}

pub(crate) fn prepare_hidden_transport_with_key_check<F>(
    arguments: &[String],
    mut key_is_taken: F,
) -> Result<Invocation>
where
    F: FnMut(&str) -> bool,
{
    const FOR_VARIABLES: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let keys = loop {
        let sequence = HIDDEN_TRANSPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let namespace = format!("RTK_INTERNAL_CMD_{}_{sequence}", std::process::id());
        let keys = (0..arguments.len())
            .map(|index| format!("{namespace}_ARG_{index}"))
            .collect::<Vec<_>>();
        if keys.iter().all(|key| !key_is_taken(key)) {
            break keys;
        }
    };

    let mut environment = Vec::new();
    let mut for_prefix = String::new();
    let mut clear = Vec::new();
    let echo_data = arguments
        .first()
        .is_some_and(|command| command.eq_ignore_ascii_case("echo"));
    let cmd_host = arguments.first().is_some_and(|command| {
        command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe")
    });
    let nested_source_index = cmd_host
        .then(|| {
            arguments
                .iter()
                .position(|argument| argument.eq_ignore_ascii_case("/c"))
                .filter(|index| {
                    arguments.get(index + 1).is_some_and(|command| {
                        let name = command.split_whitespace().next().unwrap_or(command);
                        builtins().iter().any(|entry| entry.matches(name))
                    })
                })
                .map(|index| index + 1)
        })
        .flatten();
    let mut command = Vec::new();
    let mut transported = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let source_data = index > 0
            && (echo_data || nested_source_index.is_some_and(|source_index| index >= source_index));
        let needs_transport =
            argument.contains(['\r', '\n']) || (!source_data && argument.contains('%'));
        transported.push(needs_transport);
    }
    let transport_count = transported.iter().filter(|transport| **transport).count();
    if transport_count > FOR_VARIABLES.len() {
        bail!("too many CMD arguments for line-break transport");
    }
    let mut variable_index = 0usize;
    for (index, argument) in arguments.iter().enumerate() {
        if argument.is_empty() {
            command.push("\"\"".to_owned());
            continue;
        }
        if !transported[index] {
            let source_data = index > 0
                && (echo_data
                    || nested_source_index.is_some_and(|source_index| index >= source_index));
            command.push(escape_cmd_argument(argument, source_data)?);
            continue;
        }
        if !source_data_is_safe_for_transport(arguments, index, echo_data, nested_source_index) {
            bail!("CMD line-break transport cannot safely carry quoted external arguments");
        }
        let key = &keys[variable_index];
        let variable = FOR_VARIABLES[variable_index] as char;
        variable_index += 1;
        let percent_sentinel = if argument.contains('%') {
            unique_percent_sentinel(argument, key)
        } else {
            String::new()
        };
        if percent_sentinel.is_empty() {
            for_prefix.push_str(&format!("for %{variable} in (\"!{key}!\") do @"));
        } else {
            for_prefix.push_str(&format!(
                "for %{variable} in (\"!{key}:{percent_sentinel}=%!\") do @"
            ));
        }
        clear.push(format!("set \"{key}=\""));
        command.push(
            if index == 0
                || echo_data
                || nested_source_index.is_some_and(|source_index| index >= source_index)
            {
                format!("%~{variable}")
            } else {
                format!("%{variable}")
            },
        );
        let encoded_argument = if percent_sentinel.is_empty() {
            argument.to_owned()
        } else {
            argument.replace('%', &percent_sentinel)
        };
        let environment_value =
            if !echo_data && nested_source_index.is_none_or(|source_index| index <= source_index) {
                double_trailing_backslashes(&encoded_argument)
            } else {
                encoded_argument
            };
        environment.push((OsString::from(key), OsString::from(environment_value)));
    }
    let expression = format!("{for_prefix}{} & {}", clear.join(" & "), command.join(" "));
    Ok(Invocation::HiddenTransport {
        expression,
        environment,
    })
}

#[cfg(test)]
pub(crate) fn prepare_line_break_transport_with_key_check<F>(
    arguments: &[String],
    key_is_taken: F,
) -> Result<Invocation>
where
    F: FnMut(&str) -> bool,
{
    prepare_hidden_transport_with_key_check(arguments, key_is_taken)
}

/// Reconstruct one independently supplied argument as inert CMD data. Caret
/// escaping keeps metacharacters, quotes, expansion markers, and whitespace
/// in the argument that supplied them instead of turning them into syntax.
fn escape_cmd_argument(argument: &str, echo_data: bool) -> Result<String> {
    if argument.is_empty() {
        // Preserve an empty echo operand as the two literal quote characters
        // that native `cmd /C echo ""` emits.
        return Ok(if echo_data {
            "^\"^\"".to_owned()
        } else {
            "\"\"".to_owned()
        });
    }
    if !echo_data && (argument.chars().any(char::is_whitespace) || argument.contains('"')) {
        // CMD does not treat a backslash as a source-level escape, but the
        // Windows argv parser does. Keep the enclosing quotes for whitespace
        // and use `\"` for literal quotes; caret-escape CMD metacharacters so
        // they cannot become operators when an embedded quote temporarily
        // closes the enclosing pair.
        let mut quoted = String::with_capacity(argument.len() + 2);
        let embedded_quote = argument.contains('"');
        quoted.push('"');
        let mut backslashes = 0usize;
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                '&' | '|' | '<' | '>' | '^' | '(' | ')' if embedded_quote => {
                    quoted.extend(std::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                    quoted.push('^');
                    quoted.push(character);
                }
                _ => {
                    quoted.extend(std::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                    quoted.push(character);
                }
            }
        }
        quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
        quoted.push('"');
        return Ok(quoted);
    }

    let mut escaped = String::with_capacity(argument.len());
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            // Outside a quoted argument, a backslash before a quote is the
            // form understood by the child CRT argv parser and preserves the
            // quote as data (for example JSON `{\"a\":1}`).
            '"' if echo_data => {
                escaped.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                escaped.push_str("^\"");
            }
            '"' => {
                escaped.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                backslashes = 0;
                escaped.push('"');
            }
            character
                if character.is_whitespace()
                    || "&|<>^()%".contains(character)
                    || (echo_data && character == '!') =>
            {
                escaped.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                escaped.push('^');
                escaped.push(character);
            }
            _ => {
                escaped.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                escaped.push(character);
            }
        }
    }
    escaped.extend(std::iter::repeat_n('\\', backslashes));
    Ok(escaped)
}

fn source_data_is_safe_for_transport(
    arguments: &[String],
    index: usize,
    echo_data: bool,
    nested_source_index: Option<usize>,
) -> bool {
    if arguments[index].contains(['\r', '\n']) && arguments[index].contains('"') {
        // Quotes alter the FOR capture grammar once delayed data is inserted;
        // rejecting this narrow shape keeps a malformed value from changing
        // the number of command executions.
        return false;
    }
    let source_data = index > 0
        && (echo_data || nested_source_index.is_some_and(|source_index| index >= source_index));
    let nested_cmd = arguments.first().is_some_and(|command| {
        command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe")
    });
    if nested_cmd && !source_data {
        // A nested external command is itself encoded as the child CMD
        // source after `/C`; line-spanning or delayed transport values would
        // become a second source parse. Keep those shapes explicit and fail
        // open instead of guessing at their boundaries.
        return false;
    }
    source_data || index == 0 || !arguments[index].contains('"')
}

fn double_trailing_backslashes(argument: &str) -> String {
    let trailing = argument
        .chars()
        .rev()
        .take_while(|character| *character == '\\')
        .count();
    if trailing == 0 {
        return argument.to_owned();
    }
    let mut escaped = String::with_capacity(argument.len() + trailing);
    escaped.push_str(argument);
    escaped.extend(std::iter::repeat_n('\\', trailing));
    escaped
}

fn unique_percent_sentinel(argument: &str, key: &str) -> String {
    let base = format!("__RTK_CMD_PERCENT_{key}__");
    if !argument.contains(&base) {
        return base;
    }
    let mut suffix = 0usize;
    loop {
        let candidate = format!("{base}{suffix}__");
        if !argument.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Rewrite only cataloged, stateless query segments. Any opaque parser result
/// is binding and therefore executes byte-for-byte through the parent CMD.
#[cfg(test)]
pub fn rewrite_expression(source: &str, rtk_executable: &Path) -> String {
    rewrite_expression_for_terminal(source, rtk_executable, true)
}

/// Rewrite only when stdout is attached to a terminal. Redirected or piped
/// output is machine-consumed data and therefore retains the exact CMD path.
#[cfg(test)]
pub fn rewrite_expression_for_terminal(
    source: &str,
    rtk_executable: &Path,
    stdout_is_terminal: bool,
) -> String {
    rewrite_expression_for_command_line(
        source,
        rtk_executable,
        Path::new("cmd.exe"),
        stdout_is_terminal,
    )
}

pub(crate) fn rewrite_expression_for_command_line(
    source: &str,
    rtk_executable: &Path,
    cmd_executable: &Path,
    stdout_is_terminal: bool,
) -> String {
    if !stdout_is_terminal {
        return source.to_owned();
    }
    let Some(rtk_path) = rtk_executable.to_str() else {
        return source.to_owned();
    };
    if !cmd_interpolation_path_is_safe(rtk_path) {
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
        let eligible = match recognize_command_in(command, &catalog) {
            CommandRecognition::Builtin(AdapterStrategy::Structured { adapter }) => {
                adapters::is_display_form(adapter, original)
            }
            CommandRecognition::Builtin(AdapterStrategy::Identity { .. })
            | CommandRecognition::ExternalRaw
            | CommandRecognition::Unknown => false,
        };
        if !eligible {
            continue;
        }

        let at_prefix = if original.starts_with('@') { "@" } else { "" };
        let replacement = format!(
            "{at_prefix}{} {SEGMENT_RUNNER} --hex {}",
            quote_cmd_path(rtk_path),
            hex_encode(original.as_bytes())
        );
        rewritten.replace_range(segment.span.start..segment.span.end, &replacement);
    }
    if rewritten != source
        && cmd_command_line_utf16_len(cmd_executable, &rewritten) > CMD_COMMAND_LINE_UTF16_LIMIT
    {
        source.to_owned()
    } else {
        rewritten
    }
}

/// Run the public route. This path intentionally does not track the compound
/// expression: only hidden runners that actually filter output record savings.
pub fn run(args: &[OsString]) -> Result<i32> {
    if !cfg!(windows) {
        bail!("rtk cmd is only supported on Windows 10 and 11");
    }
    let cmd_executable = resolve_cmd_executable()?;
    match prepare_invocation(args, &cmd_executable)? {
        Invocation::Passthrough(arguments) => execute_cmd(&cmd_executable, &arguments),
        Invocation::Execute(source) => {
            let executable =
                std::env::current_exe().context("Failed to resolve the current RTK executable")?;
            let expression = rewrite_expression_for_command_line(
                &source,
                &executable,
                &cmd_executable,
                io::stdout().is_terminal(),
            );
            execute_cmd(
                &cmd_executable,
                &[
                    OsString::from("/D"),
                    OsString::from("/S"),
                    OsString::from("/C"),
                    OsString::from(expression),
                ],
            )
        }
        Invocation::Direct(arguments) => execute_direct_external(&arguments),
        Invocation::Reconstructed(source) => {
            let executable =
                std::env::current_exe().context("Failed to resolve the current RTK executable")?;
            let expression = rewrite_expression_for_command_line(
                &source,
                &executable,
                &cmd_executable,
                io::stdout().is_terminal(),
            );
            execute_cmd_expression(&cmd_executable, &expression)
        }
        Invocation::HiddenTransport {
            expression,
            environment,
        } => execute_hidden_transport(&cmd_executable, &expression, &environment),
    }
}

/// Execute one encoded source segment without rewriting it again.
pub fn run_segment(encoded: &str) -> Result<i32> {
    if !cfg!(windows) {
        bail!("rtk cmd is only supported on Windows 10 and 11");
    }
    let timer = crate::core::tracking::TimedExecution::start();
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
        SegmentStdout::Native(output.stdout.clone())
    };
    if let SegmentStdout::Lossless(commit) = &stdout {
        if let Ok(raw) = std::str::from_utf8(&output.stdout) {
            let shown = std::str::from_utf8(commit.as_bytes()).unwrap_or_default();
            timer.track(&source, "rtk cmd (filtered segment)", raw, shown);
        }
    }
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
    crate::core::tee::commit_lossless_if_better_for_cmd(raw, &filtered, reservation).map_or_else(
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

fn execute_cmd(cmd_executable: &Path, arguments: &[OsString]) -> Result<i32> {
    let status = Command::new(cmd_executable)
        .args(arguments)
        .status()
        .context("Failed to execute cmd.exe")?;
    Ok(crate::core::utils::exit_code_from_status(&status, "cmd"))
}

fn execute_cmd_expression(cmd_executable: &Path, expression: &str) -> Result<i32> {
    let mut command = Command::new(cmd_executable);
    command.args(["/D", "/S", "/C"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(expression);
    }
    #[cfg(not(windows))]
    command.arg(expression);

    let status = command.status().context("Failed to execute cmd.exe")?;
    Ok(crate::core::utils::exit_code_from_status(&status, "cmd"))
}

fn execute_direct_external(arguments: &[OsString]) -> Result<i32> {
    let Some((program, args)) = arguments.split_first() else {
        return Ok(0);
    };
    let status = Command::new(program).args(args).status().with_context(|| {
        format!(
            "Failed to execute external command: {}",
            program.to_string_lossy()
        )
    })?;
    Ok(crate::core::utils::exit_code_from_status(&status, "cmd"))
}

fn execute_hidden_transport(
    cmd_executable: &Path,
    expression: &str,
    environment: &[(OsString, OsString)],
) -> Result<i32> {
    let mut command = Command::new(cmd_executable);
    command
        .args(["/D", "/S", "/V:ON", "/C"])
        .envs(environment.iter().map(|(key, value)| (key, value)));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(expression);
    }
    #[cfg(not(windows))]
    command.arg(expression);

    let status = command.status().context("Failed to execute cmd.exe")?;
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

fn cmd_interpolation_path_is_safe(path: &str) -> bool {
    !path
        .chars()
        .any(|character| matches!(character, '%' | '!' | '"' | '\r' | '\n'))
}

fn cmd_command_line_utf16_len(cmd_executable: &Path, expression: &str) -> usize {
    let prefix = format!(
        "{} /D /S /C ",
        quote_cmd_path(&cmd_executable.to_string_lossy())
    );
    prefix.encode_utf16().count() + windows_encoded_argument_utf16_len(expression)
}

/// `Command::arg` uses the Windows C-runtime quoting convention. Count the
/// encoded expression, including its surrounding quotes and quote-adjacent
/// backslash expansion, before asking CMD to accept the rewritten line.
fn windows_encoded_argument_utf16_len(argument: &str) -> usize {
    let needs_quotes = argument.is_empty()
        || argument
            .chars()
            .any(|character| character.is_whitespace() || character == '"');
    if !needs_quotes {
        return argument.encode_utf16().count();
    }

    let mut length = 2usize;
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                length += backslashes * 2 + 2;
                backslashes = 0;
            }
            _ => {
                length += backslashes + character.len_utf16();
                backslashes = 0;
            }
        }
    }
    length + backslashes * 2
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
