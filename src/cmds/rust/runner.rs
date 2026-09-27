//! Argv and explicit-shell wrappers over the shared err/test command runners in core.

use crate::core::runner::{
    TestEcosystem, run_err_cmd, run_err_unrunnable, run_test_cmd, run_test_unrunnable,
};
use crate::core::shell::{Launch, command_from_args, display_args, program_name, spawn_failure};
use anyhow::{Context, Result};

/// Run a command and filter output to show only errors/warnings.
///
/// Arguments execute directly, preserving every boundary Clap parsed. With
/// `shell`, the single supplied script runs through that shell instead.
pub fn run_err(command: &[String], shell: Option<&str>, verbose: u8) -> Result<i32> {
    let display = display_args(command);
    let program = program_name(command, shell);
    match command_from_args(command, shell).context("Failed to prepare err command")? {
        Launch::Ready(cmd) => match run_err_cmd(
            isolate_nested_rtk(cmd, command, shell),
            "err",
            &display,
            "err",
            verbose,
        ) {
            Ok(code) => Ok(code),
            // Resolution proves the name resolves, not that `execve` accepts the
            // file: a CRLF shebang or a bad binary format fails here instead.
            Err(error) => match spawn_failure(program, &error) {
                Some(outcome) => Ok(run_err_unrunnable("err", &display, &outcome, verbose)),
                None => Err(error),
            },
        },
        Launch::Unrunnable(outcome) => Ok(run_err_unrunnable("err", &display, &outcome, verbose)),
    }
}

/// Run tests and show only failures.
///
/// Arguments execute directly, preserving every boundary Clap parsed. With
/// `shell`, the single supplied script runs through that shell instead.
pub fn run_test(command: &[String], shell: Option<&str>, verbose: u8) -> Result<i32> {
    let display = display_args(command);
    let program = program_name(command, shell);
    let eco = TestEcosystem::detect(&display);
    match command_from_args(command, shell).context("Failed to prepare test command")? {
        Launch::Ready(cmd) => match run_test_cmd(
            isolate_nested_rtk(cmd, command, shell),
            "test",
            &display,
            "test",
            eco,
            verbose,
        ) {
            Ok(code) => Ok(code),
            Err(error) => match spawn_failure(program, &error) {
                Some(outcome) => Ok(run_test_unrunnable(
                    "test", &display, &outcome, eco, verbose,
                )),
                None => Err(error),
            },
        },
        Launch::Unrunnable(outcome) => Ok(run_test_unrunnable(
            "test", &display, &outcome, eco, verbose,
        )),
    }
}

fn contains_rtk_invocation(command: &str) -> bool {
    command
        .split(['&', '|', ';'])
        .any(|segment| contains_rtk_command(shell_words(segment)))
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;

    for character in segment.chars() {
        match (quote, character) {
            (Some(active), character) if character == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, character) if character.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            (_, character) => current.push(character),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn contains_rtk_command(words: Vec<String>) -> bool {
    let mut index = 0;
    if words.first().is_some_and(|word| word == "env") {
        index = 1;
        while let Some(word) = words.get(index) {
            if word == "--" {
                index += 1;
                break;
            }
            if word == "-u" || word == "--unset" {
                index = index.saturating_add(2);
                continue;
            }
            if word.starts_with('-') {
                index += 1;
                continue;
            }
            break;
        }
    }

    while let Some(word) = words.get(index) {
        if is_shell_assignment(word) {
            index += 1;
        } else {
            break;
        }
    }

    let Some(program) = words.get(index) else {
        return false;
    };
    let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
    matches!(program, "rtk" | "rtk.exe")
}

fn is_shell_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    matches!(characters.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn isolate_nested_rtk(
    mut child: std::process::Command,
    command: &[String],
    shell: Option<&str>,
) -> std::process::Command {
    let nested = if shell.is_some() {
        command
            .first()
            .is_some_and(|script| contains_rtk_invocation(script))
    } else {
        contains_rtk_command(command.to_vec())
    };
    if nested {
        child.env_remove("RTK_EXECUTION_ID");
    }
    if crate::service::debug_enabled() {
        eprintln!(
            "[rtk-debug] runner.child explicit_shell={} nested_rtk={} execution_id={}",
            shell.is_some(),
            nested,
            if nested { "removed" } else { "inherited" }
        );
    }
    child
}

#[cfg(test)]
mod tests {
    use super::{contains_rtk_invocation, isolate_nested_rtk};

    #[test]
    fn nested_rtk_tracking_respects_direct_argument_boundaries() {
        for (args, shell, expected_removed) in [
            (vec!["rtk", "git", "status"], None, true),
            (vec!["echo", "rtk git status"], None, false),
            (vec!["rtk git status"], None, false),
            (vec!["echo before && rtk git status"], Some("sh"), true),
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            let child = isolate_nested_rtk(std::process::Command::new("unused"), &args, shell);
            let removed = child
                .get_envs()
                .any(|(key, value)| key == "RTK_EXECUTION_ID" && value.is_none());
            assert_eq!(removed, expected_removed, "args={args:?} shell={shell:?}");
        }
    }

    #[test]
    fn detects_nested_rtk_invocations_without_matching_similar_names() {
        for command in [
            "rtk read file.txt",
            "rtk.exe read file.txt",
            r#"C:\\Tools\\rtk.exe read file.txt"#,
            "echo before && rtk rg pattern file.txt",
            "echo before & \"C:/Tools/rtk.exe\" read file.txt",
            "FOO=bar rtk git status",
            "env RTK_TEE=0 rtk read file.txt",
            "env -i -- RTK_TEE=0 rtk read file.txt",
        ] {
            assert!(contains_rtk_invocation(command), "{command}");
        }

        for command in [
            "echo rtk read file.txt",
            "my-rtk-tool read file.txt",
            "rtk-helper read file.txt",
            "cargo test --package rtk",
        ] {
            assert!(!contains_rtk_invocation(command), "{command}");
        }
    }
}
