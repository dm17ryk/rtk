use super::catalog::{builtins, validate_catalog, AdapterStrategy, CommandMode};
use super::parser::{parse_expression, OpaqueReason, Span};

fn segment_text<'a>(source: &'a str, span: Span) -> &'a str {
    &source[span.start..span.end]
}

#[test]
fn parses_top_level_chains_with_exact_replacement_spans() {
    let source = "  echo hello  && dir /b || type readme.txt";
    let parsed = parse_expression(source);

    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.span))
            .collect::<Vec<_>>(),
        ["echo hello", "dir /b", "type readme.txt"]
    );
    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.command_span))
            .collect::<Vec<_>>(),
        ["echo", "dir", "type"]
    );
    assert_eq!(parsed.opaque_reason, None);
}

#[test]
fn quotes_and_caret_escapes_keep_operators_inside_simple_segments() {
    let source = r#"echo "a & b" ^& literal & dir"#;
    let parsed = parse_expression(source);

    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.span))
            .collect::<Vec<_>>(),
        [r#"echo "a & b" ^& literal"#, "dir"]
    );
    assert_eq!(parsed.opaque_reason, None);
}

#[test]
fn single_quotes_do_not_quote_cmd_operators() {
    let source = "echo 'a & b' & dir";
    let parsed = parse_expression(source);

    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.span))
            .collect::<Vec<_>>(),
        ["echo 'a", "b'", "dir"]
    );
    assert_eq!(parsed.opaque_reason, None);
}

#[test]
fn preserves_crlf_and_variable_expansions_without_reformatting() {
    let source = "@echo %USERNAME%\r\ndir %CD%";
    let parsed = parse_expression(source);

    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.span))
            .collect::<Vec<_>>(),
        ["@echo %USERNAME%", "dir %CD%"]
    );
    assert_eq!(
        segment_text(source, parsed.segments[0].command_span),
        "echo"
    );
    assert_eq!(parsed.opaque_reason, None);
}

#[test]
fn crlf_is_a_cmd_command_boundary() {
    let source = "echo one\r\necho two";
    let parsed = parse_expression(source);

    assert_eq!(
        parsed
            .segments
            .iter()
            .map(|segment| segment_text(source, segment.span))
            .collect::<Vec<_>>(),
        ["echo one", "echo two"]
    );
    assert_eq!(parsed.opaque_reason, None);
}

#[test]
fn quoted_batch_path_is_opaque() {
    let source = r#""C:\Program Files\build.cmd" --release"#;
    let parsed = parse_expression(source);

    assert_eq!(parsed.opaque_reason, Some(OpaqueReason::BatchInvocation));
    assert_eq!(
        segment_text(source, parsed.segments[0].command_span),
        r#""C:\Program Files\build.cmd""#
    );
}

#[test]
fn empty_operands_around_chain_operators_are_malformed() {
    for source in [
        "& echo hi",
        "echo hi &",
        "&& echo hi",
        "echo hi &&",
        "|| echo hi",
        "echo hi ||",
    ] {
        assert_eq!(
            parse_expression(source).opaque_reason,
            Some(OpaqueReason::MalformedInput),
            "{source}"
        );
    }
}

#[test]
fn fails_open_for_opaque_or_malformed_cmd_constructs() {
    let cases = [
        ("dir | findstr rs", OpaqueReason::OutputPipeline),
        ("dir > listing.txt", OpaqueReason::OutputRedirection),
        ("(echo one & echo two)", OpaqueReason::ControlGroup),
        ("if exist file echo yes", OpaqueReason::ControlCommand),
        ("build.bat", OpaqueReason::BatchInvocation),
        ("echo !PATH!", OpaqueReason::DelayedExpansion),
        ("C:", OpaqueReason::DriveChange),
        ("echo \"unterminated", OpaqueReason::MalformedInput),
        ("echo trailing^", OpaqueReason::MalformedInput),
    ];

    for (source, reason) in cases {
        assert_eq!(
            parse_expression(source).opaque_reason,
            Some(reason),
            "{source}"
        );
    }
}

#[test]
fn builtin_catalog_has_a_strategy_for_every_intrinsic_and_extension() {
    let names = builtins()
        .iter()
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "assoc", "break", "call", "cd", "chcp", "cls", "color", "copy", "date", "del", "dir",
            "echo", "endlocal", "exit", "for", "ftype", "goto", "help", "if", "md", "mklink",
            "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "set",
            "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
        ]
    );
    assert!(builtins().iter().all(|command| match command.strategy {
        Some(AdapterStrategy::Identity { .. } | AdapterStrategy::Structured { .. }) => true,
        None => false,
    }));
}

#[test]
fn builtin_catalog_marks_stateful_control_and_interactive_commands() {
    let catalog = builtins();
    let find = |name| {
        catalog
            .iter()
            .find(|command| command.matches(name))
            .unwrap()
    };

    assert_eq!(find("chdir").name, "cd");
    assert_eq!(find("erase").name, "del");
    assert_eq!(find("mkdir").name, "md");
    assert_eq!(find("rmdir").name, "rd");
    assert_eq!(find("rename").name, "ren");
    assert_eq!(find("if").mode, CommandMode::Control);
    assert_eq!(find("setlocal").mode, CommandMode::Stateful);
    assert_eq!(find("pause").mode, CommandMode::Interactive);
}

#[test]
fn catalog_validation_rejects_duplicate_aliases_and_missing_strategies() {
    assert_eq!(validate_catalog(&builtins()), Ok(()));

    let duplicate = [
        builtins()[0].clone(),
        builtins()[1].clone().with_aliases(&["assoc"]),
    ];
    assert!(validate_catalog(&duplicate).is_err());

    let missing_strategy = [builtins()[0].clone().without_strategy()];
    assert!(validate_catalog(&missing_strategy).is_err());
}
