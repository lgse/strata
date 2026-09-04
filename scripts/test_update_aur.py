#!/usr/bin/env python3
"""Tests for `scripts/update_aur.py`."""

from __future__ import annotations

import unittest

from update_aur import (
    PACKAGES,
    PackagingError,
    archive_name,
    package_values,
    package_version,
    parse_checksum,
    preview_release_version,
    release_version,
    render,
    stable_release_version,
)

DIGEST = "0" * 64
OTHER_DIGEST = "1" * 64


class PackageVersionTests(unittest.TestCase):

    def test_a_stable_version_is_unchanged(self):
        self.assertEqual(package_version("0.7.0"), "0.7.0")

    def test_a_leading_v_is_dropped(self):
        self.assertEqual(package_version("v0.7.0"), "0.7.0")

    def test_only_the_prerelease_hyphen_is_removed(self):
        self.assertEqual(package_version("0.8.0-rc.1"), "0.8.0rc.1")
        self.assertEqual(package_version("0.8.0-beta.2"), "0.8.0beta.2")
        self.assertEqual(package_version("0.8.0-alpha.10"), "0.8.0alpha.10")

    def test_a_nightly_keeps_its_date_and_disambiguator_separate(self):
        self.assertEqual(
            package_version("0.8.0-nightly.20260901"), "0.8.0nightly.20260901"
        )
        self.assertEqual(
            package_version("0.8.0-nightly.20260901.2"), "0.8.0nightly.20260901.2"
        )

    def test_the_result_never_contains_a_character_pkgver_forbids(self):
        for version in (
            "0.7.0",
            "0.8.0-rc.1",
            "0.8.0-alpha.10",
            "0.8.0-nightly.20260901.2",
        ):
            mangled = package_version(version)
            self.assertFalse(
                set(mangled) & set(":/- \t"),
                f"{mangled!r} contains a character pkgver forbids",
            )

    def test_the_result_never_contains_a_hyphen(self):
        for version in ("0.8.0-rc.1", "0.8.0-nightly.20260901.2"):
            self.assertNotIn("-", package_version(version))

    def test_a_malformed_version_is_rejected(self):
        for version in (
            "0.8",
            "0.8.0.1",
            "latest",
            "0.8.0-rc 1",
            "0.8.0-preview.1",
            "",
        ):
            with self.assertRaises(PackagingError):
                package_version(version)


class ChecksumTests(unittest.TestCase):
    def test_a_digest_is_read_for_the_matching_archive(self):
        archive = archive_name("0.7.0", "x86_64")
        self.assertEqual(parse_checksum(f"{DIGEST}  {archive}\n", archive), DIGEST)

    def test_a_binary_mode_marker_is_tolerated(self):
        archive = archive_name("0.7.0", "aarch64")
        self.assertEqual(parse_checksum(f"{DIGEST} *{archive}\n", archive), DIGEST)

    def test_a_checksum_for_another_release_is_rejected(self):
        with self.assertRaises(PackagingError):
            parse_checksum(
                f"{DIGEST}  {archive_name('0.6.0', 'x86_64')}\n",
                archive_name("0.7.0", "x86_64"),
            )

    def test_a_checksum_for_the_other_architecture_is_rejected(self):
        with self.assertRaises(PackagingError):
            parse_checksum(
                f"{DIGEST}  {archive_name('0.7.0', 'aarch64')}\n",
                archive_name("0.7.0", "x86_64"),
            )

    def test_a_malformed_digest_is_rejected(self):
        archive = archive_name("0.7.0", "x86_64")
        with self.assertRaises(PackagingError):
            parse_checksum(f"not-a-digest  {archive}\n", archive)

    def test_an_empty_checksum_file_is_rejected(self):
        with self.assertRaises(PackagingError):
            parse_checksum("", archive_name("0.7.0", "x86_64"))


class RenderTests(unittest.TestCase):
    def test_every_placeholder_is_substituted(self):
        self.assertEqual(render("@A@/@B@", {"A": "one", "B": "two"}), "one/two")

    def test_an_unrendered_placeholder_is_rejected(self):
        with self.assertRaises(PackagingError) as error:
            render("@A@/@FORGOTTEN@", {"A": "one"})
        self.assertIn("FORGOTTEN", str(error.exception))


class PackageValuesTests(unittest.TestCase):
    def setUp(self):
        self.checksums = {"x86_64": DIGEST, "aarch64": OTHER_DIGEST}

    def test_the_stable_package_tracks_the_stable_channel(self):
        values = package_values("strata-bin", "0.7.0", 1, self.checksums)

        self.assertEqual(values["PKGVER"], "0.7.0")
        self.assertEqual(values["RELEASEVER"], "0.7.0")
        self.assertEqual(values["CHANNEL"], "stable")
        self.assertEqual(values["ALTERNATE"], "strata-rc-bin")

    def test_no_package_names_a_pacman_update_command(self):
        for pkgname in PACKAGES:
            values = package_values(pkgname, "0.7.0", 1, self.checksums)
            self.assertNotIn("UPDATE_COMMAND", values)

    def test_the_rc_package_keeps_the_unmangled_release_version(self):
        values = package_values("strata-rc-bin", "0.8.0-rc.1", 1, self.checksums)

        self.assertEqual(values["PKGVER"], "0.8.0rc.1")
        self.assertEqual(
            values["RELEASEVER"],
            "0.8.0-rc.1",
            "the download URL must use the real release tag, not the mangled pkgver",
        )
        self.assertEqual(values["CHANNEL"], "rc")
        self.assertEqual(values["ALTERNATE"], "strata-bin")

    def test_each_architecture_keeps_its_own_checksum(self):
        values = package_values("strata-bin", "0.7.0", 1, self.checksums)

        self.assertEqual(values["SHA256_X86_64"], DIGEST)
        self.assertEqual(values["SHA256_AARCH64"], OTHER_DIGEST)

    def test_the_packages_name_each_other_as_alternates(self):
        for pkgname, package in PACKAGES.items():
            self.assertEqual(PACKAGES[package["alternate"]]["alternate"], pkgname)


class ReleaseVersionTests(unittest.TestCase):
    def test_a_leading_v_and_surrounding_space_are_removed(self):
        self.assertEqual(release_version("  v0.8.0-rc.1 "), "0.8.0-rc.1")

    def test_an_invalid_version_is_rejected(self):
        with self.assertRaises(PackagingError):
            release_version("main")

    def test_stable_rejects_prereleases(self):
        with self.assertRaises(PackagingError):
            stable_release_version("0.10.0-rc.1")

    def test_preview_accepts_final_and_staged_prereleases(self):
        for version in ("0.9.0", "0.10.0-alpha.1", "0.10.0-beta.2", "0.10.0-rc.3"):
            self.assertEqual(preview_release_version(version), version)

    def test_preview_rejects_nightly(self):
        with self.assertRaises(PackagingError):
            preview_release_version("0.10.0-nightly.20260901")


if __name__ == "__main__":
    unittest.main()
