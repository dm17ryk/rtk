# MCP server

RTK includes a local, synchronous stdio MCP server: `rtk mcp`.

## Automatic client registration

`rtk init` installs the selected client's normal hook, plugin, or rules and
also registers the MCP server by default:

```bash
rtk init -g                  # Claude Code + MCP
rtk init -g --copilot       # Copilot CLI and VS Code + MCP
rtk init -g --gemini        # Gemini CLI + MCP
rtk init -g --codex         # Codex CLI + MCP
rtk init -g --agent cursor  # Cursor + MCP
rtk init -g --opencode      # OpenCode + MCP
```

The registration uses the absolute path of the running RTK executable and
`["mcp"]` as its arguments, so paths containing spaces work on native Windows.
Running the same init command again is safe and refreshes a stale executable
path while preserving unrelated MCP servers.

Use `--no-mcp` when a client must receive only the hook/plugin/rules
integration. `--hook-only` also skips MCP registration. `--uninstall` removes
only RTK's MCP entry along with the selected integration.

| Client | MCP configuration written by `rtk init` |
|---|---|
| Claude Code | `~/.claude.json` globally or `.mcp.json` per project |
| Gemini CLI | `~/.gemini/settings.json` or `.gemini/settings.json` |
| Codex CLI | `~/.codex/config.toml` or `.codex/config.toml` |
| GitHub Copilot | `~/.copilot/mcp-config.json` plus VS Code user `mcp.json`; project mode uses `.vscode/mcp.json` |
| Cursor | `~/.cursor/mcp.json` |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` |
| Cline / Roo Code | Both extensions' VS Code `globalStorage` MCP settings |
| Kilo Code | Its VS Code `globalStorage` MCP settings |
| Google Antigravity | `.agents/mcp_config.json` |
| Kimi Code | `.kimi-code/mcp.json` |
| Hermes | `~/.hermes/config.yaml` under `mcp_servers.rtk` |
| Factory Droid | `~/.factory/mcp.json` or `.factory/mcp.json` |
| OpenCode | the `mcp.rtk` entry in `opencode.json` |
| Pi | No native MCP client; RTK installs the Pi extension instead |
| Mistral Vibe | No native MCP client; RTK installs the Vibe hook instead |

The server implements `initialize`, `notifications/initialized`, `tools/list`,
and `tools/call`. Its current tool inventory is:

| Tool | Contract |
|---|---|
| `rewrite_command` | Return a safe RTK rewrite for one raw command without executing it. |
| `run_filtered` | Execute typed RTK argv with bounded compact/legacy output. |
| `run_cmd` | Execute one native Windows CMD expression with safe filtering where supported. |
| `run_powershell` | Execute one Windows PowerShell 5.1 or PowerShell 7+ expression using the explicitly selected host. |
| `gain_summary` | Return bounded savings/tracking rows. |
| `discover_unhandled` | Return bounded missed-routing candidates. |
| `list_tee_artifacts` | List recovery artifacts in RTK's configured tee directory. |
| `read_tee_file` | Read a bounded legacy tee artifact by validated identifier. |
| `read_recovery` | Page or range-read original stored output by recovery ID. |
| `search_recovery` | Search original stored output with bounded context and exact source locations. |

Recovery tools read existing files and never rerun the producer. They redact
recognized secrets and bound oversized lines. Unknown IDs, paths outside the
configured tee directory, invalid ranges, and unsupported host schemas fail
explicitly instead of degrading to an unrestricted read.

The `initialize` response includes a direct-first instruction for the model,
and the tool descriptions repeat it. MCP-aware agents are told to prefer typed
RTK argv over launching a host shell. For Windows CMD syntax, use `run_cmd` (or
the `rtk cmd` CLI route) so operators, expansion, state, and exit codes remain
CMD-native. For PowerShell, use `run_powershell` with `host: "powershell"` for
Desktop 5.1 or `host: "pwsh"` for PowerShell 7.x. Raw hosts remain fallbacks
for interactive, exact-output, redirected, machine-consumed, batch, or opaque
shell behavior.

MCP transport is nonterminal. `run_cmd` can still filter eligible captured CMD
output such as a normal directory listing, but structured or redirected output
stays exact. `run_powershell` preserves host-specific exit and stream semantics;
dynamic/remoting/job/script-file/XML/encoded modes and undecodable bytes remain
native or unsupported rather than being relabeled as verified.

`run_filtered` accepts only a typed argument array such as
`["git", "status"]`; it never accepts a shell command string. On Windows,
`run_cmd` accepts one raw `expression` string such as
`{"expression":"echo %CD% & dir /b"}` and applies the same safety bounds.
`run_powershell` requires `host` (`powershell` or `pwsh`) and one raw
`expression`, for example `{"host":"pwsh","expression":"Get-Process"}`.
The selected host's native flags and streams are preserved; uncertain syntax,
encoded/file/XML modes, jobs, remoting, and redirection stay unfiltered.
Working directories must already exist and output is bounded by an explicit
byte limit. Execution uses the installed RTK executable, preserves the
underlying exit code, and applies an explicit timeout.

MCP execution is enabled by default and gives a connected client local-machine
command capabilities. Run it only for trusted local clients. The server opens
no network listener, and tee-file reads are restricted to RTK's configured tee
directory and `.log` files.

## Dashboard

Run rtk dashboard (or its rtk tui alias) for a local interactive view of the
same tracking data. It provides Overview, Commands, Activity, Health, and
Artifacts tabs, supports global or current-project scope, and refreshes every
30 seconds. Press 1-5 to select a tab, Tab or n/p to navigate, and q or Esc
to exit.
