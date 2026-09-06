#!/usr/bin/env python3
"""Behavior tests for the deterministic/live agent coverage validator."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
SCRIPT = REPO / "scripts" / "verify-agent-coverage.py"


def run_validator(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=REPO,
        text=True,
        capture_output=True,
        check=False,
    )


def emit_json_lines(*events: dict[str, object]) -> tuple[str, ...]:
    return (
        sys.executable,
        "-c",
        "import json, sys; "
        "[print(json.dumps(event)) for event in json.loads(sys.argv[1])]",
        json.dumps(events),
    )


class VerifyAgentCoverageTests(unittest.TestCase):
    def test_offline_validation_is_explicitly_unverified(self) -> None:
        completed = run_validator()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["fixture_validation"], "passed")
        self.assertEqual(report["live_verification"], "unverified")

    def test_marker_only_live_command_is_smoke_unverified(self) -> None:
        completed = run_validator(
            "--expect-stdout",
            "rtk-live-smoke",
            "--live-command",
            sys.executable,
            "-c",
            "print('rtk-live-smoke')",
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["live_exit_code"], 0)
        self.assertGreater(report["live_stdout_bytes"], 0)
        self.assertTrue(report["live_expected_stdout_found"])
        self.assertIn("command/result evidence", report["live_reason"])

    def test_codex_jsonl_command_result_evidence_is_verified(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "rtk git status",
                    "aggregated_output": "On branch test",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {
                "type": "item.completed",
                "item": {"type": "agent_message", "text": "RTK_LIVE_CODEX_OK"},
            },
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "verified")
        self.assertEqual(report["rtk_evidence"][0]["command"], "rtk git status")
        self.assertEqual(report["rtk_evidence"][0]["exit_code"], 0)
        self.assertGreater(report["rtk_evidence"][0]["result_bytes"], 0)

    def test_codex_safe_shell_wrapper_is_verified(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "bash -lc 'rtk git status'",
                    "aggregated_output": "On branch test",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "verified")
        self.assertEqual(report["rtk_evidence"][0]["command"], "bash -lc 'rtk git status'")

    def test_codex_outer_shell_compound_is_not_verified(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "bash -lc 'rtk git status' < /dev/null/rtk-review-input || true",
                    "aggregated_output": "",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])

    def test_codex_attached_shell_metacharacters_are_not_verified(self) -> None:
        for command in (
            "true||/bin/sh -c 'rtk git status'",
            "true>/tmp/bash -c 'rtk git status'",
        ):
            with self.subTest(command=command):
                events = (
                    {
                        "type": "item.completed",
                        "item": {
                            "id": "command-1",
                            "type": "command_execution",
                            "command": command,
                            "aggregated_output": "",
                            "exit_code": 0,
                            "status": "completed",
                        },
                    },
                    {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
                )
                completed = run_validator(
                    "--expect-stdout",
                    "RTK_LIVE_CODEX_OK",
                    "--evidence-format",
                    "codex-jsonl",
                    "--expect-rtk-command",
                    "rtk git status",
                    "--require-verified",
                    "--live-command",
                    *emit_json_lines(*events),
                )

                self.assertEqual(completed.returncode, 1, completed.stderr)
                report = json.loads(completed.stdout)
                self.assertEqual(report["live_verification"], "unverified")
                self.assertEqual(report["live_smoke"], "passed")
                self.assertEqual(report["rtk_evidence"], [])

    def test_codex_wrapper_script_newline_is_not_verified(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "bash -c 'rtk\n git status'",
                    "aggregated_output": "",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])

    def test_claude_stream_json_command_result_evidence_is_verified(self) -> None:
        events = (
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "tool-1",
                            "name": "Bash",
                            "input": {"command": "rtk git status"},
                        }
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "On branch test",
                            "is_error": False,
                        }
                    ]
                },
            },
            {"type": "result", "subtype": "success", "result": "RTK_LIVE_CLAUDE_OK"},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CLAUDE_OK",
            "--evidence-format",
            "claude-stream-json",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "verified")
        self.assertEqual(report["rtk_evidence"][0]["command"], "rtk git status")
        self.assertFalse(report["rtk_evidence"][0]["is_error"])

    def test_claude_safe_shell_wrapper_is_verified(self) -> None:
        events = (
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "tool-1",
                            "name": "Bash",
                            "input": {"command": "bash -lc 'rtk git status'"},
                        }
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "On branch test",
                            "is_error": False,
                        }
                    ]
                },
            },
            {"type": "result", "subtype": "success", "result": "RTK_LIVE_CLAUDE_OK"},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CLAUDE_OK",
            "--evidence-format",
            "claude-stream-json",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "verified")
        self.assertEqual(
            report["rtk_evidence"][0]["command"], "bash -lc 'rtk git status'"
        )

    def test_claude_outer_shell_compound_is_not_verified(self) -> None:
        events = (
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "tool-1",
                            "name": "Bash",
                            "input": {
                                "command": "bash -lc 'rtk git status' < /dev/null/rtk-review-input || true"
                            },
                        }
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "",
                            "is_error": False,
                        }
                    ]
                },
            },
            {"type": "result", "subtype": "success", "result": "RTK_LIVE_CLAUDE_OK"},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CLAUDE_OK",
            "--evidence-format",
            "claude-stream-json",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])

    def test_claude_attached_shell_metacharacters_are_not_verified(self) -> None:
        for command in (
            "true||/bin/sh -c 'rtk git status'",
            "true>/tmp/bash -c 'rtk git status'",
        ):
            with self.subTest(command=command):
                events = (
                    {
                        "type": "assistant",
                        "message": {
                            "content": [
                                {
                                    "type": "tool_use",
                                    "id": "tool-1",
                                    "name": "Bash",
                                    "input": {"command": command},
                                }
                            ]
                        },
                    },
                    {
                        "type": "user",
                        "message": {
                            "content": [
                                {
                                    "type": "tool_result",
                                    "tool_use_id": "tool-1",
                                    "content": "",
                                    "is_error": False,
                                }
                            ]
                        },
                    },
                    {
                        "type": "result",
                        "subtype": "success",
                        "result": "RTK_LIVE_CLAUDE_OK",
                    },
                )
                completed = run_validator(
                    "--expect-stdout",
                    "RTK_LIVE_CLAUDE_OK",
                    "--evidence-format",
                    "claude-stream-json",
                    "--expect-rtk-command",
                    "rtk git status",
                    "--require-verified",
                    "--live-command",
                    *emit_json_lines(*events),
                )

                self.assertEqual(completed.returncode, 1, completed.stderr)
                report = json.loads(completed.stdout)
                self.assertEqual(report["live_verification"], "unverified")
                self.assertEqual(report["live_smoke"], "passed")
                self.assertEqual(report["rtk_evidence"], [])

    def test_claude_wrapper_script_crlf_is_not_verified(self) -> None:
        command = "bash -lc 'rtk\r\n git status'"
        events = (
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "tool-1",
                            "name": "Bash",
                            "input": {"command": command},
                        }
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "tool-1",
                            "content": "",
                            "is_error": False,
                        }
                    ]
                },
            },
            {"type": "result", "subtype": "success", "result": "RTK_LIVE_CLAUDE_OK"},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CLAUDE_OK",
            "--evidence-format",
            "claude-stream-json",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])

    def test_wrong_rtk_command_is_unverified_and_fails_required_gate(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "rtk --version",
                    "aggregated_output": "rtk 0.46.1-dev.12",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])
        self.assertIn("rtk git status", report["live_reason"])

    def test_compound_rtk_prefix_is_not_verified_without_direct_execution(self) -> None:
        events = (
            {
                "type": "item.completed",
                "item": {
                    "id": "command-1",
                    "type": "command_execution",
                    "command": "rtk git status < /dev/null/rtk-review-input || true",
                    "aggregated_output": "",
                    "exit_code": 0,
                    "status": "completed",
                },
            },
            {"type": "item.completed", "item": {"text": "RTK_LIVE_CODEX_OK"}},
        )
        completed = run_validator(
            "--expect-stdout",
            "RTK_LIVE_CODEX_OK",
            "--evidence-format",
            "codex-jsonl",
            "--expect-rtk-command",
            "rtk git status",
            "--require-verified",
            "--live-command",
            *emit_json_lines(*events),
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "unverified")
        self.assertEqual(report["live_smoke"], "passed")
        self.assertEqual(report["rtk_evidence"], [])
        self.assertIn("rtk git status", report["live_reason"])

    def test_missing_live_marker_stays_failed(self) -> None:
        completed = run_validator(
            "--expect-stdout",
            "rtk-live-smoke",
            "--live-command",
            sys.executable,
            "-c",
            "print('different-output')",
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "failed")
        self.assertFalse(report["live_expected_stdout_found"])
        self.assertIn("expected stdout marker", report["live_reason"])

    def test_missing_live_runtime_is_unsupported_and_not_success(self) -> None:
        completed = run_validator(
            "--live-command",
            "rtk-host-runtime-that-does-not-exist",
            "--version",
        )

        self.assertEqual(completed.returncode, 3, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["fixture_validation"], "passed")
        self.assertEqual(report["live_verification"], "unsupported")
        self.assertIn("not found", report["live_reason"])

    def test_live_command_failure_stays_failed(self) -> None:
        completed = run_validator(
            "--live-command",
            sys.executable,
            "-c",
            "import sys; sys.exit(7)",
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["live_verification"], "failed")
        self.assertEqual(report["live_exit_code"], 7)


if __name__ == "__main__":
    unittest.main()
