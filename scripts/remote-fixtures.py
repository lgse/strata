#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Disposable SFTP, FTP/FTPS, and WebDAV/DAVS servers for remote-location tests.

The servers run in local containers published only on 127.0.0.1. Clients run
on a private D-Bus session with private XDG directories, a private SSH
known_hosts file, and fixture-only SSH keys, so neither GVfs nor OpenSSH
touches the developer's desktop session, keyring, or ~/.ssh.

  scripts/remote-fixtures.py test [cargo test args]   run the ignored fixture tests
  scripts/remote-fixtures.py up                       start servers, print endpoints
  scripts/remote-fixtures.py shell [command...]       run a command in the isolated client session
  scripts/remote-fixtures.py down                     stop servers and remove state
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
FIXTURES = REPOSITORY / "tests/remote-fixtures"
PREFIX = "strata-remote-fixture"
IMAGE_VERSION = "1"
PASSWORD = "fixture-password"
PASSPHRASE = "fixture-passphrase"
TEST_FILTER = "adapters::remote_mount::tests::fixtures"
# OpenSSH ControlPath sockets must fit in sockaddr_un, so state lives under /tmp.
STATE_POINTER = REPOSITORY / "target/remote-fixtures/state"
SERVICE_DIRECTORIES = ("/usr/share/dbus-1/services", "/usr/local/share/dbus-1/services")


def engine() -> str:
    selected = os.environ.get("STRATA_CONTAINER_ENGINE")
    if selected:
        return selected
    for candidate in ("docker", "podman"):
        if shutil.which(candidate):
            return candidate
    sys.exit("Remote fixtures need Docker or Podman.")


def run(*command: str, check: bool = True, **kwargs) -> None:
    subprocess.run(command, check=check, text=True, **kwargs)


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def free_range(size: int) -> int:
    for _ in range(200):
        base = free_port()
        if base + size >= 65535:
            continue
        sockets = []
        try:
            for port in range(base, base + size):
                probe = socket.socket()
                sockets.append(probe)
                probe.bind(("127.0.0.1", port))
            return base
        except OSError:
            continue
        finally:
            for probe in sockets:
                probe.close()
    sys.exit("No free loopback port range for FTP passive mode.")


def state_directory() -> Path | None:
    try:
        path = Path(STATE_POINTER.read_text().strip())
    except FileNotFoundError:
        return None
    return path if (path / "fixtures.json").is_file() else None


def build_images(container: str) -> None:
    for name in ("sftp", "ftp", "webdav"):
        run(
            container,
            "build",
            "--quiet",
            "--tag",
            f"{PREFIX}-{name}:{IMAGE_VERSION}",
            str(FIXTURES / name),
            stdout=subprocess.DEVNULL,
        )


def generate_keys(state: Path) -> None:
    for name, passphrase in (("keyed", ""), ("locked", PASSPHRASE)):
        run(
            "ssh-keygen",
            "-q",
            "-t",
            "ed25519",
            "-N",
            passphrase,
            "-C",
            f"strata-fixture-{name}",
            "-f",
            str(state / "keys" / name),
        )
        shutil.copy(state / "keys" / f"{name}.pub", state / "authorized" / f"{name}.pub")
    run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(state / "keys" / "impostor"))


