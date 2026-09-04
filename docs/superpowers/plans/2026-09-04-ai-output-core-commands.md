# AI Output Core Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `rtk rg`, `rtk read`, and `rtk find` emit smaller, structured AI-first output with explicit lossless recovery and accurate output-contract telemetry.

**Architecture:** Use the semantic output path introduced by PR #10. A small shared path-inventory builder returns `AiDocument` records for both `find` and `rg` inventory modes. Command-specific parsers build `AiDocument` instances, while the runner prepares, emits, and tracks them with `BudgetClass::Source` or `BudgetClass::Collection`; opaque routes remain exact.

**Tech Stack:** Rust 2021, clap, existing `AiDocument`/`PreparedEmission`, RTK runner and tracking, Cargo tests.

**Spec:** `docs/superpowers/specs/2026-09-04-ai-output-core-commands-design.md`

## Global Constraints

- Token reduction is the primary success metric; do not claim a percentage without fixture measurements.
- Optimize `rtk rg`, `rtk read`, and `rtk find`; preserve `rtk grep` behavior.
- Preserve native exit codes and emit captured stderr exactly once.
- Use lossless recovery and exact omission metadata whenever RTK omits data.
- Keep NUL/binary, interactive, unsupported, and unknown `rg` modes exact.
- Do not add dependencies or change release metadata.
- Use `rtk` wrappers for cargo, git, and gh commands.

---

### Task 1: Expose semantic emission for command adapters

**Files:**
- Modify: `src/core/runner.rs:15-201`
- Modify: `src/core/runner.rs:1240-1769`

**Interfaces:**
- Consumes: `AiDocument`, `BudgetClass`, `prepare_emission`, `EmissionMeta`, and `TimedExecution`.
- Produces: `pub(crate) fn emit_ai_document(timer, original_cmd, rtk_cmd, raw, command_slug, budget, document, trailing_newline) -> String`.
- Produces: semantic tracking with `OutputContract::AiOwned(budget)` and `output_tracking_from_emission`.

- [ ] **Step 1: Write the failing unit test for adapter-level semantic tracking**

```rust
#[test]
fn emit_ai_document_tracks_recovery_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("tracking.db");
    let tee_dir = temp.path().join("tee");
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("RTK_DB_PATH", &database);
    std::env::set_var("RTK_TEE_DIR", &tee_dir);
    let raw = (0..500).map(|n| format!("line-{n}")).collect::<Vec<_>>().join("\n");
    let mut document = AiDocument::new(Some("source"));
    for n in 0..500 {
        document.push(AiRecord::new(Severity::Info, format!("{n}: line-{n}")));
    }
    let shown = emit_ai_document(TimedExecution::start(), "cat sample", "rtk read",
        &raw, "read", BudgetClass::Source, document, true);
    assert!(shown.contains("recover=rtk read -l none --recovery"));
    let conn = rusqlite::Connection::open(&database).unwrap();
    let stored: (String, bool) = conn.query_row(
        "SELECT output_contract, recovery_created FROM commands ORDER BY id DESC LIMIT 1", [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored.0, "ai_owned");
    assert!(stored.1);
    std::env::remove_var("RTK_TEE_DIR");
    std::env::remove_var("RTK_DB_PATH");
}
```

- [ ] **Step 2: Run the focused test and verify it fails because the adapter does not exist**

Run: `rtk test cargo test --bin rtk emit_ai_document_tracks_recovery_metadata`

Expected: compile failure naming `emit_ai_document`.

- [ ] **Step 3: Implement the semantic adapter with the existing prepared-emission pipeline**

```rust
pub(crate) fn emit_ai_document(
    timer: tracking::TimedExecution,
    original_cmd: &str,
    rtk_cmd: &str,
    raw: &str,
    command_slug: &str,
    budget: BudgetClass,
    document: AiDocument,
    trailing_newline: bool,
) -> String {
    let prepared = prepare_emission(raw, command_slug, render(&document, budget), trailing_newline);
    let shown = prepared.as_str().to_string();
    let meta = prepared.meta();
    emit_prepared(&prepared);
    timer.track_output(original_cmd, rtk_cmd, raw, &shown,
        output_tracking_from_emission(OutputContract::AiOwned(budget), meta));
    shown
}
```

