//! Span-preserving parsing for PowerShell command expressions.
//!
//! This is intentionally an orchestration parser rather than a PowerShell
//! evaluator. It understands enough of the language's lexical structure to
//! identify command and pipeline boundaries. Anything it cannot prove safe is
//! returned as an opaque expression and is executed by the native host.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Statement,
    Pipeline,
    Command,
    Opaque,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpaqueReason {
    Empty,
    ParseError,
    String,
    ScriptBlock,
    Redirection,
    ControlFlow,
    DynamicInvocation,
    NativeCommand,
    ExplicitFormatting,
    MachineOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedScript {
    pub nodes: Vec<Node>,
    pub opaque_reason: Option<OpaqueReason>,
    command_names: Vec<String>,
}

impl ParsedScript {
    pub fn is_opaque(&self) -> bool {
        self.opaque_reason.is_some()
    }

    pub fn command_names(&self) -> Vec<String> {
        self.command_names.clone()
    }

    pub fn final_command_span(&self) -> Option<Span> {
        self.nodes
            .iter()
            .rev()
            .find(|node| node.kind == NodeKind::Command)
            .map(|node| node.span)
    }
}

/// Parse PowerShell source without evaluating it or changing any source text.
pub fn parse_expression(source: &str) -> ParsedScript {
    let source_start = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let Some(trimmed) = trimmed_span(source, source_start, source.len()) else {
        return ParsedScript {
            nodes: Vec::new(),
            opaque_reason: Some(OpaqueReason::Empty),
            command_names: Vec::new(),
        };
    };

    let bytes = source.as_bytes();
    let mut nodes = Vec::new();
    let mut command_names = Vec::new();
    let mut opaque_reason = None;
    let mut segment_start = trimmed.start;
    let mut index = trimmed.start;
    let mut quote = None;
    let mut stack = Vec::new();

    while index < trimmed.end {
        let current = bytes[index];
        if let Some(active_quote) = quote {
            if active_quote == b'\'' {
                if current == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                    } else {
                        quote = None;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            } else if current == b'`' {
                if index + 1 >= trimmed.end {
                    opaque_reason.get_or_insert(OpaqueReason::ParseError);
                    index += 1;
                } else {
                    index += 2;
                }
            } else if current == active_quote {
                quote = None;
                index += 1;
            } else {
                index += 1;
            }
            continue;
        }

        match current {
            b'-' if bytes
                .get(index..)
                .is_some_and(|tail| tail.starts_with(b"--%")) =>
            {
                // PowerShell's stop-parsing token hands the remainder to a
                // native executable; RTK must not reinterpret that tail.
                opaque_reason.get_or_insert(OpaqueReason::NativeCommand);
                index = trimmed.end;
            }
            b'\'' | b'"' => {
                quote = Some(current);
                index += 1;
            }
            b'`' => {
                if index + 1 >= trimmed.end {
                    opaque_reason.get_or_insert(OpaqueReason::ParseError);
                    index += 1;
                } else {
                    index += 2;
                }
            }
            b'#' => {
                while index < trimmed.end && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'{' if stack.is_empty() && bytes.get(index.wrapping_sub(1)) != Some(&b'@') => {
                opaque_reason.get_or_insert(OpaqueReason::ScriptBlock);
                stack.push(current);
                index += 1;
            }
            b'{' => {
                // A preceding `@` denotes a hashtable literal.
                stack.push(current);
                index += 1;
            }
            b'(' | b'[' => {
                stack.push(current);
                index += 1;
            }
            b'}' | b')' | b']' => {
                if stack
                    .pop()
                    .is_none_or(|opening| !delimiters_match(opening, current))
                {
                    opaque_reason.get_or_insert(OpaqueReason::ParseError);
                }
                index += 1;
            }
            b'@' if bytes
                .get(index + 1)
                .is_some_and(|next| *next == b'\'' || *next == b'"') =>
            {
                // Here-strings have line-sensitive terminators. Treat them as
                // opaque rather than risking a delimiter inside their body.
                opaque_reason.get_or_insert(OpaqueReason::String);
                index += 2;
            }
            b'>' | b'<' if stack.is_empty() => {
                opaque_reason.get_or_insert(OpaqueReason::Redirection);
                index += 1;
            }
            b'&' if stack.is_empty() => {
                if bytes.get(index + 1) == Some(&b'&') {
                    finish_segment(
                        source,
                        segment_start,
                        index,
                        &mut nodes,
                        &mut command_names,
                        &mut opaque_reason,
                    );
                    segment_start = index + 2;
                    index += 2;
                } else {
                    opaque_reason.get_or_insert(OpaqueReason::DynamicInvocation);
                    index += 1;
                }
            }
            b'|' if stack.is_empty() => {
                if bytes.get(index + 1) == Some(&b'|') {
                    finish_segment(
                        source,
                        segment_start,
                        index,
                        &mut nodes,
                        &mut command_names,
                        &mut opaque_reason,
                    );
                    segment_start = index + 2;
                    index += 2;
                } else {
                    finish_segment(
                        source,
                        segment_start,
                        index,
                        &mut nodes,
                        &mut command_names,
                        &mut opaque_reason,
                    );
                    nodes.push(Node {
                        kind: NodeKind::Pipeline,
                        span: Span {
                            start: index,
                            end: index + 1,
                        },
                    });
                    segment_start = index + 1;
                    index += 1;
                }
            }
            b';' if stack.is_empty() => {
                finish_segment(
                    source,
                    segment_start,
                    index,
                    &mut nodes,
                    &mut command_names,
                    &mut opaque_reason,
                );
                segment_start = index + 1;
                index += 1;
            }
            b'\r' | b'\n' if stack.is_empty() => {
                let separator_start = index;
                if current == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                    index += 2;
                } else {
                    index += 1;
                }
                finish_segment(
                    source,
                    segment_start,
                    separator_start,
                    &mut nodes,
                    &mut command_names,
                    &mut opaque_reason,
                );
                segment_start = index;
            }
            _ => index += 1,
        }
    }

    if quote.is_some() {
        opaque_reason.get_or_insert(OpaqueReason::ParseError);
    }
    if !stack.is_empty() {
        opaque_reason.get_or_insert(OpaqueReason::ParseError);
    }
    finish_segment(
        source,
        segment_start,
        trimmed.end,
        &mut nodes,
        &mut command_names,
        &mut opaque_reason,
    );

    if nodes.len() > 1 {
        nodes.insert(
            0,
            Node {
                kind: NodeKind::Statement,
                span: trimmed,
            },
        );
    }
    if opaque_reason.is_some() {
        nodes.push(Node {
            kind: NodeKind::Opaque,
            span: trimmed,
        });
    }
    ParsedScript {
        nodes,
        opaque_reason,
        command_names,
    }
}

fn finish_segment(
    source: &str,
    start: usize,
    end: usize,
    nodes: &mut Vec<Node>,
    command_names: &mut Vec<String>,
    opaque_reason: &mut Option<OpaqueReason>,
) {
    let Some(span) = trimmed_span(source, start, end) else {
        return;
    };
    let command_span = command_name_span(source, span);
    let command = &source[command_span.start..command_span.end];
    let normalized = command.trim_matches(['\'', '"']).to_ascii_lowercase();
    if normalized.starts_with('#') {
        return;
    }
    if command.starts_with('$') {
        opaque_reason.get_or_insert(OpaqueReason::ControlFlow);
    }
    if is_control_command(&normalized) {
        opaque_reason.get_or_insert(OpaqueReason::ControlFlow);
    }
    if is_explicit_formatting(&normalized) {
        opaque_reason.get_or_insert(OpaqueReason::ExplicitFormatting);
    }
    command_names.push(command.trim_matches(['\'', '"']).to_owned());
    nodes.push(Node {
        kind: NodeKind::Command,
        span,
    });
}

fn command_name_span(source: &str, span: Span) -> Span {
    let bytes = source.as_bytes();
    if bytes.get(span.start) == Some(&b'\'') || bytes.get(span.start) == Some(&b'"') {
        let quote = bytes[span.start];
        if let Some(relative) = source[span.start + 1..span.end]
            .bytes()
            .position(|byte| byte == quote)
        {
            return Span {
                start: span.start,
                end: span.start + relative + 2,
            };
        }
    }
    let end = source[span.start..span.end]
        .char_indices()
        .find_map(|(offset, character)| character.is_whitespace().then_some(span.start + offset))
        .unwrap_or(span.end);
    Span {
        start: span.start,
        end,
    }
}

fn trimmed_span(source: &str, start: usize, end: usize) -> Option<Span> {
    let slice = &source[start..end];
    let leading = slice
        .char_indices()
        .find_map(|(offset, character)| (!character.is_whitespace()).then_some(offset))?;
    let trailing = slice.char_indices().rev().find_map(|(offset, character)| {
        (!character.is_whitespace()).then_some(offset + character.len_utf8())
    })?;
    Some(Span {
        start: start + leading,
        end: start + trailing,
    })
}

fn is_control_command(command: &str) -> bool {
    matches!(
        command,
        "if" | "elseif"
            | "else"
            | "for"
            | "foreach"
            | "while"
            | "do"
            | "switch"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "return"
            | "break"
            | "continue"
            | "function"
            | "filter"
            | "class"
            | "enum"
            | "data"
            | "trap"
    )
}

fn delimiters_match(opening: u8, closing: u8) -> bool {
    matches!(
        (opening, closing),
        (b'{', b'}') | (b'(', b')') | (b'[', b']')
    )
}

fn is_explicit_formatting(command: &str) -> bool {
    command.starts_with("format-")
        || command.starts_with("out-")
        || command.starts_with("export-")
        || command.starts_with("convertto-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_command_without_marking_it_opaque() {
        let parsed = parse_expression("Get-ChildItem -Force");

        assert!(!parsed.is_opaque());
        assert_eq!(parsed.command_names(), vec!["Get-ChildItem".to_owned()]);
    }

    #[test]
    fn preserves_pipelines_and_quoted_metacharacters() {
        let source = "Write-Output 'a | b' | Select-Object Length";
        let parsed = parse_expression(source);

        assert!(!parsed.is_opaque());
        assert_eq!(
            parsed.command_names(),
            vec!["Write-Output".to_owned(), "Select-Object".to_owned()]
        );
        let span = parsed.final_command_span().unwrap();
        assert_eq!(&source[span.start..span.end], "Select-Object Length");
    }

    #[test]
    fn rejects_redirection_and_control_flow_for_rewriting() {
        for source in [
            "Get-ChildItem > output.txt",
            "if ($true) { Get-ChildItem }",
            "Get-ChildItem & Write-Output done",
        ] {
            assert!(parse_expression(source).is_opaque(), "source: {source}");
        }
    }

    #[test]
    fn balances_nested_subexpressions_and_reports_unterminated_quotes() {
        assert!(!parse_expression("Write-Output $(Get-Date)").is_opaque());
        assert!(parse_expression("Write-Output 'unterminated").is_opaque());
    }

    #[test]
    fn stop_parsing_native_tail_is_opaque() {
        let parsed = parse_expression("git --% status --porcelain");
        assert_eq!(parsed.opaque_reason, Some(OpaqueReason::NativeCommand));
    }

    #[test]
    fn scriptblocks_are_opaque_but_hashtables_remain_lexically_balanced() {
        assert_eq!(
            parse_expression("ForEach-Object { $_.Name }").opaque_reason,
            Some(OpaqueReason::ScriptBlock)
        );
        assert_eq!(
            parse_expression("Write-Output @{Name='value'}").opaque_reason,
            None
        );
        assert!(parse_expression("Write-Output ([)]").is_opaque());
        assert!(parse_expression("$items = Get-ChildItem").is_opaque());
    }

    #[test]
    fn comments_unicode_and_crlf_keep_command_spans() {
        let source = "\u{feff}# comment\r\nGet-ChildItem '資料'\r\n";
        let parsed = parse_expression(source);
        assert!(!parsed.is_opaque());
        assert_eq!(parsed.command_names(), vec!["Get-ChildItem"]);
        let span = parsed.final_command_span().expect("command span");
        assert_eq!(&source[span.start..span.end], "Get-ChildItem '資料'");
    }
}
