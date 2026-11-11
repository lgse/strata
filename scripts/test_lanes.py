# SPDX-License-Identifier: MIT

from pathlib import Path
import tomllib
import unittest

import lanes

REPOSITORY = Path(__file__).resolve().parents[1]
RUNE_CRATES = {"rune", "rune-kit"}


def manifest(path):
    return tomllib.loads(path.read_text())


def dependencies(package):
    return {
        name
        for table in ("dependencies", "dev-dependencies", "build-dependencies")
        for name in package.get(table, {})
    }


def members():
    root = manifest(REPOSITORY / "Cargo.toml")
    found = {root["package"]["name"]: dependencies(root)}
    for pattern in root["workspace"]["members"]:
        for path in REPOSITORY.glob(f"{pattern}/Cargo.toml"):
            package = manifest(path)
            found[package["package"]["name"]] = dependencies(package)
    return found


class LaneTests(unittest.TestCase):
    def test_every_package_that_builds_on_rune_is_in_the_rune_lane(self):
        packages = members()
        on_rune = set()
        while True:
            reached = {name for name, needs in packages.items() if needs & (RUNE_CRATES | on_rune)}
            if reached == on_rune:
                break
            on_rune = reached
        self.assertTrue(on_rune, "no workspace package depends on Rune")
        self.assertEqual(on_rune - set(lanes.RUNE_SIDE), set())

    def test_the_rune_lane_names_only_workspace_members(self):
        self.assertEqual(set(lanes.RUNE_SIDE) - set(members()), set())


if __name__ == "__main__":
    unittest.main()
