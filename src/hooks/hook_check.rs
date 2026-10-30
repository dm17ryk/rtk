//! Detects whether RTK hooks are installed and warns if they are outdated.

use super::constants::{HOOKS_SUBDIR, PRE_TOOL_USE_KEY, REWRITE_HOOK_FILE, SETTINGS_JSON};
use super::is_claude_hook_command;
use crate::core::user_dirs;
use crate::core::utils::from_json_str;
use std::path::{Path, PathBuf};

pub const CURRENT_HOOK_VERSION: u8 = 4;
const WARN_INTERVAL_SECS: u64 = 24 * 3600;

/// Hook status for diagnostics and `rtk gain`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HookStatus {
    /// Hook is installed and up to date.
    Ok,
    /// Hook exists but is outdated or unreadable.
    Outdated,
    /// No hook file found (but Claude Code is installed).
    Missing,
}

/// Native Codex hook installation status.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CodexHookStatus {
    /// A single usable RTK Bash PreToolUse hook is configured.
    Ready,
    /// Codex's config.toml does not exist.
    MissingConfig,
    /// config.toml exists but has no usable RTK Bash hook.
    MissingHook,
    /// Codex config.toml cannot be parsed or has an invalid hook shape.
    InvalidConfig,
    /// More than one RTK Codex hook is registered.
    ConflictingConfig,
    /// The configured absolute RTK executable no longer exists.
    MissingBinary,
}

impl CodexHookStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::MissingConfig => "missing-config",
            Self::MissingHook => "missing-hook",
            Self::InvalidConfig => "invalid-config",
            Self::ConflictingConfig => "conflicting-config",
            Self::MissingBinary => "missing-binary",
        }
    }
}

/// Return true when any supported RTK integration is present for this user.
///
/// `rtk gain` must not report a missing Claude hook when the active agent uses
/// a plugin or a different native hook integration.
pub fn any_integration_installed() -> bool {
    let Some(home) = crate::core::user_dirs::home() else {
        return false;
    };
    integration_installed_in(&home)
}

fn integration_installed_in(home: &std::path::Path) -> bool {
    let paths = [
        home.join(super::constants::CONFIG_DIR)
            .join(super::constants::OPENCODE_SUBDIR)
            .join(super::constants::PLUGIN_SUBDIR)
            .join(super::constants::OPENCODE_PLUGIN_FILE),
        home.join(super::constants::CURSOR_DIR)
            .join(HOOKS_SUBDIR)
            .join(REWRITE_HOOK_FILE),
        home.join(super::constants::GEMINI_DIR)
            .join(HOOKS_SUBDIR)
            .join(super::constants::GEMINI_HOOK_FILE),
        home.join(super::constants::HERMES_DIR)
            .join(super::constants::HERMES_PLUGINS_SUBDIR)
            .join(super::constants::HERMES_PLUGIN_NAME)
            .join(super::constants::HERMES_PLUGIN_MANIFEST_FILE),
    ];
    paths.iter().any(|path| path.is_file()) || codex_integration_installed(home)
}

fn codex_integration_installed(home: &std::path::Path) -> bool {
    let codex_dir = home.join(super::constants::CODEX_DIR);
    let agents_has_rtk = std::fs::read_to_string(codex_dir.join("AGENTS.md"))
        .ok()
        .is_some_and(|content| {
            content.contains("RTK.md") || content.contains("<!-- rtk-instructions")
        });
    let instructions_are_rtk = std::fs::read_to_string(codex_dir.join("RTK.md"))
        .ok()
        .is_some_and(|content| content.contains("<!-- rtk-instructions"));

    if crate::service::debug_enabled() {
        eprintln!(
            "[rtk-debug] hook_check.codex decision={} agents_reference={} instructions_marker={}",
            if agents_has_rtk || instructions_are_rtk {
                "installed"
            } else {
                "not-installed"
            },
            agents_has_rtk,
            instructions_are_rtk
        );
    }
    agents_has_rtk || instructions_are_rtk || codex_status_in(&codex_dir) == CodexHookStatus::Ready
}

/// Inspect the effective Codex config without modifying it or starting Codex.
/// The hook is considered ready only when exactly one RTK Bash handler is
/// registered and an absolute executable path, if supplied, still exists.
pub fn codex_status() -> CodexHookStatus {
    let Some(home) = crate::core::user_dirs::home() else {
        return CodexHookStatus::MissingConfig;
    };
    let codex_home = crate::core::user_env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(super::constants::CODEX_DIR));
    codex_status_in(&codex_home)
}

