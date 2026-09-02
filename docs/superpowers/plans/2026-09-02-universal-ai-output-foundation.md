# Universal AI-First Output Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the shared semantic-output, budget, recovery, tracking, and verification foundation that every RTK command family will migrate onto.

**Architecture:** Add a small `core::ai_output` model and renderer while leaving domain parsing in command modules. Existing string filters enter through a behavior-compatible legacy adapter; new semantic filters use `run_ai_filtered`, and exact paths record an explicit reason through `run_passthrough_with_reason`. Lossy output is emitted only when a complete private recovery artifact can be committed and the final body plus recovery instruction remains smaller than native output.

**Tech Stack:** Rust 2021, existing `anyhow`, `rusqlite`, `serde`, RTK byte-based token estimator, existing private/lossless tee infrastructure, inline Rust unit tests.

**Spec:** `docs/superpowers/specs/2026-09-02-universal-ai-output-design.md`

## Global Constraints

- RTK output is an interface for AI agents; optimize residual tokens, deterministic structure, and actionable information rather than terminal aesthetics.
- Use the exact budget limits from the spec: Acknowledgement 128, State 512, Collection 1,024, Diagnostic 2,048, and Source 4,096 estimated tokens.
- Preserve native arguments, bytes, streams, interaction, exit codes, signals, and cancellation for exact routes.
- Unknown commands and flags default to `Exact(Unknown)`.
- Never emit more estimated tokens than the native semantic input.
- Never emit an omission unless complete raw recovery was committed successfully.
- Keep existing string-filter output byte-for-byte compatible during this foundation phase.
- Add no dependency, network service, AI inference, or tokenizer.
- Do not change release or version metadata.
- Use test-first development and commit after every task.

---

### Task 1: Add output contracts, semantic documents, and deterministic rendering

**Files:**
- Create: `src/core/ai_output.rs`
- Modify: `src/core/mod.rs:3-17`
- Test: inline tests in `src/core/ai_output.rs`

**Interfaces:**
- Consumes: `crate::core::tracking::estimate_tokens(&str) -> usize`.
- Produces: `BudgetClass`, `ExactReason`, `OutputContract`, `Severity`, `AiRecord`, `Omission`, `AiDocument`, `RenderedOutput`, and `render(&AiDocument, BudgetClass) -> RenderedOutput`.
- Invariant: `AiDocument::legacy` is never reordered, deduplicated, or budget-truncated.

- [ ] **Step 1: Register the new core module and write failing budget/contract tests**

Add `pub mod ai_output;` to `src/core/mod.rs`. Start `src/core/ai_output.rs` with tests that require the exact contract vocabulary and limits:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_limits_match_the_product_contract() {
        assert_eq!(BudgetClass::Acknowledgement.max_tokens(), 128);
        assert_eq!(BudgetClass::State.max_tokens(), 512);
        assert_eq!(BudgetClass::Collection.max_tokens(), 1_024);
        assert_eq!(BudgetClass::Diagnostic.max_tokens(), 2_048);
        assert_eq!(BudgetClass::Source.max_tokens(), 4_096);
    }

    #[test]
    fn unknown_exact_reason_is_stable_for_tracking() {
        assert_eq!(ExactReason::Unknown.as_str(), "unknown");
        assert_eq!(OutputContract::Exact(ExactReason::Unknown).as_str(), "exact");
    }
}
```

- [ ] **Step 2: Run the focused test and verify the missing-type failure**

Run: `rtk test cargo test --bin rtk ai_output::tests::budget_limits_match_the_product_contract`

Expected: compilation fails because `BudgetClass`, `ExactReason`, and `OutputContract` are not defined.

- [ ] **Step 3: Implement contract and budget types**

Use these public definitions:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetClass {
    Acknowledgement,
    State,
    Collection,
    Diagnostic,
    Source,
}

impl BudgetClass {
    pub const fn max_tokens(self) -> usize {
        match self {
            Self::Acknowledgement => 128,
            Self::State => 512,
            Self::Collection => 1_024,
            Self::Diagnostic => 2_048,
            Self::Source => 4_096,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acknowledgement => "acknowledgement",
            Self::State => "state",
            Self::Collection => "collection",
            Self::Diagnostic => "diagnostic",
            Self::Source => "source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactReason {
    Structured,
    Interactive,
    Binary,
    Streaming,
    Unknown,
    Sensitive,
}

impl ExactReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::Interactive => "interactive",
            Self::Binary => "binary",
            Self::Streaming => "streaming",
            Self::Unknown => "unknown",
            Self::Sensitive => "sensitive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputContract {
    AiOwned(BudgetClass),
    Exact(ExactReason),
    Legacy,
}

impl OutputContract {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AiOwned(_) => "ai_owned",
            Self::Exact(_) => "exact",
            Self::Legacy => "legacy",
        }
    }
}
```

`Legacy` is a migration marker, not a final route contract. The universal-enforcement phase removes it.

- [ ] **Step 4: Write failing semantic rendering tests**

Add tests for severity ordering, exact duplicate counts, stable source order within a severity, compact summary grammar, budget omission, and untouched legacy output:

