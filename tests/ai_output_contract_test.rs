use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn visit_rs(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read command source") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            visit_rs(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn has_production_stdout(text: &str) -> bool {
    let production = text
        .split_once("\n#[cfg(test)]\n")
        .map_or(text, |(before_tests, _)| before_tests);
    production.lines().any(|line| {
        let without_stderr = line.replace("eprintln!(", "").replace("eprint!(", "");
        without_stderr.contains("println!(") || without_stderr.contains("print!(")
    })
}

#[test]
fn legacy_stdout_paths_match_the_checked_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    visit_rs(&root.join("src/cmds"), &mut files);

    let actual: BTreeSet<String> = files
        .into_iter()
        .filter(|path| has_production_stdout(&std::fs::read_to_string(path).unwrap()))
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let expected: BTreeSet<String> = include_str!("fixtures/ai_output_legacy_stdout_paths.txt")
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    let added: Vec<_> = actual.difference(&expected).cloned().collect();
    let removed: Vec<_> = expected.difference(&actual).cloned().collect();

    assert!(
        added.is_empty() && removed.is_empty(),
        "legacy stdout inventory changed; added={added:?} removed={removed:?}"
    );
}
