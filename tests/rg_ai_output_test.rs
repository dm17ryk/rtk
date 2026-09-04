use std::process::Command;

use rusqlite::Connection;
use tempfile::tempdir;

fn rg_available() -> bool {
    Command::new("rg")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn rtk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rtk"))
}

#[test]
fn recognized_rg_text_search_uses_compact_ai_records() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let path = temp.path().join("matches.txt");
    let filler = "noise ".repeat(30);
    let content = (1..=80)
        .map(|line| format!("{filler}NEEDLE at line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&path, &content).unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let path = path.to_str().unwrap();

    let native = Command::new("rg").args(["NEEDLE", path]).output().unwrap();
    let output = rtk()
        .args(["rg", "NEEDLE", path])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    assert!(output.stderr.is_empty(), "stderr={:?}", output.stderr);
    let shown = String::from_utf8(output.stdout).unwrap();
    assert!(shown.starts_with("status=search"), "shown={shown}");
    assert!(shown.contains("1: ..."), "shown={shown}");
    assert!(shown.len() < native.stdout.len(), "shown={shown}");
    assert!(!shown.contains('\0'), "shown={shown:?}");

    let recovery = shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .map(|(_, identifier)| identifier.trim())
        .expect("a shortened rg record must advertise lossless recovery");
    let recovered = rtk()
        .args(["read", "-l", "none", "--recovery", recovery])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, native.stdout);
}

#[test]
fn no_filename_multi_file_recovery_matches_native_stdout() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    let filler = "noise ".repeat(30);
    let content = (1..=30)
        .map(|line| format!("{filler}NEEDLE at line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&first, &content).unwrap();
    std::fs::write(&second, &content).unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();

    let native = Command::new("rg")
        .args(["--no-filename", "--threads", "1", "NEEDLE", first, second])
        .output()
        .unwrap();
    let output = rtk()
        .args([
            "rg",
            "--no-filename",
            "--threads",
            "1",
            "NEEDLE",
            first,
            second,
        ])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    let shown = String::from_utf8(output.stdout).unwrap();
    let recovery = shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .map(|(_, identifier)| identifier.trim())
        .expect("a shortened no-filename search must advertise lossless recovery");
    let recovered = rtk()
        .args(["read", "-l", "none", "--recovery", recovery])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, native.stdout);
}

#[test]
fn no_filename_directory_recovery_matches_native_stdout() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let directory = temp.path().join("matches");
    std::fs::create_dir(&directory).unwrap();
    let filler = "noise ".repeat(30);
    let content = (1..=30)
        .map(|line| format!("{filler}NEEDLE at line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(directory.join("first.txt"), &content).unwrap();
    std::fs::write(directory.join("second.txt"), &content).unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let directory = directory.to_str().unwrap();

    let native = Command::new("rg")
        .args(["--no-filename", "--threads", "1", "NEEDLE", directory])
        .output()
        .unwrap();
    let output = rtk()
        .args(["rg", "--no-filename", "--threads", "1", "NEEDLE", directory])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    let shown = String::from_utf8(output.stdout).unwrap();
    let recovery = shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .map(|(_, identifier)| identifier.trim())
        .expect("a shortened no-filename search must advertise lossless recovery");
    let recovered = rtk()
        .args(["read", "-l", "none", "--recovery", recovery])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, native.stdout);
}

