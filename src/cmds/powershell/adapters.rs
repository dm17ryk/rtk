//! Output adapters for PowerShell's terminal-facing text.

/// Compact a known PowerShell display layout while retaining every displayed
/// value. `None` means that the layout was not recognized confidently.
pub fn filter_output(adapter: &str, source: &str) -> Option<String> {
    if source.is_empty() || contains_ansi(source) {
        return None;
    }
    match adapter.to_ascii_lowercase().as_str() {
        "generic" | "table" | "list" | "filesystem" | "discovery" | "process" | "service"
        | "job" | "events" | "cim" | "networking" | "storage" | "active-directory" | "hyper-v"
        | "defender" | "bitlocker" | "scheduled-tasks" | "containers" | "get-childitem"
        | "get-command" | "get-help" | "get-process" | "get-service" | "get-module" | "get-job"
        | "get-eventlog" | "get-winevent" | "get-ciminstance" | "get-wmiobject" => {
            filter_table(source).or_else(|| filter_list(source))
        }
        _ => None,
    }
}

fn filter_table(source: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let separator = lines.iter().position(|line| is_separator(line))?;
    if separator == 0 || separator + 1 >= lines.len() {
        return None;
    }
    let headers = split_columns(lines[separator - 1]);
    if headers.len() < 2 || headers.iter().any(|header| header.is_empty()) {
        return None;
    }

    let mut output = String::new();
    let mut rows = 0;
    for line in &lines[separator + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let values = split_columns(line);
        if values.len() != headers.len() {
            return None;
        }
        if rows > 0 {
            output.push('\n');
        }
        for (index, (header, value)) in headers.iter().zip(values).enumerate() {
            if index > 0 {
                output.push(' ');
            }
            output.push_str(header);
            output.push('=');
            output.push_str(value.trim());
        }
        rows += 1;
    }
    if rows == 0 || output.len() >= source.trim_end_matches(['\r', '\n']).len() {
        return None;
    }
    Some(format!("{output}\n"))
}

fn filter_list(source: &str) -> Option<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in source.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    if blocks.is_empty() {
        return None;
    }

    let mut compacted = String::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let mut fields = Vec::new();
        for line in block {
            let (name, value) = line.split_once(':')?;
            if name.trim().is_empty() || value.trim().is_empty() {
                return None;
            }
            fields.push(format!("{}={}", name.trim(), value.trim()));
        }
        if fields.len() < 2 {
            return None;
        }
        if block_index > 0 {
            compacted.push('\n');
        }
        compacted.push_str(&fields.join(" "));
    }
    if compacted.len() >= source.trim_end_matches(['\r', '\n']).len() {
        return None;
    }
    Some(format!("{compacted}\n"))
}

fn split_columns(line: &str) -> Vec<&str> {
    let mut columns = Vec::new();
    let mut start = None;
    let mut whitespace_start = None;
    let mut whitespace = 0usize;
    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            whitespace_start.get_or_insert(index);
            whitespace += 1;
            continue;
        }
        if whitespace >= 2 {
            if let Some(column_start) = start.take() {
                let column_end = whitespace_start.unwrap_or(index);
                columns.push(line[column_start..column_end].trim());
            }
        }
        if start.is_none() {
            start = Some(index);
        }
        whitespace_start = None;
        whitespace = 0;
    }
    if let Some(column_start) = start {
        columns.push(line[column_start..].trim());
    }
    columns
}

fn is_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 2
        && trimmed
            .chars()
            .all(|character| character == '-' || character.is_whitespace())
}

fn contains_ansi(source: &str) -> bool {
    source.contains('\u{1b}')
}
