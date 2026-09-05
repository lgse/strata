"""Focused tests for the interactive Bash installer helpers."""

import os
import pathlib
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "install.sh"


def bash(script: str, *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    test_env = os.environ.copy()
    test_env["STRATA_INSTALLER_TESTING"] = "1"
    if env:
        test_env.update(env)
    return subprocess.run(
        ["bash", "-c", f'source "$1"; {script}', "bash", str(INSTALLER)],
        check=False,
        capture_output=True,
        text=True,
        env=test_env,
    )


class InstallerTests(unittest.TestCase):
    def test_installer_has_valid_bash_syntax(self) -> None:
        result = subprocess.run(
            ["bash", "-n", str(INSTALLER)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_banner_remains_readable_without_terminal_color(self) -> None:
        result = bash("show_banner", env={"NO_COLOR": "1"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("S T R A T A", result.stdout)
        self.assertIn("Navigate every layer.", result.stdout)
        self.assertIn("Interactive installer", result.stdout)
        self.assertNotIn("\033", result.stdout)

    def test_unattended_flags_select_only_requested_integrations(self) -> None:
        result = bash(
            "parse_args --with-folder-association --with-raw; "
            'printf "%s %s %s %s %s %s %s" "$NON_INTERACTIVE" "$WITH_SMB" '
            '"$WITH_RAW" "$WITH_DESKTOP_ENTRY" "$WITH_FOLDER_ASSOCIATION" '
            '"$WITH_FILE_MANAGER" "$WITH_OMARCHY_KEYBINDS"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "yes ask yes yes yes yes ask")

    def test_file_manager_flag_can_be_selected_independently(self) -> None:
        result = bash(
            "parse_args --with-file-manager; "
            'printf "%s %s %s" "$NON_INTERACTIVE" "$WITH_DESKTOP_ENTRY" '
            '"$WITH_FILE_MANAGER"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "yes ask yes")

    def test_non_interactive_mode_declines_unspecified_options(self) -> None:
        result = bash("NON_INTERACTIVE=yes; ! want_option ask ignored")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_file_manager_service_uses_the_installed_binary(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            result = bash(
                'TEMP_DIR="$HOME/tmp"; mkdir -p "$TEMP_DIR"; '
                'BIN_PATH="$HOME/.local/bin/strata"; '
                'install_file_manager_service "$(dirname "$1")/data"',
                env={"HOME": home, "XDG_DATA_HOME": f"{home}/data"},
            )
            service = (
                pathlib.Path(home)
                / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service"
            )
            contents = service.read_text()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"Exec={home}/.local/bin/strata --gapplication-service", contents)

    def test_file_manager_service_refuses_another_per_user_provider(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            service_dir = pathlib.Path(home) / "data/dbus-1/services"
            service_dir.mkdir(parents=True)
            other = service_dir / "other.service"
            other.write_text(
                "[D-BUS Service]\n"
                "Name=org.freedesktop.FileManager1\n"
                "Exec=/usr/bin/other-files\n"
            )
            result = bash(
                'TEMP_DIR="$HOME/tmp"; mkdir -p "$TEMP_DIR"; '
                'BIN_PATH="$HOME/.local/bin/strata"; '
                'install_file_manager_service "$(dirname "$1")/data"',
                env={"HOME": home, "XDG_DATA_HOME": f"{home}/data"},
            )
            target = service_dir / "io.github.lgse.Strata.FileManager1.service"
            self.assertFalse(target.exists())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Another per-user FileManager1 provider", result.stderr)

    def test_non_interactive_pacman_disables_sudo_and_pacman_prompts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fake_sudo = pathlib.Path(directory) / "sudo"
            fake_sudo.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            fake_sudo.chmod(0o755)
            result = bash(
                "NON_INTERACTIVE=yes; run_pacman gvfs-smb",
                env={"PATH": f"{directory}:{os.environ['PATH']}"},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.splitlines(),
            ["-n", "pacman", "-S", "--needed", "--noconfirm", "--", "gvfs-smb"],
        )

    def test_unknown_installer_option_is_rejected(self) -> None:
        result = bash("parse_args --definitely-unknown")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unknown option", result.stderr)

    def test_version_comparison_accepts_minimum_and_newer(self) -> None:
        for version in ("2.39", "2.39.1", "2.40"):
            with self.subTest(version=version):
                result = bash(f'version_at_least "{version}" 2.39')
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_version_comparison_rejects_older_glibc(self) -> None:
        result = bash('version_at_least "2.38" 2.39')
        self.assertNotEqual(result.returncode, 0)

    def test_detects_omarchy_major_from_command(self) -> None:
        with self.subTest(major="3"):
            result = bash(
                "detect_omarchy_major",
                env={"PATH": f"{ROOT / 'scripts' / 'testdata' / 'omarchy3'}:{os.environ['PATH']}"},
            )
            self.assertEqual(result.stdout.strip(), "3")
        with self.subTest(major="4"):
            result = bash(
                "detect_omarchy_major",
                env={"PATH": f"{ROOT / 'scripts' / 'testdata' / 'omarchy4'}:{os.environ['PATH']}"},
            )
            self.assertEqual(result.stdout.strip(), "4")

    def test_omarchy_detection_without_command_is_not_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            result = bash(
                'PATH=/usr/bin:/bin; value=$(detect_omarchy_major); printf "%s" "$value"',
                env={"HOME": home},
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_generated_omarchy_bindings_are_idempotent(self) -> None:
        for major, suffix in (("3", "conf"), ("4", "lua")):
            with self.subTest(major=major), tempfile.TemporaryDirectory() as home:
                result = bash(
                    f'BIN_PATH="$HOME/.local/bin/strata"; '
                    f"configure_omarchy_bindings {major}; "
                    f"configure_omarchy_bindings {major}",
                    env={"HOME": home, "HYPRLAND_INSTANCE_SIGNATURE": ""},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                bindings = pathlib.Path(home) / ".config" / "hypr" / f"bindings.{suffix}"
                contents = bindings.read_text()
                self.assertEqual(contents.count("strata-installer: file-manager start"), 1)
                self.assertIn(f"{home}/.local/bin/strata", contents)
                if major == "4":
                    self.assertIn(
                        f'"uwsm-app -- {home}/.local/bin/strata '
                        '\\"$(omarchy-cmd-terminal-cwd)\\""',
                        contents,
                    )


if __name__ == "__main__":
    unittest.main()
