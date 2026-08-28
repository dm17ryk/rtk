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
    assert!(
        !injected.exists(),
        "embedded metacharacters must stay data, not execute a redirected command"
    );
}
