## Command Selection Priority

Use the narrowest RTK route that can perform the task:

1. **Direct RTK first**: use supported commands such as `rtk read`, `rtk rg`, `rtk grep`, `rtk find`, `rtk ls`, `rtk git ...`, `rtk cargo ...`, and `rtk gh ...`. Use `rtk --help` when unsure.
2. **Executable fallback**: use `rtk proxy <program> <args>` only when RTK has no matching route or exact unfiltered output is required.
3. **Shell fallback last**: use `rtk proxy pwsh -NoProfile -Command ...` or `rtk proxy cmd /d /c ...` only for shell built-ins, scripts, or control flow that cannot be expressed as a direct RTK command.

On Windows, never hide an RTK-supported command inside PowerShell or Command Prompt. Use `rtk git status`, not `rtk proxy pwsh -Command "git status"`.
