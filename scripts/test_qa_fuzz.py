#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""scripts/qa-fuzz.sh hands the fuzz driver's commands to the runner, tests/qa.rs, in the
pinned image. The commands are the runner's, in tests/qa/fuzz/options.rs; the script has to
know their names to refuse a typo before it starts a container, and it has to build the
runner exactly as scripts/qa.sh does, or the two would rebuild each other's cache."""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
SCRIPT = REPOSITORY / "scripts/qa-fuzz.sh"
TEXT = SCRIPT.read_text(encoding="utf-8")
OPTIONS = (REPOSITORY / "tests/qa/fuzz/options.rs").read_text(encoding="utf-8")
RUNNER = "cargo test --locked --features qa,invariant-checks --test qa"


def script_commands() -> list[str]:
    case = re.search(r'case "\$\{1:-\}" in\n\s+([a-z|]+)\) ;;', TEXT)
    if case is None:
        raise AssertionError("qa-fuzz.sh no longer checks its command in a case")
    return case.group(1).split("|")


class QaFuzzScriptTests(unittest.TestCase):
    def test_the_script_takes_the_commands_the_runner_parses_and_no_other(self):
        runner = re.findall(r'Some\("([a-z]+)"\) => Command::', OPTIONS)
        self.assertTrue(runner, "tests/qa/fuzz/options.rs no longer matches commands by name")
        self.assertEqual(sorted(script_commands()), sorted(runner))
        for command in runner:
            self.assertIn(f"  {command} ", OPTIONS, f"the runner's usage leaves out {command}")

    def test_a_command_or_a_target_it_does_not_have_is_refused_before_any_container(self):
        for arguments in ([], ["fuzz"], ["--alphabet", "calibration"], ["run", "--target", "qt"], ["run", "--target"]):
            with self.subTest(arguments=arguments):
                refused = subprocess.run([str(SCRIPT), *arguments], capture_output=True, text=True, check=False)
                self.assertEqual(refused.returncode, 2)
                self.assertIn("Usage: scripts/qa-fuzz.sh", refused.stderr)
                self.assertEqual(refused.stdout, "")

    def test_the_runner_is_built_as_qa_sh_builds_it_and_sent_to_its_fuzz_commands(self):
        self.assertIn(RUNNER, (REPOSITORY / "scripts/qa.sh").read_text(encoding="utf-8"))
        self.assertIn(f'{RUNNER} -- fuzz "$@"', TEXT)
        argument = re.search(r'pub\(crate\) const ARGUMENT: &str = "([a-z]+)";', (REPOSITORY / "tests/qa/fuzz.rs").read_text(encoding="utf-8"))
        self.assertEqual(argument and argument.group(1), "fuzz")

    def test_the_target_reaches_the_runner_and_rune_brings_its_binary(self):
        self.assertIn('--env "STRATA_QA_TARGET=$target"', TEXT)
        self.assertIn("cargo build --locked -p strata-rune --features qa", TEXT)
        self.assertIn('export STRATA_QA_RUNE_BINARY="$CARGO_TARGET_DIR/debug/strata-rune"', TEXT)
        parity = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        self.assertIn("cargo build --locked -p strata-rune --features qa", parity)


if __name__ == "__main__":
    unittest.main()