Keep the existing capture-overflow and exact replay behavior unchanged. The helper is for
in-process adapters (`read` and `find`); `rg` uses `run_ai_filtered` so its native stderr and
exit handling remain centralized.

- [ ] **Step 4: Run the focused runner tests**

Run: `rtk test cargo test --bin rtk emit_ai_document_tracks_recovery_metadata`

Run: `rtk test cargo test --bin rtk semantic_`

Expected: all selected tests pass, with an isolated tracking database and a recoverable artifact.

- [ ] **Step 5: Commit the independently working adapter**

Run: `rtk git add src/core/runner.rs`

Run: `rtk git commit -m "feat(output): expose semantic command emission"`

Expected: one commit containing only the runner adapter and its tests.

### Task 2: Build and integrate a shared compact path inventory

**Files:**
- Create: `src/core/path_inventory.rs`
- Modify: `src/core/mod.rs:3-17`
- Modify: `src/cmds/system/find_cmd.rs:344-740`
- Test: `src/core/path_inventory.rs` module tests
- Test: `src/cmds/system/find_cmd.rs` module tests

**Interfaces:**
- Consumes: `&[String]` paths in caller-established stable order.
- Produces: `pub fn document(paths: &[String]) -> AiDocument` with `files`, `dirs`, and optional `root` facts.
- Produces: `pub fn canonical_groups(paths: &[String]) -> Vec<(String, Vec<String>)>` for deterministic grouped records.
- Consumed later by: the `rg` inventory routes in Task 4.

- [ ] **Step 1: Write failing path-inventory tests**

```rust
#[test]
fn inventory_uses_unambiguous_common_root_once() {
    let doc = document(&vec!["src/core/runner.rs".into(), "src/cmds/read.rs".into()]);
    assert_eq!(render(&doc, BudgetClass::Collection).text,
        "files=2 dirs=2 root=src\ncmds/{read.rs}\ncore/{runner.rs}\n");
}

#[test]
fn inventory_keeps_disjoint_roots_explicit() {
    let doc = document(&vec!["src/main.rs".into(), "tests/read.rs".into()]);
    assert!(!render(&doc, BudgetClass::Collection).text.contains("root="));
}
```

Add a test with 500 paths that asserts the rendered output reports exact omissions and has a
lossless recovery artifact once used by `find`.

- [ ] **Step 2: Run the tests and verify the module is absent**

Run: `rtk test cargo test --bin rtk path_inventory`

Expected: compile failure because `core::path_inventory` is not declared.

- [ ] **Step 3: Implement deterministic common-root and directory grouping**

```rust
pub fn canonical_groups(paths: &[String]) -> Vec<(String, Vec<String>)> {
    // Normalize only a leading "./", retain every other supplied path segment,
    // sort directories and leaf names lexically, and never combine disjoint roots.
}

pub fn document(paths: &[String]) -> AiDocument {
    let mut doc = AiDocument::new(Some("inventory"));
    doc.fact("files", paths.len().to_string());
    doc.fact("dirs", canonical_groups(paths).len().to_string());
    // Add root only when every path has the same first component.
    // Push one `dir/{leaf,...}` record per directory group.
    doc
}
```

Do not use a filesystem walk; the caller's result list is the source of truth. Represent a root
directory as `./{leaf}` and paths outside the common root explicitly.

- [ ] **Step 4: Migrate `find_cmd::render` to the inventory document**

Replace hand-built `"{}F {}D"` output with `path_inventory::document(&files)` and
`runner::emit_ai_document`. Preserve the current parser, `run_verbatim`, `max` behavior,
stderr handling, native raw output, and caller-selected limits. Use `BudgetClass::Collection`
and command slug `"find"`.

- [ ] **Step 5: Add and run `find` recovery and passthrough tests**

