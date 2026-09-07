#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for `scripts/trash-restore-fixture/trash_restore_fixture.py`.

Run with:

    python3 scripts/trash-restore-fixture/test_trash_restore_fixture.py

or, from the repo root the same way CI discovers script tests:

    python3 -m unittest discover -s scripts -p 'test_*.py'
"""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from trash_restore_fixture import (
    FixtureError,
    config_from_args,
    destination_path,
    discover_volumes,
    encode_trashinfo_path,
    fixture_markers,
    is_fixture_entry,
    parent_escape_path,
    parse_args,
    rewrite_value,
    selected_specs,
    spec_title,
    validate_identity,
)


class EncodingTests(unittest.TestCase):
    def test_percent_encodes_parent_dir_and_spaces(self) -> None:
        self.assertEqual(
            encode_trashinfo_path("../../home/user/My File.txt"),
            "%2e%2e/%2e%2e/home/user/My%20File.txt",
        )

    def test_parent_escape_walks_from_topdir_to_absolute_destination(self) -> None:
        topdir = Path("/run/media/user/Stick")
        dest = Path("/home/user/sink/payload.txt")
        self.assertEqual(
            parent_escape_path(topdir, dest),
            "%2e%2e/%2e%2e/%2e%2e/%2e%2e/home/user/sink/payload.txt",
        )

    def test_parent_escape_rejects_relative_destination(self) -> None:
        with self.assertRaisesRegex(FixtureError, "absolute"):
            parent_escape_path(Path("/media/usb"), Path("etc/passwd"))


class MarkerTests(unittest.TestCase):
    def test_markers_dedupe_when_label_contains_prefix(self) -> None:
        self.assertEqual(fixture_markers("478", "strata-478"), ("478-", "strata-478"))

    def test_fixture_entry_matches_prefix_or_label(self) -> None:
        markers = fixture_markers("478", "strata-478")
        self.assertTrue(
            is_fixture_entry("478-RESTORE-volume-file.txt.trashinfo", "", markers)
        )
        self.assertTrue(
            is_fixture_entry(
                "unrelated.txt.trashinfo",
                "Path=strata-478/restore-here/x.txt\n",
                markers,
            )
        )
        self.assertFalse(
            is_fixture_entry(
                "screenshot.png.trashinfo",
                "Path=test/screenshot.png\n",
                markers,
            )
        )

    def test_validate_identity_rejects_path_pieces(self) -> None:
        with self.assertRaisesRegex(FixtureError, "invalid prefix"):
            validate_identity("../x", "label")
        with self.assertRaisesRegex(FixtureError, "invalid label"):
            validate_identity("ok", "a/b")
        with self.assertRaisesRegex(FixtureError, "at least 2"):
            validate_identity("x", "label")


class SpecTests(unittest.TestCase):
    def test_only_restore_omits_rejects(self) -> None:
        keys = [spec.key for spec in selected_specs("restore")]
        self.assertIn("RESTORE-volume-file.txt", keys)
        self.assertNotIn("REJECT-volume-to-home.txt", keys)

    def test_collision_titles_include_side(self) -> None:
        specs = selected_specs("restore")
        collisions = [spec for spec in specs if spec.key == "RESTORE-collision.txt"]
        self.assertEqual(len(collisions), 2)
        titles = {spec_title(spec, specs) for spec in collisions}
        self.assertEqual(
            titles,
            {
                "RESTORE-collision.txt (volume copy)",
                "RESTORE-collision.txt (home copy)",
            },
        )


class ConfigTests(unittest.TestCase):
    def test_plant_parses_identity_flags(self) -> None:
        args = parse_args(
            [
                "plant",
                "--volume",
                "/mnt/usb",
                "--prefix",
                "demo",
                "--label",
                "strata-demo",
                "--only",
                "reject",
                "--no-restart-gvfs",
            ]
        )
        self.assertEqual(args.command, "plant")
        self.assertEqual(args.prefix, "demo")
        self.assertEqual(args.label, "strata-demo")
        self.assertEqual(args.only, "reject")
        self.assertFalse(args.restart_gvfs)

    def test_defaults_fill_home_paths_from_label(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            volume = root / "usb"
            home = root / "home"
            volume.mkdir()
            home.mkdir()
            args = parse_args(["plant", "--volume", str(volume), "--label", "demo"])
            with mock.patch.dict(os.environ, {"XDG_DATA_HOME": ""}):
                with mock.patch(
                    "trash_restore_fixture.resolve_volume",
                    return_value=volume.resolve(),
                ):
                    cfg = config_from_args(args, home=home, uid=1000, require_volume=True)
            self.assertEqual(cfg.home_dir, home / "demo-home")
            self.assertEqual(cfg.home_sink, home / "demo-home-sink")
            self.assertEqual(cfg.volume_dir, volume.resolve() / "demo")
            self.assertEqual(cfg.home_trash, home / ".local/share/Trash")
            self.assertEqual(
                cfg.manifest, Path(tempfile.gettempdir()).resolve() / "demo-trash-fixture.md"
            )

    def test_rewrite_paths_follow_volume_and_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            volume = root / "usb"
            home = root / "home"
            volume.mkdir()
            home.mkdir()
            args = parse_args(
                [
                    "plant",
                    "--volume",
                    str(volume),
                    "--prefix",
                    "zz",
                    "--label",
                    "lab",
                    "--tmp-dir",
                    str(root / "tmp"),
                ]
            )
            with mock.patch(
                "trash_restore_fixture.resolve_volume",
                return_value=volume.resolve(),
            ):
                cfg = config_from_args(args, home=home, uid=1000, require_volume=True)
            specs = {spec.key: spec for spec in selected_specs("reject")}
            self.assertEqual(
                rewrite_value(cfg, specs["REJECT-volume-to-home.txt"]),
                str(cfg.home_dir / "reject-sink" / "zz-REJECT-volume-to-home.txt"),
            )
            self.assertEqual(
                rewrite_value(cfg, specs["REJECT-volume-symlink.txt"]),
                "lab/link-home/zz-REJECT-volume-symlink.txt",
            )
            self.assertEqual(
                destination_path(cfg, specs["REJECT-home-to-volume.txt"]),
                cfg.volume_dir / "reject-sink" / "zz-REJECT-home-to-volume.txt",
            )
            escaped = rewrite_value(cfg, specs["REJECT-volume-dotdot.txt"])
            self.assertIsNotNone(escaped)
            self.assertTrue(escaped.startswith("%2e%2e/"))

    def test_same_filesystem_volume_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            volume = root / "usb"
            home = root / "home"
            volume.mkdir()
            home.mkdir()
            args = parse_args(["plant", "--volume", str(volume)])
            with self.assertRaisesRegex(FixtureError, "same filesystem"):
                config_from_args(args, home=home, uid=1000, require_volume=True)

    def test_discover_volumes_skips_home_device_and_unwritable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "home"
            media = root / "media"
            usb = media / "usb"
            home.mkdir()
            usb.mkdir(parents=True)
            found = discover_volumes(home, uid=os.getuid(), roots=[media])
            self.assertEqual(found, [])


if __name__ == "__main__":
    unittest.main()
