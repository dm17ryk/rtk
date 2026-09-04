# AI-first output optimization for `rg`, `read`, and `find`

**Status:** approved design, pending implementation  
**Branch:** `codex/ai-output-core-commands`  
**Base:** `origin/develop` after merge of PR #10

## Goal

Make the three highest-use RTK routes produce materially smaller, AI-readable
output by default. Token reduction is the primary success metric. Every
compact rendering must remain truthful, preserve process exit status and
stderr, disclose omissions exactly, and offer lossless recovery when RTK—not
the caller—omits source data.

The dashboard's current route percentages are baselines, not targets:
`rtk rg` 30.2%, `rtk read` 20.2%, and `rtk find` 41.6%.

## Scope and non-goals

- Optimize `rtk rg`, `rtk read`, and `rtk find`.
- Leave `rtk grep` behavior unchanged even though it shares search plumbing.
- Reuse the merged AI-output contract for budgets, recovery, and telemetry.
- Do not change release metadata or add dependencies.
- Do not silently rewrite binary/NUL, interactive, or unrecognized `rg`
  semantics. Those forms are exact until a dedicated, tested semantic renderer
  exists.

## Alternatives considered

1. **Small presentation tweaks.** Low risk but cannot materially improve the
   30.2% `rg` and 20.2% `read` baselines.
2. **Budgeted semantic renderers with lossless recovery (selected).** Each
   recognized result shape has a concise AI representation. The full output is
   recoverable whenever RTK omits data.
3. **AST summaries.** Potentially small but language-specific, costly, and too
   likely to hide the source details an agent needs.

## Shared output contract

Each renderer produces an `AiDocument` and emits it through the existing
AI-output preparation path. It reports one of two contracts:

- `AiOwned(Source)` for compact source/search records and
  `AiOwned(Collection)` for inventories.
- `Exact(reason)` for an unsupported, binary, interactive, or otherwise
  opaque shape.

The renderer preserves the original exit code and writes native stderr once.
It uses the raw stdout as the lossless artifact whenever the output budget
causes RTK to omit records, groups, or suffixes. The visible result states
`omitted items=<n> groups=<n> recover=rtk read -l none --recovery <id>`.
`prepare_emission` keeps the raw output whenever the proposed AI rendering is
not shorter, so compact formatting cannot make a command worse.

## `rtk rg`

`rg` receives a route classifier separate from the existing shared `grep`
flow. It recognizes every currently supported, text-bearing output shape:

| Native shape | AI rendering |
|---|---|
| ordinary matches, context, replacement output | first-seen file blocks with `path`, line markers, match/context distinction, and pattern-focused snippets |
| `--json` event stream | the same semantic match/context records, without begin/end JSON-event noise |
| `--files`, `-l`, `-L` | compact path inventory |
| `-c`, `--count-matches` | `path=count` records plus a total where available |
| `-o`, `--only-matching` | file and line groups containing exact matched values |

The match renderer preserves first-seen file and line order, rather than
sorting or inventing an order. It emits file blocks incrementally and applies
per-record and total budgets. Long content is trimmed around the match when the
match is known; the complete native stdout is recoverable.

`--help`, `--version`, NUL/binary output, streaming/interactive forms, and
unknown flags are exact passthrough. This is a conservative boundary, not a
token-saving exemption: telemetry identifies such routes as `Exact`, and a
future route may graduate only with its own parser and contract tests.

## `rtk read`

The default becomes the existing minimal language-aware filtering level,
rendered as compact source records with dense line markers. This makes
declarations, imports, and retained source lines directly addressable by an
agent without repeatedly printing a wide line-number column.

Minimal filtering must preserve every retained line's original one-based line
number. Add a line-preserving filter result in `core::filter`; retain the
existing string-returning filter API as a compatibility wrapper. RTK must
never present a filtered line with a renumbered location.

- `rtk read -l none` remains an explicit complete source read.
- User-selected `-m` and `--tail-lines` retain their requested windowing
  semantics.
- Default filtering, long-line clipping, and budget-driven omissions are
  disclosed and recoverable from the original file content.
- If the compact rendering is not smaller, RTK emits the exact source instead.

This avoids pretending that a lossy source summary is complete while allowing
the high-frequency default to save tokens.

## `rtk find`

Extract a shared compact path-inventory renderer. `find` supplies the current
stable list of paths and receives output such as:

```text
files=80 dirs=12 root=src
cmds/system/{read.rs,search.rs}
core/{runner.rs,tracking.rs}
```

The renderer uses a common root only when it is unambiguous, lists relative
directory groups in stable order, and retains exact file and directory counts.
Disjoint roots stay explicit. It does not alter the current grammar classifier:
unsupported expression forms continue to run exactly.

## Testing and measurement

1. Table-driven route-classifier tests cover every recognized `rg` shape and
   exact fallback boundary, including JSON, files, counts, only-matching,
   replacement, binary/NUL, help/version, interactive, and unknown flags.
2. Unit tests validate deterministic semantic rendering, first-seen ordering,
   path-root elision, line markers, exact omission counts, and never-worse
   output.
3. CLI integration tests validate exit codes, stderr, recovery commands, and
   round-trip recovery contents for all three routes.
4. Tracking tests assert the correct AI/exact contract, omission metadata, and
   recovery state using isolated databases.
5. Representative fixtures compare UTF-8 output byte/token estimates before
   and after each renderer. The implementation reports measured improvements;
   it does not claim targets before measurements exist.

## Acceptance criteria

- The default outputs of all three commands are compact and AI-readable.
- Each recognized `rg` output mode is optimized, not merely default matches.
- No output is silently omitted; recovery is valid whenever RTK omits data.
- `rtk grep` has unchanged behavior.
- Full tests, lint, formatting, verification, and documentation validation
  pass from this branch.
