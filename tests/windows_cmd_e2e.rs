#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn native_cmd(expression: &str) -> Output {
    Command::new("cmd.exe")
        .args(["/D", "/S", "/C", expression])
        .output()
        .expect("native cmd.exe should start")
}

fn rtk_cmd(expression: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", expression])
        .output()
        .expect("rtk cmd should start")
}

fn assert_cmd_parity(expression: &str) {
    let native = native_cmd(expression);
    let rtk = rtk_cmd(expression);

    assert_eq!(rtk.status.code(), native.status.code(), "{expression}");
    assert_eq!(rtk.stdout, native.stdout, "{expression}");
    assert_eq!(rtk.stderr, native.stderr, "{expression}");
}

fn assert_cmd_parity_in(expression: &str, current_dir: &Path) {
    let native = Command::new("cmd.exe")
        .current_dir(current_dir)
        .args(["/D", "/S", "/C", expression])
        .output()
        .expect("native cmd.exe should start");
    let rtk = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .current_dir(current_dir)
        .args(["cmd", expression])
        .output()
        .expect("rtk cmd should start");

    assert!(native.status.success(), "native cmd failed: {expression}");
    assert_eq!(rtk.status.code(), native.status.code(), "{expression}");
    assert_eq!(rtk.stdout, native.stdout, "{expression}");
    assert_eq!(rtk.stderr, native.stderr, "{expression}");
}

#[test]
fn query_chains_keep_cmd_operator_and_stateful_semantics() {
    assert_cmd_parity("echo %CD% & dir /b");
    assert_cmd_parity("set RTK_CMD_E2E=kept & echo %RTK_CMD_E2E% & set RTK_CMD_E2E=");
    assert_cmd_parity("cd /D . & dir /b");
    assert_cmd_parity("cmd /D /S /C \"exit /b 0\" && echo success || echo failure");
    assert_cmd_parity("cmd /D /S /C \"exit /b 1\" && echo success || echo failure");
}

#[test]
fn unicode_spaces_and_failures_have_native_parity() {
    let directory = tempdir().unwrap();
    let unicode_dir = directory.path().join("spaced Привет");
    fs::create_dir(&unicode_dir).unwrap();
    fs::write(unicode_dir.join("данные.txt"), "payload").unwrap();

    assert_cmd_parity_in("dir /b", &unicode_dir);
    assert_cmd_parity("exit /b 37");
}

#[test]
fn redirection_and_batch_input_fail_open_to_native_cmd() {
    let directory = tempdir().unwrap();
    let redirected = directory.path().join("listing.txt");
    let batch = directory.path().join("returns-23.cmd");
    fs::write(&batch, "@echo batch:%~1\r\n@exit /b 23\r\n").unwrap();

    let redirect_expression = format!("echo redirected > {}", redirected.display());
    assert_cmd_parity(&redirect_expression);
    assert_eq!(
        fs::read_to_string(&redirected).unwrap().trim(),
        "redirected"
    );

    assert_cmd_parity(&format!("{} hello", batch.display()));

    let input = directory.path().join("input.txt");
    fs::write(&input, "input through redirect\r\n").unwrap();
    assert_cmd_parity(&format!("type < {}", input.display()));
}

#[test]
fn multi_argument_embedded_quote_and_metacharacters_do_not_execute_an_extra_command() {
    let directory = tempdir().unwrap();
    let injected = directory.path().join("must-not-exist.txt");
    let payload = format!(r#"safe" & echo injected > {}"#, injected.display());

    let output = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", "echo", &payload])
        .output()
        .expect("rtk cmd should start");

    assert!(output.status.success());
    assert_eq!(output.stdout, format!("{payload}\r\n").as_bytes());
    assert!(
        !injected.exists(),
        "embedded metacharacters must stay data, not execute a redirected command"
    );
}

#[test]
fn multi_argument_empty_and_bang_values_match_default_cmd_semantics() {
    let empty_native = native_cmd(r#"echo """#);
    let empty_rtk = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", "echo", ""])
        .output()
        .expect("rtk cmd should start");
    assert_eq!(empty_rtk.status.code(), empty_native.status.code());
    assert_eq!(empty_rtk.stdout, empty_native.stdout);
    assert_eq!(empty_rtk.stderr, empty_native.stderr);

    let bang_native = native_cmd("echo !RTK_CMD_UNSET!");
    let bang_rtk = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", "echo", "!RTK_CMD_UNSET!"])
        .output()
        .expect("rtk cmd should start");
    assert_eq!(bang_rtk.status.code(), bang_native.status.code());
    assert_eq!(bang_rtk.stdout, bang_native.stdout);
    assert_eq!(bang_rtk.stderr, bang_native.stderr);
}

#[test]
fn multi_argument_percent_and_crlf_payloads_remain_data() {
    let directory = tempdir().unwrap();
    let percent_injected = directory.path().join("percent-must-not-exist.txt");
    let percent_payload = format!(
        "100% complete & echo injected > {}",
        percent_injected.display()
    );
    let percent_output = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", "echo", &percent_payload])
        .output()
        .expect("rtk cmd should start");
    assert!(percent_output.status.success());
    assert_eq!(
        percent_output.stdout,
        format!("{percent_payload}\r\n").as_bytes()
    );
    assert!(
        !percent_injected.exists(),
        "percent-bearing payload must not create a redirected marker"
    );

    let crlf_injected = directory.path().join("crlf-must-not-exist.txt");
    let crlf_payload = format!(
        "first line\r\n& echo injected > {}",
        crlf_injected.display()
    );
    let crlf_output = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .args(["cmd", "echo", &crlf_payload])
        .output()
        .expect("rtk cmd should start");
    assert!(crlf_output.status.success());
    assert_eq!(crlf_output.stdout, format!("{crlf_payload}\r\n").as_bytes());
    assert!(
        !crlf_injected.exists(),
        "CR/LF payload must not create a redirected marker"
    );
}