```rust
#[test]
fn semantic_render_orders_failures_and_counts_duplicates() {
    let mut doc = AiDocument::new(Some("fail"));
    doc.fact("passed", "12");
    doc.push(AiRecord::new(Severity::Warning, "src/a.rs:2 W unused"));
    doc.push(AiRecord::new(Severity::Error, "src/b.rs:7 E0308 expected=u32 actual=String"));
    doc.push(AiRecord::new(Severity::Error, "src/b.rs:7 E0308 expected=u32 actual=String"));

    let rendered = render(&doc, BudgetClass::Diagnostic);

    assert_eq!(
        rendered.text,
        "status=fail passed=12\nsrc/b.rs:7 E0308 expected=u32 actual=String repeats=2\nsrc/a.rs:2 W unused"
    );
    assert_eq!(rendered.omission, None);
}

#[test]
fn semantic_render_stops_before_collection_budget() {
    let mut doc = AiDocument::new(Some("ok"));
    for index in 0..300 {
        doc.push(AiRecord::new(
            Severity::Info,
            format!("src/generated/{index:03}.rs match=value"),
        ));
    }

    let rendered = render(&doc, BudgetClass::Collection);

    assert!(crate::core::tracking::estimate_tokens(&rendered.text) <= 1_024);
    assert!(rendered.omission.as_ref().is_some_and(|o| o.items > 0));
}

#[test]
fn legacy_render_is_byte_compatible() {
    let raw = "native heading\n  native spacing\n";
    let rendered = render(&AiDocument::legacy(raw), BudgetClass::State);
    assert_eq!(rendered.text, raw);
    assert_eq!(rendered.omission, None);
}
```

- [ ] **Step 5: Run semantic tests and verify they fail**

Run: `rtk test cargo test --bin rtk ai_output::tests::semantic_render`

Expected: compilation fails because the semantic document and renderer types are missing.

- [ ] **Step 6: Implement the minimal semantic document and renderer**

Use a small model with preformatted record bodies so command modules retain semantic ownership:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRecord {
    pub severity: Severity,
    pub text: String,
    pub group: Option<String>,
    source_order: usize,
}

impl AiRecord {
    pub fn new(severity: Severity, text: impl Into<String>) -> Self {
        Self {
            severity,
            text: text.into(),
            group: None,
            source_order: 0,
        }
    }

