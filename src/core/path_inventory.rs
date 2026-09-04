use crate::core::ai_output::{AiDocument, AiRecord, Severity};
use std::collections::BTreeMap;

fn normalized(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

fn components_with_ends(path: &str) -> Vec<(&str, usize)> {
    let mut components = Vec::new();
    let mut start = 0;
    for (index, character) in path.char_indices() {
        if matches!(character, '/' | '\\') {
            if start < index {
                components.push((&path[start..index], index));
            }
            start = index + character.len_utf8();
        }
    }
    if start < path.len() {
        components.push((&path[start..], path.len()));
    }
    components
}

fn split_parent(path: &str) -> Option<(&str, &str)> {
    let separator = path.rfind(['/', '\\'])?;
    Some((&path[..separator], &path[separator + 1..]))
}

fn relative_to<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let remainder = path.strip_prefix(root)?;
    remainder.strip_prefix(['/', '\\'])
}

fn escape_component(component: &str) -> String {
    let mut escaped = String::with_capacity(component.len());
    for character in component.chars() {
        if matches!(character, '\\' | ',' | '{' | '}') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn common_root(paths: &[String]) -> Option<String> {
    let first = normalized(paths.first()?);
    let mut common = components_with_ends(&first);
    common.pop();
    if common.is_empty() {
        return None;
    }

    for path in paths.iter().skip(1) {
        let normalized = normalized(path);
        let mut components = components_with_ends(&normalized);
        components.pop();
        let shared = common
            .iter()
            .zip(&components)
            .take_while(|((left, _), (right, _))| left == right)
            .count();
        common.truncate(shared);
        if common.is_empty() {
            return None;
        }
    }

    Some(first[..common.last()?.1].to_string())
}

pub fn canonical_groups(paths: &[String]) -> Vec<(String, Vec<String>)> {
    let root = common_root(paths);
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in paths {
        let normalized = normalized(path);
        let relative = root
            .as_deref()
            .and_then(|root| relative_to(&normalized, root))
            .unwrap_or(&normalized);
        let (directory, leaf) = split_parent(relative).unwrap_or((".", relative));
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
        let record = format!(
            "{directory}/{{{}}}",
            leaves
                .iter()
                .map(|leaf| escape_component(leaf))
                .collect::<Vec<_>>()
                .join(",")
        );
        document.push(
            AiRecord::new(Severity::Info, record)
                .grouped(directory)
                .representing(item_count),
        );
    }

    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_inventory_escapes_group_delimiters() {
        let rendered = crate::core::ai_output::render(
            &document(&["src/a,comma.rs".into(), "src/brace{file}.rs".into()]),
            crate::core::ai_output::BudgetClass::Collection,
        )
        .text;

        assert!(rendered.contains(r"./{a\,comma.rs,brace\{file\}.rs}"));
    }
}