```rust
#[test]
fn render_uses_collection_contract_and_recovers_omitted_paths() {
    let temp = tempfile::tempdir().unwrap();
    let files = (0..2_000).map(|n| format!("src/pkg{n}/file{n}.rs")).collect::<Vec<_>>();
    let raw = files.join("\n");
    let document = path_inventory::document(&files);
    let prepared = prepare_emission(&raw, "find", render(&document, BudgetClass::Collection), true);
    assert!(prepared.as_str().contains("recover=rtk read -l none --recovery"));
    assert!(prepared.meta().recovery_created);
}

#[test]
fn unsupported_expression_dispatches_verbatim() {
    assert!(matches!(dispatch(&args(&["src", "-exec", "echo", "{}", ";"])), Dispatch::Verbatim(_)));
}
```

Run: `rtk test cargo test --bin rtk path_inventory`

Run: `rtk test cargo test --bin rtk render_uses_collection_contract_and_recovers_omitted_paths`

Expected: compact paths retain counts; overflow has a valid recovery command; unsupported grammar remains exact.

- [ ] **Step 6: Commit the inventory renderer and `find` migration**

Run: `rtk git add src/core/mod.rs src/core/path_inventory.rs src/cmds/system/find_cmd.rs`

Run: `rtk git commit -m "feat(find): render compact AI path inventories"`

Expected: the commit introduces the reusable renderer and its first caller.

### Task 3: Make `rtk read` AI-first by default with source recovery

**Files:**
- Modify: `src/main.rs:107-125`
- Modify: `src/main.rs:1917-1945`
- Modify: `src/cmds/system/read.rs:10-159`
- Test: `src/main.rs:3244-3259`
- Test: `src/cmds/system/read.rs:206-299`

**Interfaces:**
- Consumes: file content, `FilterLevel`, explicit window flags, and the new runner adapter.
- Produces: `fn source_document(file: &Path, content: &str, filtered: &str) -> AiDocument`.
- Produces: default CLI level `FilterLevel::Minimal`; explicit `-l none` remains exact.

- [ ] **Step 1: Write failing parser and renderer tests**

```rust
#[test]
fn read_defaults_to_minimal_ai_filtering() {
    let cli = Cli::try_parse_from(["rtk", "read", "src/main.rs"]).unwrap();
    assert!(matches!(read_level(cli), FilterLevel::Minimal));
}

#[test]
fn source_document_uses_dense_line_markers() {
    let doc = source_document(Path::new("sample.rs"), "fn main() {}\n", "fn main() {}\n");
    assert!(render(&doc, BudgetClass::Source).text.contains("1: fn main() {}"));
}
```

Add a multi-hundred-line source test asserting that default filtering generates recovery, while
`-l none` prints the full content without a recovery hint.

- [ ] **Step 2: Run focused tests and verify the current default is `none`**

Run: `rtk test cargo test --bin rtk read_defaults_to_minimal_ai_filtering`

Run: `rtk test cargo test --bin rtk source_document_uses_dense_line_markers`

Expected: default-level assertion fails before implementation.

- [ ] **Step 3: Implement source records and default minimal filtering**

```rust
fn source_document(file: &Path, filtered: &str) -> AiDocument {
    let mut doc = AiDocument::new(Some("source"));
    doc.fact("file", file.display().to_string());
    for (index, line) in filtered.lines().enumerate() {
        doc.push(AiRecord::new(Severity::Info, format!("{}: {}", index + 1, line)));
    }
    doc
}
```

Make `level` default to `"minimal"` and update its clap help text. Preserve actual original
line numbers when the language filter removes earlier lines: change the filter boundary to carry
`(original_line_number, text)` records instead of renumbering filtered lines. Keep explicit
`-l none` on the exact legacy content path. For default filtering, send the full original content
to `emit_ai_document` with `BudgetClass::Source` and slug `"read"`.

- [ ] **Step 4: Preserve stdin and explicit-window semantics**

Add equivalent source-record rendering for `run_stdin`, using `"stdin"` as the file fact.
Apply `-m` and `--tail-lines` before document construction. Treat caller-selected windows as
intentional selection, not an RTK omission; their output has no misleading recovery count.

