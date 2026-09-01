use std::process::Command;

fn rtk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rtk"))
}

#[cfg(windows)]
#[test]
fn powershell_route_executes_an_explicit_command() {
    let output = rtk()
        .args([
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Write-Output rtk-powershell-smoke",
        ])
        .output()
        .expect("rtk powershell should launch");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "rtk-powershell-smoke"
    );
}

#[cfg(windows)]
#[test]
fn pwsh_route_executes_a_raw_expression() {
    let output = rtk()
        .args([
            "pwsh",
            "-NoProfile",
            "-NonInteractive",
            "Write-Output rtk-pwsh-smoke",
        ])
        .output()
        .expect("rtk pwsh should launch");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "rtk-pwsh-smoke"
    );
}

#[cfg(windows)]
#[test]
fn powershell_multiple_arguments_preserve_metacharacters_as_data() {
    let output = rtk()
        .args([
            "pwsh",
            "-NoProfile",
            "-NonInteractive",
            "Write-Output",
            "a & b",
        ])
        .output()
        .expect("rtk pwsh should launch");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "a & b");
}

#[cfg(windows)]
#[test]
fn pwsh_model_facing_filter_uses_runtime_probe_without_leaking_transport_errors() {
    let output = rtk()
        .env("RTK_POWERSHELL_FILTER", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args([
            "pwsh",
            "-NoProfile",
            "-NonInteractive",
            "Get-Process",
            "-Name",
            "System",
        ])
        .output()
        .expect("rtk pwsh filter should launch");

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("__rtk_probe_"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("System"), "stdout: {:?}", output.stdout);
}

#[cfg(not(windows))]
#[test]
fn powershell_routes_report_windows_only() {
    let output = rtk()
        .args(["powershell", "Write-Output nope"])
        .output()
        .expect("rtk should launch");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Windows"));
}