/// Testable Codex hook status inspection for one config directory.
pub fn codex_status_in(codex_home: &Path) -> CodexHookStatus {
    let config_path = codex_home.join("config.toml");
    if !config_path.is_file() {
        return CodexHookStatus::MissingConfig;
    }
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return CodexHookStatus::InvalidConfig;
    };
    let Ok(root) = content.parse::<toml::Value>() else {
        return CodexHookStatus::InvalidConfig;
    };
    let Some(hooks) = root.get("hooks").and_then(toml::Value::as_table) else {
        return CodexHookStatus::MissingHook;
    };
    let Some(groups) = hooks.get("PreToolUse").and_then(toml::Value::as_array) else {
        return CodexHookStatus::MissingHook;
    };

    let mut rtk_hooks = Vec::new();
    for group in groups {
        let Some(group) = group.as_table() else {
            return CodexHookStatus::InvalidConfig;
        };
        if group.get("matcher").and_then(toml::Value::as_str) != Some("Bash") {
            continue;
        }
        let Some(handlers) = group.get("hooks").and_then(toml::Value::as_array) else {
            return CodexHookStatus::InvalidConfig;
        };
        for handler in handlers {
            let Some(handler) = handler.as_table() else {
                return CodexHookStatus::InvalidConfig;
            };
            let Some(command) = handler.get("command").and_then(toml::Value::as_str) else {
                continue;
            };
            if let Some(executable) = codex_hook_executable(command) {
                rtk_hooks.push(executable.to_string());
            }
        }
    }

    match rtk_hooks.as_slice() {
        [] => CodexHookStatus::MissingHook,
        [_first, _second, ..] => CodexHookStatus::ConflictingConfig,
        [executable] if codex_executable_missing(executable) => CodexHookStatus::MissingBinary,
        [_] => CodexHookStatus::Ready,
    }
}

fn codex_hook_executable(command: &str) -> Option<&str> {
    let executable = command.strip_suffix(" hook codex")?.trim();
    let executable = executable
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            executable
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(executable)
        .trim();
    let binary = executable.rsplit(['/', '\\']).next()?;
    if binary.eq_ignore_ascii_case("rtk") || binary.eq_ignore_ascii_case("rtk.exe") {
        Some(executable)
    } else {
        None
    }
}

fn codex_executable_missing(executable: &str) -> bool {
    if !executable.contains('/') && !executable.contains('\\') {
        // A bare `rtk`/`rtk.exe` is resolved by Codex through PATH; checking
        // it here would make the status depend on the checker's own PATH.
        return false;
    }
    !Path::new(executable).is_file()
}

pub(crate) fn claude_hook_installed_in(claude_dir: &std::path::Path) -> bool {
    binary_hook_registered(claude_dir)
        || claude_dir
            .join(HOOKS_SUBDIR)
            .join(REWRITE_HOOK_FILE)
            .is_file()
}

/// Return the current hook status without printing anything.
/// Returns `Ok` if no Claude Code is detected (not applicable).
/// Check if the native binary command is registered in settings.json
pub(crate) fn binary_hook_registered(claude_dir: &std::path::Path) -> bool {
    let settings_path = claude_dir.join(SETTINGS_JSON);
    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return false,
    };
    let root: serde_json::Value = match from_json_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let pre_tool_use = match root
        .get("hooks")
        .and_then(|h| h.get(PRE_TOOL_USE_KEY))
        .and_then(|p| p.as_array())
    {
        Some(arr) => arr,
        None => return false,
    };
    pre_tool_use
        .iter()
        .filter_map(|entry| entry.get("hooks")?.as_array())
        .flatten()
        .filter_map(|hook| hook.get("command")?.as_str())
        .any(is_claude_hook_command)
}

/// All non-Claude agent probes for active RTK hook/plugin installation.
/// Every agent owns its own probe inside `src/hooks/init/<agent>.rs`.
const AGENT_PROBES: &[fn() -> bool] = &[
    super::init::antigravity::is_configured,
    super::init::cursor::is_configured,
    super::init::gemini::is_configured,
    super::init::opencode::is_configured,
    super::init::codex::is_configured,
    super::init::droid::is_configured,
    super::init::trae::is_configured,
    super::init::hermes::is_configured,
    super::init::pi::is_configured,
    super::init::vibe::is_configured,
];

