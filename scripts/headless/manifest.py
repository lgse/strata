# SPDX-License-Identifier: MIT
"""The shared harness manifest, `tests/fixtures/harness.json`, also read by `tests/qa.rs`."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

PATH = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "harness.json"

MANIFEST: dict[str, Any] = json.loads(PATH.read_text(encoding="utf-8"))
