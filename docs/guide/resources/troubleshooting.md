---
title: Troubleshooting
description: Common RTK issues and how to fix them
sidebar:
  order: 2
---

# Troubleshooting

## `rtk gain` says "not a rtk command"

**Symptom:**
```bash
$ rtk gain
rtk: 'gain' is not a rtk command. See 'rtk --help'.
```

**Cause:** You installed **Rust Type Kit** (`reachingforthejack/rtk`) instead of **Rust Token Killer** (`rtk-ai/rtk`). They share the same binary name.

**Fix:**
```bash
cargo uninstall rtk
curl -fsSL https://raw.githubusercontent.com/rtk-ai/rtk/master/install.sh | sh
rtk gain    # should now show token savings stats
```

## How to tell which rtk you have

| If `rtk gain`... | You have |
|------------------|----------|
| Shows token savings dashboard | Rust Token Killer ✅ |
| Returns "not a rtk command" | Rust Type Kit ❌ |

## AI assistant not using RTK

**Symptom:** Claude Code (or another agent) runs `cargo test` instead of `rtk cargo test`.

**Checklist:**

1. Verify RTK is installed:
   ```bash
   rtk --version
   rtk gain
   ```

2. Initialize the hook:
   ```bash
   rtk init --global    # Claude Code
   rtk init --global --cursor    # Cursor
   rtk init --global --opencode  # OpenCode
   ```

3. Restart your AI assistant.

4. Verify hook status:
   ```bash
   rtk init --show
   ```

5. Check `settings.json` has the hook registered (Claude Code):
   ```bash
   cat ~/.claude/settings.json | grep rtk
   ```

## RTK not found after `cargo install`

**Symptom:**
```bash
$ rtk --version
zsh: command not found: rtk
```

**Cause:** `~/.cargo/bin` is not in your PATH.

**Fix:**

For bash (`~/.bashrc`) or zsh (`~/.zshrc`):
```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

For fish (`~/.config/fish/config.fish`):
```fish
set -gx PATH $HOME/.cargo/bin $PATH
```

Then reload:
```bash
source ~/.zshrc    # or ~/.bashrc
rtk --version
```

## RTK on Windows

### Double-clicking rtk.exe does nothing

**Symptom:** You double-click `rtk.exe`, a terminal flashes and closes instantly.

**Cause:** RTK is a command-line tool. With no arguments, it prints usage and exits. The console window opens and closes before you can read anything.

**Fix:** Open a terminal first, then run RTK from there:
- Press `Win+R`, type `cmd`, press Enter
- Or open PowerShell or Windows Terminal
- Then run: `rtk --version`

### Hook not working (no auto-rewrite)

**Symptom:** `rtk init -g` shows an old shell-hook or fallback message on Windows.

**Cause:** The installation still contains a legacy pre-v0.37.2 shell hook, or the binary hook is not registered in the agent settings.

**Fix:** Re-run the native installation from PowerShell or Command Prompt:
```bash
rtk init -g
rtk init --show
```

If you need POSIX shell semantics or Linux-only utilities, use [WSL](https://learn.microsoft.com/en-us/windows/wsl/install) as an optional separate environment. Native PowerShell aliases such as `dir` are recognized, but commands that require external POSIX tools still need those tools installed on PATH.

### Choosing the PowerShell route

Use `rtk powershell` when the machine has Windows PowerShell 5.1 and `rtk pwsh`
when it has PowerShell 7.x. Both routes preserve native flags and fail open for
interactive, `-File`, encoded, XML, redirected, remoting, job, or uncertain
expressions. If `pwsh.exe` is not installed, `rtk pwsh` reports the normal
missing-host error; install PowerShell 7.x or use `rtk powershell` instead.
For exact output, bypass filtering with the native host or `rtk proxy pwsh`.

### Node.js tools not found

**Symptom:**
```
rtk vitest --run
Error: program not found
```

**Cause:** On Windows, Node.js tools are installed as `.CMD`/`.BAT` wrappers. Older RTK versions couldn't find them.

**Fix:** Update to RTK v0.23.1+:
```bash
cargo install --git https://github.com/rtk-ai/rtk --branch master
rtk --version    # should be 0.23.1+
```

## Compilation error during installation

```bash
rustup update stable
rustup default stable
cargo clean
cargo build --release
cargo install --path . --force
```

Minimum required Rust version: 1.70+.

## OpenCode not using RTK

```bash
rtk init --global --opencode
# restart OpenCode
rtk init --show    # should show "OpenCode: plugin installed"
```

## `cargo install rtk` installs the wrong package

If Rust Type Kit is published to crates.io under the name `rtk`, `cargo install rtk` may install the wrong one.

Always use the explicit URL, pinned to the release branch:

```bash
cargo install --git https://github.com/rtk-ai/rtk --branch master
```

## Does RTK break Claude's prompt cache?

No. RTK filters command output once, at execution time. The filtered result is written into the
conversation history and never changes afterwards, and prompt caching matches on a stable prefix
— RTK does not rewrite anything the cache has already seen.

Smaller tool results also make caching cheaper: cache writes bill at 1.25x and cache reads at
0.1x the input rate, so fewer tokens in means less to write once and less to re-read every turn.

To see your own cache write and read volumes next to RTK's savings:

```bash
rtk cc-economics
```

`rtk gain` reports token savings only; the cache breakdown lives in `rtk cc-economics`.

## Run the diagnostic script

From the RTK repository root:

```bash
bash scripts/check-installation.sh
```

Checks:
- RTK installed and in PATH
- Correct version (Token Killer, not Type Kit)
- Available features
- Claude Code integration
- Hook status

## Codex reports an untrusted or inactive hook

Run the candidate binary's read-only diagnostic first:

```bash
rtk doctor --agent codex --format json
```

Re-run `rtk init --codex` for the current project or `rtk init -g --codex`
for the selected `CODEX_HOME`. Review the config and hook command before trusting
the project or restarting Codex. RTK does not bypass hook trust, native sandbox
rules, or approval policy. An unknown Codex version/schema is unsupported until
its payload and output contract are verified.

## Need exact or omitted output

Interactive, redirected, binary, machine-readable, and caller-formatted output
should stay native or use an explicit exact RTK route. Exact execution preserves
bytes and exit status and records no false compression credit.

When compact output prints a recovery ID, read the stored producer output rather
than executing the producer again:

```bash
rtk read -l none --recovery <id>
rtk read -l none --recovery <id> --lines 120:160
```

MCP clients can use `read_recovery` and `search_recovery` for bounded paging and
search. If recovery storage was disabled, exceeded its limit, or could not be
written, RTK reports recovery unavailable; it does not invent missing output.

## Still stuck?

Open an issue: https://github.com/rtk-ai/rtk/issues
