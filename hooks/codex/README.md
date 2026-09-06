# Codex CLI integration

> Part of [`hooks/`](../README.md) — see also [`src/hooks/`](../../src/hooks/README.md) for installation code.

`rtk init --codex` installs three complementary local integrations:

- a native Codex `PreToolUse` hook in `config.toml` that calls `rtk hook codex`;
- `AGENTS.md` plus `RTK.md` direct-route instructions; and
- the local `rtk mcp` stdio server unless `--no-mcp` or `--hook-only` is used.

Project mode writes under the current repository. Global mode writes under
`$CODEX_HOME` when set, otherwise `~/.codex`. The MCP command and instruction
reference use the absolute path of the running RTK binary, so rerun the same
init command after moving or replacing that binary.

```bash
rtk init --codex             # current project
rtk init -g --codex          # selected Codex home
rtk doctor --agent codex --format json
```

## Permission and exact-output boundary

Current Codex requires a rewritten `updatedInput` to carry
`permissionDecision: "allow"`. RTK emits that pair only when the incoming event
already reports `permission_mode: "bypassPermissions"`. In normal approval
modes RTK is silent, leaving the original command and Codex permission flow
unchanged. Unsupported tools, redirects, substitutions, heredocs, malformed
payloads, and unknown schemas also pass through without a rewrite.

Hooks are a routing aid, not an authorization boundary. Project hooks load only
for trusted projects; review hook/config changes before granting trust. Exact,
interactive, redirected, binary, and machine-consumed work should use the
native program or an explicit RTK exact route, not a forced semantic rewrite.

## Upgrade and removal

Re-running init is idempotent. It migrates the old awareness-only setup, removes
stale RTK hook/MCP entries, refreshes absolute paths, and preserves unrelated
TOML keys, MCP servers, hooks, instructions, and model settings.

```bash
rtk init -g --codex
rtk init -g --codex --dry-run
rtk init -g --codex --uninstall
```

The installer does not select a model or reasoning effort. Named Codex profiles
and custom-agent role files remain operator-owned configuration.
