# Universal AI-First Output Design

## Summary

RTK output is an interface for AI agents, not a replacement terminal UI for humans. Humans who want native presentation run the underlying command directly. RTK therefore optimizes every owned output path for minimum residual tokens while preserving actionable facts, native exit behavior, and exact recovery.

Every RTK route receives an explicit output contract. Safe human-readable output is converted to a compact semantic document and rendered under a shared budget. Structured, binary, interactive, sensitive, genuinely streaming, and otherwise unsafe output uses exact passthrough. Exact passthrough is a supported contract decision, not an unsupported command.

The final system covers Rust filters, TOML filters, streaming filters, custom command runners, RTK meta-commands, and future routes. Compatibility adapters permit staged migration, but RTK must not claim universal AI-output coverage until strict verification proves that every route has a contract and every owned output path uses the shared emitter.

## Goals

- Make token-efficient, deterministic, AI-readable text the default for every RTK-owned output path.
- Minimize residual output tokens rather than optimize terminal aesthetics.
- Preserve actionable facts, locations, failures, state changes, exit codes, signals, and cancellation.
- Bound large outputs by category and make every omission explicit and exactly recoverable.
- Treat stdout and stderr as one semantic input for AI-owned routes while preserving their native separation for exact routes.
- Measure weighted savings and total residual tokens so high-volume, high-output commands receive priority.
- Make output-contract coverage mechanically verifiable.

## Non-goals

- Reproduce native human-oriented formatting inside RTK.
- Reformat explicitly requested JSON, templates, binary data, downloads, or interactive sessions.
- Invent one generic text compressor that guesses the meaning of arbitrary output.
- Add AI inference, a tokenizer dependency, or a network service.
- Change release or version metadata without a separate request.

## Product Contract

Each route declares one of these contracts:

```text
AiOwned(Acknowledgement | State | Collection | Diagnostic | Source)
Exact(Structured | Interactive | Binary | Streaming | Unknown | Sensitive)
```

`AiOwned` means RTK may parse, group, deduplicate, prioritize, and omit recoverable repetition. `Exact` means RTK preserves native arguments, bytes, streams, interaction, and exit behavior. Unknown commands and flags default to `Exact(Unknown)` until their semantics are proven safe.

Noninteractive mutations may use `AiOwned(Acknowledgement)` when their meaningful result can be preserved. Authentication flows, prompts, editors, browser flows, downloads, machine formats, and opaque future behavior remain exact.

## Architecture

### Semantic document

Command-aware parsers produce an `AiDocument` rather than final presentation text. The model contains:

- outcome and summary facts;
- findings, failures, warnings, and successful records;
- source locations and grouping keys;
- status and count fields;
- omitted item and group counts;
- completeness state;
- recovery metadata; and
- exit metadata.

The model is intentionally small. Domain-specific parsing remains in command modules; the core does not attempt to understand Git, compiler, test, filesystem, or cloud semantics.

Existing string filters can initially return a legacy document through a compatibility adapter. The adapter receives shared guarding and recovery behavior but cannot claim strict semantic coverage until migrated.

### Shared renderer and emitter

A central renderer converts `AiDocument` into deterministic line-oriented text under the route's budget. A central emitter then:

1. validates budget and completeness metadata;
2. creates a raw-output recovery artifact when information was omitted;
3. applies the never-worse guard;
4. writes the final output;
5. preserves the native exit result; and
6. tracks raw, rendered, omitted, recovery, and route-contract metrics.

The existing captured, streaming, TOML, and custom runners converge on this emitter. Direct command-module writes to stdout become prohibited except for audited exact paths.

### Data flow

```text
route classifier
  |-- Exact(reason) -> native passthrough -> contract tracking
  `-- AiOwned(class)
        -> capture semantic input
        -> command-aware parser
        -> AiDocument
        -> budget-aware renderer
        -> never-worse/recovery emitter
        -> token tracking and native exit result
