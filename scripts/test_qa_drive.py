# SPDX-License-Identifier: MIT

"""scripts/qa-drive.sh launches `strata --qa` as tests/qa.rs launches each
run; these tests fail when a value it copies from the runner drifts, or when
scripts/qa_drive.py writes a settings.toml the application cannot read."""

import io
import os
import re
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest import mock

import qa_drive

REPOSITORY = Path(__file__).resolve().parent.parent
RUNNER = (REPOSITORY / "tests/qa.rs").read_text(encoding="utf-8")
DRIVE = (REPOSITORY / "scripts/qa-drive.sh").read_text(encoding="utf-8")
SEED = (REPOSITORY / "scripts/qa_drive.py").read_text(encoding="utf-8")


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
                self.assertIn(drive, DRIVE + SEED)


SAVED = """\
browser_mode = "columns"
list_column_order = [
    "name",
    "size",
    "modified",
]
show_hidden = false

[keymap.default]
copy = ["ctrl+c"]
rename = ["F2"]

[folder_customization."/tmp/fixture/pictures"]
icon = "camera"
"""


class PreferencesAreMergedIntoTheSavedSettings(unittest.TestCase):
    def merged(self, *changes):
        merged = qa_drive.merge(SAVED, list(changes))
        return tomllib.loads(merged)

    def test_a_multi_line_array_is_replaced_whole(self):
        settings = self.merged(("list_column_order", ["size", "name"]))
        self.assertEqual(settings["list_column_order"], ["size", "name"])
        self.assertEqual(settings["browser_mode"], "columns")
        self.assertEqual(settings["show_hidden"], False)

    def test_a_nested_table_replaces_the_saved_table(self):
        keymap = {"default": {"copy": ["ctrl+shift+c"]}, "vim": {"left": ["h"]}}
        settings = self.merged(("keymap", keymap))
        self.assertEqual(settings["keymap"], keymap)
        self.assertEqual(settings["folder_customization"], {"/tmp/fixture/pictures": {"icon": "camera"}})

    def test_a_new_key_stays_out_of_the_saved_tables(self):
        settings = self.merged(("text_size", "large"))
        self.assertEqual(settings["text_size"], "large")
        self.assertNotIn("text_size", settings["keymap"]["default"])

    def test_a_saved_file_that_does_not_parse_is_refused_unchanged(self):
        with self.assertRaises(qa_drive.PreferenceError):
            qa_drive.merge("list_column_order = [\n", [("show_hidden", True)])


class PreferenceValuesAreWrittenAsTheyWereGiven(unittest.TestCase):
    def test_each_value_reads_back_unchanged(self):
        for value in (
            True,
            7,
            1.5,
            "plain",
            'quote " and \\ backslash',
            "line\nbreak\ttab\x7fdel\x01",
            "Ünïcode 名前",
            [],
            ["a", 2, [False]],
            {},
            {"default": {"copy": ["ctrl+c"]}, "needs quotes": 1},
            [{"name": "a"}, {"name": "b"}],
        ):
            with self.subTest(value=value):
                written = f"key = {qa_drive.toml(value)}\n"
                self.assertEqual(tomllib.loads(written)["key"], value)
                self.assertEqual(tomllib.loads(qa_drive.dumps({"key": value}))["key"], value)

    def test_a_value_is_json_or_else_a_string(self):
        prefs = "list_file_clicks=1\ntext_size=large\nkeymap={\"vim\":{\"left\":[\"h\"]}}\nlabel=a=b\n"
        self.assertEqual(
            list(qa_drive.parse_prefs(prefs)),
            [("list_file_clicks", 1), ("text_size", "large"), ("keymap", {"vim": {"left": ["h"]}}),
             ("label", "a=b")],
        )

    def test_values_settings_toml_cannot_hold_are_refused(self):
        for prefs in ("show_hidden=null", "show_hidden", "=true"):
            with self.subTest(prefs=prefs), self.assertRaises(qa_drive.PreferenceError):
                list(qa_drive.parse_prefs(prefs))

    def test_restart_rewrites_the_file_only_when_every_value_is_valid(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "settings.toml"
            path.write_text(SAVED, encoding="utf-8")
            for prefs, status in (("show_hidden=null\ntext_size=large", 1), ("list_column_order=[\"size\"]", 0)):
                with (
                    self.subTest(prefs=prefs),
                    mock.patch.dict(os.environ, {"QA_DRIVE_PREFS": prefs}),
                    mock.patch("sys.stderr", io.StringIO()),
                ):
                    self.assertEqual(qa_drive.main(["check"]), status)
                    self.assertEqual(qa_drive.main(["prefs", str(path)]), status)
            settings = tomllib.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(settings["list_column_order"], ["size"])
            self.assertNotIn("text_size", settings)


if __name__ == "__main__":
    unittest.main()
