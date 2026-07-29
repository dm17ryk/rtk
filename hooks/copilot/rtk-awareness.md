# RTK — Copilot Integration (VS Code Copilot Chat + Copilot CLI)

**Usage**: Token-optimized CLI proxy (cuts up to 90% of command output)

## Command Selection Priority

Use the narrowest RTK route that can perform the task:

1. **Direct RTK first**: use supported commands such as `rtk read`, `rtk rg`, `rtk grep`, `rtk find`, `rtk ls`, `rtk git ...`, `rtk cargo ...`, and `rtk gh ...`. Use `rtk --help` when unsure.
2. **Executable fallback**: use `rtk proxy <program> <args>` only when RTK has no matching route or exact unfiltered output is required.
3. **Shell fallback last**: use `rtk proxy pwsh -NoProfile -Command ...` or `rtk proxy cmd /d /c ...` only for shell built-ins, scripts, or control flow that cannot be expressed as a direct RTK command.

On Windows, use `rtk git status`, not `rtk proxy pwsh -Command "git status"`.

## What's automatic

The `.github/copilot-instructions.md` file is loaded at session start by both Copilot CLI and VS Code Copilot Chat.
It instructs Copilot to prefix commands with `rtk` automatically.

The `.github/hooks/rtk-rewrite.json` hook adds a `PreToolUse` safety net via `rtk hook` —
a cross-platform Rust binary that intercepts raw bash tool calls and rewrites them.
No shell scripts, no `jq` dependency, works on Windows natively.

## Meta commands (always use directly)

```bash
rtk gain              # Token savings dashboard for this session
rtk gain --history    # Per-command history with savings %
rtk discover          # Scan session history for missed rtk opportunities
rtk proxy <cmd>       # Run raw (no filtering) but still track it
```

## Installation verification

```bash
rtk --version   # Should print: rtk X.Y.Z
rtk gain        # Should show a dashboard (not "command not found")
which rtk       # Verify correct binary path
```

> ⚠️ **Name collision**: If `rtk gain` fails, you may have `reachingforthejack/rtk`
> (Rust Type Kit) installed instead. Check `which rtk` and reinstall from rtk-ai/rtk.

## How the hook works

`rtk hook` reads `PreToolUse` JSON from stdin, detects the agent format, and responds appropriately:

**VS Code Copilot Chat** (supports `updatedInput` — transparent rewrite, no denial):
1. Agent runs `git status` → `rtk hook` intercepts via `PreToolUse`
2. `rtk hook` detects VS Code format (`tool_name`/`tool_input` keys)
3. Returns `hookSpecificOutput.updatedInput.command = "rtk git status"`
4. Agent runs the rewritten command silently — no denial, no retry

**GitHub Copilot CLI** (deny-with-suggestion — CLI ignores `updatedInput` today, see [issue #2013](https://github.com/github/copilot-cli/issues/2013)):
1. Agent runs `git status` → `rtk hook` intercepts via `PreToolUse`
2. `rtk hook` detects Copilot CLI format (`toolName`/`toolArgs` keys)
3. Returns `permissionDecision: deny` with reason: `"Token savings: use 'rtk git status' instead"`
4. Copilot reads the reason and re-runs `rtk git status`

When Copilot CLI adds `updatedInput` support, only `rtk hook` needs updating — no config changes.

## Integration comparison

| Tool                  | Mechanism                               | Hook output              | File                               |
|-----------------------|-----------------------------------------|--------------------------|------------------------------------|
| Claude Code           | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `hooks/rtk-rewrite.sh`             |
| VS Code Copilot Chat  | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `.github/hooks/rtk-rewrite.json`   |
| GitHub Copilot CLI    | `PreToolUse` deny-with-suggestion       | Denial + retry           | `.github/hooks/rtk-rewrite.json`   |
| OpenCode              | Plugin `tool.execute.before`            | Transparent rewrite      | `hooks/opencode-rtk.ts`            |
| (any)                 | Custom instructions                     | Prompt-level guidance    | `.github/copilot-instructions.md`  |
