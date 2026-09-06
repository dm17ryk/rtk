#!/usr/bin/env python3
"""Validate RTK's deterministic agent fixtures and optionally run a typed host command.

The default path is offline and does not start an agent.  ``--live-command`` is
an explicit opt-in subprocess check; its arguments are passed exactly as typed
and the result is reported as host evidence, not as a model benchmark.
"""

from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import sys
from pathlib import Path


def validate_manifest(repo: Path) -> dict[str, object]:
    manifest_path = repo / "tests" / "fixtures" / "agent_capabilities.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    cases = manifest.get("cases")
    if manifest.get("schema_version") != 1 or not isinstance(cases, list) or not cases:
        raise ValueError("manifest must have schema_version=1 and a non-empty cases list")

    ids: set[str] = set()
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("every manifest case must be an object")
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id or case_id in ids:
            raise ValueError(f"invalid or duplicate case id: {case_id!r}")
        ids.add(case_id)
        route = case.get("route")
        argv = case.get("argv")
        fixture = case.get("fixture")
        if not isinstance(route, str) or not route:
            raise ValueError(f"{case_id}: route is required")
        if not isinstance(argv, list) or not argv or argv[0] == "rtk":
            raise ValueError(f"{case_id}: argv must be a typed RTK argument vector")
        fixture_path = repo / fixture if isinstance(fixture, str) else Path()
        if isinstance(fixture, str) and not fixture_path.is_file():
            fixture_path = repo / "tests" / "fixtures" / fixture
        if not isinstance(fixture, str) or not fixture_path.is_file():
            raise ValueError(f"{case_id}: fixture file is missing")

    return {"schema_version": 1, "cases": len(cases), "fixture_validation": "passed"}


def command_tokens(command: str) -> list[str]:
    try:
        tokens = shlex.split(command, posix=True)
    except ValueError:
        return []
    if len(tokens) >= 3 and tokens[0].replace("\\", "/").rsplit("/", 1)[-1] in {
        "bash",
        "sh",
        "zsh",
    } and tokens[1] in {"-c", "-lc"}:
        try:
            tokens = shlex.split(tokens[2], posix=True)
        except ValueError:
            return []
    if tokens:
        executable = tokens[0].replace("\\", "/").rsplit("/", 1)[-1].lower()
        if executable == "rtk.exe":
            tokens[0] = "rtk"
    return tokens


def command_matches(command: str, expected: str) -> bool:
    actual_tokens = command_tokens(command)
    expected_tokens = command_tokens(expected)
    return bool(expected_tokens) and actual_tokens[: len(expected_tokens)] == expected_tokens


def result_bytes(value: object) -> int:
    if isinstance(value, str):
        rendered = value
    else:
        rendered = json.dumps(value, sort_keys=True)
    return len(rendered.encode("utf-8"))


def json_lines(stdout: str) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def codex_evidence(stdout: str, expected: str) -> list[dict[str, object]]:
    evidence: list[dict[str, object]] = []
    for event in json_lines(stdout):
        item = event.get("item")
        if not isinstance(item, dict) or item.get("type") != "command_execution":
            continue
        command = item.get("command")
        exit_code = item.get("exit_code")
        if not isinstance(command, str) or exit_code != 0:
            continue
        if not command_matches(command, expected):
            continue
        output = item.get("aggregated_output", item.get("output", ""))
        evidence.append(
            {
                "command": command,
                "event_id": item.get("id"),
                "exit_code": exit_code,
                "result_bytes": result_bytes(output),
            }
        )
    return evidence


def claude_evidence(stdout: str, expected: str) -> list[dict[str, object]]:
    tool_uses: dict[str, str] = {}
    evidence: list[dict[str, object]] = []
    for event in json_lines(stdout):
        message = event.get("message")
        content = message.get("content") if isinstance(message, dict) else None
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use":
                tool_id = block.get("id")
                tool_input = block.get("input")
                command = tool_input.get("command") if isinstance(tool_input, dict) else None
                if (
                    isinstance(tool_id, str)
                    and block.get("name") in {"Bash", "PowerShell"}
                    and isinstance(command, str)
                    and command_matches(command, expected)
                ):
                    tool_uses[tool_id] = command
            elif block.get("type") == "tool_result":
                tool_id = block.get("tool_use_id")
                if (
                    isinstance(tool_id, str)
                    and tool_id in tool_uses
                    and block.get("is_error") is not True
                    and "content" in block
                ):
                    evidence.append(
                        {
                            "command": tool_uses.pop(tool_id),
                            "tool_use_id": tool_id,
                            "is_error": False,
                            "result_bytes": result_bytes(block["content"]),
                        }
                    )
    return evidence


