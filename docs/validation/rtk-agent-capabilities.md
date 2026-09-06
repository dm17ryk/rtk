# RTK agent capability validation

Recorded 2026-09-06 on the Windows checkout `D:\src\rtk` from baseline
`b87d349` plus the Task 13 candidate changes.

## Observed environment

| Item | Observed value |
|---|---|
| Branch | `codex/rtk-codex-implementation` |
| Candidate / installed RTK before final reinstall | `0.46.1-dev.12` |
| Candidate binary used for integration checks | `D:\src\rtk\target\debug\rtk.exe` |
| Codex CLI | `codex-cli 0.153.0` |
| Claude Code | `2.1.258` |
| Active Codex home | `C:\Users\dmitr\.codex` (`default`) |
| Task 13 implementer binding | requested and host-bound `gpt-5.6-sol` / `high` |
| Base Codex config | `gpt-5.6-luna` / `medium`, unchanged by RTK init |

Before migration, candidate `rtk doctor --agent codex --format json` reported
instructions and MCP `present`, hook `missing-hook`, and live verification
`unverified`. After backing up `config.toml`, the candidate ran
`rtk init -g --codex`; doctor then reported hook `ready`. A second
`rtk init -g --codex --dry-run` reported `Nothing written` and MCP `already up
to date`.

The redacted before/after diff added one `hooks.PreToolUse` matcher/handler and
changed only `mcp_servers.rtk.command` to the candidate path. TOML serialization
reordered existing keys without changing their values. Unrelated MCP servers,
marketplaces, feature flags, and the base model/effort remained present.

## Protocol contract

Current Codex documentation requires rewritten `updatedInput` to be paired with
`permissionDecision: "allow"`. RTK now emits that pair only when the event is
canonical `Bash`, the command is safely rewritable, and Codex already reports
`permission_mode = "bypassPermissions"`. Normal approval modes and unsafe or
unknown input remain silent, preserving the original host permission flow. See
the [Codex hook contract](https://learn.chatgpt.com/docs/hooks).

The deterministic boundary is covered by `tests/codex_hook_test.rs`,
`tests/agent_integration_test.rs`, and the unit tests in
`src/hooks/codex.rs`/`src/hooks/hook_cmd.rs`. Claude, native tool, MCP, recovery,
and tracking fixtures remain separate from live-agent evidence.

## End-to-end matrix

| Case | Status | Evidence / boundary |
|---|---|---|
| Current Codex main agent | verified | This host executed direct candidate RTK commands; profile doctor is `ready`. Current session hook reload is not inferred. |
| Codex fresh child | blocked | Explicit Task 13 instruction prohibited subagents; fixture coverage is not a live child. |
| Codex resumed child/follow-up | blocked | Same authorization boundary; no duplicate-execution claim. |
| Task-specific model selection | verified | Host role bound this implementer to `gpt-5.6-sol` / `high`; base config remained unchanged. |
| Changed complexity / custom override | implemented | Precedence is documented and deterministic; no live rebinding was authorized. |
| Delegation unavailable | verified | Work continued in the same provider/permission scope and the missing live rows stayed explicit. |
| Nested child | blocked | No nested delegation authorized. |
| Claude main/background subagents | implemented | Deterministic hook tests pass; no paid live Claude session was run. |
| Superpowers implementer/reviewer | implemented | Implementer used host integration; separate Astra reviewer was prohibited and is not claimed. |
| Ruflo headless worker | unsupported | No compatible live Ruflo runtime was established in this environment. |
| SDK worker | unsupported | No SDK host/adapter was supplied; defaults are not assumed. |
| Worktree/nested directory | verified | Absolute global RTK reference and executable paths are covered by installer tests. |
| Alternate `CODEX_HOME` | verified | `test_resolve_codex_dir_prefers_codex_home_and_ignores_empty_value` plus fake-home integration tests. |
| Windows MCP CMD listing | verified | `windows_cmd_e2e`, MCP service tests, and nonterminal capture tests. |
| Windows redirected/structured output | verified | CMD/PowerShell passthrough and exact-output tests. |
| Native Read/Grep replacement | verified | `native_tool_output_test`; producer input is consumed once and unknown/error schemas pass through. |
| Unknown host/tool schema | verified | Host-specific no-op/fallback tests; no success label is synthesized. |
| Raw/exact invocation | verified | Exact contract tests preserve bytes/status and avoid false compression credit. |
| Long output / late failure | verified | Large-output, bounded drain, diagnostics, and recovery tests. |
| Recovery request | verified | `recovery_navigation_test`; stored data is read without rerunning the producer. |
| Tracking DB absent/locked | verified | Tracking fail-open tests keep command output/status authoritative. |
| Originally denied operation | verified | Permission tests and Codex default-mode no-op show RTK cannot authorize it. |

`scripts/verify-agent-coverage.py` validates the offline fixture manifest by
default and reports live verification `unverified`. Its opt-in
`--live-command` path reports `verified` only after a zero exit and any requested
stdout marker; missing runtimes are `unsupported` with exit 3, and host failures
remain failures. The manual CI host jobs are skipped unless explicitly enabled.
