<!-- rtk-instructions v3 -->
# RTK - Rust Token Killer (Kilo Code)

**Usage**: Token-optimized CLI proxy for shell commands.

## Command Selection Priority

Use the narrowest RTK route that can perform the task:

1. **Direct RTK first**: use supported commands such as `rtk read`, `rtk rg`, `rtk grep`, `rtk find`, `rtk ls`, `rtk git ...`, `rtk cargo ...`, and `rtk gh ...`. Use `rtk --help` when unsure.
2. **Executable fallback**: use `rtk proxy <program> <args>` only when RTK has no matching route or exact unfiltered output is required.
3. **Shell fallback last**: use `rtk proxy pwsh -NoProfile -Command ...` or `rtk proxy cmd /d /c ...` only for shell built-ins, scripts, or control flow that cannot be expressed as a direct RTK command.

On Windows, use `rtk git status`, not `rtk proxy pwsh -Command "git status"`.

Examples:

```bash
rtk git status
rtk cargo test
rtk ls src/
rtk grep "pattern" src/
rtk find "*.rs" .
rtk docker ps
rtk gh pr list
```

## Meta Commands

```bash
rtk gain              # Show token savings
rtk gain --history    # Command history with savings
rtk discover          # Find missed RTK opportunities
rtk proxy <cmd>       # Run raw (no filtering, for debugging)
```

## Why

RTK filters and compresses command output before it reaches the LLM context, cutting up to 90% of common command output. Always use `rtk <cmd>` instead of raw commands.
<!-- /rtk-instructions -->
