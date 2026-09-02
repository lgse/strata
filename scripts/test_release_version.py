#!/usr/bin/env python3
"""Tests for `scripts/release_version.py`.

Run with:

    python3 scripts/test_release_version.py

or, to run every script test in the repo the same way CI does:

    python3 -m unittest discover -s scripts -p 'test_*.py'

Stdlib `unittest` only -- no dependencies, matching the script it tests.
"""

from __future__ import annotations

import unittest

from release_version import (
    VersionError,
    compute_next_version,
    ensure_tag_available,
    next_prerelease_ordinal,
    nightly_version,
    split_tags,
)


class BumpStableTests(unittest.TestCase):
    """Each bump level for `stable`, behaviour unchanged from the original
    inline heredoc."""

    def test_patch(self) -> None:
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "stable", []), "0.5.1"
        )

    def test_minor(self) -> None:
        self.assertEqual(
            compute_next_version("0.5.7", "minor", "stable", []), "0.6.0"
        )

    def test_major(self) -> None:
        self.assertEqual(
            compute_next_version("0.5.7", "major", "stable", []), "1.0.0"
        )

    def test_rejects_tag_collision(self) -> None:
        with self.assertRaisesRegex(VersionError, "already exists"):
            compute_next_version("0.5.0", "patch", "stable", ["v0.5.1"])

    def test_unrelated_existing_tags_do_not_interfere(self) -> None:
        self.assertEqual(
            compute_next_version(
                "0.5.0", "patch", "stable", ["v0.4.0", "v0.5.0", "v0.5.0-rc.1"]
            ),
            "0.5.1",
        )


class PrereleaseOrdinalTests(unittest.TestCase):
    """Ordinal selection for alpha, beta, and RC publication modes."""

    def test_each_stage_has_an_independent_ordinal(self) -> None:
        tags = ["v0.5.1-alpha.2", "v0.5.1-beta.3", "v0.5.1-rc.4"]
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "alpha", tags),
            "0.5.1-alpha.3",
        )
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "beta", tags),
            "0.5.1-beta.4",
        )
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "rc", tags),
            "0.5.1-rc.5",
        )

    def test_first_rc_for_a_core_with_no_existing_rc_tags(self) -> None:
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "rc", []), "0.5.1-rc.1"
        )

    def test_first_rc_ignores_other_cores_and_other_kinds(self) -> None:
        self.assertEqual(
            compute_next_version(
                "0.5.0",
                "patch",
                "rc",
                ["v0.5.0", "v0.4.0-rc.3", "v0.5.1-nightly.20260101"],
            ),
            "0.5.1-rc.1",
        )

    def test_rc_2_after_rc_1(self) -> None:
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "rc", ["v0.5.1-rc.1"]),
            "0.5.1-rc.2",
        )

    def test_rc_10_ordering_after_rc_9_is_numeric_not_lexicographic(self) -> None:
        # A lexicographic comparison would sort "rc.10" before "rc.9" (the
        # string "1" < "9"), and could pick 10 as the max and produce
        # `rc.11` while never having "seen" `rc.9`, or worse, treat `rc.9`
        # as the max and collide by reissuing `rc.10`. Numeric comparison
        # must pick 10 as the max regardless of insertion order.
        tags = [f"v0.5.1-rc.{n}" for n in range(1, 11)]  # rc.1 .. rc.10
        self.assertEqual(
            compute_next_version("0.5.0", "patch", "rc", tags), "0.5.1-rc.11"
        )

    def test_next_rc_ordinal_numeric_ordering_directly(self) -> None:
        self.assertEqual(
            next_prerelease_ordinal(
                "0.5.1", "rc", ["v0.5.1-rc.9", "v0.5.1-rc.10"]
            ),
            11,
        )
        self.assertEqual(
            next_prerelease_ordinal(
                "0.5.1", "rc", ["v0.5.1-rc.10", "v0.5.1-rc.9"]
            ),
            11,
        )

    def test_non_numeric_or_mismatched_suffixes_are_ignored(self) -> None:
        self.assertEqual(
            next_prerelease_ordinal(
                "0.5.1",
                "rc",
                ["v0.5.1-rc.abc", "v0.5.1-rc.", "v0.5.2-rc.9"],
            ),
            1,
        )


