//! Keep failed native output distinct from unavailable or normalized success baselines.

use std::ffi::OsString;
use std::fs;

mod common;

fn ls_fixture(program: &str) -> (tempfile::TempDir, OsString) {
    let dir = common::tempdir();
    let source = dir.path().join("ls_fixture.rs");
    let binary = dir
        .path()
        .join(format!("ls{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source, program).expect("write ls fixture");
    let compiled = common::native_command("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile ls fixture");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let path = std::env::join_paths(std::iter::once(dir.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("fixture PATH");
    (dir, path)
}

#[test]
fn failed_native_listing_does_not_leak_the_parse_aid_header() {
    let (_dir, path) = ls_fixture(
        r#"fn main() {
    if std::env::args().skip(1).any(|arg| arg == "-l" || arg == "-la") {
        println!("total 0");
    }
    eprintln!("ls-fixture: locked: Permission denied");
    std::process::exit(1);
}"#,
    );
    for args in [["locked"].as_slice(), ["-A", "locked"].as_slice()] {
        let output = common::rtk_command()
            .arg("ls")
            .args(args)
            .env("PATH", &path)
            .output()
            .expect("run rtk ls with failed native listing");
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("ls-fixture:"));
        assert!(output.stdout.is_empty(), "{args:?}: {:?}", output.stdout);
    }
}

#[test]
fn a_normalized_option_does_not_turn_native_rejection_into_empty_success() {
    let (_dir, path) = ls_fixture(
        r#"fn main() {
    if std::env::args().skip(1).any(|arg| arg == "--all") {
        eprintln!("ls-fixture: unsupported --all");
        std::process::exit(1);
    }
    println!("total 1");
    println!("-rw-r--r-- 1 user group 1 Jan  1 00:00 visible.txt");
}"#,
    );
    let output = common::rtk_command()
        .args(["ls", "--all", "."])
        .env("PATH", &path)
        .output()
        .expect("run rtk ls with normalized option");
    assert!(
        !output.status.success() || String::from_utf8_lossy(&output.stdout).contains("visible.txt"),
        "native rejection became an empty or incomplete success: {:?}",
        output.stdout,
    );
}
