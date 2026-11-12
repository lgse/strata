#!/usr/bin/env python3
"""Tests for `scripts/build_srpm.py`."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

from build_srpm import (
    build_kind,
    inspect_vendor,
    license_expression,
    parse_license_summary,
    require_updater_switch,
    rpm_version,
)
from update_aur import PackagingError


class RpmVersionTests(unittest.TestCase):

    def test_release_tags_map_onto_rpm_versions(self):
        for release, expected in (
            ("1.2.1", "1.2.1"),
            ("v1.2.1", "1.2.1"),
            ("1.3.0-alpha.1", "1.3.0~alpha.1"),
            ("1.3.0-beta.2", "1.3.0~beta.2"),
            ("v1.3.0-rc.10", "1.3.0~rc.10"),
        ):
            with self.subTest(release=release):
                self.assertEqual(rpm_version(release), expected)

    def test_nightlies_and_malformed_tags_are_rejected(self):
        for release in ("1.3.0-nightly.20261101", "1.3.0-nightly.20261101.2", "1.3", "1.3.0-rc1"):
            with self.subTest(release=release), self.assertRaises(PackagingError):
                rpm_version(release)

    def test_the_build_kind_follows_the_prerelease_stage(self):
        for release, expected in (
            ("1.2.1", "stable"),
            ("1.3.0-alpha.1", "alpha"),
            ("1.3.0-beta.1", "beta"),
            ("1.3.0-rc.2", "rc"),
        ):
            with self.subTest(release=release):
                self.assertEqual(build_kind(release), expected)


class LicenseExpressionTests(unittest.TestCase):

    def test_a_cargo2rpm_summary_joins_the_bundled_asset_licenses(self):
        summary = (
            "### BEGIN LICENSE SUMMARY ###\n"
            "# (Apache-2.0 OR MIT) AND Unicode-3.0\n"
            "# Apache-2.0 OR MIT\n"
            "# MIT\n"
            "# MIT OR Apache-2.0\n"
            "# Unicode-3.0 AND (MIT OR Apache-2.0)\n"
            "# (MIT AND BSD-3-Clause) OR Zlib\n"
            "###  END LICENSE SUMMARY  ###\n"
        )
        self.assertEqual(
            license_expression(parse_license_summary(summary)),
            "Apache-2.0 AND (Apache-2.0 OR MIT) AND ISC AND MIT AND ((MIT AND BSD-3-Clause) OR Zlib)"
            " AND OFL-1.1 AND Unicode-3.0",
        )

    def test_an_exception_needs_no_parentheses(self):
        self.assertIn(
            " AND Apache-2.0 WITH LLVM-exception AND ",
            license_expression(["Apache-2.0 WITH LLVM-exception"]),
        )


class UpdaterSwitchTests(unittest.TestCase):

    def test_only_sources_that_can_compile_out_the_updater_are_packaged(self):
        switch = 'println!("cargo::rerun-if-env-changed=STRATA_UPDATER");'
        other = 'println!("cargo::rerun-if-env-changed=STRATA_BUILD_COMMIT");'
        for build_scripts, packaged in (
            ({"build.rs": switch}, True),
            ({"build.rs": other, "crates/strata-desktop/build.rs": switch}, True),
            ({"build.rs": other, "crates/strata-core/build.rs": other}, False),
        ):
            with self.subTest(build_scripts=build_scripts), tempfile.TemporaryDirectory() as scratch:
                source = pathlib.Path(scratch)
                for path, contents in build_scripts.items():
                    (source / path).parent.mkdir(parents=True, exist_ok=True)
                    (source / path).write_text(contents)
                if packaged:
                    require_updater_switch(source, "v1.2.1")
                else:
                    with self.assertRaises(PackagingError):
                        require_updater_switch(source, "v1.1.1")


class InspectVendorTests(unittest.TestCase):

    def crate(self, vendor, directory, name, version, lib="", extra=None):
        crate = vendor / directory
        (crate / "src").mkdir(parents=True)
        (crate / "Cargo.toml").write_text(
            f'[dependencies.libc]\nversion = "0.2"\n\n[package]\nname = "{name}"\nversion = "{version}"\n'
        )
        (crate / ".cargo-checksum.json").write_text("{}")
        (crate / "src" / "lib.rs").write_text(lib)
        if extra:
            (crate / extra).parent.mkdir(parents=True, exist_ok=True)
            (crate / extra).write_text("source")

    def test_stubs_are_listed_and_forbidden_crates_must_be_stubs(self):
        with tempfile.TemporaryDirectory() as scratch:
            vendor = pathlib.Path(scratch)
            self.crate(vendor, "unrar_sys", "unrar_sys", "0.5.8")
            self.crate(vendor, "windows-sys-0.52.0", "windows-sys", "0.52.0")
            self.crate(vendor, "unrar", "unrar", "0.5.8", lib="pub mod archive;")
            self.crate(vendor, "serde", "serde", "1.0.0", extra="src/de.rs")
            self.assertEqual(
                inspect_vendor(vendor),
                (["unrar_sys v0.5.8", "windows-sys v0.52.0"], ["unrar"]),
            )


if __name__ == "__main__":
    unittest.main()
