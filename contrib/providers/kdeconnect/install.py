#!/usr/bin/python3
# SPDX-License-Identifier: MIT
"""Opt in to the KDE Connect provider without changing other Strata settings."""

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import stat
import tempfile


def checked_directory(path):
    if path.is_symlink():
        raise ValueError(f"Refusing symbolic-link directory: {path}")
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    info = path.stat()
    if info.st_uid != os.getuid() or info.st_mode & 0o022:
        raise ValueError(f"Directory is not owned and protected by this user: {path}")


def install(config_home, state_home):
    folder = config_home / "strata/providers/kdeconnect"
    for directory in (config_home / "strata", folder.parent, folder):
        checked_directory(directory)
    folder.chmod(0o700)
    backups = state_home / "strata-kdeconnect/backups" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    manifest = {
        "version": 1, "id": "kdeconnect", "name": "KDE Connect",
        "command": ["/usr/bin/python3", str(folder / "provider.py")], "icons": {},
    }
    source = Path(__file__).parent
    files = {name: (source / name).read_bytes() for name in ("provider.py", "messages.json")}
    files["provider.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    # Preflight every destination before replacing any provider file.
    for name in files:
        path = folder / name
        if path.is_symlink():
            raise ValueError(f"Refusing symbolic-link file: {path}")
        if path.exists():
            info = path.stat()
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise ValueError(f"Refusing file not owned by this user: {path}")
    changes = []
    for name, data in files.items():
        path = folder / name
        if path.exists():
            current = path.read_bytes()
            if current == data:
                path.chmod(0o600)
                continue
            checked_directory(backups)
            backup = backups / name
            backup.write_bytes(current)
            backup.chmod(0o600)
        fd, temporary = tempfile.mkstemp(prefix=".install-", dir=folder)
        try:
            with os.fdopen(fd, "wb") as stream:
                stream.write(data)
            os.replace(temporary, path)
        finally:
            Path(temporary).unlink(missing_ok=True)
        changes.append(path)
    return changes, backups if backups.exists() else None


def xdg_path(name, fallback):
    value = os.environ.get(name)
    return Path(value) if value and Path(value).is_absolute() else Path.home() / fallback


if __name__ == "__main__":
    from gi.repository import Gio  # Verify the system interpreter has PyGObject.

    os.umask(0o077)
    changes, backups = install(xdg_path("XDG_CONFIG_HOME", ".config"), xdg_path("XDG_STATE_HOME", ".local/state"))
    for path in changes:
        print(f"Updated: {path}")
    if backups:
        print(f"Backups: {backups}")
    print("Close all Strata windows and reopen Strata to load the provider.")
