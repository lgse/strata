# SPDX-License-Identifier: GPL-3.0-or-later
"""Screen capture and golden-image comparison."""

from __future__ import annotations

import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageChops
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gtk  # noqa: E402

# GTK minors differ in icon sizing, text metrics, and popup layout even with
# the same theme and renderer. Each supported rendering profile is reviewed.
RENDERING_PROFILE = f"gtk-{Gtk.get_major_version()}.{Gtk.get_minor_version()}"
BASELINE_DIRECTORY = Path(__file__).resolve().parents[1] / "baselines" / RENDERING_PROFILE

# Software rendering in the pinned environment is stable but not bit-exact:
# text antialiasing shifts a handful of subpixels between runs. A pixel counts
# as different when any channel moves by more than CHANNEL_TOLERANCE, and a
# comparison fails when more than DIFFERENT_PIXEL_BUDGET of them do.
CHANNEL_TOLERANCE = 24
DIFFERENT_PIXEL_BUDGET = 0.005


class CaptureError(RuntimeError):
    """The screen could not be captured."""


def _capture_command(destination: Path) -> list[str]:
    if shutil.which("import"):
        return ["import", "-silent", "-window", "root", str(destination)]
    if shutil.which("magick"):
        return ["magick", "import", "-silent", "-window", "root", str(destination)]
    raise CaptureError(
        "no screen capture tool found; install ImageMagick (provides `import`)"
    )


def capture(display: str, destination: Path) -> Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        _capture_command(destination),
        env={**os.environ, "DISPLAY": display},
        capture_output=True,
        text=True,
        timeout=15,
    )
    if result.returncode != 0 or not destination.exists():
        raise CaptureError(f"screen capture failed: {result.stderr.strip()}")
    return destination


@dataclass(frozen=True)
class Comparison:
    name: str
    matched: bool
    different_fraction: float
    detail: str
    expected: Path | None
    actual: Path
    diff: Path | None

    @property
    def summary(self) -> str:
        return (
            f"{self.name}: {self.detail} "
            f"({self.different_fraction * 100:.3f}% of pixels differ, "
            f"budget {DIFFERENT_PIXEL_BUDGET * 100:.3f}%)"
        )


def baseline_path(name: str) -> Path:
    return BASELINE_DIRECTORY / f"{name}.png"


def updating_baselines() -> bool:
    return os.environ.get("STRATA_E2E_UPDATE_BASELINES") == "1"


def compare_to_baseline(name: str, actual: Path, artifacts: Path) -> Comparison:
    """Compare a capture with its committed baseline.

    A missing or mismatched baseline is never accepted automatically; the
    developer regenerates it with `STRATA_E2E_UPDATE_BASELINES=1` and commits
    the new image so the change is reviewable in the pull request.
    """

    expected = baseline_path(name)
    artifacts.mkdir(parents=True, exist_ok=True)
    stored_actual = artifacts / f"{name}.actual.png"
    shutil.copyfile(actual, stored_actual)

    if updating_baselines():
        expected.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(actual, expected)
        return Comparison(name, True, 0.0, "baseline written", expected, stored_actual, None)

    if not expected.exists():
        return Comparison(
            name,
            False,
            1.0,
            f"no baseline at {expected.relative_to(BASELINE_DIRECTORY.parent)}",
            None,
            stored_actual,
            None,
        )

    with Image.open(expected) as baseline, Image.open(actual) as candidate:
        baseline = baseline.convert("RGB")
        candidate = candidate.convert("RGB")
        if baseline.size != candidate.size:
            return Comparison(
                name,
                False,
                1.0,
                f"size changed from {baseline.size} to {candidate.size}",
                expected,
                stored_actual,
                None,
            )
        difference = ImageChops.difference(baseline, candidate)
        red, green, blue = difference.split()
        mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(
            lambda value: 255 if value > CHANNEL_TOLERANCE else 0
        )
        different = sum(mask.histogram()[1:])
        fraction = different / (mask.width * mask.height)
        matched = fraction <= DIFFERENT_PIXEL_BUDGET
        diff_path = None
        if not matched:
            diff_path = artifacts / f"{name}.diff.png"
            _write_diff(baseline, candidate, mask, diff_path)
            shutil.copyfile(expected, artifacts / f"{name}.expected.png")
        detail = "matched" if matched else "differs from the committed baseline"
    return Comparison(name, matched, fraction, detail, expected, stored_actual, diff_path)


def _write_diff(
    baseline: Image.Image, candidate: Image.Image, mask: Image.Image, destination: Path
) -> None:
    highlight = Image.new("RGB", baseline.size, (255, 0, 128))
    blended = Image.blend(baseline, candidate, 0.5)
    blended.paste(highlight, mask=mask)
    destination.parent.mkdir(parents=True, exist_ok=True)
    blended.save(destination)
