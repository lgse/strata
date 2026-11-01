#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run the tests of every bundled integration under integrations/.

Each integration directory holds a catalog.toml naming its runtime and entry
script. The entry is byte-compiled with that runtime, and tests under
integrations/<id>/tests/ run through unittest discovery. Packages listed one per
line in tests/apt-packages.txt are printed by --apt-packages for CI to install.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys
import tomllib


def integration_directories(root: Path) -> list[Path]:
    if not root.is_dir():
        return []
    return sorted(
        path
        for path in root.iterdir()
        if path.is_dir() and not path.name.startswith((".", "_"))
    )


def apt_packages(root: Path) -> list[str]:
    packages: list[str] = []
    for directory in integration_directories(root):
        listing = directory / "tests" / "apt-packages.txt"
        if not listing.is_file():
            continue
        for line in listing.read_text().splitlines():
            package = line.strip()
            if package and not package.startswith("#") and package not in packages:
                packages.append(package)
    return packages


def run_integration(directory: Path) -> bool:
    manifest = tomllib.loads((directory / "catalog.toml").read_text())
    runtime = manifest["runtime"]
    entry = directory / manifest["entry"]
    print(f"== {directory.name}: compile {entry.name} with {runtime}", flush=True)
    compiled = subprocess.run([runtime, "-m", "py_compile", str(entry)], check=False)
    if compiled.returncode != 0:
        return False
    tests = directory / "tests"
    if not tests.is_dir():
        print(f"== {directory.name}: no tests directory", flush=True)
        return True
    print(f"== {directory.name}: unittest discover", flush=True)
    # Run from the integration directory so tests import the entry as a module.
    result = subprocess.run(
        [runtime, "-m", "unittest", "discover", "-v", "-s", "tests"],
        cwd=directory,
        check=False,
    )
    return result.returncode == 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent / "integrations")
    parser.add_argument("--apt-packages", action="store_true", help="print the packages tests need")
    arguments = parser.parse_args(argv)
    if arguments.apt_packages:
        print("\n".join(apt_packages(arguments.root)))
        return 0
    directories = integration_directories(arguments.root)
    if not directories:
        print("no bundled integrations")
        return 0
    failures = [directory.name for directory in directories if not run_integration(directory)]
    if failures:
        print(f"failed: {', '.join(failures)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
