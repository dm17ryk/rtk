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

fn blank(bytes: &mut [u8], start: usize, end: usize) {
    for byte in &mut bytes[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// Build a byte-aligned Rust code view. Comments and literals are blanked so
/// braces and sink-like words inside them cannot affect the audit.
fn code_mask(source: &str) -> Vec<u8> {
    let input = source.as_bytes();
    let mut mask = input.to_vec();
    let mut index = 0;
    while index < input.len() {
        if input[index..].starts_with(b"//") {
            let end = input[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(input.len(), |offset| index + offset);
            blank(&mut mask, index, end);
            index = end;
        } else if input[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            let mut depth = 1_usize;
            while index < input.len() && depth > 0 {
                if input[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if input[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            blank(&mut mask, start, index);
        } else if input[index] == b'r' {
            let mut quote = index + 1;
            while quote < input.len() && input[quote] == b'#' {
                quote += 1;
            }
            if quote < input.len() && input[quote] == b'"' {
                let hashes = quote - index - 1;
                let start = index;
                index = quote + 1;
                while index < input.len() {
                    if input[index] == b'"'
                        && input
                            .get(index + 1..index + 1 + hashes)
                            .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
                    {
                        index += 1 + hashes;
                        break;
                    }
                    index += 1;
                }
                blank(&mut mask, start, index);
            } else {
                index += 1;
            }
        } else if input[index] == b'"' {
            let start = index;
            index += 1;
            while index < input.len() {
                if input[index] == b'\\' {
                    index = (index + 2).min(input.len());
                } else if input[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            blank(&mut mask, start, index);
        } else if input[index] == b'\'' {
            // A character literal always closes quickly; a lifetime such as
            // `'static` has no nearby quote and must remain code.
            let search_end = (index + 12).min(input.len());
            let closing = input[index + 1..search_end]
                .iter()
                .position(|byte| *byte == b'\'')
                .map(|offset| index + 1 + offset);
            if let Some(end) = closing {
                if !input[index + 1..end].contains(&b'\n') {
                    blank(&mut mask, index, end + 1);
                    index = end + 1;
                    continue;
                }
            }
            index += 1;
        } else {
            index += 1;
        }
    }
    mask
}

fn skip_space(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn token_at(bytes: &[u8], index: usize, token: &[u8]) -> bool {
    bytes.get(index..index + token.len()) == Some(token)
        && index
            .checked_sub(1)
            .and_then(|before| bytes.get(before))
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        && bytes
            .get(index + token.len())
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
}

fn cfg_arguments(predicate: &[u8]) -> Option<Vec<&[u8]>> {
    if predicate.is_empty() {
        return None;
    }
    let mut arguments = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    for (index, byte) in predicate.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                if start == index {
                    return None;
                }
                arguments.push(&predicate[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || start == predicate.len() {
        return None;
    }
    arguments.push(&predicate[start..]);
    Some(arguments)
}

/// Return true only when the predicate itself proves the item cannot exist in
/// a non-test build. Unknown or mixed cfg syntax stays visible to the audit.
fn cfg_implies_test(predicate: &[u8]) -> bool {
    if predicate == b"test" {
        return true;
    }
    for (operator, require_all) in [(b"all".as_slice(), false), (b"any".as_slice(), true)] {
        let Some(arguments) = predicate
            .strip_prefix(operator)
            .and_then(|rest| rest.strip_prefix(b"("))
            .and_then(|rest| rest.strip_suffix(b")"))
            .and_then(cfg_arguments)
        else {
            continue;
        };
        return if require_all {
            arguments.iter().all(|argument| cfg_implies_test(argument))
        } else {
            arguments.iter().any(|argument| cfg_implies_test(argument))
        };
    }
    false
}

fn is_test_exclusive_cfg(attribute: &[u8]) -> bool {
    let compact: Vec<u8> = attribute
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    compact
        .strip_prefix(b"#[cfg(")
        .and_then(|rest| rest.strip_suffix(b")]"))
        .is_some_and(cfg_implies_test)
}

fn test_module_ranges(mask: &[u8]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor + 2 <= mask.len() {
        let Some(offset) = mask[cursor..].windows(2).position(|window| window == b"#[") else {
            break;
        };
        let attribute_start = cursor + offset;
        let Some(close_offset) = mask[attribute_start + 2..]
            .iter()
            .position(|byte| *byte == b']')
        else {
            break;
        };
        let attribute_end = attribute_start + 2 + close_offset + 1;
        let attribute = &mask[attribute_start..attribute_end];
        let is_test_cfg = is_test_exclusive_cfg(attribute);
        let mut item = skip_space(mask, attribute_end);
        if token_at(mask, item, b"pub") {
            item = skip_space(mask, item + 3);
        }
        if !is_test_cfg || !token_at(mask, item, b"mod") {
            cursor = attribute_end;
            continue;
        }
        let Some(open_offset) = mask[item + 3..].iter().position(|byte| *byte == b'{') else {
            cursor = attribute_end;
            continue;
        };
        let open = item + 3 + open_offset;
        let mut depth = 1_usize;
        let mut end = open + 1;
        while end < mask.len() && depth > 0 {
            match mask[end] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        if depth == 0 {
            ranges.push((attribute_start, end));
            cursor = end;
        } else {
            cursor = attribute_end;
        }
    }
    ranges
}

fn production_views(text: &str) -> (String, Vec<u8>) {
    let normalized = text.replace("\r\n", "\n");
    let mut source = normalized.into_bytes();
    let mut mask = code_mask(std::str::from_utf8(&source).unwrap());
    for (start, end) in test_module_ranges(&mask) {
        blank(&mut source, start, end);
        blank(&mut mask, start, end);
    }
    (String::from_utf8(source).unwrap(), mask)
}

fn count_macro(mask: &[u8], name: &[u8]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index + name.len() <= mask.len() {
        if token_at(mask, index, name) {
            let bang = skip_space(mask, index + name.len());
            if mask.get(bang) == Some(&b'!') {
                count += 1;
            }
        }
        index += 1;
    }
    count
}

fn count_stdout_handles(mask: &[u8]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index + b"stdout".len() <= mask.len() {
        if token_at(mask, index, b"stdout") {
            let open = skip_space(mask, index + b"stdout".len());
            if mask.get(open) == Some(&b'(') {
                let close = skip_space(mask, open + 1);
                if mask.get(close) == Some(&b')') {
                    let after = skip_space(mask, close + 1);
                    let terminal_probe = mask
                        .get(after..)
                        .is_some_and(|tail| tail.starts_with(b".is_terminal"));
                    if !terminal_probe {
                        count += 1;
                    }
                }
            }
        }
        index += 1;
    }
    count
}

fn fingerprint(source: &str) -> u64 {
    let canonical = source.split_whitespace().collect::<Vec<_>>().join(" ");
    canonical.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Audit {
    print_macros: usize,
    stdout_handles: usize,
    fingerprint: u64,
}

fn audit_source(text: &str) -> Option<Audit> {
    let (production, mask) = production_views(text);
    let print_macros = count_macro(&mask, b"print") + count_macro(&mask, b"println");
    let stdout_handles = count_stdout_handles(&mask);
    (print_macros + stdout_handles > 0).then(|| Audit {
        print_macros,
        stdout_handles,
        fingerprint: fingerprint(&production),
    })
}

fn inventory_line(root: &Path, path: &Path, text: &str) -> Option<String> {
    let audit = audit_source(text)?;
    let relative = path
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    Some(format!(
        "{relative}\tprints={}\tstdout_handles={}\tfingerprint={:016x}",
        audit.print_macros, audit.stdout_handles, audit.fingerprint
    ))
}

#[test]
fn new_direct_write_in_an_existing_file_changes_its_inventory_fingerprint() {
    let root = Path::new("repo");
    let path = root.join("src/cmds/already-listed.rs");
    let before = inventory_line(root, &path, "pub fn run() { println!(\"one\"); }").unwrap();
    let after = inventory_line(
        root,
        &path,
        "pub fn run() { println!(\"one\"); println!(\"new direct write\"); }",
    )
    .unwrap();

    assert_ne!(before, after);
}

#[test]
fn item_level_cfg_test_does_not_hide_later_production_stdout() {
    let source = "#[cfg(test)]\nconst ONLY_IN_TESTS: bool = true;\npub fn run() { println!(\"production\"); }\n";

    assert_eq!(audit_source(source).unwrap().print_macros, 1);
}

#[test]
fn crlf_test_module_is_excluded_without_hiding_later_production() {
    let source = "#[cfg(test)]\r\nmod tests {\r\n fn prints() { println!(\"test only\"); }\r\n}\r\npub fn run() { println!(\"production\"); }\r\n";

    assert_eq!(audit_source(source).unwrap().print_macros, 1);
}

#[test]
fn cfg_not_test_module_remains_in_the_production_audit() {
    let source =
        "#[cfg(not(test))]\nmod production { pub fn run() { println!(\"production\"); } }\n";

    assert_eq!(audit_source(source).unwrap().print_macros, 1);
}

#[test]
fn cfg_mixed_test_or_feature_module_remains_in_the_production_audit() {
    let source = "#[cfg(any(test, feature = \"audit\"))]\nmod mixed { pub fn run() { println!(\"production capable\"); } }\n";

    assert_eq!(audit_source(source).unwrap().print_macros, 1);
}

#[test]
fn known_non_macro_stdout_sinks_are_audited() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/cmds/cloud/curl_cmd.rs",
        "src/cmds/powershell/orchestrator.rs",
        "src/cmds/system/find_cmd.rs",
    ] {
        let text = std::fs::read_to_string(root.join(relative)).unwrap();
        let audit = audit_source(&text).unwrap_or_else(|| panic!("missing audit for {relative}"));
        assert!(
            audit.stdout_handles > 0,
            "missing stdout handle in {relative}"
        );
    }
}

#[test]
fn legacy_stdout_sources_match_the_checked_fingerprints() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    visit_rs(&root.join("src/cmds"), &mut files);

    let actual: BTreeSet<String> = files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            inventory_line(root, &path, &text)
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
        "legacy stdout source fingerprints changed; added={added:#?} removed={removed:#?}"
    );
}
