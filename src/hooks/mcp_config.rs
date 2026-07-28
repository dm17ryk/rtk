//! MCP client registration installed alongside each RTK agent integration.
//!
//! Every registration launches the currently running RTK executable with the
//! `mcp` subcommand over stdio. Config edits are scoped to the `rtk` server
//! entry, preserve unrelated servers, and use atomic writes.

use crate::hooks::init::InitContext;
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const SERVER_NAME: &str = "rtk";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClient {
    Claude,
    Gemini,
    Codex,
    Copilot,
    Cursor,
    Windsurf,
    Cline,
    Kilocode,
    Antigravity,
    Kimi,
    Pi,
    Hermes,
    Droid,
    OpenCode,
}

impl McpClient {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Gemini => "Gemini CLI",
            Self::Codex => "Codex CLI",
            Self::Copilot => "GitHub Copilot",
            Self::Cursor => "Cursor",
            Self::Windsurf => "Windsurf",
            Self::Cline => "Cline / Roo Code",
            Self::Kilocode => "Kilo Code",
            Self::Antigravity => "Google Antigravity",
            Self::Kimi => "Kimi Code",
            Self::Pi => "Pi",
            Self::Hermes => "Hermes",
            Self::Droid => "Factory Droid",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Debug, Clone)]
struct McpEnvironment {
    home: PathBuf,
    cwd: PathBuf,
    config_dir: PathBuf,
    vscode_user_dir: PathBuf,
    rtk_exe: PathBuf,
    codex_home: Option<PathBuf>,
    copilot_home: Option<PathBuf>,
    hermes_home: Option<PathBuf>,
    factory_home_override: Option<PathBuf>,
    kimi_code_home: Option<PathBuf>,
}

impl McpEnvironment {
    fn discover() -> Result<Self> {
        let home = dirs::home_dir().context(if cfg!(windows) {
            "Cannot determine home directory. Is %USERPROFILE% set?"
        } else {
            "Cannot determine home directory. Is $HOME set?"
        })?;
        let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));

        Ok(Self {
            vscode_user_dir: config_dir.join("Code").join("User"),
            home,
            cwd: std::env::current_dir().context("Cannot determine current directory")?,
            config_dir,
            rtk_exe: std::env::current_exe()
                .context("Cannot determine the installed RTK executable path")?,
            codex_home: nonempty_env_path("CODEX_HOME"),
            copilot_home: nonempty_env_path("COPILOT_HOME"),
            hermes_home: nonempty_env_path("HERMES_HOME"),
            factory_home_override: nonempty_env_path("FACTORY_HOME_OVERRIDE"),
            kimi_code_home: nonempty_env_path("KIMI_CODE_HOME"),
        })
    }

    fn codex_dir(&self, global: bool) -> PathBuf {
        if global {
            self.codex_home
                .clone()
                .unwrap_or_else(|| self.home.join(".codex"))
        } else {
            self.cwd.join(".codex")
        }
    }

    fn copilot_dir(&self) -> PathBuf {
        self.copilot_home
            .clone()
            .unwrap_or_else(|| self.home.join(".copilot"))
    }

    fn hermes_dir(&self) -> PathBuf {
        self.hermes_home
            .clone()
            .unwrap_or_else(|| self.home.join(".hermes"))
    }

    fn droid_dir(&self, global: bool) -> PathBuf {
        if global {
            self.factory_home_override
                .clone()
                .unwrap_or_else(|| self.home.clone())
                .join(".factory")
        } else {
            self.cwd.join(".factory")
        }
    }
}

fn nonempty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Standard,
    Copilot,
    VsCode,
    OpenCode,
}

#[derive(Debug, Clone)]
enum Destination {
    Json {
        label: &'static str,
        path: PathBuf,
        servers_key: &'static str,
        kind: EntryKind,
    },
    Toml {
        label: &'static str,
        path: PathBuf,
    },
    HermesYaml {
        label: &'static str,
        path: PathBuf,
    },
}