#[test]
fn ambiguous_dash_prefixed_replace_value_uses_native_passthrough() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    let filler = "noise ".repeat(30);
    let content = (1..=30)
        .map(|line| format!("{filler}NEEDLE at line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&first, &content).unwrap();
    std::fs::write(&second, &content).unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();

    let native = Command::new("rg")
        .args(["--replace", "-h", "--threads", "1", "NEEDLE", first, second])
        .output()
        .unwrap();
    let output = rtk()
        .args([
            "rg",
            "--replace",
            "-h",
            "--threads",
            "1",
            "NEEDLE",
            first,
            second,
        ])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    assert_eq!(output.stdout, native.stdout);
    assert_eq!(output.stderr, native.stderr);
}

#[test]
fn inline_dash_prefixed_replace_value_has_lossless_recovery() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    let filler = "noise ".repeat(30);
    let content = (1..=30)
        .map(|line| format!("{filler}NEEDLE at line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&first, &content).unwrap();
    std::fs::write(&second, &content).unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();

    let native = Command::new("rg")
        .args(["--replace=-h", "--threads", "1", "NEEDLE", first, second])
        .output()
        .unwrap();
    let output = rtk()
        .args([
            "rg",
            "--replace=-h",
            "--threads",
            "1",
            "NEEDLE",
            first,
            second,
        ])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    let shown = String::from_utf8(output.stdout).unwrap();
    let recovery = shown
        .rsplit_once(" recover=rtk read -l none --recovery ")
        .map(|(_, identifier)| identifier.trim())
        .expect("a shortened inline replacement search must advertise lossless recovery");
    let recovered = rtk()
        .args(["read", "-l", "none", "--recovery", recovery])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert!(recovered.status.success(), "stderr={:?}", recovered.stderr);
    assert_eq!(recovered.stdout, native.stdout);
}

#[test]
fn small_rg_search_falls_back_to_native_stdout_not_internal_parse_aids() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let path = temp.path().join("one-match.txt");
    std::fs::write(&path, "NEEDLE\n").unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let path = path.to_str().unwrap();

    let native = Command::new("rg").args(["NEEDLE", path]).output().unwrap();
    let output = rtk()
        .args(["rg", "NEEDLE", path])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    assert_eq!(output.stdout, native.stdout);
    assert!(output.stderr.is_empty(), "stderr={:?}", output.stderr);
}

#[test]
fn json_and_inventory_routes_emit_ai_documents() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let source = temp.path().join("events.txt");
    let filler = "event-noise ".repeat(24);
    let content = (1..=80)
        .map(|line| format!("{filler}NEEDLE json line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(&source, content).unwrap();
    let inventory = temp.path().join("inventory");
    std::fs::create_dir_all(&inventory).unwrap();
    for index in 0..80 {
        std::fs::write(inventory.join(format!("item-{index:03}.txt")), "content\n").unwrap();
    }
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let source = source.to_str().unwrap();
    let inventory = inventory.to_str().unwrap();

    let json = rtk()
        .args(["rg", "--json", "NEEDLE", source])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert!(json.status.success(), "stderr={:?}", json.stderr);
    let json_shown = String::from_utf8(json.stdout).unwrap();
    assert!(
        json_shown.starts_with("status=search"),
        "shown={json_shown}"
    );
    assert!(!json_shown.contains("\"type\":"), "shown={json_shown}");

    let files = rtk()
        .args(["rg", "--files", inventory])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert!(files.status.success(), "stderr={:?}", files.stderr);
    let files_shown = String::from_utf8(files.stdout).unwrap();
    assert!(!files_shown.contains('\0'), "shown={files_shown}");
    let connection = Connection::open(&database).unwrap();
    let contract = connection
        .query_row(
            "SELECT output_contract FROM commands ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(contract, "ai_owned");
}

#[test]
fn count_and_nul_routes_preserve_their_native_boundaries() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let path = temp.path().join("count.txt");
    let path_two = temp.path().join("count-two.txt");
    std::fs::write(&path, "NEEDLE\nNEEDLE\n").unwrap();
    std::fs::write(&path_two, "NEEDLE\n").unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let path = path.to_str().unwrap();
    let path_two = path_two.to_str().unwrap();

    let native_count = Command::new("rg")
        .args(["-c", "NEEDLE", path])
        .output()
        .unwrap();
    let count = rtk()
        .args(["rg", "-c", "NEEDLE", path])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert_eq!(count.status.code(), native_count.status.code());
    assert!(!count.stdout.contains(&0), "stdout={:?}", count.stdout);

    let native_nul = Command::new("rg")
        .args(["--null", "NEEDLE", path])
        .output()
        .unwrap();
    let nul = rtk()
        .args(["rg", "--null", "NEEDLE", path])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert_eq!(nul.status.code(), native_nul.status.code());
    assert_eq!(nul.stdout, native_nul.stdout);

    let native_no_filename = Command::new("rg")
        .args([
            "--threads",
            "1",
            "--count",
            "--no-filename",
            "NEEDLE",
            path,
            path_two,
        ])
        .output()
        .unwrap();
    let no_filename = rtk()
        .args([
            "rg",
            "--threads",
            "1",
            "--count",
            "--no-filename",
            "NEEDLE",
            path,
            path_two,
        ])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();
    assert_eq!(no_filename.status.code(), native_no_filename.status.code());
    assert_eq!(no_filename.stdout, native_no_filename.stdout);
    assert_eq!(no_filename.stderr, native_no_filename.stderr);
}

#[test]
fn recognized_rg_selector_and_replacement_flags_keep_their_semantics() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let directory = temp.path().join("sources");
    std::fs::create_dir_all(&directory).unwrap();
    let filler = "selector-noise ".repeat(24);
    let content = (1..=80)
        .map(|line| format!("{filler}NEEDLE selected line {line} {filler}\n"))
        .collect::<String>();
    std::fs::write(directory.join("kept.txt"), content).unwrap();
    std::fs::write(directory.join("skipped.rs"), "NEEDLE ignored\n").unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let directory = directory.to_str().unwrap();

    let native = Command::new("rg")
        .args([
            "--glob",
            "*.txt",
            "--replace",
            "HIT",
            "--regexp=NEEDLE",
            directory,
        ])
        .output()
        .unwrap();
    let output = rtk()
        .args([
            "rg",
            "--glob",
            "*.txt",
            "--replace",
            "HIT",
            "--regexp=NEEDLE",
            directory,
        ])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    assert!(output.stderr.is_empty(), "stderr={:?}", output.stderr);
    let shown = String::from_utf8(output.stdout).unwrap();
    assert!(shown.contains("HIT"), "shown={shown}");
    assert!(!shown.contains("skipped.rs"), "shown={shown}");
    assert!(!shown.contains('\0'), "shown={shown:?}");
}

#[test]
fn unknown_rg_flags_replay_native_stdout_stderr_and_exit_code() {
    if !rg_available() {
        return;
    }

    let temp = tempdir().unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let native = Command::new("rg")
        .args(["--future-flag", "needle"])
        .output()
        .unwrap();
    let output = rtk()
        .args(["rg", "--future-flag", "needle"])
        .env("RTK_DB_PATH", &database)
        .env("RTK_TEE_DIR", &tee_dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), native.status.code());
    assert_eq!(output.stdout, native.stdout);
    assert_eq!(output.stderr, native.stderr);
}
