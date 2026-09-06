//! Shell-string wrappers over the shared err/test command runners in core.

use crate::core::runner::{run_err_cmd, run_test_cmd};
use anyhow::Result;
use std::process::Command;

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
    matches!(
        characters.next(),
        Some('_' | 'A'..='Z' | 'a'..='z')
    ) && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn build_shell_command(command: &str) -> Command {
    let mut shell = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    // A nested RTK process must not reuse the parent integration's execution
    // id. The parent wrapper records the final shell output; sharing the id
    // would make execution_by_id add the child record a second time.
    if contains_rtk_invocation(command) {
        shell.env_remove("RTK_EXECUTION_ID");
    }

    shell
}

/// Run a command via the shell and filter output to show only errors/warnings.
pub fn run_err(command: &str, verbose: u8) -> Result<i32> {
    run_err_cmd(build_shell_command(command), "err", command, "err", verbose)
}

/// Run tests via the shell and show only failures.
pub fn run_test(command: &str, verbose: u8) -> Result<i32> {
    run_test_cmd(
        build_shell_command(command),
        "test",
        command,
        "test",
        crate::core::runner::TestEcosystem::detect(command),
        verbose,
    )
}

#[cfg(test)]
mod tests {
    use super::contains_rtk_invocation;

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