- [ ] **Step 5: Run the focused read and parser suite**

Run: `rtk test cargo test --bin rtk read_`

Run: `rtk test cargo test --bin rtk source_document_`

Run: `rtk test cargo test --bin rtk apply_line_window`

Expected: default mode is compact and recoverable, explicit exact mode and windows retain their documented behavior.

- [ ] **Step 6: Commit the read behavior change**

Run: `rtk git add src/main.rs src/cmds/system/read.rs`

Run: `rtk git commit -m "feat(read): default to compact AI source output"`

Expected: one commit with parser help, source rendering, stdin behavior, and tests.

### Task 4: Classify all recognized `rtk rg` output routes

**Files:**
- Modify: `src/cmds/system/search.rs:22-252`
- Modify: `src/cmds/system/search.rs:500-789`
- Test: `src/cmds/system/search.rs:355-1156`

**Interfaces:**
- Consumes: `Engine`, parsed flags, patterns, and paths.
- Produces: `enum RgRoute { Matches, JsonEvents, Inventory, Counts, OnlyMatching, Exact(ExactReason) }`.
- Produces: `fn classify_rg(args: &[String]) -> RgRoute`.
- Preserves: `Engine::Grep` continues through its existing path without calling `classify_rg`.

- [ ] **Step 1: Write table-driven failing classifier tests**

```rust
#[test]
fn rg_route_table_is_conservative_and_complete() {
    let cases = [
        (&["needle"], RgRoute::Matches),
        (&["--json", "needle"], RgRoute::JsonEvents),
        (&["--files"], RgRoute::Inventory),
        (&["-l", "needle"], RgRoute::Inventory),
        (&["-c", "needle"], RgRoute::Counts),
        (&["-o", "needle"], RgRoute::OnlyMatching),
        (&["--null", "needle"], RgRoute::Exact(ExactReason::Structured)),
        (&["--help"], RgRoute::Exact(ExactReason::Interactive)),
        (&["--future-flag", "needle"], RgRoute::Exact(ExactReason::Unknown)),
    ];
    for (args, expected) in cases { assert_eq!(classify_rg(&args(args)), expected); }
}
```

Include `--replace`, context, count-matches, `-L`, `--version`, binary, `--passthru`, and
short-flag clusters. Each expected exact route must name the precise `ExactReason` used for telemetry.

- [ ] **Step 2: Run the classifier test and verify it fails**

Run: `rtk test cargo test --bin rtk rg_route_table_is_conservative_and_complete`

Expected: compile failure because `RgRoute` and `classify_rg` do not exist.

- [ ] **Step 3: Implement the classifier without changing grep routing**

```rust
fn classify_rg(args: &[String]) -> RgRoute {
    // Parse value-taking flags with the existing cluster parser.
    // Choose exactly one recognized output shape.
    // Reject NUL, binary, interactive, help/version, and unrecognized flags.
}
```

Use the same flag-consumption table as `extract_pattern_path`; do not inspect a value token as a
second flag. Unknown flag spelling is exact even when it resembles a known long option.

- [ ] **Step 4: Run classifier and existing search parser tests**

Run: `rtk test cargo test --bin rtk rg_route_`

Run: `rtk test cargo test --bin rtk test_extract_`

Run: `rtk test cargo test --bin rtk test_parse_cluster_`

Expected: all route and existing argument-consumption tests pass.

- [ ] **Step 5: Commit the classifier**

Run: `rtk git add src/cmds/system/search.rs`

Run: `rtk git commit -m "feat(rg): classify AI output routes"`

Expected: classifier-only commit; no renderer integration yet.

### Task 5: Render and run semantic `rg` routes

**Files:**
- Modify: `src/cmds/system/search.rs:255-485`
- Modify: `src/cmds/system/search.rs:500-789`
- Modify: `tests/search_faithful_test.rs:1-406`
- Test: `src/cmds/system/search.rs` module tests

