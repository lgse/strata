#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Seeds an empty Cargo cache from the dependencies the pinned test image holds.

The image's dependency stage (tests/container/Dockerfile) leaves under /opt/strata-dependencies
every crate Cargo.lock names, fetched, and the third-party crates of the QA builds, compiled.
qa.sh, qa-parity.sh, qa-drive.sh and quality.sh run this inside the container before Cargo, so
a first run in a new checkout fetches nothing.

A cache Cargo has already fetched into is never touched. The compiled crates are copied only
into the cache they were built for, target/qa-container: Cargo's fingerprints hold the path of
a crate's sources, so anywhere else they would all be rebuilt.
"""

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

SEED = Path("/opt/strata-dependencies")
# Where an interrupted copy stays until the next run removes it.
PARTIAL = ".seeding"


@dataclass(frozen=True)
class Cache:
    """One run's Cargo cache: CARGO_HOME and CARGO_TARGET_DIR."""
    home: Path
    target: Path

    def fetched(self):
        """Whether Cargo, or an earlier seeding, has put crates here."""
        return (self.home / "registry").exists() or (self.home / "git").exists()

    def built(self):
        return self.target.is_dir() and any(self.target.iterdir())

    def partials(self):
        """Where a copy is made before it is moved in, and where a cut-short one stays."""
        return [path.with_name(path.name + PARTIAL) for path in (self.target, self.home)]


@dataclass(frozen=True)
class Plan:
    sources: bool = False
    builds: bool = False
    notes: tuple = ()


def read(path):
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return None


def lock_digest(lock):
    try:
        return hashlib.sha256(lock.read_bytes()).hexdigest()
    except OSError:
        return None


def stale_lock(seed, lock):
    """What to say when the image's dependencies were fetched for another Cargo.lock."""
    held, wanted = read(seed / "lock.sha256"), lock_digest(lock)
    if held is not None and held == wanted:
        return ()
    return (
        "WARNING: Cargo.lock has moved past the image.",
        f"  The image's dependencies were fetched for another Cargo.lock ({str(held)[:12]}; "
        f"this checkout's is {str(wanted)[:12]}).",
        "  Cargo now fetches the crates that differ from the network, and builds them.",
        "  A base built from this checkout holds them: python3 scripts/image_base.py build.",
    )


def plan(seed, cache, lock):
    """What to copy into the cache, and what to tell whoever reads the run's output."""
    if not (seed / "cargo").is_dir():
        return Plan(notes=("this base holds none; Cargo fetches what it needs.",))
    if cache.fetched():
        return Plan(notes=(f"reusing the Cargo cache in {cache.home.parent}; nothing is copied.",))
    built_for = read(seed / "cache-path")
    here = built_for is not None and (cache.home, cache.target) == (
        Path(built_for) / "cargo", Path(built_for) / "build")
    notes = list(stale_lock(seed, lock))
    if not here:
        notes.append(f"the image's compiled crates are for {built_for}; this cache builds its own "
                     "from the fetched sources.")
    elif cache.built():
        notes.append(f"{cache.target} is not empty, so the image's compiled crates are not copied.")
    builds = here and not cache.built() and (seed / "build").is_dir()
    return Plan(sources=True, builds=builds, notes=tuple(notes))


def copy(source, partial):
    """A copy that keeps every timestamp: Cargo compares them to decide what is fresh."""
    shutil.rmtree(partial, ignore_errors=True)
    partial.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cp", "--archive", "--reflink=auto", str(source), str(partial)], check=True)


def seed_builds(seed, cache):
    partial = cache.partials()[0]
    copy(seed / "build", partial)
    if cache.target.exists():
        cache.target.rmdir()
    partial.rename(cache.target)


def seed_sources(seed, cache):
    """Copies the Cargo home beside the real one, then moves it in, `registry` last.

    `registry` is what marks a cache as fetched, so a copy cut short is made again.
    """
    partial = cache.partials()[1]
    copy(seed / "cargo", partial)
    cache.home.mkdir(parents=True, exist_ok=True)
    for entry in sorted(partial.iterdir(), key=lambda entry: entry.name == "registry"):
        destination = cache.home / entry.name
        if destination.is_dir():
            shutil.rmtree(destination)
        entry.replace(destination)
    partial.rmdir()


def seed_cache(seed, cache, lock, say=print):
    chosen = plan(seed, cache, lock)
    for note in chosen.notes:
        say(note if note.startswith(" ") else f"Dependencies: {note}")
    started = time.monotonic()
    # The compiled crates go first: once `registry` is there the cache counts as fetched.
    if chosen.builds:
        seed_builds(seed, cache)
    if chosen.sources:
        seed_sources(seed, cache)
        what = "the fetched and the compiled crates" if chosen.builds else "the fetched crates"
        say(f"Dependencies: copied {what} from the image into {cache.home.parent} "
            f"in {time.monotonic() - started:.0f} s.")
    return chosen


def main():
    try:
        cache = Cache(Path(os.environ["CARGO_HOME"]), Path(os.environ["CARGO_TARGET_DIR"]))
    except KeyError as missing:
        print(f"Dependencies: {missing} is not set.", file=sys.stderr)
        return 2
    try:
        seed_cache(SEED, cache, Path("Cargo.lock"), lambda line: print(line, flush=True))
    except (OSError, subprocess.CalledProcessError) as error:
        # A cache that could not be seeded is an empty one: Cargo fetches, as it did before.
        for partial in cache.partials():
            shutil.rmtree(partial, ignore_errors=True)
        print(f"Dependencies: could not copy the image's ({error}); Cargo fetches what it needs.",
              file=sys.stderr, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
