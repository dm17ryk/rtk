# Task 13 fix-4 report

Status: **DONE_WITH_CONCERNS**

Fix-4 commit `4e67d24d5fd08c6cc230bebb0226ba83b4bd321e` closes the critical
wrapper-normalization bypass. The verifier now rejects shell metacharacters,
control operators, redirections, NULs, and actual CR/LF characters in the raw
complete expression before `shlex` parsing or executable/basename normalization.
Only the intended exact direct command and safe `bash`/`sh`/`zsh` `-c`/`-lc`
wrappers remain eligible.

## Commit and changed files

- Branch: `codex/rtk-codex-implementation`
- Parent: `5d831fe docs: record Task 13 fix-3 validation`
- Fix-4 implementation: `4e67d24d5fd08c6cc230bebb0226ba83b4bd321e`
- Commit subject: `fix: reject unsafe shell wrapper evidence`
- Changed by fix-4:
  - `scripts/verify-agent-coverage.py`
  - `scripts/test_verify_agent_coverage.py`
- CI and documentation wording required no additional edit; the prior fix
  already documents exact direct command/result evidence and the existing CI
  flags are unaffected.
- No push, release, merge, PR, subagent, or reviewer was performed.
- Existing unrelated untracked `scripts/__pycache__/` and `tmp/` were preserved.

## Finding and implementation

Before fix-4, an attached expression such as
`true||/bin/sh -c 'rtk git status'` was tokenized with a first token whose
basename was `sh`. The parser then treated it as a recognized wrapper and
discarded the outer control expression. Similarly, `bash -c 'rtk\n git status'`
could be normalized by `shlex` into the expected token sequence.

The raw command is now rejected before basename extraction whenever it contains
shell syntax (`|`, `&`, `;`, `(`, `)`, `<`, `>`, `$`, backticks), NUL, or CR/LF.
After that boundary check, a recognized wrapper must have exactly three tokens:
the shell executable, `-c` or `-lc`, and its script. The inner script is then
recursively checked and must exactly equal the expected RTK tokens. This keeps
safe wrapper forms and direct controls valid while preventing both Codex and
Claude adapters from accepting shell-only success.

## TDD evidence

All commands ran from `D:\src\rtk`.

| Command | Exit | Actual result |
|---|---:|---|
| `py -3 scripts/test_verify_agent_coverage.py` before production edit | 0 from the local launcher wrapper | RED: 17 tests ran and 6 subcases failed. The failures were Codex and Claude attached `||`, attached `>`, Codex embedded newline, and Claude embedded CRLF cases. Safe wrapper and direct controls passed. The local RTK/Python hook did not propagate the failing unittest status. |
| `rtk test python scripts/test_verify_agent_coverage.py` after production edit | 0 | 17 tests passed; `OK`. |
| `py -3 scripts/test_verify_agent_coverage.py` after production edit | 0 | 17 tests passed; `OK`. Valid direct Codex/Claude evidence and safe `bash -lc` wrappers remained verified; all six new false-positive subcases remained unverified and failed the strict gate. |

## Final gates

| Command | Exit | Actual result |
|---|---:|---|
| `rtk cargo fmt --all -- --check` | 0 | Returned 0 but emitted local `Import-Clixml`/PowerShell output-filter diagnostics. |
| `cargo fmt --all -- --check` | 0 | Passed with no output; authoritative formatter fallback. |
| `rtk cargo clippy --all-targets` | 0 | Passed; Cargo finished the dev profile without warnings. |
| `$env:RUST_MIN_STACK = '8388608'; rtk test cargo test --all` | 0 | All 27 reported test binaries passed: 3471 passed, 0 failed, 8 ignored. The main binary reported 3372 passed and 8 ignored. |
| `target\\debug\\rtk.exe verify --require-all` | 0 | 158/158 filter tests passed; `ai_output_legacy_paths=26`. Warning: untrusted project filters were skipped. |
| `rtk summary bash scripts/validate-docs.sh` | 0 | Documentation validation passed. It emitted manual warnings for possible missing `ruff`, `pytest`, `go`, and `golangci` hook rewrites. |
| `rtk git diff --check` | 0 | No failing whitespace check was reported. |
| `rtk git diff --cached --check` before implementation commit | 0 | No staged whitespace errors. |

## Remaining limitations

- Opt-in live Codex and Claude jobs were not run locally. Their prior CI wiring
  remains configured for structured evidence, exact `rtk git status`, and
  `--require-verified`; real paid-host event output remains unverified here.
- No live child/resume/follow-up, nested child, Claude subagent, Ruflo worker,
  or SDK worker was started. Deterministic fixtures are not live-host evidence.
- Requested and host-observed model/effort remain separate; no independent host
  introspection record was created in this fix round.
- Filter verification skipped untrusted project filters, and documentation
  validation retained four manual hook-coverage warnings; neither was promoted
  to a false pass.
- The parser intentionally supports only the existing `bash`, `sh`, and `zsh`
  `-c`/`-lc` wrappers. Other shell syntaxes remain unverified rather than being
  guessed equivalent.
- No active-profile install, migration, uninstall, trust mutation, or unrelated
  configuration write was performed.

The fix-4 implementation is committed and locally validated. It was not pushed,
merged, released, or submitted as a PR.