class NightlyVersionTests(unittest.TestCase):
    def test_first_nightly_uses_the_utc_date_without_a_suffix(self) -> None:
        self.assertEqual(
            compute_next_version("0.7.0", "minor", "nightly", [], "20260904"),
            "0.8.0-nightly.20260904",
        )

    def test_repeated_same_day_nightlies_get_numeric_suffixes(self) -> None:
        tags = [
            "v0.8.0-nightly.20260904",
            "v0.8.0-nightly.20260904.1",
            "v0.8.0-nightly.20260904.2",
        ]
        self.assertEqual(
            compute_next_version(
                "0.7.0", "minor", "nightly", tags, "20260904"
            ),
            "0.8.0-nightly.20260904.3",
        )

    def test_other_dates_and_cores_do_not_affect_the_suffix(self) -> None:
        self.assertEqual(
            nightly_version(
                "0.8.0",
                "20260904",
                [
                    "v0.8.0-nightly.20260903.9",
                    "v0.9.0-nightly.20260904.9",
                ],
            ),
            "0.8.0-nightly.20260904",
        )

    def test_nightly_requires_a_real_calendar_date(self) -> None:
        for release_date in ["", "20261301", "20260230", "2026-09-04"]:
            with self.subTest(release_date=release_date):
                with self.assertRaises(VersionError):
                    compute_next_version(
                        "0.7.0", "minor", "nightly", [], release_date
                    )

    def test_nightly_requires_the_date_argument(self) -> None:
        with self.assertRaises(VersionError):
            compute_next_version("0.7.0", "minor", "nightly", [])


class TagCollisionTests(unittest.TestCase):
    """Rejection when the computed tag already exists, mirroring the
    stable guard. RC's `N = max + 1` scan can never organically reproduce
    an existing tag, so this exercises the shared guard function directly
    -- the same one `compute_next_version` calls for every mode -- as
    defense in depth against a manually created or out-of-band tag."""

    def test_ensure_tag_available_raises_on_duplicate(self) -> None:
        with self.assertRaisesRegex(VersionError, "v0.5.1-rc.3 already exists"):
            ensure_tag_available("v0.5.1-rc.3", ["v0.5.1-rc.3"])

    def test_ensure_tag_available_passes_when_absent(self) -> None:
        ensure_tag_available("v0.5.1-rc.3", ["v0.5.1-rc.1", "v0.5.1-rc.2"])


class InputValidationTests(unittest.TestCase):
    def test_rejects_non_plain_current_version(self) -> None:
        with self.assertRaises(VersionError):
            compute_next_version("0.5.0-rc.1", "patch", "stable", [])

    def test_rejects_unsupported_bump(self) -> None:
        with self.assertRaises(VersionError):
            compute_next_version("0.5.0", "sideways", "stable", [])

    def test_rejects_unsupported_mode(self) -> None:
        with self.assertRaises(VersionError):
            compute_next_version("0.5.0", "patch", "canary", [])

    def test_rejects_unsupported_prerelease_kind(self) -> None:
        with self.assertRaises(VersionError):
            next_prerelease_ordinal("0.5.1", "preview", [])


class SplitTagsTests(unittest.TestCase):
    def test_splits_on_any_whitespace(self) -> None:
        self.assertEqual(
            split_tags("v0.5.0\nv0.5.1-rc.1\n\nv0.4.0"),
            ["v0.5.0", "v0.5.1-rc.1", "v0.4.0"],
        )

    def test_empty_input_yields_no_tags(self) -> None:
        self.assertEqual(split_tags(""), [])
        self.assertEqual(split_tags("   \n  "), [])


if __name__ == "__main__":
    unittest.main()
