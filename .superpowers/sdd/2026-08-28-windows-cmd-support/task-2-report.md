# Task 2 Report: Public CLI and CMD Orchestration

## Status

Complete. The public `rtk cmd` route now executes CMD expressions through a
resolved `cmd.exe`; a hidden `__cmd-run` route executes encoded identity
segments without recursive orchestration.

## Commit

`feat(windows): add cmd orchestration` (the containing commit)

## Files Changed

- `src/main.rs`: adds public `cmd` and hidden `__cmd-run` clap routes, dispatch,
  parsing coverage, and meta-command validation handling.
- `src/core/constants.rs`: classifies both CMD routes as RTK-only commands.
- `src/cmds/windows/mod.rs`: exposes the orchestration module.
- `src/cmds/windows/orchestrator.rs`: normalizes invocations, reconstructs
  multi-argument expressions, applies catalog/parser-safe rewriting, and runs
  both public and hidden CMD routes.
- `src/cmds/windows/tests.rs`: covers invocation normalization, source-span
  preservation, stateful pass-through, opaque fail-open, percent-expansion
  fail-open, and hidden-runner rewrite output.
- `tests/windows_cmd_e2e.rs`: native-vs-RTK Windows parity tests for chains,
  state, variables, Unicode/spaces, redirection, batch invocation, and failures.

## TDD Evidence

1. RED: `rtk test cargo test test_cmd_accepts_a_raw_cmd_expression --bin rtk`
   initially failed because Clap had no `cmd` subcommand: `0 passed; 1 failed`.
2. GREEN: the same focused command passed after adding the public route:
   `1 passed; 0 failed`.
3. RED: `rtk test cargo test --test windows_cmd_e2e` initially reported native
   parity failures for the new path (`0 passed; 3 failed`).
4. GREEN: after preserving the normal current executable path, preserving
   segment trailing whitespace, and failing open for percent expansion, the
   parity command passed: `3 passed; 0 failed`.
5. A complete-suite red check caught the required meta-command-test update:
   `2726 passed; 1 failed; 8 ignored`; its focused regression command then
   passed: `1 passed; 0 failed`.

## Verification Commands and Exact Summaries

- `rtk cargo fmt --check` — passed (exit 0).
- `rtk test cargo test "cmds::windows::tests::" --bin rtk` — `16 passed; 0 failed`.
- `rtk test cargo test --test windows_cmd_e2e` — `3 passed; 0 failed`.
- `rtk test cargo test "test_cmd_" --bin rtk` — `2 passed; 0 failed`.
- `rtk test cargo test test_meta_commands_reject_bad_flags --bin rtk` —
  `1 passed; 0 failed`.
- `rtk test cargo test --all` — main unit suite `2727 passed; 0 failed; 8
  ignored`; all remaining test targets also passed, including the Windows CMD
  suite `3 passed; 0 failed`.
- `rtk git diff --check` — passed (exit 0).

## Self-Review

- The Task 1 parser is the only syntax classifier. Any `opaque_reason` returns
  the original expression without even partial rewriting.
- Query built-ins are the only rewritten catalog class. Mutation, stateful,
  control, interactive, and unknown commands stay in the parent CMD process.
- Percent-expansion expressions also fail open. This prevents child CMD from
  changing expansion timing after an earlier `set` command.
- Hidden runner payloads are hexadecimal UTF-8 source, so rewritten execution
  does not expose original metacharacters, spaces, quotes, or variables to the
  parent CMD parser a second time.
- The public compound route does not record savings. The hidden runner is the
  future sole accounting boundary, so Task 2 cannot double-count chains.
- Both routes use `resolve_binary("cmd.exe")`; `status()` inherits stdout,
  stderr, encoding, and console handles and returns the native exit code.

## Concerns

- Structured filters and per-segment savings accounting are intentionally not
  implemented until Task 3. Hidden runners are identity execution only.
- Multi-argument public invocations are reconstructed with CMD-safe quoting;
  callers requiring exact raw CMD metacharacter or quote behavior should pass a
  single raw expression, which is preserved exactly.
