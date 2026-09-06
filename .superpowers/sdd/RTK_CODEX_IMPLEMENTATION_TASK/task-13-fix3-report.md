# Task 13 fix-3 report

Status: **DONE_WITH_CONCERNS**

Fix-3 commit `e714d73f139f0d89390a943aba5494651a399642` closes the third
wrapper-bypass finding. The verifier now validates a complete recognized shell
wrapper before unwrapping it. `bash/sh/zsh -c/-lc 'rtk git status'` remains an
accepted equivalent form, but any outer redirection, control operator, extra
argument, or other compound token invalidates the wrapper. Existing direct
Codex JSONL and Claude stream-JSON evidence remains accepted.

## Commit and changed files

- Branch: `codex/rtk-codex-implementation`
- Parent: `21882b0 docs: record Task 13 fix-2 validation`
- Fix-3 implementation: `e714d73f139f0d89390a943aba5494651a399642`
- Commit subject: `fix: validate complete shell evidence wrappers`
- Changed by fix-3:
  - `scripts/verify-agent-coverage.py`
  - `scripts/test_verify_agent_coverage.py`
- No CI or documentation wording change was needed; the prior commits already
  require exact direct RTK evidence and describe the evidence states accurately.
- No push, release, merge, PR, subagent, or reviewer was performed.
- Existing unrelated untracked `scripts/__pycache__/` and `tmp/` were preserved.

## Finding and implementation

Before fix-3, `command_tokens()` saw `bash -lc 'rtk git status'`, discarded all
outer tokens, and recursively matched only the inner command. Consequently
`bash -lc 'rtk git status' < /dev/null/rtk-review-input || true` could look like
a successful direct RTK event even when the shell itself short-circuited.

Fix-3 checks the outer token list first. A recognized shell wrapper is unwrapped
only when it has exactly three tokens: the recognized shell executable, `-c` or
`-lc`, and the script. The inner command is then tokenized and compared exactly
to the expected RTK invocation. A separate direct event can still independently
establish evidence when present.

## TDD evidence

All commands ran from `D:\src\rtk`.

| Command | Exit | Actual result |
|---|---:|---|
| `py -3 scripts/test_verify_agent_coverage.py` before production edit | 0 from the local launcher wrapper | RED: 13 tests ran; `test_codex_outer_shell_compound_is_not_verified` and `test_claude_outer_shell_compound_is_not_verified` failed because both false-positive expressions were accepted. Safe wrapper and direct controls passed. The local RTK/Python hook did not propagate the failing unittest status. |
| `rtk test python scripts/test_verify_agent_coverage.py` after production edit | 0 | 13 tests passed; `OK`. |
| `py -3 scripts/test_verify_agent_coverage.py` after production edit | 0 | 13 tests passed; `OK`. This preserved direct Codex/Claude evidence and safe wrapper acceptance while rejecting both outer compounds. |

## Final gates

| Command | Exit | Actual result |
|---|---:|---|
| `rtk cargo fmt --all -- --check` | 0 | Passed with no output. |
| `cargo fmt --all -- --check` | 0 | Passed with no output. |
| `rtk cargo clippy --all-targets` | 0 | Passed; Cargo finished the dev profile without warnings. |
| `$env:RUST_MIN_STACK = '8388608'; rtk test cargo test --all` | 0 | All 27 reported test binaries passed: 3471 passed, 0 failed, 8 ignored. The main binary reported 3372 passed and 8 ignored. |
| `target\\debug\\rtk.exe verify --require-all` | 0 | 158/158 filter tests passed; `ai_output_legacy_paths=26`. Warning: untrusted project filters were skipped. |
| `rtk summary bash scripts/validate-docs.sh` | 0 | Documentation validation passed. It emitted manual warnings for possible missing `ruff`, `pytest`, `go`, and `golangci` hook rewrites. |
| `rtk git diff --check` | 0 | No failing whitespace check was reported. |
| `rtk git diff --cached --check` before implementation commit | 0 | No staged whitespace errors. |

## Remaining limitations

- Opt-in live Codex and Claude jobs were not run locally. Their prior CI wiring
  uses structured evidence formats, exact `rtk git status`, and
  `--require-verified`; real paid-host event output remains unverified here.
- No live child/resume/follow-up, nested child, Claude subagent, Ruflo worker,
  or SDK worker was started. Deterministic fixtures are not live-host evidence.
- Requested and host-observed model/effort remain separate; no independent host
  introspection record was created in this fix round.
- The filter gate skipped untrusted project filters, and docs validation kept
  its four manual hook-coverage warnings; neither was promoted to a false pass.
- The verifier intentionally recognizes only the existing `bash`, `sh`, and
  `zsh` `-c`/`-lc` wrapper forms. Other shell syntaxes remain unverified rather
  than being guessed equivalent.
- No active-profile install, migration, uninstall, trust mutation, or unrelated
  configuration write was performed.

The fix-3 implementation is committed and locally validated. It was not pushed,
merged, released, or submitted as a PR.
