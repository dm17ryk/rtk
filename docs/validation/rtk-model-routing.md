# RTK model routing ledger

Model assignment is host configuration, not an RTK compression feature. RTK
install, upgrade, and uninstall do not add or remove model preferences.

| Task | Role | Requested model/effort | Host-observed binding | Selection mechanism | Status | RTK evidence |
|---|---|---|---|---|---|---|
| 1–12 | imported checkpoint | plan-specific roles | not replayed in Task 13 | checked-in checkpoint at `b87d349` | implemented | Full source/test checkpoint imported; no new live-worker claim. |
| 13 | `rtk_implementer` | `gpt-5.6-sol` / `high` | not observed | User-requested task role; no host-introspection record | pending | Candidate RTK commands prove execution behavior, not the effective model or effort. |
| 13 final review | `rtk_reviewer` | `gpt-6-astra` / `xhigh` | none | User prohibited subagents/reviewers | blocked | Implementer self-review is recorded but is not represented as Astra evidence. |

The candidate's recorded config inspection found the active global Codex base
config at `gpt-5.6-luna` / `medium` and no `rtk_implementer` or `rtk_reviewer`
files under the global Codex home. That supports only the claim that RTK did not
install an unrelated global role override. It does not prove which model or
effort the task host actually selected for Task 13.

## Precedence and process boundaries

When a child has no model/effort override, Codex inherits the parent values. An
explicit spawn or `[agents]` default can select a value, and a custom-agent file
can override the resolved model/effort. A model-only override may use the
selected model's default effort, so bind both values when reproducibility
matters. See the [Codex subagent configuration](https://learn.chatgpt.com/docs/agent-configuration/subagents).

Normal host-native subtasks share the parent executor and inherited RTK
integration. Separate CLI/headless or SDK workers are new process boundaries:
they need their own effective PATH, instructions, hooks/settings, MCP setup, and
profile selection. A fresh worker is not a fictitious in-place model switch.

Optional project-specific role files belong under `.codex/agents/`; personal
ones belong under `$CODEX_HOME/agents/`. Remove those role files and any
project-only assignment table to clean up. Do not use `rtk init --uninstall` as
a model-role cleanup command: it intentionally removes only RTK integration.

Model choice, reasoning effort, retries, and provider billing can change total
task cost. Those effects must be reported separately from measured RTK output
compression.