impl Destination {
    fn label(&self) -> &'static str {
        match self {
            Self::Json { label, .. }
            | Self::Toml { label, .. }
            | Self::HermesYaml { label, .. } => label,
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Json { path, .. } | Self::Toml { path, .. } | Self::HermesYaml { path, .. } => {
                path
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteState {
    Created,
    Updated,
    Unchanged,
    WouldCreate,
    WouldUpdate,
    Removed,
    WouldRemove,
    NotFound,
}

impl WriteState {
    fn description(self) -> &'static str {
        match self {
            Self::Created => "installed",
            Self::Updated => "updated",
            Self::Unchanged => "already up to date",
            Self::WouldCreate => "would install",
            Self::WouldUpdate => "would update",
            Self::Removed => "removed",
            Self::WouldRemove => "would remove",
            Self::NotFound => "not installed",
        }
    }
}

pub fn install(client: McpClient, global: bool, ctx: InitContext) -> Result<()> {
    let env = McpEnvironment::discover()?;
    install_with_env(client, global, ctx, &env)
}

fn install_with_env(
    client: McpClient,
    global: bool,
    ctx: InitContext,
    env: &McpEnvironment,
) -> Result<()> {
    let destinations = destinations(client, global, env);
    debug_log(&format!(
        "install client={} scope={} destinations={}",
        client.display_name(),
        if global { "global" } else { "project" },
        destinations.len()
    ));
    if destinations.is_empty() {
        println!(
            "  MCP: {} has no native MCP client; the RTK Pi extension is the complete integration.",
            client.display_name()
        );
        return Ok(());
    }

    println!("\nRTK MCP registration for {}:", client.display_name());
    for destination in destinations {
        debug_log(&format!(
            "install destination={} path={}",
            destination.label(),
            destination.path().display()
        ));
        let state = install_destination(&destination, &env.rtk_exe, ctx)?;
        debug_log(&format!(
            "install result destination={} state={}",
            destination.label(),
            state.description()
        ));
        println!(
            "  {}: {} ({})",
            destination.label(),
            destination.path().display(),
            state.description()
        );
    }
    println!(
        "  Server: {} mcp (local stdio; trusted clients only)",
        env.rtk_exe.display()
    );
    Ok(())
}

pub fn uninstall(client: McpClient, global: bool, ctx: InitContext) -> Result<()> {
    let env = McpEnvironment::discover()?;
    uninstall_with_env(client, global, ctx, &env)
}

fn uninstall_with_env(
    client: McpClient,
    global: bool,
    ctx: InitContext,
    env: &McpEnvironment,
) -> Result<()> {
    let destinations = destinations(client, global, env);
    debug_log(&format!(
        "uninstall client={} scope={} destinations={}",
        client.display_name(),
        if global { "global" } else { "project" },
        destinations.len()
    ));
    if destinations.is_empty() {
        return Ok(());
    }

    println!("\nRTK MCP cleanup for {}:", client.display_name());
    for destination in destinations {
        debug_log(&format!(
            "uninstall destination={} path={}",
            destination.label(),
            destination.path().display()
        ));
        let state = uninstall_destination(&destination, ctx)?;
        debug_log(&format!(
            "uninstall result destination={} state={}",
            destination.label(),
            state.description()
        ));
        println!(
            "  {}: {} ({})",
            destination.label(),
            destination.path().display(),
            state.description()
        );
    }
    Ok(())
}

fn destinations(client: McpClient, global: bool, env: &McpEnvironment) -> Vec<Destination> {
    let standard_json =
        |label: &'static str, path: PathBuf, servers_key: &'static str| Destination::Json {
            label,
            path,
            servers_key,
            kind: EntryKind::Standard,
        };

    match client {
        McpClient::Claude => vec![standard_json(
            "MCP config",
            if global {
                env.home.join(".claude.json")
            } else {
                env.cwd.join(".mcp.json")
            },
            "mcpServers",
        )],
        McpClient::Gemini => vec![standard_json(
            "MCP config",
            if global {
                env.home.join(".gemini").join("settings.json")
            } else {
                env.cwd.join(".gemini").join("settings.json")
            },
            "mcpServers",
        )],
        McpClient::Codex => vec![Destination::Toml {
            label: "MCP config",
            path: env.codex_dir(global).join("config.toml"),
        }],
        McpClient::Copilot if global => vec![
            Destination::Json {
                label: "Copilot CLI MCP config",
                path: env.copilot_dir().join("mcp-config.json"),
                servers_key: "mcpServers",
                kind: EntryKind::Copilot,
            },
            Destination::Json {
                label: "VS Code MCP config",
                path: env.vscode_user_dir.join("mcp.json"),
                servers_key: "servers",
                kind: EntryKind::VsCode,
            },
        ],
        McpClient::Copilot => vec![Destination::Json {
            label: "VS Code MCP config",
            path: env.cwd.join(".vscode").join("mcp.json"),
            servers_key: "servers",
            kind: EntryKind::VsCode,
        }],
        McpClient::Cursor => vec![standard_json(
            "MCP config",
            env.home.join(".cursor").join("mcp.json"),
            "mcpServers",
        )],
        McpClient::Windsurf => vec![standard_json(
            "MCP config",
            env.home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            "mcpServers",
        )],
        McpClient::Cline => vec![
            standard_json(
                "Cline MCP config",
                env.vscode_user_dir
                    .join("globalStorage")
                    .join("saoudrizwan.claude-dev")
                    .join("settings")
                    .join("cline_mcp_settings.json"),
                "mcpServers",
            ),
            standard_json(
                "Roo Code MCP config",
                env.vscode_user_dir
                    .join("globalStorage")
                    .join("rooveterinaryinc.roo-cline")
                    .join("settings")
                    .join("mcp_settings.json"),
                "mcpServers",
            ),
        ],
        McpClient::Kilocode => vec![standard_json(
            "MCP config",
            env.vscode_user_dir
                .join("globalStorage")
                .join("kilocode.kilo-code")
                .join("settings")
                .join("mcp_settings.json"),
            "mcpServers",
        )],
        McpClient::Antigravity => vec![standard_json(
            "MCP config",
            if global {
                env.home
                    .join(".gemini")
                    .join("config")
                    .join("mcp_config.json")
            } else {
                env.cwd.join(".agents").join("mcp_config.json")
            },
            "mcpServers",
        )],
        McpClient::Kimi => vec![standard_json(
            "MCP config",
            if global {
                env.kimi_code_home
                    .clone()
                    .unwrap_or_else(|| env.home.join(".kimi-code"))
                    .join("mcp.json")
            } else {
                env.cwd.join(".kimi-code").join("mcp.json")
            },
            "mcpServers",
        )],
        McpClient::Pi => Vec::new(),
        McpClient::Hermes => vec![Destination::HermesYaml {
            label: "MCP config",
            path: env.hermes_dir().join("config.yaml"),
        }],
        McpClient::Droid => vec![standard_json(
            "MCP config",
            env.droid_dir(global).join("mcp.json"),
            "mcpServers",
        )],
        McpClient::OpenCode => vec![Destination::Json {
            label: "MCP config",
            path: env.config_dir.join("opencode").join("opencode.json"),
            servers_key: "mcp",
            kind: EntryKind::OpenCode,
        }],
    }
}

fn install_destination(
    destination: &Destination,
    rtk_exe: &Path,
    ctx: InitContext,
) -> Result<WriteState> {
    let new_content = match destination {
        Destination::Json {
            path,
            servers_key,
            kind,
            ..
        } => patch_json(path, servers_key, entry(*kind, rtk_exe))?,
        Destination::Toml { path, .. } => patch_toml(path, rtk_exe)?,
        Destination::HermesYaml { path, .. } => patch_hermes_yaml(path, rtk_exe)?,
    };
    write_changed(destination.path(), &new_content, ctx)
}

fn uninstall_destination(destination: &Destination, ctx: InitContext) -> Result<WriteState> {
    let path = destination.path();
    if !path.exists() {
        return Ok(WriteState::NotFound);
    }

    let changed = match destination {
        Destination::Json {
            servers_key, path, ..
        } => remove_json(path, servers_key)?,
        Destination::Toml { path, .. } => remove_toml(path)?,
        Destination::HermesYaml { path, .. } => remove_hermes_yaml(path)?,
    };

    let Some(new_content) = changed else {
        return Ok(WriteState::NotFound);
    };
    if ctx.dry_run {
        return Ok(WriteState::WouldRemove);
    }
    atomic_write(path, &new_content)?;
    Ok(WriteState::Removed)
}

fn entry(kind: EntryKind, rtk_exe: &Path) -> Value {
    let command = rtk_exe.to_string_lossy();
    match kind {
        EntryKind::Standard => json!({
            "command": command,
            "args": ["mcp"]
        }),
        EntryKind::Copilot => json!({
            "type": "local",
            "command": command,
            "args": ["mcp"],
            "tools": ["*"]
        }),
        EntryKind::VsCode => json!({
            "type": "stdio",
            "command": command,
            "args": ["mcp"]
        }),
        EntryKind::OpenCode => json!({
            "type": "local",
            "command": [command, "mcp"],
            "enabled": true
        }),
    }
}

fn read_optional(path: &Path) -> Result<String> {
    if path.exists() {
        fs::read_to_string(path)
            .with_context(|| format!("Failed to read MCP config: {}", path.display()))
    } else {
        Ok(String::new())
    }
}

fn parse_json_object(path: &Path) -> Result<Value> {
    let content = read_optional(path)?;
    if content.trim().is_empty() {
        debug_log(&format!(
            "JSON config branch=empty-object path={}",
            path.display()
        ));
        return Ok(Value::Object(Map::new()));
    }
    debug_log(&format!(
        "JSON config branch=parse-existing path={}",
        path.display()
    ));
    let mut normalized = normalize_jsonc(&content);
    if normalized.trim().is_empty() {
        normalized = "{}".to_string();
    }
    let value: Value = serde_json::from_str(&normalized)
        .with_context(|| format!("Invalid JSON in MCP config: {}", path.display()))?;
    if !value.is_object() {
        anyhow::bail!("MCP config root must be a JSON object: {}", path.display());
    }
    Ok(value)
}

fn normalize_jsonc(content: &str) -> String {
    let mut without_comments = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(character) = chars.next() {
        if escaped {
            without_comments.push(character);
            escaped = false;
            continue;
        }
        if in_string {
            if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            without_comments.push(character);
            continue;
        }

        match character {
            '"' => {
                in_string = true;
                without_comments.push(character);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for comment_character in chars.by_ref() {
                    if comment_character == '\n' {
                        without_comments.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for comment_character in chars.by_ref() {
                    if previous == '*' && comment_character == '/' {
                        break;
                    }
                    previous = comment_character;
                }
            }
            _ => without_comments.push(character),
        }
    }

    let characters: Vec<char> = without_comments.chars().collect();
    let mut normalized = String::with_capacity(without_comments.len());
    in_string = false;
    escaped = false;
    for (index, character) in characters.iter().copied().enumerate() {
        if escaped {
            normalized.push(character);
            escaped = false;
            continue;
        }
        if in_string {
            if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            normalized.push(character);
            continue;
        }
        if character == '"' {
            in_string = true;
            normalized.push(character);
            continue;
        }
        if character == ',' {
            let next = characters[index + 1..]
                .iter()
                .copied()
                .find(|candidate| !candidate.is_whitespace());
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        normalized.push(character);
    }
    normalized
}

fn patch_json(path: &Path, servers_key: &str, server_entry: Value) -> Result<String> {
    let mut root = parse_json_object(path)?;
    debug_log(&format!(
        "JSON config branch=upsert key={servers_key}.{SERVER_NAME} path={}",
        path.display()
    ));
    let servers = root
        .as_object_mut()
        .expect("JSON root was validated as an object")
        .entry(servers_key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .with_context(|| {
            format!(
                "`{servers_key}` must be a JSON object in {}",
                path.display()
            )
        })?;
    servers.insert(SERVER_NAME.to_string(), server_entry);
    let mut output = serde_json::to_string_pretty(&root)?;
    output.push('\n');
    Ok(output)
}

fn remove_json(path: &Path, servers_key: &str) -> Result<Option<String>> {
    let mut root = parse_json_object(path)?;
    let Some(servers) = root
        .as_object_mut()
        .and_then(|object| object.get_mut(servers_key))
        .and_then(Value::as_object_mut)
    else {
        debug_log(&format!(
            "JSON config branch=missing-server-map key={servers_key} path={}",
            path.display()
        ));
        return Ok(None);
    };
    if servers.remove(SERVER_NAME).is_none() {
        debug_log(&format!(
            "JSON config branch=missing-rtk-entry key={servers_key} path={}",
            path.display()
        ));
        return Ok(None);
    }
    if servers.is_empty() {
        root.as_object_mut()
            .expect("JSON root was validated as an object")
            .remove(servers_key);
    }
    let mut output = serde_json::to_string_pretty(&root)?;
    output.push('\n');
    Ok(Some(output))
}

fn patch_toml(path: &Path, rtk_exe: &Path) -> Result<String> {
    let content = read_optional(path)?;
    debug_log(&format!(
        "TOML config branch={} path={}",
        if content.trim().is_empty() {
            "empty-table"
        } else {
            "parse-existing"
        },
        path.display()
    ));
    let mut root = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        content
            .parse::<toml::Value>()
            .with_context(|| format!("Invalid TOML in MCP config: {}", path.display()))?
    };
    let root_table = root
        .as_table_mut()
        .with_context(|| format!("MCP config root must be a TOML table: {}", path.display()))?;
    let servers = root_table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .with_context(|| format!("`mcp_servers` must be a TOML table in {}", path.display()))?;
    let mut server = toml::map::Map::new();
    server.insert(
        "command".to_string(),
        toml::Value::String(rtk_exe.to_string_lossy().into_owned()),
    );
    server.insert(
        "args".to_string(),
        toml::Value::Array(vec![toml::Value::String("mcp".to_string())]),
    );
    servers.insert(SERVER_NAME.to_string(), toml::Value::Table(server));
    toml::to_string_pretty(&root).context("Failed to serialize Codex MCP config")
}

fn remove_toml(path: &Path) -> Result<Option<String>> {
    let content = read_optional(path)?;
    let mut root = content
        .parse::<toml::Value>()
        .with_context(|| format!("Invalid TOML in MCP config: {}", path.display()))?;
    let Some(root_table) = root.as_table_mut() else {
        anyhow::bail!("MCP config root must be a TOML table: {}", path.display());
    };
    let Some(servers) = root_table
        .get_mut("mcp_servers")
        .and_then(toml::Value::as_table_mut)
    else {
        return Ok(None);
    };
    if servers.remove(SERVER_NAME).is_none() {
        return Ok(None);
    }
    if servers.is_empty() {
        root_table.remove("mcp_servers");
    }
    Ok(Some(toml::to_string_pretty(&root)?))
}

fn patch_hermes_yaml(path: &Path, rtk_exe: &Path) -> Result<String> {
    let content = read_optional(path)?;
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let entry = hermes_entry(rtk_exe);

    if let Some((section_start, section_end)) = top_level_yaml_section(&lines, "mcp_servers:") {
        if let Some((child_start, child_end)) =
            yaml_child_range(&lines, section_start, section_end, "rtk:")
        {
            debug_log(&format!(
                "Hermes YAML branch=replace-existing-rtk path={}",
                path.display()
            ));
            lines.splice(child_start..child_end, entry);
        } else {
            debug_log(&format!(
                "Hermes YAML branch=append-to-existing-section path={}",
                path.display()
            ));
            lines.splice(section_end..section_end, entry);
        }
    } else {
        debug_log(&format!(
            "Hermes YAML branch=create-section path={}",
            path.display()
        ));
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("mcp_servers:".to_string());
        lines.extend(entry);
    }
    Ok(with_trailing_newline(lines))
}

fn remove_hermes_yaml(path: &Path) -> Result<Option<String>> {
    let content = read_optional(path)?;
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let Some((section_start, section_end)) = top_level_yaml_section(&lines, "mcp_servers:") else {
        return Ok(None);
    };
    let Some((child_start, child_end)) =
        yaml_child_range(&lines, section_start, section_end, "rtk:")
    else {
        return Ok(None);
    };
    lines.drain(child_start..child_end);

    let remaining_section_end = top_level_yaml_section(&lines, "mcp_servers:")
        .map(|(_, end)| end)
        .unwrap_or(section_start + 1);
    let has_other_children = lines[section_start + 1..remaining_section_end]
        .iter()
        .any(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'));
    if !has_other_children {
        lines.remove(section_start);
        if section_start < lines.len() && lines[section_start].trim().is_empty() {
            lines.remove(section_start);
        }
    }
    Ok(Some(with_trailing_newline(lines)))
}

fn hermes_entry(rtk_exe: &Path) -> Vec<String> {
    let quoted = serde_json::to_string(&rtk_exe.to_string_lossy())
        .expect("serializing a path string cannot fail");
    vec![
        "  rtk:".to_string(),
        format!("    command: {quoted}"),
        "    args: [\"mcp\"]".to_string(),
    ]
}

fn top_level_yaml_section(lines: &[String], name: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|line| line == name)?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| {
            !line.trim().is_empty()
                && !line.trim_start().starts_with('#')
                && !line.starts_with([' ', '\t'])
        })
        .map_or(lines.len(), |(index, _)| index);
    Some((start, end))
}

fn yaml_child_range(
    lines: &[String],
    section_start: usize,
    section_end: usize,
    child: &str,
) -> Option<(usize, usize)> {
    let start = (section_start + 1..section_end).find(|&index| {
        lines[index]
            .strip_prefix("  ")
            .is_some_and(|line| !line.starts_with([' ', '\t']) && line.trim_end() == child)
    })?;
    let end = (start + 1..section_end)
        .find(|&index| {
            let line = &lines[index];
            !line.trim().is_empty()
                && !line.trim_start().starts_with('#')
                && line
                    .strip_prefix("  ")
                    .is_none_or(|rest| !rest.starts_with([' ', '\t']))
        })
        .unwrap_or(section_end);
    Some((start, end))
}

fn with_trailing_newline(lines: Vec<String>) -> String {
    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn write_changed(path: &Path, content: &str, ctx: InitContext) -> Result<WriteState> {
    let existed = path.exists();
    if existed && read_optional(path)? == content {
        debug_log(&format!("write branch=unchanged path={}", path.display()));
        return Ok(WriteState::Unchanged);
    }
    if ctx.dry_run {
        debug_log(&format!(
            "write branch={} path={}",
            if existed {
                "dry-run-update"
            } else {
                "dry-run-create"
            },
            path.display()
        ));
        return Ok(if existed {
            WriteState::WouldUpdate
        } else {
            WriteState::WouldCreate
        });
    }
    debug_log(&format!(
        "write branch={} path={}",
        if existed {
            "atomic-update"
        } else {
            "atomic-create"
        },
        path.display()
    ));
    atomic_write(path, content)?;
    Ok(if existed {
        WriteState::Updated
    } else {
        WriteState::Created
    })
}

fn debug_log(message: &str) {
    if crate::service::debug_enabled() {
        eprintln!("[rtk:debug:mcp-init] {message}");
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = target.parent().with_context(|| {
        format!(
            "Cannot write MCP config without a parent directory: {}",
            target.display()
        )
    })?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create MCP config directory: {}",
            parent.display()
        )
    })?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file in {}", parent.display()))?;
    temporary
        .write_all(content.as_bytes())
        .with_context(|| format!("Failed to write MCP config: {}", target.display()))?;
    temporary
        .persist(&target)
        .with_context(|| format!("Failed to replace MCP config: {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_env(temp: &TempDir) -> McpEnvironment {
        let home = temp.path().join("home");
        let cwd = temp.path().join("project");
        let config_dir = temp.path().join("config");
        McpEnvironment {
            vscode_user_dir: config_dir.join("Code").join("User"),
            home,
            cwd,
            config_dir,
            rtk_exe: temp.path().join("bin with spaces").join("rtk.exe"),
            codex_home: None,
            copilot_home: None,
            hermes_home: None,
            factory_home_override: None,
            kimi_code_home: None,
        }
    }

    #[test]
    fn standard_json_install_is_idempotent_and_preserves_other_servers() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("mcp.json");
        fs::write(
            &path,
            r#"{"custom":true,"mcpServers":{"other":{"command":"other"}}}"#,
        )
        .unwrap();

        let first = patch_json(
            &path,
            "mcpServers",
            entry(EntryKind::Standard, Path::new("rtk")),
        )
        .unwrap();
        fs::write(&path, &first).unwrap();
        let second = patch_json(
            &path,
            "mcpServers",
            entry(EntryKind::Standard, Path::new("rtk")),
        )
        .unwrap();

        assert_eq!(first, second);
        let parsed: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(parsed["custom"], true);
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(parsed["mcpServers"]["rtk"]["args"], json!(["mcp"]));
    }

    #[test]
    fn json_uninstall_removes_only_rtk_server() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("mcp.json");
        fs::write(
            &path,
            r#"{"mcpServers":{"rtk":{"command":"rtk"},"other":{"command":"other"}}}"#,
        )
        .unwrap();

        let output = remove_json(&path, "mcpServers").unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert!(parsed["mcpServers"].get("rtk").is_none());
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
    }

    #[test]
    fn invalid_json_is_rejected_without_overwrite() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("mcp.json");
        fs::write(&path, "{broken").unwrap();

        let result = patch_json(
            &path,
            "mcpServers",
            entry(EntryKind::Standard, Path::new("rtk")),
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{broken");
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_are_accepted() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("mcp.json");
        fs::write(
            &path,
            "{\n  // user config\n  \"custom\": \"https://example.test/a//b\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"other\",},\n  },\n}\n",
        )
        .unwrap();

        let output = patch_json(
            &path,
            "mcpServers",
            entry(EntryKind::Standard, Path::new("rtk")),
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["custom"], "https://example.test/a//b");
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(parsed["mcpServers"]["rtk"]["args"], json!(["mcp"]));
    }

    #[test]
    fn codex_toml_preserves_unrelated_configuration() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            "model = \"gpt-test\"\n[mcp_servers.other]\ncommand = \"other\"\n",
        )
        .unwrap();

        let output = patch_toml(&path, Path::new(r"C:\Program Files\RTK\rtk.exe")).unwrap();
        let parsed: toml::Value = output.parse().unwrap();
        assert_eq!(parsed["model"].as_str(), Some("gpt-test"));
        assert_eq!(
            parsed["mcp_servers"]["other"]["command"].as_str(),
            Some("other")
        );
        assert_eq!(
            parsed["mcp_servers"]["rtk"]["args"].as_array().unwrap(),
            &[toml::Value::String("mcp".to_string())]
        );
    }

    #[test]
    fn hermes_yaml_adds_and_removes_rtk_without_touching_other_server() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            "model: test\nmcp_servers:\n  other:\n    command: other\nplugins:\n  enabled: true\n",
        )
        .unwrap();

