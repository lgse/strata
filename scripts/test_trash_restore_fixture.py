#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Load trash-restore-fixture tests for `unittest discover -s scripts`."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from unittest import TestLoader, TestSuite

_DIR = Path(__file__).resolve().parent / "trash-restore-fixture"
_TESTS = _DIR / "test_trash_restore_fixture.py"


def load_tests(loader: TestLoader, _tests: TestSuite, _pattern: str | None) -> TestSuite:
    if str(_DIR) not in sys.path:
        sys.path.insert(0, str(_DIR))
    spec = importlib.util.spec_from_file_location("trash_restore_fixture_tests", _TESTS)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {_TESTS}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return loader.loadTestsFromModule(module)
