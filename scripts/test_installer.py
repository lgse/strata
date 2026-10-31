"""Focused tests for the interactive Bash installer helpers."""

import fcntl
import hashlib
import http.server
import io
import json
import os
import pathlib
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tarfile
import tempfile
import termios
import threading
import time
import unittest

import sign_update_manifest as signing
from test_sign_update_manifest import test_key


ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "install.sh"
BASH = shutil.which("bash")
SIGNED_FIXTURES = ROOT / "tests/fixtures/signed-update"
PRODUCTION_KEYS = json.loads((ROOT / "data/update-keys.json").read_text(encoding="utf-8"))


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


class InstallerTerminal:
    def __init__(self, installer: pathlib.Path, environment: dict[str, str], stdout: pathlib.Path | None = None, *, piped: bool = False):
        self.pid, self.master = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 88, 0, 0))
            if stdout is not None:
                with stdout.open("w") as stream:
                    os.dup2(stream.fileno(), 1)
            arguments = [BASH, str(installer)]
            if piped:
                arguments = [BASH, "-o", "pipefail", "-c", 'cat "$1" | bash', "bash", str(installer)]
            os.execve(BASH, arguments, environment)
        self.output = ""
        self.returncode = None

    def read(self, timeout: float = 0.1) -> None:
        if select.select([self.master], [], [], timeout)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                chunk = b""
            self.output += chunk.decode("utf-8", errors="replace")
        if self.returncode is None:
            waited, status = os.waitpid(self.pid, os.WNOHANG)
            if waited:
                self.returncode = os.waitstatus_to_exitcode(status)

    def expect(self, text: str, timeout: float = 10) -> None:
        deadline = time.monotonic() + timeout
        while text not in self.output and time.monotonic() < deadline:
            self.read()
            if self.returncode is not None:
                break
        if text not in self.output:
            raise AssertionError(f"Installer did not show {text!r}:\n{self.output}")

    def send(self, keys: bytes) -> None:
        os.write(self.master, keys)

    def finish(self, timeout: float = 15) -> int:
        deadline = time.monotonic() + timeout
        while self.returncode is None and time.monotonic() < deadline:
            self.read()
        if self.returncode is None:
            raise AssertionError(f"Installer did not exit:\n{self.output}")
        self.read(0)
        return self.returncode

    def close(self) -> None:
        if self.returncode is None:
            os.killpg(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.master)


