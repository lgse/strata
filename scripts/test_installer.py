"""Focused tests for the interactive Bash installer helpers."""

import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "install.sh"
BASH = shutil.which("bash")


def bash(script: str, *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    test_env = os.environ.copy()
    test_env["STRATA_INSTALLER_TESTING"] = "1"
    if env:
        test_env.update(env)
    with tempfile.TemporaryDirectory() as directory:
        installer = pathlib.Path(directory) / "install.sh"
        installer.write_text(
            INSTALLER.read_text().replace(
                "/usr/share/omarchy/version", f"{directory}/system-version"
            )
        )
        return subprocess.run(
            [BASH, "-c", f'source "$2"; {script}', "bash", str(INSTALLER), str(installer)],
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

    def test_volume_monitor_is_required_but_smb_is_optional(self) -> None:
        result = bash('printf "%s\\n" "${REQUIRED_PACKAGES[@]}"')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("gvfs", result.stdout.splitlines())
        self.assertNotIn("gvfs-smb", result.stdout.splitlines())
        self.assertNotIn("github-cli", result.stdout.splitlines())

    def test_banner_remains_readable_without_terminal_color(self) -> None:
        result = bash("show_banner", env={"NO_COLOR": "1"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("S T R A T A", result.stdout)
        self.assertIn("Navigate every layer.", result.stdout)
        self.assertIn("Interactive installer", result.stdout)
        self.assertNotIn("\033", result.stdout)

    def test_provenance_verification_is_optional_without_github_cli(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = bash(
                'PATH="$EMPTY_PATH"; verify_provenance /tmp/strata.tar.gz',
                env={"EMPTY_PATH": directory},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("GitHub CLI is unavailable", result.stderr)

    def test_provenance_verification_is_optional_without_github_authentication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fake_gh = pathlib.Path(directory) / "gh"
            fake_gh.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\nexit 1\n', encoding="utf-8"
            )
            fake_gh.chmod(0o755)
            calls = pathlib.Path(directory) / "calls"
            result = bash(
                'PATH="$FAKE_PATH"; verify_provenance /tmp/strata.tar.gz',
                env={"FAKE_PATH": directory, "CALLS": str(calls)},
            )
            recorded_calls = calls.read_text().splitlines()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(recorded_calls, ["auth status --hostname github.com"])
        self.assertIn("GitHub CLI is not authenticated", result.stderr)

    def test_authenticated_provenance_verification_remains_mandatory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fake_gh = pathlib.Path(directory) / "gh"
            fake_gh.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\n'
                'case "$*" in "auth status"*) exit 0;; *) exit "$VERIFY_RESULT";; esac\n',
                encoding="utf-8",
            )
            fake_gh.chmod(0o755)
            calls = pathlib.Path(directory) / "calls"
            result = bash(
                'PATH="$FAKE_PATH"; verify_provenance /tmp/strata.tar.gz',
                env={"FAKE_PATH": directory, "CALLS": str(calls), "VERIFY_RESULT": "9"},
            )
            recorded_calls = calls.read_text().splitlines()
        self.assertEqual(result.returncode, 9, result.stderr)
        self.assertEqual(
            recorded_calls,
            [
                "auth status --hostname github.com",
                "attestation verify /tmp/strata.tar.gz --repo lgse/strata",
            ],
        )

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

    def test_file_chooser_flags_are_independent_and_explicit(self) -> None:
        for flag, expected in [("--with-file-chooser", "yes"), ("--without-file-chooser", "no")]:
            with self.subTest(flag=flag):
                result = bash(
                    f'parse_args {flag}; '
                    'printf "%s %s %s %s" "$NON_INTERACTIVE" "$WITH_FILE_CHOOSER" '
                    '"$WITH_FILE_MANAGER" "$WITH_FOLDER_ASSOCIATION"'
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, f"yes {expected} ask ask")

    def test_file_chooser_runs_only_the_permanently_installed_binary_after_consent(self) -> None:
        cases = [
            ("parse_args --non-interactive", []),
            ("parse_args --with-file-manager", []),
            ("parse_args --with-file-chooser", ["--install-portal"]),
            ("parse_args --without-file-chooser", ["--dismiss-portal-prompt"]),
            ("prompt() { return 1; }", ["--dismiss-portal-prompt"]),
            ("prompt() { return 0; }", ["--install-portal"]),
        ]
        for setup, expected in cases:
            with self.subTest(setup=setup), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                extracted = root / "archive"
                (extracted / "portal").mkdir(parents=True)
                (extracted / "portal/strata.portal").write_text("[portal]\\n", encoding="utf-8")
                binary = root / "permanent bin/strata"
                binary.parent.mkdir()
                binary.write_text('#!/bin/bash\nprintf "%s\\n" "$@" >> "$CALLS"\n', encoding="utf-8")
                binary.chmod(0o755)
                calls = root / "calls"
                result = bash(
                    f'{setup}; BIN_PATH="$PERMANENT_BINARY"; configure_file_chooser "$EXTRACTED" no',
                    env={"PERMANENT_BINARY": str(binary), "EXTRACTED": str(extracted), "CALLS": str(calls)},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls.read_text().splitlines() if calls.exists() else [], expected)

    def test_old_releases_do_not_receive_unknown_portal_arguments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for flags, succeeds in [("", True), ("--non-interactive", True), ("--with-file-chooser", False)]:
                with self.subTest(flags=flags):
                    result = bash(
                        f'parse_args {flags}; BIN_PATH=/nonexistent/strata; configure_file_chooser "$EXTRACTED" no',
                        env={"EXTRACTED": directory},
                    )
                    self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                    self.assertNotIn("No such file", result.stderr)

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

    def test_interactive_pacman_reads_from_the_terminal_not_stdin(self) -> None:
        source = INSTALLER.read_text(encoding="utf-8")
        self.assertIn('sudo pacman -S --needed -- "$@" </dev/tty', source)

    def test_github_cli_requirement_is_skipped_when_gh_is_already_on_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fake_pacman = pathlib.Path(directory) / "pacman"
            fake_pacman.write_text('#!/bin/sh\n[ "$1" = "-Q" ] && exit 1\nexit 0\n')
            fake_pacman.chmod(0o755)
            result = bash(
                "REQUIRED_PACKAGES=(github-cli); NON_INTERACTIVE=yes; "
                "gh() { :; }; "
                'run_pacman() { printf "run_pacman called: %s\\n" "$*" >&2; }; '
                "install_arch_dependencies",
                env={"PATH": directory},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already installed", result.stdout)
        self.assertNotIn("run_pacman called", result.stderr)

    def test_github_cli_requirement_still_installs_when_gh_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fake_pacman = pathlib.Path(directory) / "pacman"
            fake_pacman.write_text('#!/bin/sh\n[ "$1" = "-Q" ] && exit 1\nexit 0\n')
            fake_pacman.chmod(0o755)
            result = bash(
                "REQUIRED_PACKAGES=(github-cli); NON_INTERACTIVE=yes; "
                'run_pacman() { printf "run_pacman called: %s\\n" "$*" >&2; }; '
                "install_arch_dependencies",
                env={"PATH": directory},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("run_pacman called: github-cli", result.stderr)

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

    def test_omarchy_major_from_requires_a_whole_version_token(self) -> None:
        cases = [
            ("3.8.5", "3"),
            ("1:4.0.0-1", "4"),
            ("4.0.0-1", "4"),
            ("4.0.0.alpha", "4"),
            ("Omarchy 2.3.1", ""),
            ("5.4.0", ""),
            ("dev (b280f130)", ""),
        ]
        for output, expected in cases:
            with self.subTest(output=output):
                result = bash(f'omarchy_major_from "{output}"')
                self.assertEqual(result.stdout.strip(), expected)
                if expected:
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    self.assertEqual(result.returncode, 1, result.stderr)

    def test_omarchy_dev_hash_is_not_a_major(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            result = bash(
                "detect_omarchy_major",
                env={
                    "HOME": home,
                    "PATH": f"{ROOT / 'scripts' / 'testdata' / 'omarchy-dev'}:{os.environ['PATH']}",
                },
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "")

    def test_omarchy_2_3_1_command_is_not_major_3(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory) / "home"
            home.mkdir()
            fake = pathlib.Path(directory) / "omarchy"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' 'Omarchy 2.3.1'\n", encoding="utf-8")
            fake.chmod(0o755)
            result = bash(
                "detect_omarchy_major",
                env={"HOME": str(home), "PATH": f"{directory}:{os.environ['PATH']}"},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "")

    def test_omarchy_dev_command_falls_back_to_version_file(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            version = pathlib.Path(home) / ".local" / "share" / "omarchy" / "version"
            version.parent.mkdir(parents=True)
            version.write_text("4.0.0.alpha\n", encoding="utf-8")
            result = bash(
                "detect_omarchy_major",
                env={
                    "HOME": home,
                    "PATH": f"{ROOT / 'scripts' / 'testdata' / 'omarchy-dev'}:{os.environ['PATH']}",
                },
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "4")

    def test_detected_omarchy_4_from_version_file_writes_lua_bindings(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            version = pathlib.Path(home) / ".local" / "share" / "omarchy" / "version"
            version.parent.mkdir(parents=True)
            version.write_text("4.0.0.alpha\n", encoding="utf-8")
            result = bash(
                'BIN_PATH="$HOME/.local/bin/strata"; '
                "major=$(detect_omarchy_major); "
                'configure_omarchy_bindings "$major"',
                env={
                    "HOME": home,
                    "HYPRLAND_INSTANCE_SIGNATURE": "",
                    "PATH": f"{ROOT / 'scripts' / 'testdata' / 'omarchy-dev'}:{os.environ['PATH']}",
                },
            )
            lua = pathlib.Path(home) / ".config" / "hypr" / "bindings.lua"
            conf = pathlib.Path(home) / ".config" / "hypr" / "bindings.conf"
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(lua.is_file())
            self.assertFalse(conf.exists())
            self.assertIn("strata-installer: file-manager start", lua.read_text())
            self.assertIn("Omarchy 4 file-manager shortcuts now open Strata.", result.stdout)

    def test_omarchy_detection_without_command_is_not_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            result = bash(
                'PATH="$HOME"; value=$(detect_omarchy_major); printf "%s" "$value"',
                env={"HOME": home},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "")

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