/// Returns true if at least one supported non-Claude agent has an active hook/plugin configured.
pub fn is_any_agent_configured() -> bool {
    AGENT_PROBES.iter().any(|probe| probe())
}

/// Returns the hook status for Claude Code without printing anything.
/// Returns `HookStatus::Ok` if Claude Code is not installed on this system.
///
/// Note: This probe is intentionally specific to Claude Code because `rtk discover`
/// and legacy transcript analysis rely specifically on Claude's hook status.
/// General warning checks also consult [`is_any_agent_configured`].
pub fn status() -> HookStatus {
    super::init::claude::hook_status()
}

/// Check if the installed hook is missing or outdated, warn once per day.
pub fn maybe_warn() {
    // Don't block startup — fail silently on any error
    let _ = check_and_warn();
}

/// Message to print for `status`, if any.
/// `suppress_missing` only hides [`HookStatus::Missing`]; outdated stays visible.
fn warning_text(status: HookStatus, suppress_missing: bool) -> Option<&'static str> {
    match status {
        HookStatus::Ok => None,
        HookStatus::Missing if suppress_missing => None,
        HookStatus::Missing => {
            Some("[rtk] /!\\ No hook installed — run `rtk init -g` for automatic token savings")
        }
        HookStatus::Outdated => Some("[rtk] /!\\ Hook outdated — run `rtk init -g` to update"),
    }
}

/// Single source of truth: delegates to `status()` then rate-limits the warning.
fn check_and_warn() -> Option<()> {
    // Probe first so the common HookStatus::Ok path never reads config.toml.
    // Suppression is consulted only when a missing-hook warning would print.
    let status = status();
    if status == HookStatus::Ok {
        return Some(());
    }
    let suppress_missing = status == HookStatus::Missing
        && (crate::core::config::hook_warning_suppressed() || is_any_agent_configured());
    let warning = warning_text(status, suppress_missing)?;

    // Rate limit: warn once per day
    let marker = warn_marker_path()?;
    if let Ok(meta) = std::fs::metadata(&marker)
        && let Ok(modified) = meta.modified()
        && modified.elapsed().map(|e| e.as_secs()).unwrap_or(u64::MAX) < WARN_INTERVAL_SECS
    {
        return Some(());
    }

    eprintln!("{}", warning);

    // Touch marker after warning is printed
    let _ = crate::core::utils::create_private_dir(marker.parent()?);
    let _ = std::fs::write(&marker, b"");

    Some(())
}

pub fn parse_hook_version(content: &str) -> u8 {
    // Version tag must be in the first 5 lines (shebang + header convention)
    for line in content.lines().take(5) {
        if let Some(rest) = line.strip_prefix("# rtk-hook-version:")
            && let Ok(v) = rest.trim().parse::<u8>()
        {
            return v;
        }
    }
    0 // No version tag = version 0 (outdated)
}

