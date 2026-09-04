//! Runs TOML filter inline tests to make sure filter rules work correctly.

use std::path::Path;

use anyhow::{Context, Result};

use crate::core::toml_filter;

const LEGACY_STDOUT_INVENTORY: &str = "tests/fixtures/ai_output_legacy_stdout_paths.txt";

/// Run TOML filter inline tests.
///
/// - `filter`: if `Some`, only run tests for that filter name
/// - `require_all`: fail if any filter has no inline tests
pub fn run(filter: Option<String>, require_all: bool) -> Result<()> {
    let results = toml_filter::run_filter_tests(filter.as_deref());

    let total = results.outcomes.len();
    let passed = results.outcomes.iter().filter(|o| o.passed).count();
    let failed = total - passed;

    // Print failures with details
    for outcome in &results.outcomes {
        if !outcome.passed {
            eprintln!(
                "FAIL [{}] {}\n  expected: {:?}\n  actual:   {:?}",
                outcome.filter_name, outcome.test_name, outcome.expected, outcome.actual
            );
        }
    }

    if total == 0 {
        println!("No inline tests found.");
    } else {
        println!("{}/{} tests passed", passed, total);
    }

    if let Some(count) = legacy_stdout_inventory_count(&std::env::current_dir()?)? {
        println!("ai_output_legacy_paths={count}");
    }

    if require_all && !results.filters_without_tests.is_empty() {
        for name in &results.filters_without_tests {
            eprintln!("MISSING tests for filter: {}", name);
        }
        anyhow::bail!(
            "{} filter(s) have no inline tests (use --require-all in CI)",
            results.filters_without_tests.len()
        );
    }

    if failed > 0 {
        anyhow::bail!("{} test(s) failed", failed);
    }

    Ok(())
}

fn legacy_stdout_inventory_count(root: &Path) -> Result<Option<usize>> {
    let path = root.join(LEGACY_STDOUT_INVENTORY);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(count_legacy_stdout_inventory_lines(&text))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn count_legacy_stdout_inventory_lines(text: &str) -> usize {
    text.lines().filter(|line| !line.is_empty()).count()
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_stdout_inventory_count_ignores_blank_lines() {
        assert_eq!(
            super::count_legacy_stdout_inventory_lines("src/cmds/a.rs\n\nsrc/cmds/b.rs\n"),
            2
        );
    }

    #[test]
    fn legacy_stdout_inventory_count_is_absent_outside_source_checkout() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            super::legacy_stdout_inventory_count(dir.path()).unwrap(),
            None
        );
    }
}