**Interfaces:**
- Consumes: `RgRoute`, native stdout, patterns, and paths.
- Produces: `fn rg_document(route: RgRoute, raw: &str, patterns: &[String], paths: &[String]) -> Result<AiDocument>`.
- Consumes: `core::path_inventory::document` for inventory output.
- Uses: `runner::run_ai_filtered(..., BudgetClass::Source|Collection, RunOptions::stdout_only())`.

- [ ] **Step 1: Write failing semantic-renderer tests**

```rust
#[test]
fn rg_matches_group_contiguous_entries_without_reordering() {
    let raw = "a.rs:3:needle one\nb.rs:2:needle two\na.rs:9:needle three\n";
    let rendered = render(&rg_document(RgRoute::Matches, raw, &["needle".into()], &[]).unwrap(), BudgetClass::Source).text;
    assert!(rendered.contains("a.rs"));
    assert!(rendered.contains("3: needle one"));
    assert!(rendered.find("b.rs").unwrap() < rendered.rfind("a.rs").unwrap());
}

#[test]
fn rg_json_discards_event_noise_but_keeps_match_text() {
    let raw = concat!(
        "{\"type\":\"begin\",\"data\":{\"path\":{\"text\":\"a.rs\"}}}\n",
        "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"a.rs\"},",
        "\"lines\":{\"text\":\"needle here\\n\"},\"line_number\":7,",
        "\"absolute_offset\":0,\"submatches\":[{\"match\":{\"text\":\"needle\"},\"start\":0,\"end\":6}]}}\n",
        "{\"type\":\"end\",\"data\":{\"path\":{\"text\":\"a.rs\"},\"binary_offset\":null,\"stats\":{}}}\n",
    );
    let rendered = render(&rg_document(RgRoute::JsonEvents, raw, &["needle".into()], &[]).unwrap(), BudgetClass::Source).text;
    assert!(rendered.contains("a.rs"));
    assert!(rendered.contains("7: needle here"));
    assert!(!rendered.contains("\"type\":\"begin\""));
}

#[test]
fn rg_count_records_are_path_equals_count() {
    let rendered = render(&rg_document(RgRoute::Counts, "a.rs:4\nb.rs:1\n", &[], &[]).unwrap(), BudgetClass::Collection).text;
    assert!(rendered.contains("a.rs=4"));
    assert!(rendered.contains("b.rs=1"));
}
```

Add integration cases for ordinary matches, JSON, files, `-l`, `-L`, `-c`, `--count-matches`,
`-o`, context, and replacement output. Add exact-native cases for NUL, binary, unknown, help,
version, and grep. Each successful compact integration case must assert unchanged exit status,
stderr appearing once, and valid recovery when budget omission occurs.

- [ ] **Step 2: Run the focused renderer tests and verify they fail**

Run: `rtk test cargo test --bin rtk rg_matches_group_contiguous_entries_without_reordering`

Run: `rtk test cargo test --bin rtk rg_json_discards_event_noise_but_keeps_match_text`

Expected: compile failure because `rg_document` does not exist.

- [ ] **Step 3: Implement `rg_document` for every recognized route**

```rust
fn rg_document(route: RgRoute, raw: &str, patterns: &[String], paths: &[String]) -> Result<AiDocument> {
    match route {
        RgRoute::Matches => parse_match_records(raw, patterns),
        RgRoute::JsonEvents => parse_json_events(raw, patterns),
        RgRoute::Inventory => Ok(path_inventory::document(&parse_inventory_paths(raw))),
        RgRoute::Counts => parse_count_records(raw),
        RgRoute::OnlyMatching => parse_only_matching_records(raw),
        RgRoute::Exact(reason) => Err(anyhow!("exact route reached semantic renderer: {reason:?}")),
    }
}
```

Use full file paths and original line numbers in each record. Preserve record order. Distinguish
context with `-` from matches with `:`. For JSON, parse only the documented `match`, `context`,
and summary data needed to construct the equivalent records; malformed event input returns an
error so the runner emits a bounded parse-failure document with recovery.

