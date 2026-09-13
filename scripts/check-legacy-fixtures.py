#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify frozen installer bodies against their immutable published source commits."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/legacy_update"


def main():
    for record in json.loads((FIXTURES / "provenance.json").read_text()):
        source = subprocess.check_output(["git", "show", f"{record['commit']}:{record['source']}"], cwd=ROOT, text=True)
        fixture = (FIXTURES / record["fixture"]).read_text()
        assert hashlib.sha256(fixture.encode()).hexdigest() == record["sha256"]
        for name in record["functions"]:
            start = source.index(f"fn {name}(")
            end = source.index("\n}", start) + 2
            assert source[start:end] in fixture, f"modified historical routine: {record['tag']} {name}"
        print(record["tag"], record["commit"], "verified")


if __name__ == "__main__":
    main()
