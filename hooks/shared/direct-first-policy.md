## Command Selection Priority

Use the narrowest RTK route that can perform the task:

1. **Direct RTK first**: use supported commands such as `rtk read`, `rtk rg`, `rtk grep`, `rtk find`, `rtk ls`, `rtk git ...`, `rtk cargo ...`, and `rtk gh ...`. Use `rtk --help` when unsure.
2. **Windows shell expressions**: use `rtk cmd "<CMD expression>"` for CMD or `rtk powershell "<expression>"` / `rtk pwsh "<expression>"` for PowerShell (or the MCP `run_cmd` / `run_powershell` tools) when safe filtering is useful.
3. **Executable fallback**: use `rtk proxy <program> <args>` only when RTK has no matching route or exact unfiltered output is required.
4. **Native shell fallback last**: use raw `cmd.exe`, `powershell.exe`, or `pwsh.exe` (or `rtk proxy cmd.exe ...` / `rtk proxy pwsh ...`) only for interactive, exact-output, redirected, machine-consumed, batch, or opaque shell behavior.

On Windows, prefer `rtk cmd`, `rtk powershell`, or `rtk pwsh` for optimizable shell expressions and never hide an RTK-supported command inside a native shell. Use `rtk git status`, not `rtk proxy pwsh -Command "git status"`; use a native host only when its semantics must remain exact.