fn warn_marker_path() -> Option<PathBuf> {
    let data_dir = user_dirs::data()?;
    Some(data_dir.join(".hook_warn_last"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_isolation;
    use crate::core::user_env;
    use crate::hooks::constants::{
        CODEX_DIR, CONFIG_DIR, CURSOR_DIR, GEMINI_DIR, GEMINI_HOOK_FILE, HERMES_DIR,
        HERMES_PLUGIN_MANIFEST_FILE, HERMES_PLUGIN_NAME, HERMES_PLUGINS_SUBDIR,
        OPENCODE_PLUGIN_FILE, OPENCODE_SUBDIR, PLUGIN_SUBDIR,
    };

    #[test]
    fn test_parse_hook_version_present() {
        let content = "#!/usr/bin/env bash\n# rtk-hook-version: 2\n# some comment\n";
        assert_eq!(parse_hook_version(content), 2);
    }

    #[test]
    fn test_parse_hook_version_missing() {
        let content = "#!/usr/bin/env bash\n# old hook without version\n";
        assert_eq!(parse_hook_version(content), 0);
    }

    /// The shipped Claude hook script must carry the current version. `rtk init`
    /// no longer installs it, so the version grades copies already deployed:
    /// raising it reports older copies as outdated, which sends their owners to
    /// `rtk init -g` and from there to the in-process hook. The constant and the
    /// script move together.
    #[test]
    fn test_shipped_claude_hook_carries_the_current_version() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let content = std::fs::read_to_string(root.join("hooks/claude/rtk-rewrite.sh"))
            .expect("read hooks/claude/rtk-rewrite.sh");
        assert_eq!(
            parse_hook_version(&content),
            CURRENT_HOOK_VERSION,
            "hooks/claude/rtk-rewrite.sh and CURRENT_HOOK_VERSION disagree"
        );
    }

    #[test]
    fn test_parse_hook_version_future() {
        let content = "#!/usr/bin/env bash\n# rtk-hook-version: 5\n";
        assert_eq!(parse_hook_version(content), 5);
    }

    #[test]
    fn test_parse_hook_version_no_tag() {
        assert_eq!(parse_hook_version("no version here"), 0);
        assert_eq!(parse_hook_version(""), 0);
    }

    #[test]
    fn test_hook_status_enum() {
        assert_ne!(HookStatus::Ok, HookStatus::Missing);
        assert_ne!(HookStatus::Outdated, HookStatus::Missing);
        assert_eq!(HookStatus::Ok, HookStatus::Ok);
        // Clone works
        let s = HookStatus::Missing;
        assert_eq!(s.clone(), HookStatus::Missing);
    }

    #[test]
    fn test_binary_hook_registered_accepts_absolute_rtk_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(SETTINGS_JSON),
            r#"{
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [{
                            "type": "command",
                            "command": "/opt/homebrew/bin/rtk hook claude",
                            "timeout": 5
                        }]
                    }]
                }
            }"#,
        )
        .expect("write settings");

        assert!(binary_hook_registered(tmp.path()));
    }

    #[test]
    fn test_other_integration_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(!integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_opencode() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp
            .path()
            .join(CONFIG_DIR)
            .join(OPENCODE_SUBDIR)
            .join(PLUGIN_SUBDIR)
            .join(OPENCODE_PLUGIN_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"plugin").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_cursor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp
            .path()
            .join(CURSOR_DIR)
            .join(HOOKS_SUBDIR)
            .join(REWRITE_HOOK_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"hook").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_codex() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join(CODEX_DIR).join("AGENTS.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"unrelated agent instructions").unwrap();
        assert!(!integration_installed_in(tmp.path()));

        std::fs::write(&path, b"@RTK.md").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_codex_rtk_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join(CODEX_DIR).join("RTK.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"<!-- rtk-instructions v2 -->").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_gemini() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp
            .path()
            .join(GEMINI_DIR)
            .join(HOOKS_SUBDIR)
            .join(GEMINI_HOOK_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"hook").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_hermes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp
            .path()
            .join(HERMES_DIR)
            .join(HERMES_PLUGINS_SUBDIR)
            .join(HERMES_PLUGIN_NAME)
            .join(HERMES_PLUGIN_MANIFEST_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"plugin").unwrap();
        assert!(integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_other_integration_empty_dirs_not_enough() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(CURSOR_DIR).join(HOOKS_SUBDIR)).unwrap();
        std::fs::create_dir_all(tmp.path().join(CODEX_DIR)).unwrap();
        std::fs::create_dir_all(tmp.path().join(GEMINI_DIR)).unwrap();
        std::fs::create_dir_all(
            tmp.path()
                .join(HERMES_DIR)
                .join(HERMES_PLUGINS_SUBDIR)
                .join(HERMES_PLUGIN_NAME),
        )
        .unwrap();
        assert!(!integration_installed_in(tmp.path()));
    }

    #[test]
    fn test_codex_status_requires_a_valid_bash_hook_and_binary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let rtk = tmp
            .path()
            .join(if cfg!(windows) { "rtk.exe" } else { "rtk" });
        std::fs::write(&rtk, b"rtk").expect("write fake rtk");
        let rtk_toml_path = rtk.to_string_lossy().replace('\\', "/");
        let config = format!(
            "[mcp_servers.rtk]\ncommand = \"{}\"\nargs = [\"mcp\"]\n\n[hooks]\n[[hooks.PreToolUse]]\nmatcher = \"Bash\"\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = \"{} hook codex\"\n",
            rtk_toml_path, rtk_toml_path
        );
        std::fs::write(tmp.path().join("config.toml"), config).expect("write config");

        assert_eq!(codex_status_in(tmp.path()), CodexHookStatus::Ready);
    }

    #[test]
    fn test_codex_status_reports_missing_binary_and_duplicate_hooks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("config.toml"),
            "[hooks]\n[[hooks.PreToolUse]]\nmatcher = \"Bash\"\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = \"C:\\\\missing\\\\rtk.exe hook codex\"\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = \"rtk hook codex\"\n",
        )
        .expect("write config");

        assert_eq!(
            codex_status_in(tmp.path()),
            CodexHookStatus::ConflictingConfig
        );
    }

    #[test]
    fn test_codex_status_reports_missing_config_and_hook() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(codex_status_in(tmp.path()), CodexHookStatus::MissingConfig);

        std::fs::write(tmp.path().join("config.toml"), "model = \"gpt-test\"\n")
            .expect("write config");
        assert_eq!(codex_status_in(tmp.path()), CodexHookStatus::MissingHook);
    }

    #[test]
    fn test_warning_text_scopes_suppression_to_missing() {
        assert_eq!(warning_text(HookStatus::Ok, false), None);
        assert_eq!(warning_text(HookStatus::Ok, true), None);
        assert!(
            warning_text(HookStatus::Missing, false).is_some(),
            "missing hook must warn when the flag is off"
        );
        assert_eq!(
            warning_text(HookStatus::Missing, true),
            None,
            "suppress_hook_warning must hide HookStatus::Missing"
        );
        assert!(
            warning_text(HookStatus::Outdated, false).is_some(),
            "outdated hook must warn when the flag is off"
        );
        assert!(
            warning_text(HookStatus::Outdated, true).is_some(),
            "suppress_hook_warning must not hide the outdated-hook upgrade prompt"
        );
    }

    #[test]
    fn test_status_returns_valid_variant() {
        // `status()` resolves through `CLAUDE_CONFIG_DIR`; pinned so both
        // states can be asserted.
        let tmp = test_isolation::tempdir();
        let claude_dir = tmp.path().join(".claude");
        user_env::with_path("CLAUDE_CONFIG_DIR", Some(&claude_dir), || {
            assert_eq!(
                status(),
                HookStatus::Ok,
                "no Claude dir: nothing to warn about"
            );
            std::fs::create_dir_all(&claude_dir).expect("create Claude dir");
            assert_eq!(
                status(),
                HookStatus::Missing,
                "a Claude dir with no rtk hook"
            );
        });
    }

    #[test]
    fn test_probes_ignore_foreign_hooks_in_agent_files() {
        let tmp = test_isolation::tempdir();
        let home = tmp.path().join("home");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(home.join(".gemini")).expect("create .gemini");
        std::fs::create_dir_all(home.join(".vibe")).expect("create .vibe");
        std::fs::create_dir_all(&ws).expect("create ws");

        test_isolation::with_root(&home, || {
            let _entered = test_isolation::enter(&ws);
            std::fs::write(
                home.join(".gemini/settings.json"),
                r#"{"hooks":{"BeforeTool":[{"matcher":"run_shell_command","hooks":[{"type":"command","command":"/usr/local/bin/audit-shell"}]}]}}"#,
            )
            .expect("write gemini settings");
            std::fs::write(
                home.join(".vibe/hooks.toml"),
                "[[hooks]]\nname = \"audit\"\ntype = \"pre_tool\"\nmatch = \"bash\"\ncommand = \"rtk-audit\"\n",
            )
            .expect("write vibe hooks");

            assert!(
                !super::super::init::gemini::is_configured(),
                "a non-rtk BeforeTool hook is not an rtk install"
            );
            assert!(
                !super::super::init::vibe::is_configured(),
                "a Vibe hook not named rtk-rewrite is not an rtk install"
            );
            assert!(!is_any_agent_configured(), "no agent is configured");
        });
    }

    #[test]
    fn test_check_and_warn_suppressed_when_agent_configured() {
        let tmp = test_isolation::tempdir();
        let home = tmp.path().join("home");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(home.join(".claude")).expect("create .claude");
        std::fs::create_dir_all(&ws).expect("create ws");

        test_isolation::with_root(&home, || {
            let _entered = test_isolation::enter(&ws);
            let marker = warn_marker_path().expect("marker path");

            // With OpenCode plugin present, check_and_warn must not warn or touch marker
            let opencode_plugin = home.join(".config/opencode/plugins/rtk.ts");
            std::fs::create_dir_all(opencode_plugin.parent().unwrap()).unwrap();
            std::fs::write(&opencode_plugin, b"export default {}").unwrap();

            check_and_warn();
            assert!(
                !marker.exists(),
                "check_and_warn must not warn when an agent is configured"
            );

            // Once removed, check_and_warn warns and touches the daily marker
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(&opencode_plugin).unwrap();
            check_and_warn();
            assert!(
                marker.exists(),
                "check_and_warn must warn and write marker when no agent is configured"
            );
        });
    }

    #[test]
    fn test_all_agent_probes_and_arms_in_process() {
        use crate::hooks::init::{InitContext, PatchMode};

        // Helper that runs `f(home, ws)` inside an isolated home + workspace
        let run_isolated = |f: &dyn Fn(&std::path::Path, &std::path::Path)| {
            let tmp = test_isolation::tempdir();
            let home = tmp.path().join("home");
            let ws = tmp.path().join("ws");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&ws).unwrap();
            test_isolation::with_root(&home, || {
                let _entered = test_isolation::enter(&ws);
                assert!(
                    !is_any_agent_configured(),
                    "precondition: fresh isolated env has no configured agent"
                );
                f(&home, &ws);
            });
        };

        // 1. Antigravity (local & global)
        run_isolated(&|_home, _ws| {
            super::super::init::antigravity::run_antigravity_mode(false, InitContext::default())
                .unwrap();
            assert!(is_any_agent_configured(), "antigravity local");
            super::super::init::antigravity::uninstall_antigravity_mode(
                false,
                InitContext::default(),
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "antigravity local uninstalled");

            super::super::init::antigravity::run_antigravity_mode(true, InitContext::default())
                .unwrap();
            assert!(is_any_agent_configured(), "antigravity global");
            super::super::init::antigravity::uninstall_antigravity_mode(
                true,
                InitContext::default(),
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "antigravity global uninstalled");
        });

        // 2. Cursor (configured vs unregistered script)
        run_isolated(&|home, _ws| {
            let cursor_hooks = home.join(".cursor/hooks");
            std::fs::create_dir_all(&cursor_hooks).unwrap();
            std::fs::write(cursor_hooks.join("rtk-rewrite.sh"), "#!/bin/sh\n").unwrap();
            assert!(
                !is_any_agent_configured(),
                "unregistered cursor script must not count as configured"
            );
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(cursor_hooks.join("rtk-rewrite.sh")).unwrap();

            std::fs::write(
                home.join(".cursor/hooks.json"),
                r#"{"version":1,"hooks":{"preToolUse":[{"matcher":"Shell","command":"rtk hook cursor"}]}}"#,
            )
            .unwrap();
            assert!(is_any_agent_configured(), "cursor hooks.json");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(home.join(".cursor/hooks.json")).unwrap();
            assert!(!is_any_agent_configured(), "cursor removed");
        });

        // 3. Gemini (run_shell_command, wildcard *, non-shell matcher, unregistered script)
        run_isolated(&|home, _ws| {
            super::super::init::gemini::run_gemini(
                true,
                false,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "gemini init");

            // Wildcard matcher
            std::fs::write(
                home.join(".gemini/settings.json"),
                r#"{"hooks":{"BeforeTool":[{"matcher":"*","hooks":[{"command":"rtk hook gemini"}]}]}}"#,
            )
            .unwrap();
            assert!(is_any_agent_configured(), "gemini wildcard matcher");

            // Non-shell matcher must be rejected
            std::fs::write(
                home.join(".gemini/settings.json"),
                r#"{"hooks":{"BeforeTool":[{"matcher":"ReadFile","hooks":[{"command":"rtk hook gemini"}]}]}}"#,
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "gemini non-shell matcher");

            // Unregistered script file alone must be rejected
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(home.join(".gemini/settings.json")).unwrap();
            let gemini_hooks = home.join(".gemini/hooks");
            std::fs::create_dir_all(&gemini_hooks).unwrap();
            std::fs::write(gemini_hooks.join("rtk-hook-gemini.sh"), "#!/bin/sh\n").unwrap();
            assert!(!is_any_agent_configured(), "gemini unregistered script");
        });

        // 4. Codex
        run_isolated(&|home, _ws| {
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            std::fs::write(
                home.join(".codex/hooks.json"),
                r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rtk hook codex"}]}]}}"#,
            )
            .unwrap();
            assert!(is_any_agent_configured(), "codex configured");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(home.join(".codex/hooks.json")).unwrap();
            assert!(!is_any_agent_configured(), "codex removed");
        });

        // 5. Droid
        run_isolated(&|_home, _ws| {
            super::super::init::droid::run_droid_mode(true, InitContext::default()).unwrap();
            assert!(is_any_agent_configured(), "droid global");
            super::super::init::droid::uninstall_droid(true, InitContext::default()).unwrap();
            assert!(!is_any_agent_configured(), "droid uninstalled");
        });

        // 6. Trae (local & global)
        run_isolated(&|_home, _ws| {
            super::super::init::trae::run_trae_mode(false, InitContext::default()).unwrap();
            assert!(is_any_agent_configured(), "trae local");
            super::super::init::trae::uninstall_trae_mode(false, InitContext::default()).unwrap();
            assert!(!is_any_agent_configured(), "trae local uninstalled");

            super::super::init::trae::run_trae_mode(true, InitContext::default()).unwrap();
            assert!(is_any_agent_configured(), "trae global");
            super::super::init::trae::uninstall_trae_mode(true, InitContext::default()).unwrap();
            assert!(!is_any_agent_configured(), "trae global uninstalled");
        });

        // 7. Hermes (full init/uninstall + individual arms)
        run_isolated(&|home, _ws| {
            super::super::init::hermes::run_hermes_mode(InitContext::default()).unwrap();
            assert!(is_any_agent_configured(), "hermes init");
            super::super::init::hermes::uninstall_hermes(InitContext::default()).unwrap();
            assert!(!is_any_agent_configured(), "hermes uninstalled");

            // Plugin file arm alone
            let plugin_init = home.join(".hermes/plugins/rtk-rewrite/__init__.py");
            std::fs::create_dir_all(plugin_init.parent().unwrap()).unwrap();
            std::fs::write(&plugin_init, "# plugin\n").unwrap();
            assert!(is_any_agent_configured(), "hermes plugin file arm");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(&plugin_init).unwrap();
            assert!(!is_any_agent_configured(), "hermes plugin file removed");

            // config.yaml arm alone
            std::fs::write(
                home.join(".hermes/config.yaml"),
                "plugins:\n  enabled:\n    - rtk-rewrite\n",
            )
            .unwrap();
            assert!(is_any_agent_configured(), "hermes config.yaml arm");
            std::fs::write(
                home.join(".hermes/config.yaml"),
                "plugins:\n  enabled:\n    - other\n",
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "hermes unrelated config.yaml");
        });

        // 8. Pi & OMP (local & global)
        run_isolated(&|_home, _ws| {
            super::super::init::pi::run_pi_mode_with_patch_mode(
                false,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "pi local");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(user_dirs::in_working_dir(".pi/extensions/rtk.ts")).unwrap();
            assert!(!is_any_agent_configured(), "pi local removed");

            super::super::init::pi::run_pi_mode_with_patch_mode(
                true,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "pi global");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(
                user_dirs::home()
                    .unwrap()
                    .join(".pi/agent/extensions/rtk.ts"),
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "pi global removed");

            super::super::init::pi::run_omp_mode_with_patch_mode(
                false,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "omp local");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(user_dirs::in_working_dir(".omp/extensions/rtk.ts")).unwrap();
            assert!(!is_any_agent_configured(), "omp local removed");

            super::super::init::pi::run_omp_mode_with_patch_mode(
                true,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "omp global");
            // nosemgrep: filesystem-deletion -- Test-only removal of fixtures inside the isolated temporary profile.
            std::fs::remove_file(
                user_dirs::home()
                    .unwrap()
                    .join(".omp/agent/extensions/rtk.ts"),
            )
            .unwrap();
            assert!(!is_any_agent_configured(), "omp global removed");
        });

        // 9. Vibe
        run_isolated(&|_home, _ws| {
            super::super::init::vibe::run_vibe_mode(
                true,
                false,
                PatchMode::Auto,
                InitContext::default(),
            )
            .unwrap();
            assert!(is_any_agent_configured(), "vibe global");
            super::super::init::vibe::uninstall_vibe(InitContext::default()).unwrap();
            assert!(!is_any_agent_configured(), "vibe uninstalled");
        });
    }
}
