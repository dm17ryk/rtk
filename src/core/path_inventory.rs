use crate::core::ai_output::{AiDocument, AiRecord, Severity};
use std::collections::BTreeMap;

fn normalized(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .to_string()
}

pub(crate) fn common_root(paths: &[String]) -> Option<String> {
    let first = paths.first()?;
    let root = normalized(first).split('/').next()?.to_string();
    (!root.is_empty()
        && paths.iter().all(|path| {
            let normalized = normalized(path);
            normalized
                .strip_prefix(&root)
                .is_some_and(|rest| rest.starts_with('/'))
        }))
    .then_some(root)
}

pub fn canonical_groups(paths: &[String]) -> Vec<(String, Vec<String>)> {
    let root = common_root(paths);
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in paths {
        let normalized = normalized(path);
        let relative = root
            .as_deref()
            .and_then(|root| normalized.strip_prefix(root)?.strip_prefix('/'))
            .unwrap_or(&normalized);
        let (directory, leaf) = relative
            .rsplit_once('/')
            .map_or((".", relative), |(directory, leaf)| (directory, leaf));
        groups
            .entry(directory.to_string())
            .or_default()
            .push(leaf.to_string());
    }

    groups
        .into_iter()
        .map(|(directory, mut leaves)| {
            leaves.sort();
            (directory, leaves)
        })
        .collect()
}

pub fn document(paths: &[String]) -> AiDocument {
    let groups = canonical_groups(paths);
    let mut document = AiDocument::new(Some("inventory"));
    document.fact("files", paths.len().to_string());
    document.fact("dirs", groups.len().to_string());
    if let Some(root) = common_root(paths) {
        document.fact("root", root);
    }

    for (directory, leaves) in groups {
        let item_count = leaves.len();
        let record = format!("{directory}/{{{}}}", leaves.join(","));
        document.push(
            AiRecord::new(Severity::Info, record)
                .grouped(directory)
                .representing(item_count),
        );
    }

    document
}
