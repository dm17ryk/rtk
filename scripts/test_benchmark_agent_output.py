import importlib.util
import os
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("benchmark-agent-output.py")
SPEC = importlib.util.spec_from_file_location("benchmark_agent_output", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BenchmarkCommandParsingTest(unittest.TestCase):
    def test_split_command_uses_platform_appropriate_quote_mode(self):
        expected = (
            ["python3", "-c", "print(123)"]
            if os.name != "nt"
            else ["python3", "-c", '"print(123)"']
        )
        self.assertEqual(MODULE.split_command('python3 -c "print(123)"'), expected)

        expected = (
            ["git", "log", "--grep=fix bug"]
            if os.name != "nt"
            else ["git", "log", '--grep="fix', 'bug"']
        )
        self.assertEqual(MODULE.split_command('git log --grep="fix bug"'), expected)


if __name__ == "__main__":
    unittest.main()