        let installed = patch_hermes_yaml(&path, Path::new("rtk")).unwrap();
        assert!(installed.contains("  rtk:\n    command: \"rtk\"\n    args: [\"mcp\"]"));
        fs::write(&path, installed).unwrap();
        let removed = remove_hermes_yaml(&path).unwrap().unwrap();
        assert!(!removed.contains("  rtk:"));
        assert!(removed.contains("  other:"));
        assert!(removed.contains("plugins:"));
    }

    #[test]
    fn dry_run_does_not_create_config() {
        let temp = TempDir::new().unwrap();
        let env = test_env(&temp);
        install_with_env(
            McpClient::Kimi,
            false,
            InitContext {
                verbose: 1,
                dry_run: true,
            },
            &env,
        )
        .unwrap();
        assert!(!env.cwd.join(".kimi-code").join("mcp.json").exists());
    }

    #[test]
    fn copilot_global_installs_cli_and_vscode_configs() {
        let temp = TempDir::new().unwrap();
        let env = test_env(&temp);
        install_with_env(McpClient::Copilot, true, InitContext::default(), &env).unwrap();

        let cli: Value = serde_json::from_str(
            &fs::read_to_string(env.home.join(".copilot").join("mcp-config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cli["mcpServers"]["rtk"]["type"], "local");
        let vscode: Value = serde_json::from_str(
            &fs::read_to_string(env.vscode_user_dir.join("mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(vscode["servers"]["rtk"]["type"], "stdio");
    }

    #[test]
    fn cline_target_registers_both_cline_and_roo_code() {
        let temp = TempDir::new().unwrap();
        let env = test_env(&temp);
        install_with_env(McpClient::Cline, false, InitContext::default(), &env).unwrap();

        let storage = env.vscode_user_dir.join("globalStorage");
        assert!(storage
            .join("saoudrizwan.claude-dev/settings/cline_mcp_settings.json")
            .exists());
        assert!(storage
            .join("rooveterinaryinc.roo-cline/settings/mcp_settings.json")
            .exists());
    }

    #[test]
    fn pi_reports_no_native_mcp_destination() {
        let temp = TempDir::new().unwrap();
        let env = test_env(&temp);
        assert!(destinations(McpClient::Pi, true, &env).is_empty());
        install_with_env(McpClient::Pi, true, InitContext::default(), &env).unwrap();
    }
}
