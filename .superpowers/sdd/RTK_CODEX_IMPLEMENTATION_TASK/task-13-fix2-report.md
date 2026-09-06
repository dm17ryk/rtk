# Task 13 fix-2 report

Status: **DONE_WITH_CONCERNS**

Fix-2 commit `84dbc3e4771e3e6290a4f3dafe01324cc87fd4d` closes the re-review
false-positive: structured evidence must now be an exact direct invocation of
the expected RTK command. A command that merely starts with `rtk git status`
and adds redirection, a control operator, or another shell token is not evidence
of RTK execution. Existing valid Codex JSONL and Claude stream-JSON evidence
remain accepted.

## Commits and changed files

- Branch: `codex/rtk-codex-implementation`
- Parent: `c024d6e docs: record Task 13 final handoff`
- Fix-2 commit: `84dbc3e4771e3e6290a4f3dafe01324cc87fd4d`
- Files changed by fix-2:
  - `scripts/verify-agent-coverage.py`
  - `scripts/test_verify_agent_coverage.py`
- CI and documentation wording required no additional edit: the prior fix
  already uses `--evidence-format`, exact `--expect-rtk-command`,
  `--require-verified`, and explicitly documents direct command/result evidence.
- No push, release, merge, PR, subagent, or reviewer was performed.
- Existing unrelated untracked `scripts/__pycache__/` and `tmp/` were preserved.

## Finding and implementation

Before fix-2, `command_matches()` compared only the expected-token prefix. The
structured Codex event
`rtk git status < /dev/null/rtk-review-input || true` therefore matched
`rtk git status` and could report verification after the shell itself returned
zero. Fix-2 requires the parsed token sequence to equal the expected direct
RTK invocation exactly. A compound event is rejected; if a separate successful
direct event exists, that independent event can still establish evidence.

The new deterministic regression is
`test_compound_rtk_prefix_is_not_verified_without_direct_execution`. It uses
the exact redirection/control shape, expects `live_verification = unverified`,
and requires the strict gate to return 1. The existing valid Codex and Claude
structured-evidence tests remain in the same suite and pass.

## TDD evidence

All commands ran from `D:\src\rtk`.

| Command | Exit | Actual result |
|---|---:|---|
| `python scripts/test_verify_agent_coverage.py` before production edit | 0 from the local wrapper | RED: 9 tests ran; the new compound-command test failed because the verifier returned 0 instead of the expected strict-gate failure. The failure body was present; this machine's Python/RTK hook wrapper did not propagate the failing unittest status. |
| `rtk test python scripts/test_verify_agent_coverage.py` after production edit | 0 | 9 tests passed; `OK`. |
| `py -3 scripts/test_verify_agent_coverage.py` after production edit | 0 | 9 tests passed; `OK`. This confirmed the real launcher result while preserving valid Codex and Claude cases. |

## Final gates

| Command | Exit | Actual result |
|---|---:|---|
| `rtk cargo fmt --all -- --check` | 0 | Returned 0 but emitted a local `Import-Clixml` diagnostic from the RTK PowerShell output filter. |
| `cargo fmt --all -- --check` | 0 | Passed with no output; used as the authoritative formatter result. |
| `rtk cargo clippy --all-targets` | 0 | Passed; Cargo finished the dev profile without warnings. |
| `$env:RUST_MIN_STACK = '8388608'; rtk test cargo test --all` | 0 | All 27 reported test binaries passed: 3471 passed, 0 failed, 8 ignored. The main binary reported 3372 passed and 8 ignored. |
| `target\\debug\\rtk.exe verify --require-all` | 0 | 158/158 filter tests passed; `ai_output_legacy_paths=26`. Warning: untrusted project filters were skipped. |
| `rtk summary bash scripts/validate-docs.sh` | 0 | Documentation validation passed. It retained manual warnings for possible missing `ruff`, `pytest`, `go`, and `golangci` hook rewrites. |
| `rtk git diff --check` | 0 | No failing whitespace check was reported. |

## Remaining concerns and unsupported evidence

- Opt-in live Codex and Claude jobs were not run locally. They are configured
  by the prior fix to require structured events and exact `rtk git status`
  evidence, but paid-host behavior and real event schemas remain unverified
  until an authorized workflow run with credentials.
- No live Codex child/resume/follow-up, nested child, Claude subagent, Ruflo
  worker, or SDK worker was started. Fixture tests are not live-host evidence.
- The requested model/effort and host-observed model/effort remain separate;
  no independent host-introspection record was created in this fix round.
- The filter gate skipped untrusted project filters, and documentation
  validation emitted its four manual hook-coverage warnings; neither was
  converted into a false pass claim.
- No active-profile install, migration, uninstall, trust mutation, or unrelated
  configuration write was performed.

The fix-2 implementation is committed and locally validated. It was not pushed,
merged, released, or submitted as a PR.
