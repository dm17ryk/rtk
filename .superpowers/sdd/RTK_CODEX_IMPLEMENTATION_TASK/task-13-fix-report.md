# Task 13 fix-round final handoff

Status: **DONE_WITH_CONCERNS**

The IMPORTANT source-review findings are addressed in implementation commit
`7f6e676dbbca194c8ddaf8c38d9488a96e219062`. Required local quality,
filter, and documentation gates passed. Live paid-host, delegated-worker,
cross-platform CI, and independently observed model-binding evidence were not
available in this fix round and are not represented as verified.

## Commits and scope

- Branch: `codex/rtk-codex-implementation`
- Candidate before fix: `61713a55db85cc79f078d25b7e4afc0e2ad16abe`
- Fix implementation: `7f6e676dbbca194c8ddaf8c38d9488a96e219062`
- Commit subject: `fix: require live RTK execution evidence`
- Audited base: `b87d34922b50daf086c3be72ec37bb1597860921`
- No push, release, merge, PR, subagent, or reviewer was performed.
- Pre-existing untracked `scripts/__pycache__/` and `tmp/` were preserved.

## Changed files in the fix commit

- `.github/workflows/ci.yml`
- `docs/guide/getting-started/supported-agents.md`
- `docs/validation/rtk-agent-capabilities.md`
- `docs/validation/rtk-model-routing.md`
- `scripts/test_verify_agent_coverage.py`
- `scripts/verify-agent-coverage.py`
- `tests/codex_hook_test.rs`

This final report is stored separately at
`.superpowers/sdd/RTK_CODEX_IMPLEMENTATION_TASK/task-13-fix-report.md`.

## Findings resolved

1. Marker-only host output is now explicitly
   `live_smoke = passed` / `live_verification = unverified`; it cannot
   become verified without structured command/result evidence for the expected
   direct `rtk ...` command.
2. Codex JSONL command-completion events and paired Claude stream-JSON
   tool-use/tool-result events are parsed deterministically. CI supplies the
   matching evidence format, expects `rtk git status`, and uses
   `--require-verified`.
3. The model-routing ledger and capability matrix now record
   `gpt-5.6-sol` / `high` as requested but not host-observed. Absence of
   global role files is no longer presented as proof of a scoped binding.
4. Live-host usage documentation distinguishes a prose marker from verified
   tool evidence and keeps requested and observed model/effort separate.
5. The stale hook test was renamed to
   `codex_pre_tool_use_returns_safe_input_update_and_allow_decision`.

## TDD and focused verification

All commands ran from `D:\src\rtk`.

| Command | Exit | Actual result |
|---|---:|---|
| `rtk test python scripts/test_verify_agent_coverage.py` before implementation | 0 from RTK wrapper | RED was observed: 8 tests ran, 4 failed. RTK incorrectly returned 0 while its filtered output reported `FAILED`. |
| `rtk log C:/Users/dmitr/AppData/Local/rtk/tee/1788658795_test.log` | 0 | Confirmed failures were unrecognized structured-evidence flags and the old marker-only `verified` result. |
| `rtk test python scripts/test_verify_agent_coverage.py` after implementation | 0 | 8 tests passed; `OK`. |
| `python scripts/test_verify_agent_coverage.py` | 0 | 8 tests passed; `OK`; native run confirmed the real unittest exit code because the RED RTK wrapper did not propagate failure. |
| `rtk test cargo test --test codex_hook_test` | 0 | 4 passed, 0 failed. |
| `python scripts/verify-agent-coverage.py` | 0 | Fixture validation passed for 8 cases; live verification remained `unverified` because no live command was supplied. |

## Required final gates

| Command | Exit | Actual result |
|---|---:|---|
| `rtk cargo fmt --all -- --check` | 0 | Cargo wrapper returned 0 but emitted PowerShell profile/filter diagnostics; a native fallback was run before accepting the gate. |
| `cargo fmt --all -- --check` | 0 | Passed with no output. |
| `rtk cargo clippy --all-targets` | 0 | Passed; `rtk v0.46.1-dev.12` finished the dev profile without warnings. |
| `$env:RUST_MIN_STACK = '8388608'; rtk test cargo test --all` | 0 | All 27 reported test binaries passed: 3471 passed, 0 failed, 8 ignored in total. The main binary reported 3372 passed and 8 ignored. |
| `target\debug\rtk.exe verify --require-all` | 0 | 158/158 filter tests passed; `ai_output_legacy_paths=26`. It warned that untrusted project filters were skipped. |
| `rtk summary bash scripts/validate-docs.sh` | 0 | Documentation validation passed. It emitted manual warnings that the checked hook file may not rewrite `ruff`, `pytest`, `go`, and `golangci`. |
| `rtk git diff --check` | 0 | No whitespace errors. |
| `rtk git diff --cached --check` | 0 | No staged whitespace errors before commit. |

## Safe active-profile checks

The candidate binary was `D:\src\rtk\target\debug\rtk.exe`, version
`0.46.1-dev.12`. Help was inspected before using profile flags.

| Command | Exit | Actual result |
|---|---:|---|
| `target\debug\rtk.exe doctor --agent codex --format json` | 0 | Instructions and MCP `present`; hook `ready`; trust `host-managed`; profile `default`; live verification `unverified`; candidate binary path reported. |
| `target\debug\rtk.exe init -g --codex --dry-run` | 0 | `[dry-run] Nothing written.` Existing MCP registration already pointed to the candidate binary. |

No active-profile install, migration, uninstall, trust mutation, or unrelated
configuration write was performed in this fix round.

## Remaining concerns and unsupported evidence

- The opt-in Codex and Claude CI jobs were configured but not executed locally;
  their real event schemas and paid-host behavior remain unverified until an
  authorized workflow dispatch with credentials succeeds.
- Linux, macOS, and hosted Windows CI were not run from this Windows checkout.
- No fresh Codex child, resumed child/follow-up, nested child, Claude subagent,
  Ruflo worker, or SDK worker was started because subagents/reviewers were
  prohibited. Fixture coverage is not live-host evidence.
- The requested `gpt-5.6-sol` / `high` implementer assignment and
  `gpt-6-astra` / `xhigh` reviewer assignment have no independent host
  introspection record. The implementation ledger therefore leaves the former
  pending and the prohibited independent review blocked.
- Filter verification skipped untrusted project filters. Documentation
  validation retained its four manual hook-coverage warnings.
- The RTK test wrapper's observed failure-exit propagation issue is outside this
  Task 13 fix scope; native Python was used to establish the regression suite's
  real exit status.

The candidate is suitable for final handoff with these external/live evidence
gaps explicit. It was not pushed, merged, released, or submitted as a PR.