```

Finite commands may be buffered because output quality and token savings take priority over immediate display. Interactive, watch, tail, and genuinely continuous operations remain exact. A finite line-oriented runner may aggregate events and reserve space for late failures and its final summary.

## Output Grammar

Default AI output is compact ASCII text. It is not JSON and does not use Markdown tables, banners, icons, colors, decorative prose, or repeated headings.

```text
status=fail errors=3 warnings=7 passed=124 duration=8.2s
src/lib.rs:42:17 E0308 expected=u32 actual=String
tests/api.rs:91 FAIL login_expired expected=401 actual=200
E0308 repeats=11 files=4
omitted items=184 groups=9 recover=rtk read -l none "<raw-path>"
```

Rules:

- The first line contains outcome and aggregate facts when a summary is useful.
- Records use stable `location code key=value` syntax.
- Errors and actionable findings precede warnings and successes.
- Repeated diagnostics are represented once with exact counts.
- Paths, package names, and other grouping context are printed once when that saves tokens.
- Ordering is deterministic: severity, semantic group, then native/source order.
- A summary does not restate information already obvious from emitted records.
- Empty native output remains empty; RTK does not expand silence into a success sentence.
- Explicit native structured formats remain exact.

## Output Budgets

RTK continues to estimate tokens using its existing byte-based estimator. Each AI-owned route selects one shared budget class:

| Class | Default limit | Typical routes |
|---|---:|---|
| Acknowledgement | 128 tokens | add, commit, install success |
| State | 512 tokens | status, environment, repository state |
| Collection | 1,024 tokens | find, rg, ls, dependency lists |
| Diagnostic | 2,048 tokens | tests, builds, linters, checks |
| Source | 4,096 tokens | read, diff, finite logs requiring context |

Output below the limit remains semantically complete. Above it, the renderer retains information in this order:

1. failures and root causes;
2. locations and actionable context;
3. counts and state changes;
4. distinct warnings;
5. representative repeated records; and
6. routine successes and progress.

Every omitted record contributes to exact item and group counts. A lossy document includes a compact recovery footer and raw artifact. Exact routes are exempt from budgets because altering them would violate their contract.

The renderer must never emit more estimated tokens than the native semantic input. A recovery footer is part of the guarded rendered output.

## Failure and Recovery Semantics

Native exit codes, signals, cancellation, and command-start failures are preserved.

- Parser success produces the normal semantic document.
- Parser failure emits `filter=parse-failed`, bounded raw head/tail context, and a full-output recovery path instead of dumping unlimited raw output.
- Renderer failure follows the same bounded recovery path.
- Lossy output stores exact raw output only when recovery is needed.
- Exact, authentication, API, binary, and known-sensitive routes are not automatically copied beyond existing native behavior.

Recovery artifacts use RTK's private tee directory, restricted user-only permissions, and bounded retention. The footer provides a directly executable RTK recovery command. Recovery generation must not change the child command's exit result.

For AI-owned routes, stdout and stderr may be captured together and represented semantically. RTK's own diagnostics are compact and emitted separately only when they are actionable. Exact routes preserve native stdout/stderr separation.

## Analytics

The primary optimization metric is residual output delivered to the AI.

RTK reports:

- total residual output tokens by route and family;
- weighted savings: `1 - sum(output) / sum(input)`;
- p50 and p95 residual tokens;
- zero-savings rate;
- exact-passthrough rate and reason;
- parse and renderer failure rates; and
- recovery artifact creation and use.

Opportunity ranking uses total residual tokens, not unweighted average savings per invocation. Exact routes remain supported and are reported separately at 0% savings rather than diluting filtered-efficiency measurements.

The representative eligible-output corpus targets at least 70% overall weighted savings. A command family may fall below that target when native output is already smaller than its budget, but it must not materially regress without a correctness justification.

## Coverage Enforcement

`rtk verify --require-all` becomes the universal contract gate. It fails when:

- a registered route lacks an output contract;
- an AI-owned route bypasses the shared emitter;
- lossy output lacks omission and recovery metadata;
- a TOML filter lacks lossiness classification;
- an unapproved command-module stdout write exists; or
- an AI-owned route lacks representative fixtures.

A source audit permits output only through approved core sinks and explicitly enumerated exact paths. New and future routes must choose a contract before verification passes.

## Testing

### Unit and property tests

- deterministic rendering and stable ordering;
- grammar and golden snapshots;
- never-worse output;
- class-budget compliance;
- priority selection, grouping, and deduplication;
- exact omitted item/group counts;
- parser and renderer failure recovery;
- recovery footer guarding;
- exit code and signal preservation; and
- raw-artifact permission and retention behavior.

### Route tests

- structured, binary, interactive, sensitive, watch, and streaming exact modes;
- noninteractive AI-owned modes;
- unknown commands and flags;
- stdout/stderr behavior;
- non-UTF-8 inputs and outputs; and
- Windows and Unix argument and path rendering.

### Corpus and integration tests

- weighted savings and residual-token benchmarks by family;
- p50/p95 budget checks;
- no material per-family regressions;
- full Rust test suite, formatting, and strict Clippy;
- `rtk verify --require-all`; and
- documentation and generated agent-policy validation.

## Delivery

The final contract spans too many independent output paths for one safe implementation PR. Delivery is staged, but the product must not claim universal completion before the final enforcement phase.

1. **Foundation**: semantic model, renderer, budgets, recovery, route contracts, analytics, verification hooks, and legacy adapters.
2. **Highest residual-token commands**: find, rg, read, ls, Git, logs, tests, builds, and linters.
3. **Complete migration**: remaining Rust, streaming, TOML, mutation, and RTK meta-command output paths.
4. **Universal enforcement**: remove compatibility bypasses, enable strict source/route verification, update user and contributor documentation, regenerate agent policy, and publish corpus measurements.

Each phase must preserve exact-mode semantics and improve or hold weighted residual-token performance. The foundation and migration boundaries should support focused review and independent rollback.

## Acceptance Criteria

The feature is complete when:

- every RTK route has an explicit `AiOwned` or `Exact` contract;
- every AI-owned path uses the shared emitter;
- every exact path records a reason and preserves native semantics;
- all AI-owned output follows the common grammar and budget policy;
- every omission and parser failure is recoverable;
- analytics prioritize total residual tokens and report weighted savings;
- the representative corpus reaches the overall target without correctness regressions;
- strict universal verification passes on Windows and Unix; and
- documentation clearly states that RTK output is for AI agents while humans use native commands.
