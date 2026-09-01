//! Public PowerShell host invocation and conservative output filtering.

use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::adapters;
use super::catalog::{self, AdapterStrategy};
use super::parser::{parse_expression, ParsedScript};
use super::transport::OutputSpool;

static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerShellHost {
    WindowsPowerShell,
    Pwsh,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostDialect {
    Desktop51,
    Pwsh,
}

impl PowerShellHost {
    fn executable(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "powershell.exe",
            Self::Pwsh => "pwsh.exe",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "powershell",
            Self::Pwsh => "pwsh",
        }
    }

    pub fn dialect(self) -> HostDialect {
        match self {
            Self::WindowsPowerShell => HostDialect::Desktop51,
            Self::Pwsh => HostDialect::Pwsh,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Invocation {
    Passthrough(Vec<OsString>),
    Command {
        host_args: Vec<OsString>,
        expression: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewriteDecision {
    Filter { adapter: AdapterStrategy },
    Raw { reason: &'static str },
}

/// Separate host flags from a one-shot source expression without interpreting
/// the expression itself. Explicit transport modes stay byte-for-byte native.
pub fn prepare_invocation(args: &[OsString]) -> Invocation {
    if args.is_empty() {
        return Invocation::Passthrough(Vec::new());
    }

    // -Command/-c is filterable when it has exactly one source argument. The
    // other execution modes have semantics that cannot be reconstructed safely.
    if let Some(command_index) = find_command_mode(args) {
        if command_index + 1 < args.len() && command_index + 2 == args.len() {
            return Invocation::Command {
                host_args: args[..command_index].to_vec(),
                expression: args[command_index + 1].to_string_lossy().into_owned(),
            };
        }
        return Invocation::Passthrough(args.to_vec());
    }
    if contains_opaque_execution_mode(args) {
        return Invocation::Passthrough(args.to_vec());
    }

    let mut host_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let token = args[index].to_string_lossy();
        let lower = token.to_ascii_lowercase();
        if !is_host_option(&lower) {
            break;
        }
        host_args.push(args[index].clone());
        if host_option_takes_value(&lower) {
            if let Some(value) = args.get(index + 1) {
                host_args.push(value.clone());
                index += 1;
            }
        }
        index += 1;
    }

    let expression_args = &args[index..];
    if expression_args.is_empty() {
        return Invocation::Passthrough(host_args);
    }
    Invocation::Command {
        host_args,
        expression: reconstruct_expression(expression_args),
    }
}

#[allow(dead_code)]
pub fn classify(parsed: &ParsedScript, host_args: &[OsString]) -> RewriteDecision {
    classify_expression(parsed, "", host_args)
}

pub fn classify_expression(
    parsed: &ParsedScript,
    source: &str,
    host_args: &[OsString],
) -> RewriteDecision {
    if parsed.is_opaque() {
        return RewriteDecision::Raw {
            reason: "parser marked the source opaque",
        };
    }
    if source.len() > 32 * 1024 {
        return RewriteDecision::Raw {
            reason: "source exceeds safe wrapper command-line limits",
        };
    }
    if host_args.iter().enumerate().any(|(index, arg)| {
        let value = arg.to_string_lossy().to_ascii_lowercase();
        let paired_value = host_args
            .get(index + 1)
            .map(|next| next.to_string_lossy().to_ascii_lowercase());
        matches!(
            value.as_str(),
            "-noexit"
                | "-login"
                | "-interactive"
                | "-sshservermode"
                | "-file"
                | "-f"
                | "-encodedcommand"
                | "-e"
                | "-ec"
                | "-encodedarguments"
                | "-encodedargs"
                | "-stdin"
        ) || value.starts_with("-outputformat:xml")
            || value.starts_with("-inputformat:")
            || ((value == "-outputformat" || value == "-inputformat")
                && paired_value.as_deref().is_some_and(|next| next == "xml"))
    }) {
        return RewriteDecision::Raw {
            reason: "host flags select a long-running or machine transport mode",
        };
    }
    if source
        .split(|character: char| character.is_whitespace() || character == ',' || character == ';')
        .map(|token| token.trim_matches(['(', ')', '{', '}', '[', ']']))
        .map(str::to_ascii_lowercase)
        .any(|token| {
            matches!(
                token.as_str(),
                "-asjob" | "-wait" | "-follow" | "-watch" | "-continuous"
            )
        })
    {
        return RewriteDecision::Raw {
            reason: "long-running or background parameters are native-only",
        };
    }
    let lowered_source = source.to_ascii_lowercase();
    if lowered_source.contains("read-host")
        || lowered_source.contains("readkey")
        || lowered_source.contains("$input")
    {
        return RewriteDecision::Raw {
            reason: "stdin and interactive reads remain native-only",
        };
    }
    let names = parsed.command_names();
    if names.len() != 1 {
        return RewriteDecision::Raw {
            reason: "compound expressions retain native state and stream ordering",
        };
    }
    let command = &names[0];
    if matches!(
        crate::core::utils::resolve_host_command(command),
        crate::core::utils::HostCommand::Executable(_)
    ) {
        return RewriteDecision::Raw {
            reason: "resolved native applications and scripts remain opaque",
        };
    }
    if command.contains('/')
        || command.contains('\\')
        || command.ends_with(".exe")
        || command.ends_with(".com")
        || command.ends_with(".bat")
        || command.ends_with(".cmd")
    {
        return RewriteDecision::Raw {
            reason: "native executable output is opaque",
        };
    }
    match catalog::strategy_for(command) {
        AdapterStrategy::Identity => RewriteDecision::Raw {
            reason: "identity commands must retain their native display",
        },
        adapter => RewriteDecision::Filter { adapter },
    }
}

pub fn classify_for_host(
    host: PowerShellHost,
    parsed: &ParsedScript,
    source: &str,
    host_args: &[OsString],
) -> RewriteDecision {
    let _dialect = host.dialect();
    classify_expression(parsed, source, host_args)
}

pub fn run(host: PowerShellHost, args: &[OsString]) -> Result<i32> {
    if !cfg!(windows) {
        bail!(
            "rtk {} is only supported on Windows 10 and 11",
            host.display_name()
        );
    }
    let executable = crate::core::utils::resolve_binary(host.executable())
        .with_context(|| format!("Failed to resolve {} from PATH", host.executable()))?;
    let invocation = prepare_invocation(args);
    match invocation {
        Invocation::Passthrough(arguments) => run_passthrough(&executable, host, &arguments),
        Invocation::Command {
            host_args,
            expression,
        } => {
            let parsed = parse_expression(&expression);
            let decision = classify_for_host(host, &parsed, &expression, &host_args);
            if filtering_requested() {
                if let RewriteDecision::Filter { adapter } = decision {
                    return run_filtered_command(
                        &executable,
                        host,
                        &host_args,
                        &expression,
                        parsed
                            .command_names()
                            .first()
                            .map(String::as_str)
                            .unwrap_or("unknown"),
                        adapter,
                    );
                }
            }
            run_command(&executable, host, &host_args, &expression)
        }
    }
}

/// Transport entry point used by hidden helpers and tests. It deliberately
/// bypasses the RTK router and executes one already-decoded source expression
/// in the selected native host.
pub fn run_raw(host: PowerShellHost, expression: &str) -> Result<i32> {
    if !cfg!(windows) {
        bail!(
            "rtk {} is only supported on Windows 10 and 11",
            host.display_name()
        );
    }
    let executable = crate::core::utils::resolve_binary(host.executable())
        .with_context(|| format!("Failed to resolve {} from PATH", host.executable()))?;
    run_command(&executable, host, &[], expression)
}

fn run_passthrough(executable: &PathBuf, host: PowerShellHost, args: &[OsString]) -> Result<i32> {
    let status = Command::new(executable)
        .args(args)
        .status()
        .with_context(|| format!("Failed to execute {}", host.executable()))?;
    Ok(crate::core::utils::exit_code_from_status(
        &status,
        host.display_name(),
    ))
}

fn run_command(
    executable: &PathBuf,
    host: PowerShellHost,
    host_args: &[OsString],
    expression: &str,
) -> Result<i32> {
    let status = Command::new(executable)
        .args(host_args)
        .arg("-Command")
        .arg(expression)
        .env_remove("RTK_POWERSHELL_FILTER")
        .status()
        .with_context(|| format!("Failed to execute {}", host.executable()))?;
    Ok(crate::core::utils::exit_code_from_status(
        &status,
        host.display_name(),
    ))
}

fn run_filtered_command(
    executable: &PathBuf,
    host: PowerShellHost,
    host_args: &[OsString],
    expression: &str,
    command_name: &str,
    strategy: AdapterStrategy,
) -> Result<i32> {
    let execution_expression = runtime_probe_expression(expression, command_name);
    let spool = match OutputSpool::create() {
        Ok(spool) => spool,
        Err(_) => return run_command(executable, host, host_args, expression),
    };
    let output = match Command::new(executable)
        .args(host_args)
        .arg("-Command")
        .arg(&execution_expression)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("RTK_POWERSHELL_FILTER")
        .output()
    {
        Ok(output) => output,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            return run_command(executable, host, host_args, expression)
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to execute {}", host.executable()))
        }
    };

    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    // A captured stderr stream may represent PowerShell streams 2-6. Keep the
    // complete native display and ordering boundary instead of compacting only
    // stream 1 when any auxiliary stream was emitted.
    if !output.status.success() || !output.stderr.is_empty() {
        stdout.write_all(&output.stdout).ok();
        stderr.write_all(&output.stderr).ok();
        return Ok(crate::core::utils::exit_code_from_status(
            &output.status,
            host.display_name(),
        ));
    }

    if spool.write(&output.stdout).is_err() {
        stdout.write_all(&output.stdout).ok();
        return Ok(crate::core::utils::exit_code_from_status(
            &output.status,
            host.display_name(),
        ));
    }
    let raw_owned = spool
        .read_utf8()
        .unwrap_or_else(|_| String::from_utf8_lossy(&output.stdout).into_owned());
    let raw = raw_owned.as_str();
    let adapter_name = match strategy {
        AdapterStrategy::Specialized(name) => name,
        AdapterStrategy::Generic => "generic",
        AdapterStrategy::Identity => "identity",
    };
    let Some(filtered) = adapters::filter_output(adapter_name, raw) else {
        stdout.write_all(&output.stdout).ok();
        return Ok(crate::core::utils::exit_code_from_status(
            &output.status,
            host.display_name(),
        ));
    };
    let Some(reservation) = crate::core::tee::reserve_lossless_tee(raw, host.display_name()) else {
        stdout.write_all(&output.stdout).ok();
        return Ok(crate::core::utils::exit_code_from_status(
            &output.status,
            host.display_name(),
        ));
    };
    let Some(commit) =
        crate::core::tee::commit_lossless_if_better_for_powershell(raw, &filtered, reservation)
    else {
        stdout.write_all(&output.stdout).ok();
        return Ok(crate::core::utils::exit_code_from_status(
            &output.status,
            host.display_name(),
        ));
    };
    stdout.write_all(commit.as_bytes()).ok();
    crate::core::tracking::TimedExecution::start().track(
        expression,
        &format!("rtk {}", host.display_name()),
        raw,
        std::str::from_utf8(commit.as_bytes()).unwrap_or_default(),
    );
    Ok(crate::core::utils::exit_code_from_status(
        &output.status,
        host.display_name(),
    ))
}

fn runtime_probe_expression(expression: &str, command_name: &str) -> String {
    let sequence = PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let identifier = format!("__rtk_probe_{timestamp:x}_{sequence:x}");
    let quoted_name = command_name.replace('\'', "''");
    format!(
        "${identifier}=Microsoft.PowerShell.Core\\Get-Command -Name '{quoted_name}' -ErrorAction SilentlyContinue; {expression}"
    )
}

fn filtering_requested() -> bool {
    io::stdout().is_terminal()
        || std::env::var("RTK_POWERSHELL_FILTER")
            .ok()
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

fn contains_opaque_execution_mode(args: &[OsString]) -> bool {
    find_initial_mode(args).is_some_and(|mode| {
        let mode = mode.to_ascii_lowercase();
        matches!(
            mode.as_str(),
            "-file"
                | "-f"
                | "-encodedcommand"
                | "-e"
                | "-ec"
                | "-encodedarguments"
                | "-encodedargs"
                | "-commandwithargs"
                | "-stdin"
        )
    })
}

fn is_command_mode(token: &str) -> bool {
    matches!(token.to_ascii_lowercase().as_str(), "-command" | "-c")
}

fn find_command_mode(args: &[OsString]) -> Option<usize> {
    find_initial_mode_with_index(args)
        .and_then(|(index, mode)| is_command_mode(mode).then_some(index))
}

fn find_initial_mode(args: &[OsString]) -> Option<&str> {
    find_initial_mode_with_index(args).map(|(_, mode)| mode)
}

fn find_initial_mode_with_index(args: &[OsString]) -> Option<(usize, &str)> {
    let mut index = 0;
    while index < args.len() {
        let token = args[index].to_string_lossy();
        let lower = token.to_ascii_lowercase();
        if is_host_option(&lower) {
            index += 1 + usize::from(host_option_takes_value(&lower));
            continue;
        }
        if lower.starts_with('-') {
            return Some((index, args[index].to_str().unwrap_or_default()));
        }
        break;
    }
    None
}

fn is_host_option(token: &str) -> bool {
    matches!(
        token,
        "-nologo"
            | "-noexit"
            | "-noprofile"
            | "-noninteractive"
            | "-sta"
            | "-mta"
            | "-version"
            | "-inputformat"
            | "-outputformat"
            | "-windowstyle"
            | "-configurationname"
            | "-executionpolicy"
            | "-psconsolefile"
            | "-login"
            | "-interactive"
            | "-noprofileloadtime"
            | "-sshservermode"
            | "-help"
            | "-?"
            | "/?"
    ) || token.starts_with("-inputformat:")
        || token.starts_with("-outputformat:")
        || token.starts_with("-executionpolicy:")
        || token.starts_with("-configurationname:")
        || token.starts_with("-windowstyle:")
        || token.starts_with("-psconsolefile:")
        || token.starts_with("-workingdirectory:")
}

fn host_option_takes_value(token: &str) -> bool {
    matches!(
        token,
        "-version"
            | "-inputformat"
            | "-outputformat"
            | "-windowstyle"
            | "-configurationname"
            | "-executionpolicy"
            | "-psconsolefile"
            | "-settingsfile"
            | "-configurationfile"
            | "-custompipename"
            | "-workingdirectory"
    )
}

fn reconstruct_expression(args: &[OsString]) -> String {
    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            let value = arg.to_string_lossy();
            if index == 0 || value.starts_with('-') {
                value.into_owned()
            } else {
                quote_argument(&value)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[allow(dead_code)]
fn resolve_host_path(host: PowerShellHost) -> Result<PathBuf> {
    crate::core::utils::resolve_binary(host.executable())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn command_mode_keeps_a_single_expression_filterable() {
        assert_eq!(
            prepare_invocation(&args(&["-NoProfile", "-Command", "Get-Process"])),
            Invocation::Command {
                host_args: args(&["-NoProfile"]),
                expression: "Get-Process".to_string(),
            }
        );
    }

    #[test]
    fn encoded_and_file_modes_are_exact_passthrough() {
        let input = args(&["-File", "script.ps1"]);
        assert_eq!(prepare_invocation(&input), Invocation::Passthrough(input));
    }

    #[test]
    fn positional_arguments_are_quoted_as_data() {
        assert_eq!(
            prepare_invocation(&args(&["Write-Output", "a & b", "O'Reilly"])),
            Invocation::Command {
                host_args: Vec::new(),
                expression: "Write-Output 'a & b' 'O''Reilly'".to_string(),
            }
        );
    }

    #[test]
    fn command_like_argument_inside_expression_is_not_a_host_flag() {
        assert!(matches!(
            prepare_invocation(&args(&["Write-Output", "-Command"])),
            Invocation::Command { expression, .. } if expression == "Write-Output -Command"
        ));
    }

    #[test]
    fn compound_and_native_commands_fail_open() {
        let parsed = parse_expression("Get-Process | sort");
        assert!(matches!(
            classify(&parsed, &[]),
            RewriteDecision::Raw { .. }
        ));
        let parsed = parse_expression("cargo --version");
        assert!(matches!(
            classify(&parsed, &[]),
            RewriteDecision::Raw { .. }
        ));
        let parsed = parse_expression("C:\\Windows\\System32\\whoami.exe");
        assert!(matches!(
            classify(&parsed, &[]),
            RewriteDecision::Raw { .. }
        ));
    }

    #[test]
    fn long_running_parameters_fail_open() {
        let parsed = parse_expression("Get-Process -AsJob");
        assert!(matches!(
            classify_expression(&parsed, "Get-Process -AsJob", &[]),
            RewriteDecision::Raw { .. }
        ));
    }

    #[test]
    fn runtime_probe_is_same_runspace_and_uses_a_randomized_identifier() {
        let wrapped = runtime_probe_expression("Get-Process", "Get-Process");
        assert!(wrapped.contains("Microsoft.PowerShell.Core\\Get-Command"));
        assert!(wrapped.contains("Get-Process"));
        assert!(wrapped.contains("__rtk_probe_"));
    }

    #[test]
    fn host_routes_expose_distinct_dialects() {
        assert_eq!(
            PowerShellHost::WindowsPowerShell.dialect(),
            HostDialect::Desktop51
        );
        assert_eq!(PowerShellHost::Pwsh.dialect(), HostDialect::Pwsh);
    }

    #[test]
    fn xml_output_modes_fail_open_even_when_value_is_separate() {
        let parsed = parse_expression("Get-Process");
        assert!(matches!(
            classify_for_host(
                PowerShellHost::Pwsh,
                &parsed,
                "Get-Process",
                &args(&["-OutputFormat", "XML"])
            ),
            RewriteDecision::Raw { .. }
        ));
    }

    #[test]
    fn stdin_reads_fail_open() {
        let parsed = parse_expression("Read-Host Name");
        assert!(matches!(
            classify_expression(&parsed, "Read-Host Name", &[]),
            RewriteDecision::Raw { .. }
        ));
    }
}