class InstallerFixture:
    def __init__(
        self, *, omarchy: bool = False,
        release_files: dict[str, str | None] | None = None, signing_seed: int = 7,
    ):
        self.directory = tempfile.TemporaryDirectory(prefix="strata-installer-")
        self.root = pathlib.Path(self.directory.name)
        self.home = self.root / "home"
        self.commands = self.root / "commands"
        self.calls = self.root / "calls"
        self.release = self.root / "release"
        self.terminals = []
        self.home.mkdir()
        self.commands.mkdir()
        self.release.mkdir()
        (self.root / "tmp").mkdir()
        (self.root / "runtime").mkdir(mode=0o700)
        for name in (
            "bash", "uname", "getconf", "awk", "sort", "head", "install", "sed", "grep",
            "dirname", "mkdir", "cp", "rm", "tar", "gzip", "sha256sum", "mktemp", "sleep", "cat",
            "tee", "stty", "date", "tail", "wc", "openssl", "timeout",
        ):
            (self.commands / name).symlink_to(shutil.which(name))
        distro = self.root / "os-release"
        distro.write_text('ID=manjaro\nID_LIKE=arch\nPRETTY_NAME="Manjaro Linux"\n', encoding="utf-8")
        # The installer pins the trusted key; the release is signed with the same key
        # unless a test asks for an untrusted one.
        self.key, self.public_key = test_key(self.root, 7)
        self.signing_key, self.signing_public_key = self.key, self.public_key
        if signing_seed != 7:
            self.signing_key, self.signing_public_key = test_key(self.root, signing_seed)
        self.installer = self.root / "install.sh"
        self.installer.write_text(
            INSTALLER.read_text(encoding="utf-8")
            .replace("/etc/os-release", str(distro))
            .replace("/usr/share/omarchy/version", str(self.root / "system-version"))
            .replace(PRODUCTION_KEYS[0], self.public_key),
            encoding="utf-8",
        )
        self.environment = os.environ.copy()
        for name in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_ADDRESS", "STRATA_INSTALLER_TESTING"):
            self.environment.pop(name, None)
        self.environment.update({
            "HOME": str(self.home), "PATH": f"{self.commands}:{self.home / '.local/bin'}",
            "XDG_DATA_HOME": str(self.home / "data"), "XDG_CONFIG_HOME": str(self.home / "config"),
            "XDG_STATE_HOME": str(self.home / "state"), "XDG_RUNTIME_DIR": str(self.root / "runtime"),
            "TMPDIR": str(self.root / "tmp"), "CALLS": str(self.calls),
            "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "LANGUAGE": "", "TERM": "xterm-256color",
            "HYPRLAND_INSTANCE_SIGNATURE": "",
        })
        self.environment.pop("NO_COLOR", None)
        target = subprocess.check_output(["uname", "-m"], text=True).strip()
        target = "aarch64" if target in ("aarch64", "arm64") else "x86_64"
        self.archive_name = f"strata-1.0.0-{target}-unknown-linux-gnu.tar.gz"
        archive = self.release / self.archive_name
        with tarfile.open(archive, "w:gz") as bundle:
            files = {
                "strata": '#!/bin/bash\nprintf "%s %s\\n" "$0" "$*" >> "$CALLS"\n',
                "portal/strata.portal": "[portal]\n",
                "udiskie/unlock": "supported\n",
                "io.github.lgse.Strata.svg": "<svg/>\n",
            }
            for name in ("io.github.lgse.Strata.desktop", "io.github.lgse.Strata.FileManager1.service"):
                files[name] = (ROOT / "data" / name).read_text(encoding="utf-8")
            files.update(release_files or {})
            for name, content in files.items():
                if content is None:
                    continue
                member = tarfile.TarInfo(f"{self.archive_name.removesuffix('.tar.gz')}/{name}")
                encoded = content.encode()
                member.size = len(encoded)
                member.mode = 0o755 if name == "strata" else 0o644
                bundle.addfile(member, io.BytesIO(encoded))
        other = "aarch64" if target == "x86_64" else "x86_64"
        (self.release / f"strata-1.0.0-{other}-unknown-linux-gnu.tar.gz").write_bytes(b"other architecture\n")
        self.sign_release()
        self.environment["FIXTURE_ARCHIVE"] = str(archive)
        self.environment["FIXTURE_RELEASE"] = str(self.release)
        self.command("curl", '''output=""
while (($#)); do
  case $1 in
    --output|-o) output=$2; shift 2 ;;
    --max-filesize|-w) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
printf "curl %s\\n" "$url" >> "$CALLS"
case $url in
  */latest) printf 'https://github.com/lgse/strata/releases/tag/v1.0.0' ;;
  *.tar.gz) cp "$FIXTURE_ARCHIVE" "$output" ;;
  *) [[ -f $FIXTURE_RELEASE/${url##*/} ]] || exit 22; cp "$FIXTURE_RELEASE/${url##*/}" "$output" ;;
esac
''')
        self.command("pacman", '''if [[ $1 == -Q ]]; then
  [[ $2 != gtksourceview5 && $2 != dcraw && $2 != gvfs-smb && $2 != xdg-desktop-portal ]]
else
  printf "pacman %s\\n" "$*" >> "$CALLS"
fi
''')
        self.command("sudo", '''printf "sudo %s\\n" "$*" >> "$CALLS"
[[ ${1:-} != -n ]] || shift
exec "$@"
''')
        self.command("xdg-mime", '''printf "xdg-mime %s\\n" "$*" >> "$CALLS"
[[ $1 != query ]] || printf 'io.github.lgse.Strata.desktop\\n'
''')
        for name in ("gdbus", "update-desktop-database", "gtk-update-icon-cache"):
            self.command(name, f'printf "{name} %s\\n" "$*" >> "$CALLS"\n')
        if omarchy:
            self.command("omarchy", "printf '4.0.4\\n'\n")

    def command(self, name: str, body: str) -> None:
        path = self.commands / name
        path.write_text("#!/bin/bash\nset -euo pipefail\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def sign_release(self, version: str = "1.0.0") -> None:
        manifest = signing.manifest_bytes(self.release, version, "b" * 40)
        signatures = signing.sign_manifest(manifest, [self.signing_key], [self.signing_public_key])
        (self.release / signing.MANIFEST_NAME).write_bytes(manifest)
        (self.release / signing.SIGNATURES_NAME).write_bytes(signatures)

    def run(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [BASH, str(self.installer), *arguments], env=self.environment,
            check=False, capture_output=True, text=True, timeout=20,
        )

    def terminal(self, stdout: pathlib.Path | None = None, *, piped: bool = False) -> InstallerTerminal:
        terminal = InstallerTerminal(self.installer, self.environment, stdout, piped=piped)
        self.terminals.append(terminal)
        return terminal

    def recorded_calls(self) -> list[str]:
        return self.calls.read_text().splitlines() if self.calls.exists() else []

    def recorded_application_calls(self) -> list[str]:
        return [call for call in self.recorded_calls() if call.startswith(str(self.root))]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        for terminal in self.terminals:
            terminal.close()
        self.directory.cleanup()


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

    def test_redirected_output_has_no_terminal_controls(self) -> None:
        result = bash(
            'setup_terminal; show_banner; info complete; warn warning; die failed',
            env={"NO_COLOR": "1"},
        )
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("\033", result.stdout + result.stderr)

    def test_pinned_release_keys_match_the_updater_trust_store(self) -> None:
        result = bash('printf "%s\\n" "${RELEASE_PUBLIC_KEYS[@]}"')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), PRODUCTION_KEYS)

    def test_release_signature_verification_accepts_only_a_trusted_signature(self) -> None:
        fixture_key = (SIGNED_FIXTURES / "public-key.hex").read_text().strip()
        manifest = (SIGNED_FIXTURES / signing.MANIFEST_NAME).read_bytes()
        signatures = (SIGNED_FIXTURES / signing.SIGNATURES_NAME).read_bytes()
        pretty = json.dumps(json.loads(signatures), indent=2).encode()
        for name, key, manifest_bytes, signature_bytes, accepted in (
            ("fixture", fixture_key, manifest, signatures, True),
            ("rotation", f"{'f' * 64} {fixture_key}", manifest, signatures, True),
            ("tampered manifest", fixture_key, manifest.replace(b"0.12.0", b"0.12.1"), signatures, False),
            ("reserialized manifest", fixture_key, manifest.rstrip(b"\n"), signatures, False),
            ("untrusted key", "f" * 64, manifest, signatures, False),
            ("non-canonical signatures", fixture_key, manifest, pretty, False),
            ("empty signatures", fixture_key, manifest, b'{"schema":1,"signatures":[]}\n', False),
        ):
            with self.subTest(name), tempfile.TemporaryDirectory() as directory:
                pathlib.Path(directory, "manifest").write_bytes(manifest_bytes)
                pathlib.Path(directory, "signatures").write_bytes(signature_bytes)
                result = bash(
                    'RELEASE_PUBLIC_KEYS=($KEYS); TEMP_DIR=$DIR; '
                    'verify_release_signature "$DIR/manifest" "$DIR/signatures"',
                    env={"KEYS": key, "DIR": directory},
                )
                self.assertEqual(result.returncode, 0 if accepted else 1, result.stderr)
                if not accepted:
                    self.assertIn("No trusted release key signed", result.stderr)

    def test_signed_artifact_must_match_the_requested_release_and_architecture(self) -> None:
        manifest = SIGNED_FIXTURES / signing.MANIFEST_NAME
        for version, target, expected in (
            ("0.12.0", "x86_64-unknown-linux-gnu", "9e08fb7fa1ad03d91a140c2c89dc9be65ff0c4fecb7cc2bbc16f81e6bc7e556d 22"),
            ("0.12.0", "aarch64-unknown-linux-gnu", "9e08fb7fa1ad03d91a140c2c89dc9be65ff0c4fecb7cc2bbc16f81e6bc7e556d 22"),
            ("0.12.1", "x86_64-unknown-linux-gnu", "does not describe Strata 0.12.1"),
            ("0.12.0", "riscv64gc-unknown-linux-gnu", "does not list strata-0.12.0-riscv64gc-unknown-linux-gnu.tar.gz"),
        ):
            with self.subTest(version=version, target=target):
                result = bash(
                    'read_signed_artifact "$MANIFEST" "$VERSION" "$TARGET" "strata-$VERSION-$TARGET.tar.gz"; '
                    'printf "%s %s" "$SIGNED_ARCHIVE_SHA256" "$SIGNED_ARCHIVE_SIZE"',
                    env={"MANIFEST": str(manifest), "VERSION": version, "TARGET": target},
                )
                if expected.startswith("does not"):
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertIn(expected, result.stderr)
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, expected)

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
            (("--non-interactive",), None, ["--install-portal"]),
            (("--with-file-manager",), None, ["--install-portal"]),
            (("--with-file-chooser",), None, ["--install-portal"]),
            (("--without-file-chooser",), None, ["--dismiss-portal-prompt"]),
            ((), "recommended", ["--install-portal"]),
            ((), "deselected", ["--dismiss-portal-prompt"]),
        ]
        for arguments, interactive_choice, expected in cases:
            with self.subTest(arguments=arguments, choice=interactive_choice), InstallerFixture() as fixture:
                home = fixture.home / "with spaces"
                home.mkdir()
                fixture.environment["HOME"] = str(home)
                if interactive_choice is None:
                    result = fixture.run(*arguments)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                else:
                    terminal = fixture.terminal(piped=True)
                    terminal.expect("Enter continue")
                    if interactive_choice == "deselected":
                        terminal.send(b"\033[B\r")
                        terminal.expect("Space toggle")
                        terminal.send(b"\033[B\033[B\033[B \r")
                    else:
                        terminal.send(b"\r")
                    self.assertEqual(terminal.finish(), 0, terminal.output)
                binary = home / ".local/bin/strata"
                self.assertEqual(fixture.recorded_application_calls(), [f"{binary} {argument}" for argument in expected])

    def test_old_releases_do_not_receive_unknown_portal_arguments(self) -> None:
        for arguments, succeeds in [((), True), (("--non-interactive",), True), (("--with-file-chooser",), False)]:
            with self.subTest(arguments=arguments), InstallerFixture(release_files={"portal/strata.portal": None}) as fixture:
                if arguments:
                    result = fixture.run(*arguments)
                    self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                    self.assertNotIn("No such file", result.stderr)
                    output = result.stdout + result.stderr
                else:
                    terminal = fixture.terminal(piped=True)
                    terminal.expect("Enter continue")
                    terminal.send(b"\r")
                    self.assertEqual(terminal.finish(), 0, terminal.output)
                    output = terminal.output
                self.assertEqual("does not include file chooser integration; skipping it" in output, succeeds)
                self.assertEqual(fixture.recorded_application_calls(), [])
                self.assertEqual((fixture.home / ".local/bin/strata").exists(), succeeds)
                if not succeeds:
                    self.assertFalse(any(call.startswith("sudo") for call in fixture.recorded_calls()))

    def test_non_interactive_mode_installs_everything_unless_declined(self) -> None:
        for name, arguments, integrated, packages in (
            ("defaults", ("--non-interactive",), True, {"gvfs-smb", "dcraw", "xdg-desktop-portal"}),
            (
                "declined",
                ("--without-desktop-entry", "--without-file-manager", "--without-file-chooser", "--without-smb", "--without-raw"),
                False,
                set(),
            ),
        ):
            with self.subTest(name), InstallerFixture() as fixture:
                result = fixture.run(*arguments)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                calls = fixture.recorded_calls()
                transaction = next(call for call in calls if call.startswith("pacman -S"))
                self.assertEqual({package for package in packages if package in transaction}, packages)
                self.assertEqual(any(package in transaction for package in ("gvfs-smb", "dcraw", "xdg-desktop-portal")), integrated)
                self.assertEqual("xdg-mime default io.github.lgse.Strata.desktop inode/directory" in calls, integrated)
                self.assertEqual((fixture.home / "data/applications/io.github.lgse.Strata.desktop").exists(), integrated)
                self.assertEqual((fixture.home / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service").exists(), integrated)
                self.assertEqual(
                    fixture.recorded_application_calls(),
                    [f"{fixture.home}/.local/bin/strata " + ("--install-portal" if integrated else "--dismiss-portal-prompt")],
                )
                self.assertFalse((fixture.home / ".config/hypr").exists())

    def test_file_manager_service_uses_the_installed_binary(self) -> None:
        for reload_status in (0, 1, None):
            with self.subTest(reload_status=reload_status), tempfile.TemporaryDirectory() as home:
                root = pathlib.Path(home)
                commands = root / "commands"
                commands.mkdir()
                for name in ("mkdir", "dirname", "install", "sed", "grep", "head", "timeout"):
                    (commands / name).symlink_to(shutil.which(name))
                if reload_status is not None:
                    gdbus = commands / "gdbus"
                    gdbus.write_text(
                        '#!/bin/sh\nprintf "%s\\n" "$@" > "$CALLS"\n'
                        '[ -r "$XDG_DATA_HOME/dbus-1/services/"'
                        '"io.github.lgse.Strata.FileManager1.service" ] || exit 91\n'
                        f"exit {reload_status}\n",
                        encoding="utf-8",
                    )
                    gdbus.chmod(0o755)
                calls = root / "calls"
                result = bash(
                    'TEMP_DIR="$HOME/tmp"; mkdir -p "$TEMP_DIR"; '
                    'BIN_PATH="$HOME/.local/bin/strata"; '
                    'install_file_manager_service "$(dirname "$1")/data"; '
                    'printf "installer continued\\n"',
                    env={
                        "HOME": home,
                        "XDG_DATA_HOME": f"{home}/data",
                        "PATH": str(commands),
                        "CALLS": str(calls),
                    },
                )
                service = root / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service"
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(
                    f"Exec={home}/.local/bin/strata --gapplication-service", service.read_text()
                )
                self.assertIn("installer continued", result.stdout)
                if reload_status is not None:
                    self.assertTrue(calls.exists(), "The installed service must be reloaded")
                    self.assertEqual(
                        calls.read_text().splitlines(),
                        [
                            "call", "--session", "--timeout", "5",
                            "--dest", "org.freedesktop.DBus",
                            "--object-path", "/org/freedesktop/DBus",
                            "--method", "org.freedesktop.DBus.ReloadConfig",
                        ],
                    )
                else:
                    self.assertFalse(calls.exists())
                active_message = 'Strata now handles "Open file location" requests.'
                if reload_status == 0:
                    self.assertIn(active_message, result.stdout)
                    self.assertEqual(result.stderr, "")
                else:
                    self.assertNotIn(active_message, result.stdout)
                    self.assertIn("Log out and back in", result.stderr)

    def test_file_manager_service_continues_when_the_bus_never_replies(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            root = pathlib.Path(home)
            commands = root / "commands"
            commands.mkdir()
            for name in ("mkdir", "dirname", "install", "sed", "grep", "head", "timeout", "sleep"):
                (commands / name).symlink_to(shutil.which(name))
            gdbus = commands / "gdbus"
            gdbus.write_text("#!/bin/sh\nexec sleep 20\n", encoding="utf-8")
            gdbus.chmod(0o755)
            started = time.monotonic()
            result = bash(
                'TEMP_DIR="$HOME/tmp"; mkdir -p "$TEMP_DIR"; '
                'BIN_PATH="$HOME/.local/bin/strata"; '
                'install_file_manager_service "$(dirname "$1")/data"; '
                'printf "installer continued\\n"',
                env={"HOME": home, "XDG_DATA_HOME": f"{home}/data", "PATH": str(commands)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertLess(time.monotonic() - started, 15)
            self.assertIn("installer continued", result.stdout)
            self.assertIn("Log out and back in", result.stderr)

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
                'gdbus() { printf "unexpected reload\\n" >&2; return 1; }; '
                'install_file_manager_service "$(dirname "$1")/data"',
                env={"HOME": home, "XDG_DATA_HOME": f"{home}/data"},
            )
            target = service_dir / "io.github.lgse.Strata.FileManager1.service"
            self.assertFalse(target.exists())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Another per-user FileManager1 provider", result.stderr)
        self.assertNotIn("unexpected reload", result.stderr)

    def test_file_manager_service_does_not_reload_after_a_failed_write(self) -> None:
        with tempfile.TemporaryDirectory() as home:
            result = bash(
                'TEMP_DIR="$HOME/tmp"; mkdir -p "$TEMP_DIR"; '
                'BIN_PATH="$HOME/.local/bin/strata"; '
                'install() { if [[ $1 == -Dm644 ]]; then return 9; else command install "$@"; fi; }; '
                'gdbus() { printf "unexpected reload\\n" >&2; }; '
                'install_file_manager_service "$(dirname "$1")/data"',
                env={"HOME": home, "XDG_DATA_HOME": f"{home}/data"},
            )
            target = pathlib.Path(home) / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service"
            self.assertFalse(target.exists())
        self.assertEqual(result.returncode, 9, result.stderr)
        self.assertNotIn("unexpected reload", result.stderr)
        self.assertNotIn('Strata now handles "Open file location" requests.', result.stdout)

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

    def test_unsupported_omarchy_flag_fails_before_installing_anything(self) -> None:
        with InstallerFixture() as fixture:
            result = fixture.run("--with-omarchy-keybinds")
            self.assertEqual(fixture.recorded_calls(), [], result.stdout + result.stderr)
            self.assertFalse((fixture.home / ".local/bin/strata").exists())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Omarchy 3 or 4", result.stderr)

    def test_recommended_setup_installs_every_supported_integration(self) -> None:
        with InstallerFixture() as fixture:
            terminal = fixture.terminal(piped=True)
            terminal.expect("Enter continue")
            terminal.send(b"\r")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            self.assertTrue((fixture.home / ".local/bin/strata").is_file())
            self.assertTrue((fixture.home / "data/applications/io.github.lgse.Strata.desktop").is_file())
            self.assertTrue((fixture.home / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service").is_file())
            calls = fixture.recorded_calls()
            self.assertIn("xdg-mime default io.github.lgse.Strata.desktop inode/directory", calls)
            transaction = next(call for call in calls if call.startswith("pacman -S"))
            for package in ("gvfs-smb", "dcraw", "xdg-desktop-portal"):
                self.assertIn(package, transaction)
            self.assertEqual(fixture.recorded_application_calls(), [f"{fixture.home}/.local/bin/strata --install-portal"])
            self.assertNotIn("stay unchanged", terminal.output)
            self.assertIn("Ready to explore", terminal.output)
            self.assertEqual(list((fixture.root / "tmp").glob("*.log")), [])

    def test_release_is_authenticated_before_the_archive_is_downloaded(self) -> None:
        with InstallerFixture() as fixture:
            result = fixture.run("--non-interactive")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            downloads = "https://github.com/lgse/strata/releases/download/v1.0.0"
            self.assertEqual(
                [call for call in fixture.recorded_calls() if call.startswith("curl")],
                [
                    "curl https://github.com/lgse/strata/releases/latest",
                    f"curl {downloads}/strata-update-manifest.json",
                    f"curl {downloads}/strata-update-manifest.signatures.json",
                    f"curl {downloads}/{fixture.archive_name}",
                ],
            )

    def test_custom_setup_starts_with_everything_selected_and_applies_deselections(self) -> None:
        with InstallerFixture() as fixture:
            terminal = fixture.terminal()
            terminal.expect("Enter continue")
            terminal.send(b"\033[B\r")
            terminal.expect("Space toggle")
            terminal.send(b"\033[B \033[B\033[B \033[B \r")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            calls = fixture.recorded_calls()
            self.assertFalse(any(call.startswith("xdg-mime") for call in calls), calls)
            transaction = next(call for call in calls if call.startswith("pacman -S"))
            self.assertNotIn("gvfs-smb", transaction)
            self.assertNotIn("xdg-desktop-portal", transaction)
            self.assertIn("dcraw", transaction)
            self.assertEqual(fixture.recorded_application_calls(), [f"{fixture.home}/.local/bin/strata --dismiss-portal-prompt"])
            self.assertTrue((fixture.home / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service").is_file())
            self.assertTrue((fixture.home / "data/applications/io.github.lgse.Strata.desktop").is_file())

    def test_deselecting_a_required_launcher_keeps_only_reveal_integration(self) -> None:
        with InstallerFixture() as fixture:
            terminal = fixture.terminal()
            terminal.expect("Enter continue")
            terminal.send(b"\033[B\r")
            terminal.expect("Space toggle")
            terminal.send(b"\033[B \033[A \r")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            self.assertFalse((fixture.home / "data/applications/io.github.lgse.Strata.desktop").exists())
            self.assertTrue((fixture.home / "data/dbus-1/services/io.github.lgse.Strata.FileManager1.service").is_file())
            self.assertFalse(any(call.startswith("xdg-mime") for call in fixture.recorded_calls()))

    def test_omarchy_defaults_to_the_file_chooser_and_shortcuts(self) -> None:
        for name, arguments, expected, integrated in (
            ("recommended", (), ["--install-portal"], True),
            ("unattended", ("--non-interactive",), ["--install-portal"], True),
            ("declined", ("--without-file-chooser", "--without-omarchy-keybinds"), ["--dismiss-portal-prompt"], False),
        ):
            with self.subTest(name), InstallerFixture(omarchy=True) as fixture:
                if arguments:
                    result = fixture.run(*arguments)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    output = result.stdout
                else:
                    terminal = fixture.terminal()
                    terminal.expect("Enter continue")
                    terminal.send(b"\r")
                    self.assertEqual(terminal.finish(), 0, terminal.output)
                    output = terminal.output
                binary = fixture.home / ".local/bin/strata"
                self.assertEqual(fixture.recorded_application_calls(), [f"{binary} {argument}" for argument in expected])
                bindings = fixture.home / ".config/hypr/bindings.lua"
                self.assertEqual(bindings.exists(), integrated, output)
                self.assertEqual("Omarchy 4 file-manager shortcuts now open Strata." in output, integrated)
                self.assertEqual("Open and Save dialogs" in output, integrated)

    def test_without_flags_decline_defaults_and_their_dependents(self) -> None:
        for flag, expected in (
            ("--without-smb", "yes no ask ask ask ask ask ask ask"),
            ("--without-raw", "yes ask no ask ask ask ask ask ask"),
            ("--without-desktop-entry", "yes ask ask no no ask ask ask ask"),
            ("--without-folder-association", "yes ask ask ask no ask ask ask ask"),
            ("--without-file-manager", "yes ask ask ask no no ask ask ask"),
            ("--without-file-chooser", "yes ask ask ask ask ask no ask ask"),
            ("--without-omarchy-keybinds", "yes ask ask ask ask ask ask no ask"),
            ("--without-udiskie-unlock", "yes ask ask ask ask ask ask ask no"),
        ):
            with self.subTest(flag=flag):
                result = bash(
                    f"parse_args {flag}; "
                    'printf "%s %s %s %s %s %s %s %s %s" "$NON_INTERACTIVE" "$WITH_SMB" "$WITH_RAW" '
                    '"$WITH_DESKTOP_ENTRY" "$WITH_FOLDER_ASSOCIATION" "$WITH_FILE_MANAGER" '
                    '"$WITH_FILE_CHOOSER" "$WITH_OMARCHY_KEYBINDS" "$WITH_UDISKIE_UNLOCK"'
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, expected)

    def test_defaults_cover_only_integrations_the_system_supports(self) -> None:
        for arch_based, omarchy, udiskie, expected in (
            ("no", "", False, "yes yes yes yes ask ask ask ask"),
            ("yes", "", False, "yes yes yes yes yes yes ask ask"),
            ("yes", "4", True, "yes yes yes yes yes yes yes yes"),
            ("yes", "", True, "yes yes yes yes yes yes ask yes"),
        ):
            with self.subTest(arch_based=arch_based, omarchy=omarchy, udiskie=udiskie):
                result = bash(
                    f"udiskie_on_path() {{ {'true' if udiskie else 'false'}; }}; "
                    f'resolve_default_integrations {arch_based} "{omarchy}"; '
                    'printf "%s %s %s %s %s %s %s %s" "$WITH_DESKTOP_ENTRY" "$WITH_FOLDER_ASSOCIATION" '
                    '"$WITH_FILE_MANAGER" "$WITH_FILE_CHOOSER" "$WITH_SMB" "$WITH_RAW" '
                    '"$WITH_OMARCHY_KEYBINDS" "$WITH_UDISKIE_UNLOCK"'
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, expected)

    def test_custom_setup_offers_omarchy_only_when_supported(self) -> None:
        for omarchy in (False, True):
            with self.subTest(omarchy=omarchy), InstallerFixture(omarchy=omarchy) as fixture:
                terminal = fixture.terminal()
                terminal.expect("Enter continue")
                terminal.send(b"\033[B\r")
                terminal.expect("Space toggle")
                self.assertEqual("Omarchy file-manager shortcuts" in terminal.output, omarchy)
                terminal.send(b"q")
                self.assertEqual(terminal.finish(), 0, terminal.output)
                self.assertEqual(fixture.recorded_calls(), [])

    def test_plain_setup_can_install_without_a_launcher(self) -> None:
        with InstallerFixture() as fixture:
            fixture.environment["TERM"] = "dumb"
            terminal = fixture.terminal()
            terminal.expect("Choose [1]")
            terminal.send(b"2\n")
            terminal.expect("Toggle an option by number")
            terminal.send(b"1\n\n")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            self.assertTrue((fixture.home / ".local/bin/strata").is_file())
            self.assertFalse((fixture.home / "data/applications/io.github.lgse.Strata.desktop").exists())
            self.assertNotIn("\033", terminal.output)

    def test_redirected_installation_keeps_choices_on_the_terminal(self) -> None:
        with InstallerFixture() as fixture:
            output = fixture.root / "output"
            terminal = fixture.terminal(stdout=output)
            terminal.expect("Customize installation")
            terminal.send(b"\n")
            self.assertEqual(terminal.finish(), 0, terminal.output + output.read_text())
            self.assertTrue((fixture.home / ".local/bin/strata").is_file())
            self.assertNotIn("\033", output.read_text())

    def test_download_shows_real_curl_progress_before_installing(self) -> None:
        with InstallerFixture() as fixture:
            body = pathlib.Path(fixture.environment["FIXTURE_ARCHIVE"]).read_bytes()

            class ArchiveHandler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    chunk_size = max(1, len(body) // 60)
                    for start in range(0, len(body), chunk_size):
                        self.wfile.write(body[start:start + chunk_size])
                        self.wfile.flush()
                        time.sleep(0.035)

                def log_message(self, *_):
                    pass

            with http.server.ThreadingHTTPServer(("127.0.0.1", 0), ArchiveHandler) as server:
                worker = threading.Thread(target=server.serve_forever, daemon=True)
                fixture.environment["REAL_CURL"] = shutil.which("curl")
                fixture.environment["DOWNLOAD_URL"] = f"http://127.0.0.1:{server.server_port}/archive"
                curl = fixture.commands / "curl"
                curl.write_text(curl.read_text().replace(
                    '*.tar.gz) cp "$FIXTURE_ARCHIVE" "$output" ;;',
                    '*.tar.gz) exec "$REAL_CURL" --noproxy "*" --fail --progress-bar --output "$output" "$DOWNLOAD_URL" ;;',
                ))
                terminal = fixture.terminal()
                worker.start()
                try:
                    terminal.expect("Enter continue")
                    terminal.send(b"\r")
                    terminal.expect("%")
                    self.assertFalse((fixture.home / ".local/bin/strata").exists())
                    self.assertEqual(terminal.finish(), 0, terminal.output)
                    self.assertTrue((fixture.home / ".local/bin/strata").is_file())
                finally:
                    server.shutdown()
                    worker.join(timeout=5)

    def test_escape_keeps_an_existing_install_untouched(self) -> None:
        with InstallerFixture() as fixture:
            binary = fixture.home / ".local/bin/strata"
            binary.parent.mkdir(parents=True)
            binary.write_text("previous version", encoding="utf-8")
            terminal = fixture.terminal()
            terminal.expect("Enter continue")
            terminal.send(b"\033")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            self.assertEqual(binary.read_text(), "previous version")
            self.assertEqual(fixture.recorded_calls(), [])

    def test_no_color_keeps_terminal_navigation_without_color_sequences(self) -> None:
        with InstallerFixture() as fixture:
            fixture.environment["NO_COLOR"] = "1"
            terminal = fixture.terminal()
            terminal.expect("Enter continue")
            terminal.send(b"\r")
            self.assertEqual(terminal.finish(), 0, terminal.output)
            self.assertNotRegex(terminal.output, r"\x1b\[[0-9;]*m")
            self.assertTrue((fixture.home / ".local/bin/strata").is_file())

    def test_interrupt_stops_a_download_before_changing_the_machine(self) -> None:
        with InstallerFixture() as fixture:
            child_pid = fixture.root / "download-pid"
            fixture.environment["DOWNLOAD_PID"] = str(child_pid)
            curl = fixture.commands / "curl"
            curl.write_text(curl.read_text().replace(
                '*.tar.gz) cp "$FIXTURE_ARCHIVE" "$output" ;;',
                '*.tar.gz) printf "%s" "$$" > "$DOWNLOAD_PID"; exec sleep 30 ;;',
            ))
            terminal = fixture.terminal()
            terminal.expect("Enter continue")
            terminal.send(b"\r")
            terminal.expect("Download Strata 1.0.0")
            deadline = time.monotonic() + 3
            while not child_pid.exists() and time.monotonic() < deadline:
                terminal.read()
            self.assertTrue(child_pid.exists(), terminal.output)
            terminal.send(b"\x03")
            self.assertEqual(terminal.finish(), 130, terminal.output)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(child_pid.read_text()), 0)
            self.assertFalse((fixture.home / ".local/bin/strata").exists())
            self.assertFalse(any(call.startswith("sudo") for call in fixture.recorded_calls()))

    def test_dependencies_share_one_transaction_with_selected_optional_support(self) -> None:
        with InstallerFixture() as fixture:
            result = fixture.run("--with-raw", "--with-smb", "--with-file-chooser")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            calls = fixture.recorded_calls()
            transactions = [call for call in calls if call.startswith("pacman -S")]
            self.assertEqual(len(transactions), 1, calls)
            self.assertIn("gtksourceview5", transactions[0])
            self.assertIn("dcraw", transactions[0])
            self.assertIn("gvfs-smb", transactions[0])
            self.assertIn("xdg-desktop-portal", transactions[0])
            self.assertEqual(fixture.recorded_application_calls(), [f"{fixture.home}/.local/bin/strata --install-portal"])
            self.assertLess(
                calls.index(f"curl https://github.com/lgse/strata/releases/download/v1.0.0/{fixture.archive_name}"),
                calls.index(transactions[0]),
            )

    def test_untrusted_release_signature_leaves_the_machine_unchanged(self) -> None:
        with InstallerFixture(signing_seed=8) as fixture:
            result = fixture.run("--non-interactive")
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertFalse((fixture.home / ".local/bin/strata").exists())
            calls = fixture.recorded_calls()
            self.assertFalse(any(call.startswith("sudo") for call in calls))
            self.assertFalse(any(call.endswith(".tar.gz") for call in calls), calls)
            self.assertIn("No trusted release key signed strata-update-manifest.json", result.stderr)
            self.assertIn("Detailed output:", result.stderr)

    def test_release_without_signed_manifest_is_refused(self) -> None:
        for missing in (signing.MANIFEST_NAME, signing.SIGNATURES_NAME):
            with self.subTest(missing=missing), InstallerFixture() as fixture:
                (fixture.release / missing).unlink()
                result = fixture.run("--non-interactive")
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((fixture.home / ".local/bin/strata").exists())
                calls = fixture.recorded_calls()
                self.assertFalse(any(call.startswith("sudo") for call in calls))
                self.assertFalse(any(call.endswith(".tar.gz") for call in calls), calls)

    def test_oversized_release_metadata_is_refused_before_verification(self) -> None:
        with InstallerFixture() as fixture:
            signatures = fixture.release / signing.SIGNATURES_NAME
            signatures.write_bytes(signatures.read_bytes() + b" " * 8192)
            result = fixture.run("--non-interactive")
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("larger than 8192 bytes", result.stderr)
            self.assertFalse(any(call.endswith(".tar.gz") for call in fixture.recorded_calls()))

    def test_archive_that_differs_from_the_signed_digest_is_refused(self) -> None:
        with InstallerFixture() as fixture:
            archive = pathlib.Path(fixture.environment["FIXTURE_ARCHIVE"])
            signed = archive.read_bytes()
            for name, replacement in (
                ("replaced", signed[:-1] + bytes([signed[-1] ^ 0x01])),
                ("truncated", signed[:-1]),
            ):
                with self.subTest(name):
                    archive.write_bytes(replacement)
                    result = fixture.run("--non-interactive")
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn("the signed manifest expects", result.stderr)
                    self.assertFalse((fixture.home / ".local/bin/strata").exists())
                    self.assertFalse(any(call.startswith("sudo") for call in fixture.recorded_calls()))

    def test_missing_openssl_fails_before_downloading_anything(self) -> None:
        with InstallerFixture() as fixture:
            (fixture.commands / "openssl").unlink()
            result = fixture.run("--non-interactive")
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("Required command not found: openssl", result.stderr)
            self.assertEqual(fixture.recorded_calls(), [])

    def test_integration_failure_reports_that_the_application_is_installed(self) -> None:
        for flag, status, xdg_mime in (
            ("--with-folder-association", 17, "exit 17\n"),
            ("--with-folder-association", 124, "exec sleep 20\n"),
            ("--with-file-chooser", 1, None),
            ("--with-udiskie-unlock", 1, None),
        ):
            with self.subTest(flag=flag, status=status), InstallerFixture(release_files={
                "strata": '#!/bin/bash\nprintf "fixture integration failed\\n" >&2\nexit 17\n',
            }) as fixture:
                if xdg_mime is not None:
                    fixture.command("xdg-mime", xdg_mime)
                elif flag == "--with-udiskie-unlock":
                    fixture.command("udiskie", "exit 0\n")
                result = fixture.run(flag)
                self.assertEqual(result.returncode, status, result.stdout + result.stderr)
                self.assertTrue((fixture.home / ".local/bin/strata").is_file())
                self.assertIn("Strata is installed, but setup did not finish.", result.stderr)
                if flag != "--with-folder-association":
                    self.assertIn("fixture integration failed", result.stderr)
                    self.assertIn("fixture integration failed", next((fixture.root / "tmp").glob("*.log")).read_text())

    def test_integration_notices_remain_visible_and_logged_on_success(self) -> None:
        for flag in ("--with-file-chooser", "--with-udiskie-unlock"):
            for verbose in (False, True):
                with self.subTest(flag=flag, verbose=verbose), InstallerFixture(release_files={
                    "strata": '#!/bin/bash\nprintf "fixture restart needs attention\\n"\nprintf "fixture recovery instructions\\n" >&2\n',
                }) as fixture:
                    fixture.command("udiskie", "exit 0\n")
                    result = fixture.run(flag, *(["--verbose"] if verbose else []))
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn("fixture restart needs attention", result.stdout)
                    self.assertIn("fixture recovery instructions", result.stdout)
                    self.assertNotIn("Ready to explore", result.stdout)
                    self.assertIn("Installed — review setup details", result.stdout)
                    logs = list((fixture.root / "tmp").glob("*.log"))
                    self.assertEqual(len(logs), 1)
                    self.assertIn(str(logs[0]), result.stdout)
                    self.assertIn("fixture restart needs attention", logs[0].read_text())
                    self.assertIn("fixture recovery instructions", logs[0].read_text())

    def test_package_failure_is_in_the_retained_log(self) -> None:
        with InstallerFixture() as fixture:
            fixture.command("pacman", '''if [[ $1 == -Q ]]; then exit 1; fi
printf 'fixture package download failed\\n' >&2
exit 23
''')
            result = fixture.run("--non-interactive")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fixture package download failed", result.stdout + result.stderr)
            logs = list((fixture.root / "tmp").glob("*.log"))
            self.assertEqual(len(logs), 1)
            self.assertIn("fixture package download failed", logs[0].read_text())
            self.assertFalse((fixture.home / ".local/bin/strata").exists())

    def test_package_logging_preserves_terminal_prompts_without_recording_passwords(self) -> None:
        with InstallerFixture() as fixture:
            fixture.command("sudo", '''printf 'Fixture sudo password: ' >/dev/tty
IFS= read -rs password </dev/tty
[[ $password == fixture-secret ]] || exit 19
exec "$@"
''')
            fixture.command("pacman", '''if [[ $1 == -Q ]]; then exit 1; fi
printf 'Proceed with fixture packages? [Y/n] '
IFS= read -r answer
[[ $answer == n ]] || exit 18
printf 'fixture package operation cancelled\\n' >&2
exit 23
''')
            terminal = fixture.terminal(piped=True)
            terminal.expect("Enter continue")
            terminal.send(b"\r")
            terminal.expect("Fixture sudo password:")
            terminal.send(b"fixture-secret\n")
            terminal.expect("Proceed with fixture packages?")
            terminal.send(b"n\n")
            self.assertEqual(terminal.finish(), 23, terminal.output)
            logs = list((fixture.root / "tmp").glob("*.log"))
            self.assertEqual(len(logs), 1)
            self.assertIn("Proceed with fixture packages?", logs[0].read_text())
            self.assertIn("fixture package operation cancelled", logs[0].read_text())
            self.assertNotIn("fixture-secret", logs[0].read_text() + terminal.output)
            self.assertFalse((fixture.home / ".local/bin/strata").exists())

    def test_reinstall_without_consent_fails_before_downloading(self) -> None:
        with InstallerFixture() as fixture:
            binary = fixture.home / ".local/bin/strata"
            binary.parent.mkdir(parents=True)
            binary.write_text("previous version", encoding="utf-8")
            result = fixture.run("--non-interactive")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(binary.read_text(), "previous version")
            self.assertEqual(fixture.recorded_calls(), [])

    def test_installer_messages_follow_locale_precedence_and_keep_paths_literal(self) -> None:
        path = "/tmp/literal & %{version} [file]"
        for language, locale, expected in (
            ("zz:fr", "C.UTF-8", f"Installer dans {path}"),
            ("fr", "C", f"Install to {path}"),
            ("de", "C.UTF-8", f"Installation nach {path}"),
            ("es", "C.UTF-8", f"Instalar en {path}"),
            ("ja", "C.UTF-8", f"インストール先: {path}"),
            ("pt_PT", "C.UTF-8", f"Instalar em {path}"),
            ("ko", "C.UTF-8", f"설치 위치: {path}"),
            ("vi", "C.UTF-8", f"Cài đặt vào {path}"),
            ("it", "C.UTF-8", f"Installa in {path}"),
            ("ru", "C.UTF-8", f"Установить в {path}"),
            ("zh_CN", "C.UTF-8", f"安装到 {path}"),
            ("zh_TW", "C.UTF-8", f"安裝到 {path}"),
            ("zh-Hant-HK", "C.UTF-8", f"安裝到 {path}"),
        ):
            with self.subTest(language=language, locale=locale):
                result = bash(
                    'text destination "path=$VALUE"',
                    env={"LANGUAGE": language, "LC_ALL": locale, "VALUE": path},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, expected)

    def test_embedded_translations_match_the_shared_catalog(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/installer_messages.py")],
            check=False, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

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

    def test_udiskie_unlock_flag_implies_non_interactive_and_leaves_other_options_ask(
        self,
    ) -> None:
        result = bash(
            "parse_args --with-udiskie-unlock; "
            'printf "%s %s %s %s %s %s %s %s %s" "$NON_INTERACTIVE" "$WITH_UDISKIE_UNLOCK" '
            '"$WITH_SMB" "$WITH_RAW" "$WITH_DESKTOP_ENTRY" "$WITH_FOLDER_ASSOCIATION" '
            '"$WITH_FILE_MANAGER" "$WITH_FILE_CHOOSER" "$WITH_OMARCHY_KEYBINDS"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "yes yes ask ask ask ask ask ask ask")

    def test_non_interactive_alone_leaves_udiskie_unlock_ask(self) -> None:
        result = bash(
            "parse_args --non-interactive; "
            'printf "%s %s" "$NON_INTERACTIVE" "$WITH_UDISKIE_UNLOCK"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "yes ask")

    def test_udiskie_unlock_runs_only_the_permanently_installed_binary_after_consent(self) -> None:
        cases = [
            ("explicit consent", ("--with-udiskie-unlock",), True, True, True, ["--install-portal", "--install-udiskie-unlock"]),
            ("unspecified", ("--non-interactive",), True, True, True, ["--install-portal", "--install-udiskie-unlock"]),
            ("declined", ("--without-udiskie-unlock",), True, True, True, ["--install-portal"]),
            ("recommended", (), True, True, True, ["--install-portal", "--install-udiskie-unlock"]),
            ("no udiskie", ("--non-interactive",), False, True, True, ["--install-portal"]),
            ("missing udiskie", ("--with-udiskie-unlock",), False, True, False, []),
            ("old release unspecified", ("--non-interactive",), True, False, True, ["--install-portal"]),
            ("old release requested", ("--with-udiskie-unlock",), True, False, False, []),
        ]
        for name, arguments, udiskie, marker, succeeds, expected in cases:
            with self.subTest(name=name), InstallerFixture(release_files={
                "udiskie/unlock": "supported\n" if marker else None,
            }) as fixture:
                if udiskie:
                    fixture.command("udiskie", "exit 0\n")
                if arguments:
                    result = fixture.run(*arguments)
                    self.assertEqual(result.returncode == 0, succeeds, result.stdout + result.stderr)
                else:
                    terminal = fixture.terminal()
                    terminal.expect("Enter continue")
                    terminal.send(b"\r")
                    self.assertEqual(terminal.finish(), 0, terminal.output)
                binary = fixture.home / ".local/bin/strata"
                self.assertEqual(fixture.recorded_application_calls(), [f"{binary} {argument}" for argument in expected])
                self.assertEqual(binary.exists(), succeeds)
                self.assertFalse((fixture.home / "config/udiskie").exists())
                if not succeeds:
                    self.assertFalse(any(call.startswith("sudo") for call in fixture.recorded_calls()))

    def test_udiskie_reinstall_uses_the_installed_binary_each_time(self) -> None:
        with InstallerFixture(omarchy=True) as fixture:
            fixture.command("udiskie", "exit 0\n")
            for _ in range(2):
                terminal = fixture.terminal()
                terminal.expect("Enter continue")
                terminal.send(b"\033[B\r")
                terminal.expect("Space toggle")
                terminal.send(b"\r")
                self.assertEqual(terminal.finish(), 0, terminal.output)
            binary = fixture.home / ".local/bin/strata"
            self.assertEqual(fixture.recorded_application_calls(), [f"{binary} {argument}" for argument in (
                "--install-portal", "--install-udiskie-unlock",
                "--install-portal", "--install-udiskie-unlock",
            )])

    def test_udiskie_unlock_capability_is_the_archive_marker_not_the_binary_string(self) -> None:
        with InstallerFixture(release_files={
            "udiskie/unlock": None,
            "strata": '#!/bin/bash\n# --install-udiskie-unlock\nprintf "EXTRACTED\\n" >> "$CALLS"\n',
        }) as fixture:
            fixture.command("udiskie", "exit 0\n")
            result = fixture.run("--with-udiskie-unlock")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("does not include encrypted-volume integration", result.stderr)
            self.assertFalse((fixture.home / ".local/bin/strata").exists())
            self.assertNotIn("EXTRACTED", fixture.recorded_calls())
            self.assertFalse(any(call.startswith("sudo") for call in fixture.recorded_calls()))


if __name__ == "__main__":
    unittest.main()
