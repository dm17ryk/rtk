use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

const RECOVERY_ID: &str = "1234567890_shell-neutral.lossless.log";
const RECOVERY_BYTES: &[u8] = b"first recovery record\nsecond recovery record\n";

fn install_test_rtk(bin_dir: &Path) {
    std::fs::create_dir_all(bin_dir).unwrap();
    let name = if cfg!(windows) { "rtk.exe" } else { "rtk" };
    let destination = bin_dir.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_rtk"), &destination).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&destination).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(destination, permissions).unwrap();
    }
}

fn path_with_test_rtk(bin_dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![bin_dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

fn configure(command: &mut Command, tee_dir: &Path, bin_dir: &Path, database: &Path) {
    command
        .env("PATH", path_with_test_rtk(bin_dir))
        .env("RTK_TEE_DIR", tee_dir)
        .env("RTK_DB_PATH", database);
}

fn assert_recovered(shell: &str, output: Output) {
    assert!(
        output.status.success(),
        "{shell} status={:?} stderr={:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let normalized = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(normalized.as_bytes(), RECOVERY_BYTES, "shell={shell}");
}

#[test]
fn advertised_recovery_command_executes_without_exposing_the_storage_path() {
    let temp = tempdir().unwrap();
    let tee_dir = temp
        .path()
        .join("default home & 100% ! $ ` [brackets] with spaces")
        .join("rtk")
        .join("tee");
    std::fs::create_dir_all(&tee_dir).unwrap();
    std::fs::write(tee_dir.join(RECOVERY_ID), RECOVERY_BYTES).unwrap();
    let bin_dir = temp.path().join("rtk bin with spaces");
    install_test_rtk(&bin_dir);
    let database = temp.path().join("history.db");
    let recovery = format!("rtk read -l none --recovery {RECOVERY_ID}");

    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/D", "/S", "/C", &recovery]);
        configure(&mut cmd, &tee_dir, &bin_dir, &database);
        assert_recovered("cmd.exe", cmd.output().unwrap());

        let mut powershell = Command::new("powershell.exe");
        powershell.args(["-NoProfile", "-NonInteractive", "-Command", &recovery]);
        configure(&mut powershell, &tee_dir, &bin_dir, &database);
        assert_recovered("powershell.exe", powershell.output().unwrap());

        let mut pwsh = Command::new("pwsh.exe");
        pwsh.args(["-NoProfile", "-NonInteractive", "-Command", &recovery]);
        configure(&mut pwsh, &tee_dir, &bin_dir, &database);
        match pwsh.output() {
            Ok(output) => assert_recovered("pwsh.exe", output),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("pwsh.exe failed to start: {error}"),
        }
    }

    #[cfg(unix)]
    {
        let mut sh = Command::new("sh");
        sh.args(["-c", &recovery]);
        configure(&mut sh, &tee_dir, &bin_dir, &database);
        assert_recovered("sh", sh.output().unwrap());
    }
}

#[cfg(windows)]
#[test]
fn recovery_command_executes_from_the_default_windows_data_directory() {
    struct RemoveOnDrop(std::path::PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let temp = tempdir().unwrap();
    let tee_dir = dirs::data_local_dir().unwrap().join("rtk").join("tee");
    std::fs::create_dir_all(&tee_dir).unwrap();
    let identifier = format!("{}_default-home.lossless.log", std::process::id());
    let artifact = tee_dir.join(&identifier);
    std::fs::write(&artifact, RECOVERY_BYTES).unwrap();
    let _cleanup = RemoveOnDrop(artifact);
    let bin_dir = temp.path().join("rtk bin");
    install_test_rtk(&bin_dir);
    let path = path_with_test_rtk(&bin_dir);
    let database = temp.path().join("history.db");
    let appdata = temp.path().join("empty appdata");
    std::fs::create_dir_all(&appdata).unwrap();
    let recovery = format!("rtk read -l none --recovery {identifier}");

    for shell in ["cmd.exe", "powershell.exe"] {
        let mut command = Command::new(shell);
        if shell == "cmd.exe" {
            command.args(["/D", "/S", "/C", &recovery]);
        } else {
            command.args(["-NoProfile", "-NonInteractive", "-Command", &recovery]);
        }
        command
            .env("PATH", &path)
            .env("APPDATA", &appdata)
            .env("RTK_DB_PATH", &database)
            .env_remove("RTK_TEE_DIR");
        assert_recovered(shell, command.output().unwrap());
    }
}
