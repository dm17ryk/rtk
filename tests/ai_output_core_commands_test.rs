use std::process::{Command, Output};

use rusqlite::Connection;
use tempfile::tempdir;

#[derive(Debug)]
struct Metrics {
    contract: String,
    input_tokens: i64,
    output_tokens: i64,
    recovery_created: bool,
}

fn rtk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rtk"))
}

fn run_rtk(database: &std::path::Path, tee_dir: &std::path::Path, args: &[&str]) -> Output {
    rtk()
        .args(args)
        .env("RTK_DB_PATH", database)
        .env("RTK_TEE_DIR", tee_dir)
        .output()
        .unwrap()
}

fn latest_metrics(database: &std::path::Path) -> Metrics {
    let connection = Connection::open(database).unwrap();
    connection
        .query_row(
            "SELECT output_contract, input_tokens, output_tokens, recovery_created
             FROM commands ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok(Metrics {
                    contract: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    recovery_created: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .unwrap()
}

fn recovery_id(shown: &str) -> &str {
    shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .map(|(_, identifier)| identifier.trim())
        .expect("expected compact output to advertise lossless recovery")
}

fn assert_ai_saves(name: &str, metrics: &Metrics) {
    assert_eq!(metrics.contract, "ai_owned", "{name}: {metrics:?}");
    assert!(
        metrics.output_tokens < metrics.input_tokens,
        "{name}: {metrics:?}"
    );
}

#[test]
fn core_command_fixtures_are_ai_owned_smaller_and_recoverable() {
    let temp = tempdir().unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");

    let source = temp.path().join("large.rs");
    let source_text = (1..=500)
        .map(|line| {
            format!(
                "// documentation noise for line {line} that AI does not need\nlet retained_{line} = {line};\n"
            )
        })
        .collect::<String>();
    std::fs::write(&source, &source_text).unwrap();
    let source = source.to_str().unwrap();
    let read = run_rtk(&database, &tee_dir, &["read", source]);
    assert!(read.status.success(), "stderr={:?}", read.stderr);
    let read_shown = String::from_utf8(read.stdout).unwrap();
    assert!(
        read_shown.starts_with("status=source"),
        "shown={read_shown}"
    );
    assert!(
        read_shown.contains("2: let retained_1 = 1;"),
        "shown={read_shown}"
    );
    let read_metrics = latest_metrics(&database);
    assert_ai_saves("read", &read_metrics);
    assert!(read_metrics.recovery_created, "read: {read_metrics:?}");
    let recovered = run_rtk(
        &database,
        &tee_dir,
        &["read", "-l", "none", "--recovery", recovery_id(&read_shown)],
    );
    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, source_text.as_bytes());

    let selected_source = temp.path().join("selected.txt");
    std::fs::write(
        &selected_source,
        (1..=20)
            .map(|line| format!("selected line {line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let selected_read = run_rtk(
        &database,
        &tee_dir,
        &["read", selected_source.to_str().unwrap(), "-m", "2"],
    );
    assert!(
        selected_read.status.success(),
        "stderr={:?}",
        selected_read.stderr
    );
    let selected_read = String::from_utf8(selected_read.stdout).unwrap();
    assert!(
        !selected_read.contains("recover=rtk read -l none --recovery"),
        "an explicit read window is a caller selection, not an RTK omission: {selected_read}"
    );

    let inventory = temp.path().join("inventory");
    std::fs::create_dir_all(&inventory).unwrap();
    for index in 0..500 {
        std::fs::write(
            inventory.join(format!("file-{index:03}.rs")),
            "fn kept() {}\n",
        )
        .unwrap();
    }
    let inventory = inventory.to_str().unwrap();
    let find = run_rtk(&database, &tee_dir, &["find", inventory, "-name", "*.rs"]);
    assert!(find.status.success(), "stderr={:?}", find.stderr);
    let find_shown = String::from_utf8(find.stdout).unwrap();
    assert!(
        find_shown.starts_with("status=inventory"),
        "shown={find_shown}"
    );
    let find_metrics = latest_metrics(&database);
    assert_ai_saves("find", &find_metrics);
    assert!(find_metrics.recovery_created, "find: {find_metrics:?}");
    let recovered = run_rtk(
        &database,
        &tee_dir,
        &["read", "-l", "none", "--recovery", recovery_id(&find_shown)],
    );
    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    let expected_find = (0..500)
        .map(|index| format!("file-{index:03}.rs"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        recovered.stdout.len(),
        expected_find.len(),
        "find recovery length must equal its native listing"
    );
    assert_eq!(recovered.stdout, expected_find.as_bytes());

    let matches = temp.path().join("matches.txt");
    let filler = "unnecessary search context ".repeat(28);
    let match_text = (1..=500)
        .map(|line| format!("{filler}NEEDLE at result {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&matches, &match_text).unwrap();
    let matches = matches.to_str().unwrap();
    let native = Command::new("rg")
        .args(["NEEDLE", matches])
        .output()
        .unwrap();
    if !native.status.success() {
        return;
    }
    let rg = run_rtk(&database, &tee_dir, &["rg", "NEEDLE", matches]);
    assert_eq!(rg.status.code(), native.status.code());
    assert!(rg.stderr.is_empty(), "stderr={:?}", rg.stderr);
    let rg_shown = String::from_utf8(rg.stdout).unwrap();
    assert!(rg_shown.starts_with("status=search"), "shown={rg_shown}");
    let rg_metrics = latest_metrics(&database);
    assert_ai_saves("rg", &rg_metrics);
    assert!(rg_metrics.recovery_created, "rg: {rg_metrics:?}");
    let recovered = run_rtk(
        &database,
        &tee_dir,
        &["read", "-l", "none", "--recovery", recovery_id(&rg_shown)],
    );
    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, native.stdout);
}
