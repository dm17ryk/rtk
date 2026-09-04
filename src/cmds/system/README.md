# System and Generic Utilities

> Part of [`src/cmds/`](../README.md) — see also [docs/contributing/TECHNICAL.md](../../../docs/contributing/TECHNICAL.md)

## Specifics

- `read.rs` uses `core/filter` for language-aware AI source output (minimal by default, none for exact content, aggressive for stronger reduction). Retained lines keep their original one-based source locations; omitted content has lossless recovery when the compact form wins.
- `search.rs` backs both `rtk grep` and `rtk rg`: it runs the invoked engine (never substituting one for the other). `rtk grep` preserves its existing grouped path. `rtk rg` classifies ordinary text, JSON events, inventories, counts, and only-matching output into compact semantic records, while NUL/binary, streaming, interactive, sensitive, and unknown modes stay exact. Both paths preserve native exit codes and stderr.
- `ctest_cmd.rs` takes the run total from the first result line (covering `--stop-on-failure`, disabled tests, and forwarded suites) and validates both result lines and the summary against it, deduplicates retries by test number+name, folds wrapped result lines until their terminator, falls back to the raw `FAILED:` trailer for unparsed failures, keeps the error trailer behind an empty run, labels and caps failure details with tee recovery, and attributes diagnostics safely under `-j`; explicit verbose/show-only/help/version flags and dashboard modes bypass filtering, except `-T Test`, which prints ordinary test output and stays filtered.
- `local_llm.rs` (`rtk smart`) uses `core/filter` for heuristic file summarization
- `format_cmd.rs` is a cross-ecosystem dispatcher: auto-detects and routes to `prettier_cmd` or `ruff_cmd` (black is handled inline, not as a separate module)

## Cross-command

- `format_cmd` routes to `cmds/js/prettier_cmd` and `cmds/python/ruff_cmd`
