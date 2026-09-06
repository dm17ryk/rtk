# Task 13 fix-6 report

Status: `DONE_WITH_CONCERNS`

Workflow implementation commit: `b5d9c8c0d98d57ac19c4c45f889065a2d988c4d2`

This round fixes the two whole-branch CI findings in
`.github/workflows/ci.yml`:

- Windows PowerShell lifecycle, candidate-routing, native-smoke,
  manifest-validation, full-test, and deterministic-fixture invocations now
  check `$LASTEXITCODE` immediately after each external command and exit with
  that code. The existing double-init and double-uninstall checks remain in
  order. Unix lifecycle and fixture blocks explicitly use `set -euo pipefail`,
  and the mixed-shell full-test/fixture steps are split by operating system.
- Semgrep now has a PR-only baseline step using
  `github.event.pull_request.base.sha` and a workflow-dispatch-only full scan.
  Both retain `--error`; manual runs no longer depend on a pull-request-only
  field.

## Changed files

- `.github/workflows/ci.yml`
  - Immediate PowerShell native-command failure checks.
  - OS-specific build/full-test/fixture steps where shell syntax differs.
  - Conditional Semgrep PR baseline versus manual full scan.
- `.superpowers/sdd/RTK_CODEX_IMPLEMENTATION_TASK/task-13-fix6-report.md`
  - This report.

No Rust source or unrelated user files were changed. Pre-existing untracked
`scripts/__pycache__/` and `tmp/` paths were preserved and not staged.

## Workflow-static validation

The repository search found no dedicated deterministic workflow/YAML
validator in `scripts/` or `tests/`, so the workflow was statically checked
after editing:

```text
Command: ruby -e "require 'yaml'; w=YAML.load_file('.github/workflows/ci.yml'); s=w['jobs']['semgrep']['steps']; raise 'missing PR baseline' unless s.any? { |step| step['if'].to_s.include?('pull_request') && step['run'].include?('--baseline-commit') && step['run'].include?('--error') }; raise 'missing manual full scan' unless s.any? { |step| step['if'].to_s.include?('workflow_dispatch') && !step['run'].include?('--baseline-commit') && step['run'].include?('--error') }; puts 'YAML parsed; conditional Semgrep PR baseline/manual full-scan steps verified'"
Result: exit 0; YAML parsed; conditional Semgrep PR baseline/manual full-scan steps verified

Command: rtk rg -n -C 3 "LASTEXITCODE|Deterministic agent fixture|Full Rust suite|Semgrep" .github/workflows/ci.yml
Result: exit 0; static inspection showed immediate checks after every listed
Windows external candidate/validator invocation at the updated workflow
locations (lines 85-184 and 193-251), and the two conditional Semgrep steps
at lines 471-476.

Command: py -3 -c "import yaml; ..."
Result: PyYAML unavailable: ModuleNotFoundError: No module named 'yaml'. The
local RTK hook reported process exit 0 despite the Python exception.

Command: where.exe actionlint
Result: actionlint unavailable: INFO: Could not find files for the given
pattern(s). The local RTK hook reported exit 0.
```

Ruby YAML parsing and the explicit structural assertions were the successful
static workflow validation. No live GitHub Actions run was available locally.

## Local gates actually run

```text
Command: rtk test python scripts/test_verify_agent_coverage.py
Result: exit 0; Ran 19 tests in 3.150s; OK

Command: rtk cargo fmt --all -- --check
Result: exit 0; no output

Command: rtk cargo clippy --all-targets
Result: exit 0; clippy finished with
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s
The RTK wrapper also emitted Import-Clixml: 'Element' is an invalid XmlNodeType.
Line 245, position 12.

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
Result: exit 0; Git warned that the working copy LF line endings for
.github/workflows/ci.yml will be replaced by CRLF on the next Git touch.

Command: rtk git diff --cached --check
Result: exit 0 before workflow implementation commit b5d9c8c
```

## Remaining concerns

- The workflow changes have not run in GitHub Actions in this environment.
- `actionlint` and PyYAML were unavailable; Ruby YAML parsing plus static
  expression and command-boundary inspection were used instead.
- The RTK Windows hook emitted `Import-Clixml` diagnostics during some local
  wrapper commands and can mask a child failure; gate output was recorded from
  the actual command result where available.
- No active profile/init lifecycle check was performed by this fix round, so
  no unrelated user state was changed.
- No push, release, merge, pull request, subagent, or reviewer action was
  performed.
