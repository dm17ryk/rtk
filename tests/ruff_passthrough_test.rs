//! `rtk ruff` bounds subcommands it has no specialized filter for with recovery,
//! except `analyze graph`, whose JSON is only useful whole. These run the real `run()`
//! path against a stub ruff that replays output captured with ruff 0.16.6: the graph of
//! a 300-module package (`ruff analyze graph big`) and `ruff rule E501`.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;

mod common;

const GRAPH: &str = include_str!("fixtures/ruff_analyze_graph_raw.json");
const RULE_E501: &str = include_str!("fixtures/ruff_rule_e501_raw.txt");

/// Writes an executable `ruff` in `dir` that prints `fixture` when called with as many
/// arguments as `expected_args`, joined to the same text, and exits 99 otherwise.
fn fake_ruff(dir: &Path, expected_args: &[&str], fixture: &str) {
    let joined = expected_args.join(" ");
    #[cfg(unix)]
    let count = expected_args.len();
    fs::write(dir.join("captured"), fixture).expect("write fixture");
    #[cfg(unix)]
    {
        let path = dir.join("ruff");
        fs::write(
        &path,
        format!(
            "#!/bin/sh\n[ $# = {count} ] && [ \"$*\" = \"{joined}\" ] || {{ echo \"ruff got $# args: $*\" >&2; exit 99; }}\ncat \"$(dirname \"$0\")/captured\"\n"
        ),
    )
    .expect("write fake ruff");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake ruff");
    }
    #[cfg(windows)]
    fs::write(
        dir.join("ruff.cmd"),
        format!(
            "@echo off\r\nif not \"%*\"==\"{joined}\" exit /b 99\r\ntype \"%~dp0captured\"\r\n",
        ),
    )
    .expect("write fake ruff");
}

/// Runs rtk with `dir` first on PATH, so the fake ruff shadows any real one.
fn rtk_ruff(dir: &Path, args: &[&str]) -> Output {
    let path = std::env::join_paths(std::iter::once(dir.to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH with Ruff stub");
    common::rtk_command()
        .arg("ruff")
        .args(args)
        .env("PATH", path)
        .output()
        .expect("run rtk")
}

/// What a failed assertion needs to show: the exit status and stderr.
fn context(out: &Output) -> String {
    format!(
        "exit {:?}, stderr {:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn analyze_graph_json_reaches_the_caller_whole() {
    let dir = common::tempdir();
    // The rule-output test independently verifies compaction and complete recovery.
    assert!(GRAPH.len() > RULE_E501.len());
    fake_ruff(dir.path(), &["analyze", "graph", "big"], GRAPH);

    let out = rtk_ruff(dir.path(), &["analyze", "graph", "big"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(out.status.code(), Some(0), "{}", context(&out));
    assert_eq!(out.stdout, GRAPH.as_bytes(), "{}", context(&out));
    serde_json::from_str::<serde_json::Value>(&stdout).expect("graph must stay valid JSON");
}

#[test]
fn other_passthrough_subcommands_are_bounded_and_losslessly_recoverable() {
    let dir = common::tempdir();
    fake_ruff(dir.path(), &["rule", "E501"], RULE_E501);

    let out = rtk_ruff(dir.path(), &["rule", "E501"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let shown = stdout.trim_end();

    assert_eq!(out.status.code(), Some(0), "{}", context(&out));
    assert!(
        shown.len() < RULE_E501.len() && shown.contains("omitted items="),
        "rule output must be compact with disclosed omissions, got {} of {} bytes; {}",
        shown.len(),
        RULE_E501.len(),
        context(&out)
    );
    let id = shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .expect("compaction must advertise recovery")
        .1;
    let recovered = common::rtk_command()
        .args(["read", "-l", "none", "--recovery", id])
        .output()
        .expect("read full Ruff output");
    assert!(recovered.status.success(), "{}", context(&recovered));
    assert_eq!(recovered.stdout, RULE_E501.as_bytes());
}