- [ ] **Step 4: Replace only the `Engine::Rg` captured route with `run_ai_filtered`**

Call `run_ai_filtered` after `classify_rg` chooses a recognized route. Select
`BudgetClass::Collection` for inventories and `BudgetClass::Source` for match/count/only-matching
documents. For `RgRoute::Exact(reason)`, use the existing native passthrough mechanism and
`track_exact`; do not change `Engine::Grep` execution or its streaming filter.

- [ ] **Step 5: Run semantic and faithfulness tests**

Run: `rtk test cargo test --bin rtk rg_`

Run: `rtk test cargo test --bin rtk test_format_flag_`

Run: `rtk test cargo test --bin rtk test_extract_`

Run: `rtk test cargo test --test search_faithful_test`

Expected: recognized forms are compact, exact forms replay natively, and every grep assertion remains unchanged.

- [ ] **Step 6: Commit semantic `rg` rendering**

Run: `rtk git add src/cmds/system/search.rs tests/search_faithful_test.rs`

Run: `rtk git commit -m "feat(rg): emit compact semantic search output"`

Expected: one commit covering parsers, runner integration, and integration tests.

### Task 6: Prove measurement, documentation, and full compatibility

**Files:**
- Create: `tests/ai_output_core_commands_test.rs`
- Modify: `src/cmds/system/README.md:1-20`
- Modify: `docs/guide/` command guidance file selected by existing `rtk read`/`rtk rg` documentation search

**Interfaces:**
- Consumes: completed route renderers and isolated `RTK_DB_PATH`/`RTK_TEE_DIR` test environments.
- Produces: regression fixtures that compare raw and rendered token estimates and validate recovery payloads.

- [ ] **Step 1: Add end-to-end measurement fixtures**

```rust
#[test]
fn core_command_fixtures_are_never_worse_and_record_ai_contracts() {
    for fixture in ["rg-many-matches", "read-large-rust", "find-tree"] {
        let result = run_fixture_with_isolated_tracking(fixture);
        assert!(estimate_tokens(&result.shown) <= estimate_tokens(&result.raw));
        assert_eq!(result.contract, "ai_owned");
    }
}
```

For each fixture, store raw byte count, shown byte count, and estimated tokens in assertion
messages. If a compact renderer omits output, execute the advertised `rtk read -l none --recovery`
command and assert byte-for-byte payload equality.

- [ ] **Step 2: Add documentation for the new defaults and exact boundary**

Document that `rtk read` is compact by default and `-l none` is exact. Document that `rtk rg`
optimizes recognized textual modes, while NUL/binary, interactive, and unknown modes remain exact
until a semantic route is added. State that recovery identifiers are shell-neutral and must be
read with `rtk read -l none --recovery`.

- [ ] **Step 3: Run focused fixtures, then capture measured values**

Run: `rtk test cargo test --test ai_output_core_commands_test`

Expected: all three fixtures print before/after byte and token counts in their assertion context and pass.

- [ ] **Step 4: Run the complete local quality gate**

Run: `rtk test cargo test --all`

Run: `rtk cargo clippy --all-targets --all-features -- -D warnings`

Run: `rtk cargo fmt --all -- --check`

Run: `rtk cargo run --quiet -- verify --require-all`

Run: `rtk git diff --check`

Expected: every command succeeds. Inspect `rtk test` summaries for `test result: ok`, not merely its wrapper exit code.

- [ ] **Step 5: Validate documentation with CRLF-normalized Git Bash input on Windows**

Run: `C:\Program Files\Git\bin\bash.exe -c 'PATH=/usr/bin:/bin:/mingw64/bin:$PATH; export PATH; tr -d "\r" < scripts/validate-docs.sh | bash'`

Expected: documentation validation succeeds without a CRLF-related parser error.

- [ ] **Step 6: Commit tests and documentation**

Run: `rtk git add tests/ai_output_core_commands_test.rs src/cmds/system/README.md docs`

Run: `rtk git commit -m "test(output): measure compact core command output"`

Expected: final commit contains fixtures and user-facing documentation, with no version metadata changes.