    pub fn grouped(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omission {
    pub items: usize,
    pub groups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocumentBody {
    Semantic,
    Legacy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDocument {
    status: Option<String>,
    facts: Vec<(String, String)>,
    records: Vec<AiRecord>,
    body: DocumentBody,
    declared_omission: Option<Omission>,
    parser_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOutput {
    pub text: String,
    pub omission: Option<Omission>,
    pub parser_failed: bool,
}
```

Add `AiDocument::with_omission(self, Omission) -> Self` for adapters that already know exact loss counts. Assign `source_order` in `AiDocument::push`. For semantic documents, sort by `(severity, source_order)`, deduplicate identical `(group, text)` records, append `repeats=N`, and stop before the class token limit. Count every represented source record not emitted in `Omission.items`; count distinct omitted groups in `Omission.groups`; then add any declared omission counts. For legacy documents, return the original string unchanged while carrying its declared omission into `RenderedOutput`.

- [ ] **Step 7: Run the complete module tests**

Run: `rtk test cargo test --bin rtk ai_output::tests`

Expected: all AI-output model and renderer tests pass.

- [ ] **Step 8: Commit the model and renderer**

```text
rtk git add src/core/mod.rs src/core/ai_output.rs
rtk git commit -m "feat(output): add AI document renderer"
```

---

### Task 2: Make lossy emission atomic, private, recoverable, and never worse

**Files:**
- Modify: `src/core/tee.rs:279-500`
- Modify: `src/core/ai_output.rs`
- Modify: `src/core/runner.rs:9-31`
- Test: inline tests in `src/core/tee.rs` and `src/core/ai_output.rs`

**Interfaces:**
- Consumes: `reserve_lossless_tee(raw, command_slug) -> Option<LosslessTeeReservation>` and `never_worse`.
- Produces: `LosslessTeeReservation::recovery_command()`, `LosslessTeeReservation::commit_output_if_better`, `PreparedEmission`, `prepare_emission`, and `runner::emit_prepared`.
- Invariant: if recovery cannot be created or the final candidate is not smaller, output is the complete raw input and no recovery artifact remains.

- [ ] **Step 1: Write failing lossless tee API tests**

Add tests beside the existing reservation tests:

```rust
#[test]
fn recovery_command_uses_rtk_read() {
    let temp = tempfile::tempdir().unwrap();
    let reservation = reserve_lossless_tee_file(
        "complete raw output",
        "cargo test",
        temp.path(),
        1_024,
        20,
    )
    .unwrap();
    let command = reservation.recovery_command();
    assert!(command.starts_with("rtk read -l none "));
}

#[test]
fn rejected_candidate_removes_pending_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let raw = "small";
    let reservation =
        reserve_lossless_tee_file(raw, "test", temp.path(), 1_024, 20).unwrap();
    assert!(reservation
        .commit_output_if_better(raw, "a much larger rendered candidate".to_string())
        .is_none());
    assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
}
```

Use the existing private file-level reservation helper so these tests do not mutate process-global environment variables.

- [ ] **Step 2: Run the tee tests and verify missing-method failures**

Run: `rtk test cargo test --bin rtk tee::tests::recovery_command_uses_rtk_read`

Expected: compilation fails because `recovery_command` and `commit_output_if_better` are missing.

- [ ] **Step 3: Implement the production lossless commit API**

Expose shell-safe recovery rendering through the reservation and generalize the existing test-only commit logic:

```rust
impl LosslessTeeReservation {
    pub fn recovery_command(&self) -> String {
        format!("rtk read -l none {}", display_shell_path(&self.committed_path))
    }

    pub fn commit_output_if_better(
        self,
        raw: &str,
        output: String,
    ) -> Option<LosslessTeeCommit> {
        if crate::core::guard::never_worse(raw, &output) == raw {
            return None;
        }
        self.commit_with_lock(output)
    }
}
```

Remove the test-only general commit implementation after redirecting its tests to the production method. Keep the specialized CMD and PowerShell helpers because their recovery syntax and CRLF behavior are separately tested.

- [ ] **Step 4: Write failing prepared-emission tests**

Avoid process-global environment mutation in parallel tests. Make `reserve_lossless_tee_file` `pub(crate)` and add a private `prepare_emission_with` helper whose final argument is a reservation function:

```rust
fn prepare_emission_with<F>(
    raw: &str,
    command_slug: &str,
    rendered: RenderedOutput,
    reserve: F,
) -> PreparedEmission
where
    F: FnOnce(&str, &str) -> Option<crate::core::tee::LosslessTeeReservation>,
{
    let parser_failed = rendered.parser_failed;
    let Some(omission) = rendered.omission else {
        let output = crate::core::guard::never_worse(raw, &rendered.text).to_string();
        let used_raw_fallback = output == raw && rendered.text != raw;
        return PreparedEmission::Plain {
            output,
            meta: EmissionMeta {
                parser_failed,
                used_raw_fallback,
                ..EmissionMeta::default()
            },
        };
    };

    let Some(reservation) = reserve(raw, command_slug) else {
        return PreparedEmission::Plain {
            output: raw.to_string(),
            meta: EmissionMeta {
                parser_failed,
                used_raw_fallback: true,
                ..EmissionMeta::default()
            },
        };
    };
    let recovery = reservation.recovery_command();
    let body = rendered
        .text
        .trim_end_matches(|ch| ch == '\r' || ch == '\n');
    let candidate = format!(
        "{body}\nomitted items={} groups={} recover={recovery}",
        omission.items, omission.groups
    );
    let meta = EmissionMeta {
        omitted_items: omission.items,
        omitted_groups: omission.groups,
        recovery_created: true,
        parser_failed,
        used_raw_fallback: false,
    };
    match reservation.commit_output_if_better(raw, candidate) {
        Some(commit) => PreparedEmission::Recovered { commit, meta },
        None => PreparedEmission::Plain {
            output: raw.to_string(),
            meta: EmissionMeta {
                parser_failed,
                used_raw_fallback: true,
                ..EmissionMeta::default()
            },
        },
    }
}
```

`prepare_emission` calls this helper with `crate::core::tee::reserve_lossless_tee`. Unit tests supply `reserve_lossless_tee_file` with a `tempfile::TempDir`, or a closure returning `None` to simulate disabled/unavailable recovery. Add these assertions:

```rust
#[test]
fn lossy_emission_contains_exact_counts_and_recovery_command() {
    let temp = tempfile::tempdir().unwrap();
    let rendered = RenderedOutput {
        text: "status=fail\nsrc/a.rs:1 E failure".to_string(),
        omission: Some(Omission { items: 14, groups: 3 }),
        parser_failed: false,
    };
    let raw = "native line\n".repeat(400);
    let prepared = prepare_emission_with(&raw, "cargo test", rendered, |raw, slug| {
        crate::core::tee::reserve_lossless_tee_file(raw, slug, temp.path(), 64_000, 20)
    });
    let shown = prepared.as_str();
    assert!(shown.contains("omitted items=14 groups=3 recover=rtk read -l none "));
    assert!(prepared.recovery_created());
}

#[test]
fn lossy_emission_falls_back_to_raw_when_tee_is_disabled() {
    let raw = "full native output\n".repeat(100);
    let rendered = RenderedOutput {
        text: "short".to_string(),
        omission: Some(Omission { items: 99, groups: 1 }),
        parser_failed: false,
    };
    let prepared = prepare_emission_with(&raw, "test", rendered, |_, _| None);
    assert_eq!(prepared.as_str(), raw);
    assert!(!prepared.recovery_created());
}
```

- [ ] **Step 5: Implement `PreparedEmission` and `prepare_emission`**

Use an enum so a committed recovery artifact stays locked until stdout receives its instruction:

```rust
pub enum PreparedEmission {
    Plain {
        output: String,
        meta: EmissionMeta,
    },
    Recovered {
        commit: crate::core::tee::LosslessTeeCommit,
        meta: EmissionMeta,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmissionMeta {
    pub omitted_items: usize,
    pub omitted_groups: usize,
    pub recovery_created: bool,
    pub parser_failed: bool,
    pub used_raw_fallback: bool,
}
```

For complete output, apply `never_worse` and return `Plain`. For lossy output, reserve a complete tee before constructing:

```text
<rendered body>
omitted items=N groups=N recover=<reservation recovery command>
```

Commit only through `commit_output_if_better`. If reservation or commit fails, return complete raw output with `used_raw_fallback=true`, zero emitted omissions, and no recovery claim.

- [ ] **Step 6: Route central printing through `emit_prepared`**

Add this runner sink and keep `emit_guarded` as a compatibility wrapper:

```rust
pub fn emit_prepared(prepared: &crate::core::ai_output::PreparedEmission, trailing_newline: bool) {
    if trailing_newline {
        println!("{}", prepared.as_str());
    } else {
        print!("{}", prepared.as_str());
    }
}
```

`PreparedEmission::as_str` returns the plain string or decodes the already-owned UTF-8 commit bytes. Do not release the recovered variant before printing.

- [ ] **Step 7: Run tee, emitter, and guard tests**

Run:

```text
rtk test cargo test --bin rtk tee::tests
rtk test cargo test --bin rtk ai_output::tests
rtk test cargo test --test guard_integration_test
```

Expected: all pass; no `.pending` artifact remains after rejected candidates.

- [ ] **Step 8: Commit atomic recovery emission**

```text
rtk git add src/core/tee.rs src/core/ai_output.rs src/core/runner.rs
rtk git commit -m "feat(output): add lossless AI output emission"
```

---

### Task 3: Add semantic captured-runner APIs and explicit exact reasons

**Files:**
- Modify: `src/core/runner.rs:33-286`
- Test: inline tests in `src/core/runner.rs`

**Interfaces:**
- Consumes: `AiDocument`, `BudgetClass`, `ExactReason`, `render`, `prepare_emission`, and `EmissionMeta`.
- Produces: `run_ai_filtered`, `run_ai_filtered_with_exit`, `run_passthrough_with_reason`, and behavior-compatible legacy wrappers.
- Invariant: existing `run_filtered`, `run_filtered_with_exit`, and `run_passthrough` call sites compile without modification and preserve current output.

- [ ] **Step 1: Write failing runner API compile and behavior tests**

Add tests using the existing platform-aware success/failure command helpers:

```rust
#[test]
fn run_mode_accepts_semantic_filter() {
    let mode = RunMode::AiFiltered {
        budget: BudgetClass::State,
        filter: Box::new(|text| {
            let mut doc = AiDocument::new(Some("ok"));
            doc.fact("bytes", text.len().to_string());
            Ok(doc)
        }),
    };
    assert!(matches!(mode, RunMode::AiFiltered { .. }));
}

#[test]
fn passthrough_reason_is_exposed_for_tracking() {
    assert_eq!(ExactReason::Structured.as_str(), "structured");
}
```

- [ ] **Step 2: Run the focused test and verify the missing-variant failure**

Run: `rtk test cargo test --bin rtk runner::tests::run_mode_accepts_semantic_filter`

Expected: compilation fails because `RunMode::AiFiltered` does not exist.

- [ ] **Step 3: Add semantic filter types and run modes**

Define:

```rust
pub type AiFilterResult = Result<crate::core::ai_output::AiDocument>;
pub type AiCaptureFilter<'a> = Box<dyn Fn(&str) -> AiFilterResult + 'a>;
pub type ExitAwareAiCaptureFilter<'a> = Box<dyn Fn(&str, i32) -> AiFilterResult + 'a>;

pub enum RunMode<'a> {
    Filtered(CaptureFilter<'a>),
    FilteredWithExit(ExitAwareCaptureFilter<'a>),
    AiFiltered {
        budget: BudgetClass,
        filter: AiCaptureFilter<'a>,
    },
    AiFilteredWithExit {
        budget: BudgetClass,
        filter: ExitAwareAiCaptureFilter<'a>,
    },
    Streamed(Box<dyn StreamFilter + 'a>),
    Passthrough(ExactReason),
}
```

Keep `run_passthrough` by delegating to `run_passthrough_with_reason(..., ExactReason::Unknown)`.

- [ ] **Step 4: Add parser-failure document tests**

Test that `AiDocument::parse_failure(raw, error)` emits `filter=parse-failed`, retains bounded head/tail records, marks omitted line counts, and never embeds the complete large raw string.

```rust
#[test]
fn parse_failure_document_is_bounded_and_recoverable() {
    let raw = (0..500).map(|n| format!("line-{n}")).collect::<Vec<_>>().join("\n");
    let doc = AiDocument::parse_failure(&raw, "unexpected table");
    let rendered = render(&doc, BudgetClass::Diagnostic);
    assert!(rendered.text.starts_with("status=error filter=parse-failed"));
    assert!(rendered.parser_failed);
    assert!(rendered.omission.as_ref().is_some_and(|o| o.items >= 480));
    assert!(!rendered.text.contains("line-250"));
}
```

- [ ] **Step 5: Implement semantic captured execution**

Implement the parse-failure constructor before wiring the runner:

```rust
pub fn parse_failure(raw: &str, error: &str) -> Self {
    const EDGE_LINES: usize = 10;
    let lines: Vec<&str> = raw.lines().collect();
    let mut doc = Self::new(Some("error"));
    doc.fact("filter", "parse-failed");
    doc.fact("detail", error.split_whitespace().collect::<Vec<_>>().join("_"));
    doc.parser_failed = true;

    if lines.len() <= EDGE_LINES * 2 {
        for line in lines {
            doc.push(AiRecord::new(Severity::Error, line));
        }
        return doc;
    }
    for line in &lines[..EDGE_LINES] {
        doc.push(AiRecord::new(Severity::Error, *line));
    }
    for line in &lines[lines.len() - EDGE_LINES..] {
        doc.push(AiRecord::new(Severity::Error, *line));
    }
    doc.with_omission(Omission {
        items: lines.len() - EDGE_LINES * 2,
        groups: 0,
    })
}
```

Refactor captured execution around a private document-producing function. Legacy closures become `AiDocument::legacy(filtered_string)`. Semantic closure errors become `AiDocument::parse_failure(text_to_filter, &error.to_string())`; they do not escape as RTK process errors after the child has run.

Render semantic documents with their declared budget, call `prepare_emission`, print only through `emit_prepared`, and pass `EmissionMeta` to tracking. Preserve `skip_filter_on_failure`, `filter_stdout_only`, inherited stdin, and `no_trailing_newline` behavior.

- [ ] **Step 6: Add public semantic wrapper functions**

```rust
pub fn run_ai_filtered<F>(
    cmd: Command,
    tool_name: &str,
    args_display: &str,
    budget: BudgetClass,
    filter_fn: F,
    opts: RunOptions<'_>,
) -> Result<i32>
where
    F: Fn(&str) -> AiFilterResult,
{
    run(
        cmd,
        tool_name,
        args_display,
        RunMode::AiFiltered { budget, filter: Box::new(filter_fn) },
        opts,
    )
}

pub fn run_passthrough_with_reason(
    tool: &str,
    args: &[std::ffi::OsString],
    verbose: u8,
    reason: ExactReason,
) -> Result<i32> {
    let mut cmd = crate::core::utils::resolved_command(tool);
    cmd.args(args);
    run(
        cmd,
        tool,
        &tracking::args_display(args),
        RunMode::Passthrough(reason),
        RunOptions::default(),
    )
}
```

Implement the exit-aware sibling with the same argument order.

- [ ] **Step 7: Run runner and representative legacy-filter tests**

Run:

```text
rtk test cargo test --bin rtk runner::tests
rtk test cargo test --bin rtk gh_cmd
rtk test cargo test --bin rtk search
rtk test cargo test --bin rtk find_cmd
```

Expected: new semantic tests pass and existing formatter output remains unchanged.

- [ ] **Step 8: Commit runner APIs**

```text
rtk git add src/core/runner.rs src/core/ai_output.rs
rtk git commit -m "feat(output): add semantic runner contracts"
```

---

### Task 4: Route TOML lossiness through the shared emitter

**Files:**
- Modify: `src/core/toml_filter.rs:500-646`
- Modify: `src/main.rs:1403-1477`
- Test: inline tests in `src/core/toml_filter.rs`

**Interfaces:**
- Consumes: `AiDocument::legacy`, `Omission`, `BudgetClass::Collection`, `render`, and `prepare_emission`.
- Produces: exact line-loss counts on every `Lossiness` variant and shared TOML emission.
- Invariant: TOML filter text stays unchanged; only recovery grammar and tracking metadata move to the shared foundation.

- [ ] **Step 1: Write failing exact-loss-count tests**

Replace tuple-only assertions with exact metadata assertions:

```rust
#[test]
fn max_lines_reports_exact_omitted_lines() {
    let filter = first_filter(
        r#"
schema_version = 1
[filters.sample]
match_command = "^sample"
max_lines = 2
"#,
    );
    let (text, loss) = apply_filter_with_info(&filter, "a\nb\nc\nd\ne");
    assert_eq!(text, "a\nb");
    assert_eq!(
        loss,
        Lossiness::Whole {
            omitted_items: 3,
            omitted_groups: 0,
        }
    );
}
```

Add corresponding tests for head, tail, replacement-only loss, stripped lines, per-line truncation, and `Lossiness::None`.

- [ ] **Step 2: Run the focused test and verify the enum-shape failure**

Run: `rtk test cargo test --bin rtk toml_filter::tests::max_lines_reports_exact_omitted_lines`

Expected: compilation fails because `Lossiness::Whole` has no count fields.

- [ ] **Step 3: Extend lossiness metadata without changing filtered text**

Use:

```rust
#[derive(Debug, PartialEq)]
pub enum Lossiness {
    None,
    Tail {
        tee_payload: String,
        tail_offset: usize,
        omitted_items: usize,
        omitted_groups: usize,
    },
    Whole {
        omitted_items: usize,
        omitted_groups: usize,
    },
}
```

Track removed lines during strip/keep/head/tail/max stages and changed lines during replacement or per-line truncation. A line changed by multiple stages counts once. Keep `omitted_groups=0` because TOML has no semantic grouping model.

- [ ] **Step 4: Write a failing TOML document-adapter regression test**

Add a pure helper in `main.rs` named `toml_document(filtered, &loss)` and test its text and exact omission metadata without touching the process environment:

```rust
#[test]
fn lossy_toml_document_preserves_text_and_exact_counts() {
    let loss = Lossiness::Whole {
        omitted_items: 299,
        omitted_groups: 0,
    };
    let rendered = crate::core::ai_output::render(
        &toml_document("line", &loss),
        crate::core::ai_output::BudgetClass::Collection,
    );
    assert_eq!(rendered.text, "line");
    assert_eq!(
        rendered.omission,
        Some(crate::core::ai_output::Omission {
            items: 299,
            groups: 0,
        })
    );
}
```

- [ ] **Step 5: Implement the TOML document adapter and replace manual tee branching**

Implement the adapter without altering the filtered text:

```rust
fn toml_document(filtered: &str, loss: &core::toml_filter::Lossiness) -> core::ai_output::AiDocument {
    let document = core::ai_output::AiDocument::legacy(filtered);
    match loss {
        core::toml_filter::Lossiness::None => document,
        core::toml_filter::Lossiness::Tail {
            omitted_items,
            omitted_groups,
            ..
        }
        | core::toml_filter::Lossiness::Whole {
            omitted_items,
            omitted_groups,
        } => document.with_omission(core::ai_output::Omission {
            items: *omitted_items,
            groups: *omitted_groups,
        }),
    }
}
```

The `main.rs` execution path calls `render(..., BudgetClass::Collection)` and `prepare_emission` with the raw command output and command label.

In `main.rs`, replace the manual `tee_and_hint`, `force_tee_tail_hint`, `force_tee_hint`, and `emit_guarded` decision tree with this helper plus `runner::emit_prepared`. Keep existing child execution, stderr capture selection, exit-code extraction, and parse-failure recording.

- [ ] **Step 6: Run TOML and verification tests**

Run:

```text
rtk test cargo test --bin rtk toml_filter
rtk test cargo test --bin rtk test_builtin_all_filters_have_inline_tests
rtk cargo run --quiet -- verify --require-all
```

Expected: every TOML inline fixture remains text-compatible, all omission tests pass, and verification remains green.

- [ ] **Step 7: Commit TOML shared emission**

```text
rtk git add src/core/toml_filter.rs src/main.rs
rtk git commit -m "refactor(output): route TOML filters through AI emitter"
```

---

### Task 5: Track contracts, residual tokens, omissions, failures, and recovery

**Files:**
- Modify: `src/core/tracking.rs:95-134, 275-447, 559-674, 1326-1435`
- Test: inline tests in `src/core/tracking.rs`

**Interfaces:**
- Consumes: `OutputContract`, `ExactReason`, and `EmissionMeta` labels supplied by the runner.
- Produces: `OutputTracking`, `ResidualCommandStats`, `Tracker::record_with_output`, `Tracker::get_residual_by_command`, `TimedExecution::track_output`, and `TimedExecution::track_exact`.
- Invariant: existing databases migrate additively and existing `record`, `track`, and `track_passthrough` calls retain their behavior through legacy defaults.

- [ ] **Step 1: Refactor schema initialization so production and file-backed tests share it**

Extract the schema statements currently embedded in `Tracker::new` and the test-only `init_schema` into one private production method:

```rust
impl Tracker {
    fn from_connection(conn: Connection) -> Result<Self> {
        let tracker = Self { conn };
        tracker.init_schema()?;
        Ok(tracker)
    }

    #[cfg(test)]
    fn new_at_path_for_test(path: &std::path::Path) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }
}
```

Remove `#[cfg(test)]` from the existing `init_schema`, then move the current production command-table creation, indexes, additive migrations, `parse_failures` table, and permission tightening into that method without changing their SQL. Make `Tracker::new` and `Tracker::new_in_memory` call `from_connection`. Run `rtk test cargo test --bin rtk tracking::tests` and require all existing tracking tests to pass before changing the schema.

- [ ] **Step 2: Write failing additive-schema migration tests**

Extend the in-memory schema test to assert these columns in `PRAGMA table_info(commands)`:

```text
output_contract TEXT NOT NULL DEFAULT 'legacy'
exact_reason TEXT
omitted_items INTEGER NOT NULL DEFAULT 0
omitted_groups INTEGER NOT NULL DEFAULT 0
recovery_created INTEGER NOT NULL DEFAULT 0
filter_failed INTEGER NOT NULL DEFAULT 0
```

Add a file-backed migration fixture that creates the pre-feature schema with `rusqlite::Connection`, drops that connection, opens the path through `Tracker::new_at_path_for_test`, and verifies all six columns are added without losing its existing row.

- [ ] **Step 3: Run the migration test and verify missing-column failures**

Run: `rtk test cargo test --bin rtk tracking::tests::migrates_output_tracking_columns`

Expected: the column assertions fail.

- [ ] **Step 4: Implement schema additions and tracking metadata**

Add idempotent `ALTER TABLE` statements to the production setup and include columns directly in the in-memory schema. Define:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputTracking {
    pub contract: String,
    pub exact_reason: Option<String>,
    pub omitted_items: usize,
    pub omitted_groups: usize,
    pub recovery_created: bool,
    pub filter_failed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResidualCommandStats {
    pub rtk_cmd: String,
    pub count: usize,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub weighted_savings_pct: f64,
    pub zero_savings_count: usize,
    pub exact_count: usize,
}
```

Implement `record_with_output` with the existing insert plus the six new fields. Keep `record` as a wrapper using `OutputTracking { contract: "legacy".into(), ..Default::default() }`.

- [ ] **Step 5: Write failing residual-ranking tests**

```rust
#[test]
fn residual_stats_rank_by_total_output_tokens() {
    let tracker = Tracker::new_in_memory().unwrap();
    tracker.record("small", "rtk small", 1_000, 10, 1).unwrap();
    tracker.record("large", "rtk large", 1_000, 400, 1).unwrap();
    tracker.record("large", "rtk large", 1_000, 400, 1).unwrap();

    let stats = tracker.get_residual_by_command(None).unwrap();
    assert_eq!(stats[0].rtk_cmd, "rtk large");
    assert_eq!(stats[0].output_tokens, 800);
    assert_eq!(stats[0].weighted_savings_pct, 60.0);
}
```

Add tests that exact rows are counted separately, filtered rows compute weighted savings from sums, and legacy zero-token passthrough rows do not masquerade as 100% savings.

- [ ] **Step 6: Implement residual statistics query**

Aggregate `SUM(input_tokens)`, `SUM(output_tokens)`, zero-savings rows, and exact rows by `rtk_cmd`; order by `SUM(output_tokens) DESC`. Compute weighted percentage from aggregate input and output in Rust. Do not use `AVG(savings_pct)` for this API.

- [ ] **Step 7: Connect runner emission metadata to tracking**

Add:

```rust
pub fn track_output(
    &self,
    original_cmd: &str,
    rtk_cmd: &str,
    input: &str,
    output: &str,
    tracking: OutputTracking,
)

pub fn track_exact(
    &self,
    original_cmd: &str,
    rtk_cmd: &str,
    reason: &str,
)
```

`run_captured_filter` maps `PreparedEmission::meta()` into `OutputTracking`. `RunMode::Passthrough(reason)` calls `track_exact`. Keep `track` and `track_passthrough` wrappers for unmigrated custom paths.

- [ ] **Step 8: Run tracking and gain regression tests**

Run:

```text
rtk test cargo test --bin rtk tracking::tests
rtk test cargo test --bin rtk gain
```

Expected: schema, weighted residual, historical summary, and export tests pass.

- [ ] **Step 9: Commit output tracking**

```text
rtk git add src/core/tracking.rs src/core/runner.rs
rtk git commit -m "feat(analytics): track residual AI output"
```

---

### Task 6: Add a checked legacy-output inventory and foundation verification

**Files:**
- Create: `tests/fixtures/ai_output_legacy_stdout_paths.txt`
- Create: `tests/ai_output_contract_test.rs`
- Modify: `src/hooks/verify_cmd.rs:1-49`
- Test: `tests/ai_output_contract_test.rs` and inline tests in `src/hooks/verify_cmd.rs`

**Interfaces:**
- Consumes: repository source files and the explicit legacy inventory.
- Produces: a monotonic migration guard and `verify` summary field `ai_output_legacy_paths=N` when running from a source checkout.
- Invariant: foundation verification prevents new direct command-module stdout paths but does not claim final universal enforcement.

- [ ] **Step 1: Generate and review the initial legacy stdout inventory**

Run:

```text
rtk rg -l "println!|print!" src/cmds
```

Write the sorted repository-relative `.rs` paths that contain production stdout writes to `tests/fixtures/ai_output_legacy_stdout_paths.txt`. Exclude `src/cmds/README.md`. Keep files whose test modules also contain prints if production code in the same file writes stdout; the integration test performs a second content check.

- [ ] **Step 2: Write the failing source-inventory integration test**

Create `tests/ai_output_contract_test.rs` with this scanner:

```rust
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
        let without_stderr = line
            .replace("eprintln!(", "")
            .replace("eprint!(", "");
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
```

The scanner:

1. recursively visits `.rs` files under `src/cmds` using `std::fs`;
2. ignores text after the first line equal to `#[cfg(test)]` only when the remainder is the module's test section;
3. detects `print!`, `println!`, `eprint!`, and `eprintln!` in production text;
4. permits stderr-only verbose/internal diagnostics;
5. collects files containing production `print!` or `println!`; and
6. asserts exact equality with the checked-in sorted inventory.

The assertion message must print `added=` and `removed=` path lists. A removed path fails deliberately so the migration commit must update the inventory and demonstrate progress.

- [ ] **Step 3: Run the inventory test and verify the fixture mismatch**

Run: `rtk test cargo test --test ai_output_contract_test -- --nocapture`

Expected: failure showing the complete `added=` set until the fixture matches the reviewed baseline.

- [ ] **Step 4: Finalize the reviewed fixture and pass the integration test**

Run: `rtk test cargo test --test ai_output_contract_test -- --nocapture`

Expected: pass with exact inventory equality.

- [ ] **Step 5: Add source-aware verification reporting**

In `verify_cmd.rs`, add a helper that looks for `tests/fixtures/ai_output_legacy_stdout_paths.txt` relative to the current source checkout. If present, count nonempty lines and append:

```text
ai_output_legacy_paths=N
```

Do not fail `--require-all` during foundation. If the fixture is absent in an installed binary's working directory, print nothing. The final universal-enforcement plan will require zero and make nonzero fail.

- [ ] **Step 6: Test verification inside and outside a source checkout**

Add pure tests for parsing blank/comment-free inventory content and for absent-file behavior. Then run:

```text
rtk test cargo test --bin rtk verify_cmd
rtk test cargo test --test ai_output_contract_test
rtk cargo run --quiet -- verify --require-all
```

Expected: all pass; source checkout output includes a nonzero migration count and does not claim completion.

- [ ] **Step 7: Commit the migration guard**

```text
rtk git add tests/fixtures/ai_output_legacy_stdout_paths.txt tests/ai_output_contract_test.rs src/hooks/verify_cmd.rs
rtk git commit -m "test(output): inventory legacy stdout paths"
```

---

### Task 7: Document the foundation APIs and run complete verification

**Files:**
- Modify: `src/cmds/README.md:55-120, 180-250`
- Modify: `src/filters/README.md`
- Modify: `docs/contributing/ARCHITECTURE.md`
- Modify: `docs/contributing/TECHNICAL.md:217-260`
- Modify: `docs/guide/resources/savings-explained.md`
- Test: documentation validation and complete Rust verification

**Interfaces:**
- Consumes: all foundation APIs and invariants implemented in Tasks 1-6.
- Produces: contributor guidance for semantic, legacy, and exact output routes.

- [ ] **Step 1: Update command-author documentation**

Document these exact choices in `src/cmds/README.md`:

```text
Safe semantic text -> run_ai_filtered(..., BudgetClass, parser, options)
Existing migration-only string filter -> run_filtered(...)
Structured/interactive/binary/streaming/sensitive/unknown -> run_passthrough_with_reason(..., ExactReason)
```

State that new filtered routes must not use the legacy API, direct stdout is migration debt, parser failures become bounded recoverable output, and native exit behavior remains authoritative.

- [ ] **Step 2: Update TOML and architecture documentation**

Explain that TOML remains line-oriented and migration-compatible but now reports exact loss metadata and uses the shared lossless emitter. Update architecture/data-flow diagrams to place `AiDocument`, budget rendering, lossless recovery, and output tracking between filters and stdout.

- [ ] **Step 3: Update savings documentation**

Define residual tokens and weighted savings as:

```text
residual_tokens = sum(output_tokens)
weighted_savings_pct = 100 * (1 - sum(output_tokens) / sum(input_tokens))
```

State that exact routes have unavailable captured residual size, are reported separately by reason, and do not dilute filtered efficiency. Keep the existing warning that token counts use bytes divided by four.

- [ ] **Step 4: Run formatting and documentation checks**

Run:

```text
rtk cargo fmt --all -- --check
rtk summary bash -lc "tr -d '\r' < scripts/validate-docs.sh | bash"
```

Expected: both pass. The normalized pipeline is validation-only and does not rewrite repository files.

- [ ] **Step 5: Run focused foundation verification**

Run:

```text
rtk test cargo test --bin rtk ai_output
rtk test cargo test --bin rtk tee
rtk test cargo test --bin rtk runner
rtk test cargo test --bin rtk toml_filter
rtk test cargo test --bin rtk tracking
rtk test cargo test --test ai_output_contract_test
rtk cargo run --quiet -- verify --require-all
```

Expected: all pass and verification reports the explicit nonzero legacy migration count.

- [ ] **Step 6: Run complete repository verification**

Run:

```text
rtk test cargo test --all
rtk cargo clippy --all-targets --all-features -- -D warnings
rtk cargo fmt --all -- --check
rtk git diff --check
```

Expected: all suites pass, Clippy emits no warnings, formatting is clean, and Git reports no whitespace errors.

- [ ] **Step 7: Commit foundation documentation**

```text
rtk git add src/cmds/README.md src/filters/README.md docs/contributing/ARCHITECTURE.md docs/contributing/TECHNICAL.md docs/guide/resources/savings-explained.md
rtk git commit -m "docs: define AI-first output contracts"
```

- [ ] **Step 8: Record the foundation boundary for the next plan**

Run:

```text
rtk git status --short --branch
rtk git log --oneline origin/develop..HEAD
rtk gain --all
```

Capture the clean status, commit list, residual-token baseline, and legacy stdout count in the execution handoff. The next plan is `highest-residual-command-migration`; it starts with `find`, `rg`, `read`, and `ls`, then Git and diagnostic runners.

---

## Foundation Completion Gate

The foundation is complete only when:

- semantic documents render deterministically under the five exact budgets;
- existing string filters remain output-compatible;
- lossy emission is atomic, private, complete, recoverable, and never worse;
- parser failures have a bounded recoverable representation;
- captured runners support semantic filters and exact routes record a reason;
- TOML loss metadata contains exact counts and uses the shared emitter;
- tracking stores contract, exact reason, residual, omission, failure, and recovery data;
- residual queries rank by total output tokens and compute weighted percentages from sums;
- the checked legacy inventory prevents new direct stdout debt;
- `verify --require-all` exposes migration debt without falsely claiming universal completion; and
- the full test, Clippy, formatting, documentation, and whitespace gates pass.

This plan intentionally does not migrate individual command-family formatters. Those changes belong to the next three independently reviewable plans defined by the approved design.
