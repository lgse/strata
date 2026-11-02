#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Seeds a scripts/qa-drive.sh session as tests/qa.rs seeds each run, and
rewrites preferences in the saved settings.toml between launches.

    qa_drive.py seed SESSION FIXTURE   HOME, settings.toml, the fixture tree, app.env
    qa_drive.py check                  QA_DRIVE_PREFS can be written as TOML
    qa_drive.py prefs SETTINGS         merges QA_DRIVE_PREFS into SETTINGS
"""

from __future__ import annotations

import datetime
import json
import math
import os
import re
import sys
import tomllib
from collections.abc import Iterator

MANIFEST = "/workspace/tests/fixtures/harness.json"
BARE_KEY = re.compile(r"[A-Za-z0-9_-]+")


class PreferenceError(ValueError):
    pass


def toml_string(text: str) -> str:
    # JSON's escapes are a subset of a TOML basic string's; TOML also needs DEL escaped.
    return json.dumps(text, ensure_ascii=False).replace("\x7f", "\\u007f")


def toml_key(key: str) -> str:
    return key if BARE_KEY.fullmatch(key) else toml_string(key)


def toml(value) -> str:
    """One value as tests/qa.rs's `render_value` writes it, objects as inline tables."""

    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        if math.isnan(value):
            return "nan"
        if math.isinf(value):
            return "inf" if value > 0 else "-inf"
        return json.dumps(value)
    if isinstance(value, str):
        return toml_string(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml(item) for item in value) + "]"
    if isinstance(value, dict):
        if not value:
            return "{}"
        return "{ " + ", ".join(f"{toml_key(str(key))} = {toml(item)}" for key, item in value.items()) + " }"
    if isinstance(value, (datetime.datetime, datetime.date, datetime.time)):
        return value.isoformat()
    raise PreferenceError(f"settings.toml cannot hold {value!r}")


def dumps(document: dict) -> str:
    """A whole settings file: plain keys first, then each table under its header."""

    lines: list[str] = []

    def table(path: list[str], values: dict) -> None:
        plain = {key: value for key, value in values.items() if not isinstance(value, dict)}
        nested = {key: value for key, value in values.items() if isinstance(value, dict)}
        if path and (plain or not nested):
            if lines:
                lines.append("")
            lines.append("[" + ".".join(toml_key(part) for part in path) + "]")
        lines.extend(f"{toml_key(key)} = {toml(value)}" for key, value in plain.items())
        for key, value in nested.items():
            table([*path, key], value)

    table([], document)
    return "\n".join(lines) + "\n"


def parse_prefs(text: str) -> Iterator[tuple[str, object]]:
    """QA_DRIVE_PREFS: one KEY=VALUE per line, the value JSON or else a string."""

    for line in filter(None, text.split("\n")):
        key, separator, raw = line.partition("=")
        if not separator or not key:
            raise PreferenceError(f"--pref needs KEY=VALUE, not {line!r}")
        try:
            value = json.loads(raw)
        except ValueError:
            value = raw
        if value is None:
            raise PreferenceError(f"--pref {key}: settings.toml cannot hold null")
        toml(value)
        yield key, value


def merge(settings: str, changes: list[tuple[str, object]]) -> str:
    """SETTINGS with each changed key replaced whole, in place, or added."""

    try:
        document = tomllib.loads(settings)
    except tomllib.TOMLDecodeError as error:
        raise PreferenceError(f"the saved settings.toml does not parse: {error}") from error
    for key, value in changes:
        document[key] = value
    return dumps(document)


def seed(session: str, fixture: str, prefs: str) -> None:
    manifest = json.load(open(MANIFEST, encoding="utf-8"))
    home = f"{session}/home"
    os.makedirs(f"{home}/config/gtk-4.0", exist_ok=True)
    with open(f"{home}/config/gtk-4.0/settings.ini", "w") as file:
        file.write(manifest["gtk_settings"])
    os.makedirs(f"{home}/config/strata", exist_ok=True)
    with open(f"{home}/config/strata/{manifest['portal_offer_marker']}", "w") as file:
        file.write("1\n")
    preferences = dict(manifest["preferences"])
    preferences.update(parse_prefs(prefs))
    with open(f"{home}/config/strata/settings.toml", "w") as file:
        file.writelines(f"{key} = {toml(value)}\n" for key, value in preferences.items())

    tree = os.environ.get("QA_DRIVE_TREE") or "canonical"
    tree = manifest["trees"][tree] if tree in manifest["trees"] else json.loads(tree)
    if not isinstance(tree, dict):
        raise SystemExit("--tree must be a manifest tree's name or a JSON object")
    populate(fixture, tree)
    pin(fixture, manifest["mtime"])

    with open(f"{session}/app.env", "w") as file:
        for key, value in manifest["environment"].items():
            file.write(f"{key}={value}\n")


def populate(directory: str, tree: dict) -> None:
    for name, value in tree.items():
        path = os.path.join(directory, name)
        if isinstance(value, dict):
            os.makedirs(path, exist_ok=True)
            os.chmod(path, 0o755)
            populate(path, value)
        elif isinstance(value, str):
            with open(path, "w") as file:
                file.write(value)
            os.chmod(path, 0o644)
        else:
            raise SystemExit(f"fixture {name} must be a file's text or a directory")


def pin(path: str, mtime: int) -> None:
    for root, directories, files in os.walk(path, topdown=False):
        for name in files + directories:
            os.utime(os.path.join(root, name), (mtime, mtime), follow_symlinks=False)
    os.utime(path, (mtime, mtime))


def main(arguments: list[str]) -> int:
    prefs = os.environ.get("QA_DRIVE_PREFS", "")
    try:
        match arguments:
            case ["seed", session, fixture]:
                seed(session, fixture, prefs)
            case ["check"]:
                list(parse_prefs(prefs))
            case ["prefs", path]:
                with open(path, encoding="utf-8") as file:
                    merged = merge(file.read(), list(parse_prefs(prefs)))
                with open(path, "w", encoding="utf-8") as file:
                    file.write(merged)
            case _:
                print(__doc__, file=sys.stderr)
                return 2
    except PreferenceError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