def extract_evidence(
    stdout: str, evidence_format: str | None, expected: str | None
) -> list[dict[str, object]]:
    if evidence_format is None or expected is None:
        return []
    if evidence_format == "codex-jsonl":
        return codex_evidence(stdout, expected)
    return claude_evidence(stdout, expected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--expect-stdout",
        help="require this literal smoke marker in live command stdout",
    )
    parser.add_argument(
        "--evidence-format",
        choices=("codex-jsonl", "claude-stream-json"),
        help="parse structured host output for concrete RTK command/result evidence",
    )
    parser.add_argument(
        "--expect-rtk-command",
        help="exact direct RTK command prefix that structured evidence must demonstrate",
    )
    parser.add_argument(
        "--require-verified",
        action="store_true",
        help="return failure unless the live result is verified, not only smoke-tested",
    )
    parser.add_argument(
        "--live-command",
        nargs=argparse.REMAINDER,
        help="explicitly run a host command after this option; no command is run by default",
    )
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]

    if (args.evidence_format is None) != (args.expect_rtk_command is None):
        parser.error("--evidence-format and --expect-rtk-command must be supplied together")
    if args.expect_rtk_command is not None:
        expected_tokens = command_tokens(args.expect_rtk_command)
        if not expected_tokens or expected_tokens[0] != "rtk":
            parser.error("--expect-rtk-command must begin with a direct rtk invocation")

    try:
        report = validate_manifest(repo)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(json.dumps({"fixture_validation": "failed", "error": str(error)}))
        return 1

    if not args.live_command:
        report["live_verification"] = "unverified"
        report["live_reason"] = "No --live-command was supplied"
    else:
        command = list(args.live_command)
        if command and command[0] == "--":
            command.pop(0)
        if not command:
            print(json.dumps({"fixture_validation": "passed", "live_verification": "invalid"}))
            return 2
        try:
            completed = subprocess.run(
                command,
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
        except FileNotFoundError:
            report["live_verification"] = "unsupported"
            report["live_command"] = command
            report["live_reason"] = f"runtime not found: {command[0]}"
            print(json.dumps(report, sort_keys=True))
            return 3

        report["live_command"] = command
        report["live_exit_code"] = completed.returncode
        report["live_stdout_bytes"] = len(completed.stdout.encode("utf-8"))
        report["live_stderr_bytes"] = len(completed.stderr.encode("utf-8"))
        marker_found = args.expect_stdout is None or args.expect_stdout in completed.stdout
        report["live_expected_stdout_found"] = marker_found
        smoke_passed = completed.returncode == 0 and marker_found
        report["live_smoke"] = "passed" if smoke_passed else "failed"
        evidence = extract_evidence(
            completed.stdout, args.evidence_format, args.expect_rtk_command
        )
        report["rtk_evidence"] = evidence
        if smoke_passed and evidence:
            report["live_verification"] = "verified"
        elif smoke_passed:
            report["live_verification"] = "unverified"
            if args.expect_rtk_command is None:
                report["live_reason"] = (
                    "Live command passed as smoke only; no structured RTK "
                    "command/result evidence was requested"
                )
            else:
                report["live_reason"] = (
                    "Live smoke passed but no successful command/result evidence matched "
                    f"{args.expect_rtk_command!r}"
                )
        else:
            report["live_verification"] = "failed"
            if completed.returncode == 0:
                report["live_reason"] = "expected stdout marker was not observed"

    print(json.dumps(report, sort_keys=True))
    acceptable_live_result = report["live_verification"] != "failed"
    if args.require_verified:
        acceptable_live_result = report["live_verification"] == "verified"
    return 0 if report["fixture_validation"] == "passed" and acceptable_live_result else 1


if __name__ == "__main__":
    sys.exit(main())