def write_client_tools(state: Path) -> None:
    ssh = shutil.which("ssh")
    if ssh is None:
        sys.exit("The SFTP fixture needs the OpenSSH client (ssh).")
    wrapper = state / "bin" / "ssh"
    # The first value of an ssh option wins, so these override GVfs's own
    # NoHostAuthenticationForLocalhost and keep host-key decisions testable.
    wrapper.write_text(
        "#!/bin/sh\n"
        f'exec {ssh} -F /dev/null -o UserKnownHostsFile={state}/known_hosts '
        "-o GlobalKnownHostsFile=/dev/null -o NoHostAuthenticationForLocalhost=no "
        "-o IdentityAgent=none -o IdentitiesOnly=yes -o ConnectTimeout=10 "
        f'-o IdentityFile={state}/keys/keyed -o IdentityFile={state}/keys/locked "$@"\n'
    )
    wrapper.chmod(0o755)

    services = state / "dbus-services"
    for directory in SERVICE_DIRECTORIES:
        source = Path(directory) / "org.gtk.vfs.Daemon.service"
        if source.is_file():
            lines = [
                line
                for line in source.read_text().splitlines()
                if not line.startswith("SystemdService=")
            ]
            (services / source.name).write_text("\n".join(lines) + "\n")
            break
    else:
        sys.exit("GVfs isn't installed (org.gtk.vfs.Daemon.service not found).")
    (state / "session.conf").write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"\n'
        ' "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        "<busconfig>\n"
        "  <type>session</type>\n"
        "  <keep_umask/>\n"
        f"  <listen>unix:dir={state}/run</listen>\n"
        f"  <servicedir>{services}</servicedir>\n"
        "  <policy context=\"default\">\n"
        '    <allow send_destination="*" eavesdrop="true"/>\n'
        '    <allow eavesdrop="true"/>\n'
        '    <allow own="*"/>\n'
        "  </policy>\n"
        "</busconfig>\n"
    )


def start_servers(container: str, state: Path) -> dict:
    sftp_port = free_port()
    ftp_port = free_port()
    passive = free_range(10)
    dav_port = free_port()
    davs_port = free_port()
    closed_port = free_port()
    loopback = "127.0.0.1"
    run(
        container, "run", "--detach", "--rm", "--name", f"{PREFIX}-sftp",
        "--publish", f"{loopback}:{sftp_port}:22",
        "--volume", f"{state / 'authorized'}:/fixture:ro,Z",
        f"{PREFIX}-sftp:{IMAGE_VERSION}",
        stdout=subprocess.DEVNULL,
    )
    run(
        container, "run", "--detach", "--rm", "--name", f"{PREFIX}-ftp",
        "--publish", f"{loopback}:{ftp_port}:21",
        "--publish", f"{loopback}:{passive}-{passive + 9}:{passive}-{passive + 9}",
        "--env", f"PASV_MIN_PORT={passive}",
        "--env", f"PASV_MAX_PORT={passive + 9}",
        f"{PREFIX}-ftp:{IMAGE_VERSION}",
        stdout=subprocess.DEVNULL,
    )
    run(
        container, "run", "--detach", "--rm", "--name", f"{PREFIX}-webdav",
        "--publish", f"{loopback}:{dav_port}:80",
        "--publish", f"{loopback}:{davs_port}:443",
        f"{PREFIX}-webdav:{IMAGE_VERSION}",
        stdout=subprocess.DEVNULL,
    )
    for port in (sftp_port, ftp_port, dav_port, davs_port):
        wait_for_port(port)
    ftp = {
        "host": loopback,
        "port": ftp_port,
        "user": "ftpuser",
        "password": PASSWORD,
        "path": "/",
        "entry": "hello-ftp.txt",
    }
    dav = {
        "host": loopback,
        "user": "davuser",
        "password": PASSWORD,
        "path": "/dav",
        "entry": "hello-dav.txt",
    }
    return {
        "sftp": {
            "host": loopback,
            "port": sftp_port,
            "user": "strata",
            "password": PASSWORD,
            "key_user": "keyed",
            "passphrase_user": "locked",
            "passphrase": PASSPHRASE,
            "known_hosts": str(state / "known_hosts"),
            "path": "/srv/shared",
            "closed_port": closed_port,
            "impostor_key": " ".join(
                (state / "keys" / "impostor.pub").read_text().split()[:2]
            ),
        },
        "ftp": ftp,
        "ftps": ftp,
        "dav": {**dav, "port": dav_port},
        "davs": {**dav, "port": davs_port},
    }


def wait_for_port(port: int) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1) as connection:
                connection.settimeout(1)
                try:
                    connection.recv(1)
                except socket.timeout:
                    pass
                return
        except OSError:
            time.sleep(0.2)
    sys.exit(f"A fixture server didn't start listening on port {port}.")


