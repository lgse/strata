# SPDX-License-Identifier: GPL-3.0-or-later
"""Failure evidence: screenshot, accessibility tree, logs, and fixture state."""

from __future__ import annotations

import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from . import screenshots


def artifact_root() -> Path:
    configured = os.environ.get("STRATA_E2E_ARTIFACTS")
    base = Path(configured) if configured else _default_root()
    base.mkdir(parents=True, exist_ok=True)
    return base


def _default_root() -> Path:
    return Path(__file__).resolve().parents[3] / "target" / "e2e-artifacts"


def _slug(name: str) -> str:
    return "".join(
        character if character.isalnum() or character in "-_." else "-"
        for character in name
    )


@dataclass
class ArtifactCollector:
    """Writes one directory of evidence per failing scenario."""

    test_name: str

    @property
    def directory(self) -> Path:
        path = artifact_root() / _slug(self.test_name)
        path.mkdir(parents=True, exist_ok=True)
        return path

    def write(self, filename: str, contents: str) -> Path:
        path = self.directory / filename
        path.write_text(contents, errors="replace")
        return path

    def copy(self, source: Path, filename: str | None = None) -> Path:
        destination = self.directory / (filename or source.name)
        shutil.copyfile(source, destination)
        return destination

    def collect(
        self,
        *,
        display: str | None,
        accessibility_tree: str,
        application_log: str,
        fixture_listing: str,
        session_logs: dict[str, str],
    ) -> list[Path]:
        written = [
            self.write("accessibility-tree.txt", accessibility_tree),
            self.write("strata.log", application_log),
            self.write("fixture-tree.txt", fixture_listing),
        ]
        for name, contents in session_logs.items():
            written.append(self.write(f"session-{name}.log", contents))
        if display:
            try:
                written.append(
                    screenshots.capture(display, self.directory / "screenshot.png")
                )
            except (screenshots.CaptureError, subprocess.TimeoutExpired, OSError) as error:
                written.append(self.write("screenshot-error.txt", str(error)))
        return written
