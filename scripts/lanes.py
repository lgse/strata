#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Cargo package selections for the two sides of the workspace.

The GTK application and the Rune side never build in one cargo invocation. Cargo would
unify their dependency features, and Rune turns on fontconfig's `dlopen` feature, under
which the application does not compile. See docs/workspace.md.
"""

import argparse

# Every workspace package that builds on Rune, and the test tooling only they use.
RUNE_SIDE = (
    "strata-platform",
    "strata-kit",
    "strata-components",
    "strata-gallery",
    "strata-test-peer",
    "strata-app",
    "strata-rune",
)


def gtk():
    """The application and the crates it shares: the workspace without the Rune side."""
    return ["--workspace", *(arg for name in RUNE_SIDE for arg in ("--exclude", name))]


def rune():
    return [arg for name in RUNE_SIDE for arg in ("--package", name)]


LANES = {"gtk": gtk, "rune": rune}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", choices=sorted(LANES))
    print(" ".join(LANES[parser.parse_args().lane]()))


if __name__ == "__main__":
    main()