def up() -> Path:
    existing = state_directory()
    if existing is not None:
        return existing
    container = engine()
    build_images(container)
    state = Path(tempfile.mkdtemp(prefix="srfx-", dir="/tmp"))
    for name in ("bin", "keys", "authorized", "run", "home", "dbus-services"):
        (state / name).mkdir(mode=0o700)
    generate_keys(state)
    write_client_tools(state)
    try:
        fixtures = start_servers(container, state)
    except BaseException:
        down()
        shutil.rmtree(state, ignore_errors=True)
        raise
    (state / "fixtures.json").write_text(json.dumps(fixtures, indent=2) + "\n")
    STATE_POINTER.parent.mkdir(parents=True, exist_ok=True)
    STATE_POINTER.write_text(f"{state}\n")
    return state


def down() -> None:
    container = engine()
    for name in ("sftp", "ftp", "webdav"):
        run(
            container, "rm", "--force", f"{PREFIX}-{name}",
            check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    state = state_directory()
    if state is not None:
        shutil.rmtree(state, ignore_errors=True)
    STATE_POINTER.unlink(missing_ok=True)


def client_environment(state: Path) -> dict[str, str]:
    home = Path.home()
    environment = {
        "PATH": f"{state / 'bin'}:{os.environ.get('PATH', '/usr/bin:/bin')}",
        "HOME": str(state / "home"),
        "XDG_RUNTIME_DIR": str(state / "run"),
        "XDG_CONFIG_HOME": str(state / "home/.config"),
        "XDG_DATA_HOME": str(state / "home/.local/share"),
        "XDG_CACHE_HOME": str(state / "home/.cache"),
        "CARGO_HOME": os.environ.get("CARGO_HOME", str(home / ".cargo")),
        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(home / ".rustup")),
        "CARGO_TARGET_DIR": os.environ.get("CARGO_TARGET_DIR", str(REPOSITORY / "target")),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "GIO_USE_VOLUME_MONITOR": "unix",
        "GSETTINGS_BACKEND": "memory",
        "NO_AT_BRIDGE": "1",
        "STRATA_REMOTE_FIXTURES": str(state / "fixtures.json"),
    }
    for name in ("RUSTFLAGS", "RUST_BACKTRACE", "TERM", "GVFS_DEBUG"):
        if name in os.environ:
            environment[name] = os.environ[name]
    return environment


def in_session(state: Path, command: list[str]) -> int:
    if shutil.which("dbus-run-session") is None:
        sys.exit("Remote fixtures need dbus-run-session for a private session bus.")
    return subprocess.run(
        ["dbus-run-session", f"--config-file={state / 'session.conf'}", "--", *command],
        cwd=REPOSITORY,
        env=client_environment(state),
        check=False,
    ).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    test = commands.add_parser("test", help="run the ignored fixture tests")
    test.add_argument("--keep", action="store_true", help="leave the servers running")
    test.add_argument("cargo_args", nargs=argparse.REMAINDER)
    commands.add_parser("up", help="start the servers and print their endpoints")
    shell = commands.add_parser("shell", help="run a command in the isolated client session")
    shell.add_argument("shell_command", nargs=argparse.REMAINDER)
    commands.add_parser("down", help="stop the servers and remove fixture state")
    arguments = parser.parse_args()

    if arguments.command == "down":
        down()
        return 0
    state = up()
    if arguments.command == "up":
        print((state / "fixtures.json").read_text(), end="")
        print(f"Client session: {Path(__file__).name} shell <command>", file=sys.stderr)
        return 0
    if arguments.command == "shell":
        command = arguments.shell_command or [os.environ.get("SHELL", "/bin/sh")]
        return in_session(state, command)
    try:
        return in_session(
            state,
            [
                "cargo", "test", "--all-features", "--bin", "strata",
                TEST_FILTER, *arguments.cargo_args,
                "--", "--ignored", "--test-threads=1",
            ],
        )
    finally:
        if not arguments.keep:
            down()


if __name__ == "__main__":
    sys.exit(main())
