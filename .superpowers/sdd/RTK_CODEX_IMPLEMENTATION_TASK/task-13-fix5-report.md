# Task 13 fix-5 report

Status: `DONE_WITH_CONCERNS`

Implementation commit: `aca43310da9488fd363e9a5fdf00b54a6e8c6003`

The fifth fix round hardens `scripts/verify-agent-coverage.py` against shell
comment syntax before shell-wrapper parsing or executable basename
normalization. A raw `#` now invalidates the complete expression, so forms
such as `#/bin/sh -c 'rtk git status'` cannot normalize to `sh`, and comment
forms inside recursively unwrapped `bash`/`sh`/`zsh` scripts cannot become
RTK evidence. Existing direct and safe quoted-wrapper controls remain
accepted.

## Changed files

- `scripts/verify-agent-coverage.py`
  - Reject `#` together with the existing shell control, redirection,
    newline, and NUL characters before `shlex` parsing and wrapper basename
    normalization. Recursive calls apply the same raw-expression check.
- `scripts/test_verify_agent_coverage.py`
  - Add deterministic Codex and Claude negative regressions for outer
    `#/bin/sh -c 'rtk git status'`, inner `bash -c '# rtk git status'`, and
    recursively nested `bash -c '#/bin/sh -c "rtk git status"'` forms.
  - Preserve the existing direct and safe `bash -lc` positive controls for
    both adapters.
- `.superpowers/sdd/RTK_CODEX_IMPLEMENTATION_TASK/task-13-fix5-report.md`
  - This report.

No CI or documentation wording changed in this round: the existing CI and
validation documentation already requires structured command/result evidence
for live verification and explicitly labels marker-only runs as smoke-only
and unverified.

## Test-first evidence

Before the production change, the new deterministic regressions were red:

```text
Command: py -3 scripts/test_verify_agent_coverage.py
Observed process exit: 0 (the local RTK hook emitted Import-Clixml diagnostics
and masked the Python process failure)
Unittest result: Ran 19 tests in 3.086s; FAILED (failures=4)
Failures: Codex and Claude accepted the outer attached-comment wrapper and the
recursively nested attached-comment wrapper.
```

After the production change:

```text
Command: py -3 scripts/test_verify_agent_coverage.py
Result: exit 0; Ran 19 tests in 3.341s; OK

Command: rtk test python scripts/test_verify_agent_coverage.py
Result: exit 0; Ran 19 tests in 3.369s; OK
```

The simple inner `bash -c '# rtk git status'` case was already unverified;
the regression keeps that boundary explicit while also covering the recursive
basename-normalization bypass.

## Final gates

All available requested gates passed after the implementation commit:

```text
Command: rtk cargo fmt --all -- --check
Result: exit 0; no output

Command: rtk cargo clippy --all-targets
Result: exit 0; Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.37s

Command: $env:RUST_MIN_STACK = '8388608'; rtk test cargo test --all
Result: exit 0; 27 reported test binaries; 3471 passed, 0 failed, 8 ignored

Command: target\debug\rtk.exe verify --require-all
Result: exit 0; 158/158 tests passed; ai_output_legacy_paths=26
Warning: untrusted project filters skipped in verify

Command: rtk summary bash scripts/validate-docs.sh
Result: exit 0; Documentation validation passed
Warnings: manual verification remains suggested for possible hook rewrites of
ruff, pytest, go, and golangci

Command: rtk git diff --check
Result: exit 0

Command: rtk git diff --cached --check
Result: exit 0 before the implementation commit
```

The implementation commit contains only the two code/test files listed above.
The report is committed separately immediately after this write; its commit
SHA is included in the final handoff alongside the implementation SHA.

## Remaining concerns and unsupported evidence

- No live Codex or Claude host session was run in this fix round. The adapter
  evidence cases are deterministic JSONL fixtures, not proof of a live host
  execution or of a selected model/effort.
- No active profile or `rtk init` check was performed in this round, so no
  user profile state was changed.
- The docs validator's hook-rewrite warnings above remain manual/unsupported
  checks, not failures of this change.
- No push, release, merge, pull request, subagent, or reviewer action was
  performed.
- Pre-existing untracked workspace paths `scripts/__pycache__/` and `tmp/`
  were preserved and were not staged.
