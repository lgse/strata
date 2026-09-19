#!/usr/bin/env python3
# SPDX-License-Identifier: MIT

from __future__ import annotations

import json
import os
import sys
from typing import Iterable, Optional

PROTOCOL_VERSION = 1

_ENV = {
    "context": "STRATA_ACTION_CONTEXT",
    "paths": "STRATA_ACTION_PATHS",
    "parent": "STRATA_ACTION_PARENT",
    "directory": "STRATA_ACTION_DIR",
    "progress": "STRATA_ACTION_PROGRESS",
    "run_directory": "STRATA_ACTION_RUN_DIR",
    "version": "STRATA_ACTION_VERSION",
    "mode": "STRATA_ACTION_MODE",
    "source": "STRATA_ACTION_SOURCE",
    "count": "STRATA_ACTION_COUNT",
    "position": "STRATA_ACTION_POSITION",
    "action_id": "STRATA_ACTION_ID",
}

MAX_MESSAGE_CHARS = 512
MAX_UNITS = 1_000_000


class ContextError(RuntimeError):
    pass


def _env(name: str) -> Optional[str]:
    value = os.environ.get(_ENV[name])
    return value if value else None


def _read_paths(path: Optional[str]) -> list:
    if not path:
        return []
    with open(path, "rb") as handle:
        raw = handle.read()
    return [os.fsdecode(chunk) for chunk in raw.split(b"\0") if chunk]


def _read_single_path(path: Optional[str]) -> Optional[str]:
    if not path:
        return None
    with open(path, "rb") as handle:
        raw = handle.read()
    return os.fsdecode(raw) if raw else None


class Context:
    def __init__(self) -> None:
        version = _env("version")
        if version is None:
            raise ContextError(
                "This script is not running as a Strata custom action. "
                "Strata provides the invocation context through environment variables."
            )
        if version != str(PROTOCOL_VERSION):
            raise ContextError(
                f"Strata sent action protocol version {version}, but this helper expects "
                f"{PROTOCOL_VERSION}. Update the helper to match Strata."
            )
        self.version = PROTOCOL_VERSION
        self.action_id = _env("action_id") or ""
        self.mode = _env("mode") or "whole-selection"
        self.source = _env("source") or "selection"
        self.directory = _env("directory")
        self.run_directory = _env("run_directory")
        self.paths = _read_paths(_env("paths"))
        self.parent = _read_single_path(_env("parent"))
        context_path = _env("context")
        self.metadata = self._read_metadata(context_path)
        self.count = len(self.paths)
        position = _env("position")
        self.position = int(position) if position and position.isdigit() else None
        self._progress_path = _env("progress")

    @staticmethod
    def _read_metadata(path: Optional[str]) -> dict:
        if not path:
            return {}
        try:
            with open(path, "r", encoding="utf-8") as handle:
                return json.load(handle)
        except (OSError, ValueError):
            return {}

    @property
    def total(self) -> Optional[int]:
        """Job-wide count in per-item mode, unlike invocation-local count."""
        position = self.metadata.get("position")
        if isinstance(position, str) and "," in position:
            _, _, total = position.partition(",")
            if total.isdigit():
                return int(total)
        return None

    @property
    def single(self) -> Optional[str]:
        if len(self.paths) == 1:
            return self.paths[0]
        return None

    def paths_bytes(self) -> Iterable[bytes]:
        return [os.fsencode(path) for path in self.paths]

    def progress(
        self,
        processed: int,
        total: Optional[int] = None,
        message: Optional[str] = None,
    ) -> None:
        """Counts describe this invocation, not the entire per-item job."""
        event = {"event": "progress", "processed": _units(processed)}
        if total is not None:
            event["total"] = _units(total)
        if message:
            event["message"] = _message(message)
        self._emit(event)

    def output(self, path: str) -> None:
        """Reporting only; this does not create a file."""
        if not path or not os.path.isabs(path):
            return
        self._emit({"event": "output", "path": path})

    def log(self, message: str) -> None:
        print(message, flush=True)

    def _emit(self, event: dict) -> None:
        if not self._progress_path:
            return
        try:
            with open(self._progress_path, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(event, ensure_ascii=False) + "\n")
        except (OSError, UnicodeError) as error:
            # Reporting is best-effort; native non-UTF8 paths cannot fit JSON text.
            print(f"strata: unable to report progress: {error}", file=sys.stderr)


def _units(value: int) -> int:
    number = int(value)
    if number < 0:
        raise ValueError("progress counts cannot be negative")
    if number > MAX_UNITS:
        raise ValueError(f"progress counts are limited to {MAX_UNITS}")
    return number


def _message(message: str) -> str:
    text = " ".join(str(message).split())
    if len(text) > MAX_MESSAGE_CHARS:
        text = text[: MAX_MESSAGE_CHARS - 1] + "…"
    return text


def context() -> Context:
    return Context()


def find_tool(name: str) -> Optional[str]:
    from shutil import which

    return which(name)


def require_tool(name: str) -> str:
    found = find_tool(name)
    if found is None:
        raise ContextError(
            f"This action needs “{name}”, which was not found on your PATH. "
            "Install it and run the action again."
        )
    return found
