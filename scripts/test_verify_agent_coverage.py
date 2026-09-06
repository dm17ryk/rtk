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


class VerifyAgentCoverageTests(unittest.TestCase):
    def test_offline_validation_is_explicitly_unverified(self) -> None:
        completed = run_validator()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["fixture_validation"], "passed")
        self.assertEqual(report["live_verification"], "unverified")

    def test_successful_live_command_is_verified(self) -> None:
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
        self.assertEqual(report["live_verification"], "verified")
        self.assertEqual(report["live_exit_code"], 0)
        self.assertGreater(report["live_stdout_bytes"], 0)
        self.assertTrue(report["live_expected_stdout_found"])

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
