//! Shared command-selection policy rendered into agent instructions and MCP hints.

pub const PLACEHOLDER: &str = "{{RTK_DIRECT_FIRST_POLICY}}";

pub const MARKDOWN: &str = include_str!("../../hooks/shared/direct-first-policy.md");

pub const MCP_INSTRUCTIONS: &str = "Prefer the typed run_filtered tool for commands supported by \
RTK. Pass RTK arguments without a leading `rtk`, for example \
{\"rtk_args\":[\"git\",\"status\"]} or {\"rtk_args\":[\"read\",\"src/main.rs\"]}. \
Use a host shell only when the task requires shell built-ins, a script, or control flow that \
cannot be expressed as RTK argv. On Windows, PowerShell/pwsh and cmd are last-resort fallbacks; \
never wrap an RTK-supported command inside them.";

pub const RUN_FILTERED_DESCRIPTION: &str = "Preferred execution tool for RTK-supported commands. \
Pass arguments without a leading `rtk` (for example [\"git\",\"status\"], [\"read\",\"file\"], or \
[\"rg\",\"TODO\",\"src\"]). Returns bounded filtered output. Do not wrap supported commands in \
PowerShell/pwsh or cmd; use a host shell only for shell-only behavior.";

pub fn render(template: &str) -> String {
    debug_assert_eq!(
        template.matches(PLACEHOLDER).count(),
        1,
        "agent template must contain exactly one direct-first policy placeholder"
    );
    template.replace(PLACEHOLDER, MARKDOWN.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_the_single_policy_placeholder() {
        let rendered = render(&format!("before\n{PLACEHOLDER}\nafter"));
        assert!(rendered.starts_with("before\n## Command Selection Priority"));
        assert!(rendered.ends_with("\nafter"));
        assert!(!rendered.contains(PLACEHOLDER));
    }
}
