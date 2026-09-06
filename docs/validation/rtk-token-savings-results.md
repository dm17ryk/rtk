# RTK agent-output measurement

Task 13 reran a deterministic paired measurement with the same repository,
input, candidate executable, model-visible boundary, and `byte_estimate`
counter (`ceil(UTF-8 bytes / 4)`). The command compared native `rg` with
candidate `rtk rg` for `permissionDecision` under `src/hooks`.

| Field | Baseline | Candidate |
|---|---:|---:|
| Producer/model-input bytes | 4,429 | 3,196 |
| Estimated tokens | 1,108 | 799 |
| Exit status | 0 | 0 |
| Initial output reduction | — | 27.8392% |
| Recovery bytes read | 0 | 0 |
| Hook/tool-schema context in this offline run | 0 | 0 |

Reproduce from the repository root with:

```text
python scripts/benchmark-agent-output.py --label task13-rg-permission-contract --baseline-command "rg -n permissionDecision src/hooks" --candidate-command "target/debug/rtk.exe rg -n permissionDecision src/hooks" --counter byte_estimate
```

This is an offline command-output comparison, not a paid model benchmark or a
billing-token count. No live agent task was run, so complete conversation
history, reasoning tokens, model output, retries, and provider cost are
unavailable. No recovery read occurred; if the full 4,429-byte producer output
had been recovered, that recovery input would need to be reported separately
and could erase the initial model-input reduction for that task.

The repository script keeps `raw_producer`, `baseline_model_input`,
`candidate_model_input`, `recovery_input`, `hook_context`, and
`tool_schema_context` separate. Model routing and reasoning-effort effects are
never counted as RTK savings.
