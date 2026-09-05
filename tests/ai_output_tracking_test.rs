use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

fn copy_executable(source: &Path, destination: &Path) {
    std::fs::copy(source, destination).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(destination).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(destination, permissions).unwrap();
    }
}

fn run_unknown_helper(helper: &Path, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rtk"))
        .arg(helper)
        .arg("--version")
        .env("RTK_DB_PATH", database)
        .output()
        .unwrap()
}

#[test]
fn unknown_exact_fallback_persists_contract_and_reason() {
    let temp = tempdir().unwrap();
    let helper = temp.path().join(if cfg!(windows) {
        "exact-helper.exe"
    } else {
        "exact-helper"
    });
    copy_executable(Path::new(env!("CARGO_BIN_EXE_rtk")), &helper);
    let database = temp.path().join("history.db");

    let output = run_unknown_helper(&helper, &database);
    assert!(output.status.success(), "stderr={:?}", output.stderr);

    let connection = rusqlite::Connection::open(database).unwrap();
    let stored: (String, Option<String>, i64, i64) = connection
        .query_row(
            "SELECT output_contract, exact_reason, input_tokens, output_tokens
             FROM commands ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(stored, ("exact".into(), Some("unknown".into()), 1, 1));
}

#[cfg(windows)]
fn make_helper(path: &Path) -> Vec<String> {
    let command = std::env::var_os("COMSPEC")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows\System32\cmd.exe"));
    copy_executable(&command, path);
    vec![
        "/D".into(),
        "/S".into(),
        "/C".into(),
        "for /L %i in (1,1,120) do @echo record-%i xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".into(),
    ]
}

#[cfg(unix)]
fn make_helper(path: &Path) -> Vec<String> {
    copy_executable(Path::new("/bin/sh"), path);
    vec![
        "-c".into(),
        "i=1; while [ $i -le 120 ]; do echo record-$i xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done".into(),
    ]
}

#[test]
fn toml_semantic_fallback_persists_emission_metadata() {
    let temp = tempdir().unwrap();
    let helper = temp
        .path()
        .join(if cfg!(windows) { "make.exe" } else { "make" });
    let helper_args = make_helper(&helper);
    let database = temp.path().join("history.db");
    let tee_dir = temp.path().join("lossless artifacts");

    let output = Command::new(env!("CARGO_BIN_EXE_rtk"))
        .arg(&helper)
        .args(helper_args)
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr={:?}", output.stderr);

    let connection = rusqlite::Connection::open(database).unwrap();
    let stored: (String, Option<String>, i64, i64, bool, bool) = connection
        .query_row(
            "SELECT output_contract, exact_reason, omitted_items, omitted_groups,
                    recovery_created, filter_failed
             FROM commands ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(stored.0, "ai_owned");
    assert_eq!(stored.1, None);
    assert!(stored.2 > 0 || stored.3 > 0, "stored={stored:?}");
    assert!(stored.4, "stored={stored:?}");
    assert!(!stored.5, "stored={stored:?}");
}
