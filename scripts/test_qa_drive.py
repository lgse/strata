# SPDX-License-Identifier: MIT

"""scripts/qa-drive.sh launches `strata --qa` as tests/qa.rs launches each
run; these tests fail when a value it copies from the runner drifts."""

import re
import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
RUNNER = (REPOSITORY / "tests/qa.rs").read_text(encoding="utf-8")
DRIVE = (REPOSITORY / "scripts/qa-drive.sh").read_text(encoding="utf-8")


def runner_pairs(constant):
    match = re.search(rf"^const {constant}: \[\(&str, &str\); \d+\] = \[(.*?)\];", RUNNER, re.M | re.S)
    if match is None:
        raise AssertionError(f"tests/qa.rs has no {constant}")
    return [f"{key}={value}" for key, value in re.findall(r'\("([^"]*)", "([^"]*)"\)', match.group(1))]


def runner_seconds(constant):
    match = re.search(rf"^const {constant}: Duration = Duration::from_secs\((\d+)\);", RUNNER, re.M)
    if match is None:
        raise AssertionError(f"tests/qa.rs has no {constant} in whole seconds")
    return int(match.group(1))


def drive_array(name):
    match = re.search(rf"^{name}=\((.*?)\)$", DRIVE, re.M)
    if match is None:
        raise AssertionError(f"qa-drive.sh has no {name} array")
    return match.group(1).split()


def drive_value(name):
    match = re.search(rf"^{name}=(\S+)$", DRIVE, re.M)
    if match is None:
        raise AssertionError(f"qa-drive.sh has no {name}")
    return match.group(1)


class QaDriveMatchesTheRunner(unittest.TestCase):
    def test_the_application_gets_the_runners_qa_environment(self):
        self.assertEqual(drive_array("qa_environment"), runner_pairs("QA_ENVIRONMENT"))

    def test_home_and_the_xdg_directories_are_laid_out_as_the_runners(self):
        self.assertEqual(drive_array("xdg_directories"), runner_pairs("XDG_DIRECTORIES"))

    def test_xvfb_starts_with_the_runners_screen_and_listening_arguments(self):
        start = RUNNER.index('Command::new("Xvfb")')
        end = RUNNER.index(".spawn()", start)
        arguments = []
        for listed in re.findall(r"\.args\(\[([^\]]*)\]\)", RUNNER[start:end]):
            literals = re.findall(r'"([^"]*)"', listed)
            # The `-displayfd` pair carries a descriptor, not literals only.
            if len(literals) == listed.count(",") + 1:
                arguments.extend(literals)
        self.assertEqual(drive_array("xvfb_arguments"), arguments)

    def test_a_run_without_a_bus_gets_the_same_unconnectable_address(self):
        match = re.search(r'launch\s*\.runtime\s*\.join\("([^"]+)"\)', RUNNER)
        self.assertIsNotNone(match, "tests/qa.rs no longer names its unconnectable bus")
        self.assertEqual(drive_value("no_session_bus"), match.group(1))

    def test_the_private_session_bus_has_the_runners_configuration(self):
        runner = re.search(r'^const SESSION_BUS_CONFIG: &str = r#"(.*?)"#;', RUNNER, re.M | re.S)
        drive = re.search(r"^session_bus_config='(.*?)'$", DRIVE, re.M | re.S)
        self.assertIsNotNone(runner, "tests/qa.rs has no SESSION_BUS_CONFIG")
        self.assertIsNotNone(drive, "qa-drive.sh has no session_bus_config")
        self.assertEqual(drive.group(1), runner.group(1))

    def test_reply_and_shutdown_budgets_are_the_runners(self):
        for constant in ("LAUNCH_TIMEOUT", "COMMAND_TIMEOUT", "WAIT_MARGIN", "EXIT_TIMEOUT", "TERMINATE_GRACE"):
            with self.subTest(constant=constant):
                self.assertEqual(int(drive_value(constant.lower())), runner_seconds(constant))

    def test_the_fixture_directory_is_named_as_the_runners(self):
        match = re.search(r'None => fixture\.prefix\("([^"]+)"\)', RUNNER)
        self.assertIsNotNone(match, "tests/qa.rs no longer names its fresh fixture")
        self.assertEqual(drive_value("fixture_prefix"), match.group(1))

    def test_home_is_seeded_and_the_application_launched_as_the_runner_does(self):
        for runner, drive in (
            ('writeln!(output, "{key} = {}", render_value(value))', 'f"{key} = {toml(value)}\\n"'),
            ('home.join("config/gtk-4.0")', '{home}/config/gtk-4.0/settings.ini'),
            ('fs::write(strata.join(&manifest.portal_offer_marker), "1\\n")', 'file.write("1\\n")'),
            ("from_mode(0o755)", "os.chmod(path, 0o755)"),
            ("from_mode(0o644)", "os.chmod(path, 0o644)"),
            (".current_dir(launch.fixture)", '--chdir="$(cat "$session/fixture")"'),
            ('.env("STRATA_INPUT_LOG"', "STRATA_INPUT_LOG="),
            (".env(SHADER_CACHE_VARIABLE", "MESA_SHADER_CACHE_DIR="),
            ('.args(["-display", &self.name, "-window", "root"])', 'import -display "$display" -window root'),
        ):
            with self.subTest(runner=runner):
                self.assertIn(runner, RUNNER, "tests/qa.rs changed; update qa-drive.sh and this test")
                self.assertIn(drive, DRIVE)


if __name__ == "__main__":
    unittest.main()
