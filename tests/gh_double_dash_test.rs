//! Native argument boundaries for the fork's universal `rtk gh` route.
//! Clap consumes RTK's leading option terminator. Once the gh arguments have
//! started, every later `--` belongs to gh and must remain byte-for-byte intact.
//! Explicit boundaries select passthrough without injecting output flags.
//! Stubs `gh` on PATH with a script that records its argv.

mod common;

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Runs `rtk <args>` against a `gh` stub and returns the argv the stub received.
fn gh_argv(args: &[&str]) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let argv_file = dir.path().join("argv.txt");
    let stub_path = dir.path().join("gh");

    #[cfg(unix)]
    {
        std::fs::write(
            &stub_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 0\n",
                shell_quote(&argv_file)
            ),
        )
        .expect("write stub");
        let mut perms = std::fs::metadata(&stub_path)
            .expect("stat stub")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&stub_path, perms).expect("chmod stub");
    }
    #[cfg(windows)]
    std::fs::write(
        stub_path.with_extension("cmd"),
        format!(
            "@echo off\r\n(for %%A in (%*) do @echo %%~A) > \"{}\"\r\nexit /b 0\r\n",
            argv_file.display()
        ),
    )
    .expect("write stub");
    let path_with_stub = std::env::join_paths(std::iter::once(dir.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH with gh stub");

    let out = common::rtk_command()
        .env("PATH", path_with_stub)
        .env("LC_ALL", "C")
        .current_dir(dir.path())
        .args(args)
        .output()
        .expect("spawn rtk");

    std::fs::read_to_string(&argv_file)
        .unwrap_or_else(|e| {
            panic!(
                "read captured argv: {e}; rtk stdout={} stderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        })
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn gh_double_dash_before_subcommand_is_dropped() {
    // gh 2.46 and 2.100 both answer `gh -- pr view 42` with `unknown command "pr" for "gh"`,
    // and the subcommand must reach gh once, not twice.
    let argv = gh_argv(&["gh", "--", "pr", "view", "42"]);
    assert_eq!(argv[..3], ["pr", "view", "42"]);
    assert!(
        !argv.iter().any(|a| a == "--"),
        "rtk's own terminator must not reach gh: {argv:?}"
    );
}

#[test]
fn gh_double_dash_after_subcommand_is_preserved() {
    let argv = gh_argv(&["gh", "api", "--", "repos/o/r"]);
    assert_eq!(argv, vec!["api", "--", "repos/o/r"]);
}

#[test]
fn gh_double_dash_after_subcommand_is_preserved_before_a_flag() {
    // Once gh arguments have started, RTK preserves the native boundary, even
    // when gh itself rejects the spelling. A wrapper must not repair user argv.
    let argv = gh_argv(&["gh", "api", "--", "repos/o/r", "--jq", ".name"]);
    assert_eq!(argv, vec!["api", "--", "repos/o/r", "--jq", ".name"]);
}

#[test]
fn gh_double_dash_inside_trailing_region_is_preserved() {
    let argv = gh_argv(&["gh", "api", "repos/o/r", "--", "--jq", ".name"]);
    assert_eq!(argv, vec!["api", "repos/o/r", "--", "--jq", ".name"]);
}

#[test]
fn gh_escaped_flag_is_not_hoisted_into_flag_position() {
    // `--watch` sits behind the boundary the user typed; reading it as the PR number and
    // re-emitting it ahead of the `--` would make rtk run a blocking watch the user escaped.
    let argv = gh_argv(&["gh", "pr", "checks", "--", "--watch", "42"]);
    let watch = argv
        .iter()
        .position(|a| a == "--watch")
        .expect("--watch forwarded");
    let boundary = argv.iter().position(|a| a == "--").expect("-- forwarded");
    assert!(
        boundary < watch,
        "the escaped flag must stay behind the boundary: {argv:?}"
    );
}

#[test]
fn gh_lone_escaped_flag_is_not_hoisted_into_flag_position() {
    // Nothing else trails the boundary, so counting escaped tokens cannot tell this apart from
    // an escaped PR number, and hoisting it runs the blocking watch the user escaped.
    let argv = gh_argv(&["gh", "pr", "checks", "--", "--watch"]);
    assert_eq!(argv, vec!["pr", "checks", "--", "--watch"]);
}

#[test]
fn gh_lone_escaped_flag_is_not_hoisted_on_the_run_grammar() {
    let argv = gh_argv(&["gh", "run", "view", "--", "--log"]);
    assert_eq!(argv, vec!["run", "view", "--", "--log"]);
}

#[test]
fn gh_lone_escaped_identifier_keeps_native_boundary() {
    // Explicit native boundaries choose passthrough; no JSON flags are injected.
    let argv = gh_argv(&["gh", "pr", "view", "--", "42"]);
    assert_eq!(argv, vec!["pr", "view", "--", "42"]);
}

#[test]
fn gh_double_dash_inside_a_filtered_subcommand_is_preserved() {
    let argv = gh_argv(&["gh", "pr", "list", "--", "--state", "open"]);
    assert_eq!(
        argv[argv.len() - 3..],
        ["--", "--state", "open"],
        "an interior -- is the user's own and reaches gh: {argv:?}"
    );
}
