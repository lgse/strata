# SPDX-License-Identifier: MIT

import contextlib
import hashlib
import io
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import dependency_seed
from dependency_seed import Cache, seed_cache

LOCK = "version = 4\n"
# Older than anything a test writes: a copy that kept it was not rebuilt or touched.
BUILT = 1_600_000_000


class SeedTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.seed = self.root / "image"
        self.qa = self.root / "workspace/target/qa-container"
        self.lock = self.root / "workspace/Cargo.lock"
        self.write(self.lock, LOCK)
        for name in ("cargo/registry/cache/serde.crate", "cargo/registry/src/serde/lib.rs",
                     "cargo/git/db/rune", "cargo/.global-cache", "build/debug/deps/libserde.rlib"):
            self.write(self.seed / name, name)
            os.utime(self.seed / name, (BUILT, BUILT))
        self.write(self.seed / "cache-path", f"{self.qa}\n")
        self.write(self.seed / "lock.sha256", hashlib.sha256(LOCK.encode()).hexdigest() + "\n")
        self.said = []

    @staticmethod
    def write(path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def cache(self, directory=None):
        directory = directory or self.qa
        return Cache(directory / "cargo", directory / "build")

    def run_seed(self, cache=None):
        return seed_cache(self.seed, cache or self.cache(), self.lock, self.said.append)

    def files(self, directory):
        return sorted(str(path.relative_to(directory)) for path in directory.rglob("*") if path.is_file())

    def test_an_empty_cache_takes_the_fetched_and_the_compiled_crates_with_their_times(self):
        # The scripts make the Cargo home before Cargo runs.
        self.cache().home.mkdir(parents=True)
        self.run_seed()
        self.assertEqual(self.files(self.qa), [
            "build/debug/deps/libserde.rlib", "cargo/.global-cache", "cargo/git/db/rune",
            "cargo/registry/cache/serde.crate", "cargo/registry/src/serde/lib.rs"])
        for name in ("build/debug/deps/libserde.rlib", "cargo/registry/src/serde/lib.rs"):
            self.assertEqual((self.qa / name).stat().st_mtime, BUILT, name)
        self.assertEqual(len(self.said), 1)
        self.assertIn("copied the fetched and the compiled crates", self.said[0])
        self.assertEqual([path.name for path in self.qa.iterdir() if "seeding" in path.name], [])

    def test_a_cache_cargo_has_used_is_left_exactly_as_it_was(self):
        for name, fetched in (("registry", "cargo/registry/cache/mine.crate"), ("git", "cargo/git/db/mine")):
            with self.subTest(name), tempfile.TemporaryDirectory() as directory:
                cache = self.cache(Path(directory) / "target/qa-container")
                self.write(cache.home.parent / fetched, "mine")
                self.write(cache.target / "debug/deps/libmine.rlib", "mine")
                self.said.clear()
                chosen = self.run_seed(cache)
                self.assertEqual((chosen.sources, chosen.builds), (False, False))
                self.assertEqual(self.files(cache.home.parent), sorted([fetched, "build/debug/deps/libmine.rlib"]))
                self.assertEqual(len(self.said), 1)
                self.assertIn("reusing the Cargo cache", self.said[0])

    def test_a_cache_at_another_path_takes_the_sources_and_builds_its_own(self):
        quality = self.root / "workspace/target/quality-container"
        self.run_seed(self.cache(quality))
        self.assertEqual(self.files(quality), [
            "cargo/.global-cache", "cargo/git/db/rune", "cargo/registry/cache/serde.crate",
            "cargo/registry/src/serde/lib.rs"])
        self.assertIn(f"compiled crates are for {self.qa}", self.said[0])
        self.assertIn("copied the fetched crates", self.said[-1])

    def test_a_target_directory_with_builds_in_it_keeps_them(self):
        self.write(self.cache().target / "debug/deps/libmine.rlib", "mine")
        self.run_seed()
        self.assertEqual(self.files(self.qa / "build"), ["debug/deps/libmine.rlib"])
        self.assertTrue((self.qa / "cargo/registry/cache/serde.crate").is_file())
        self.assertIn("is not empty", self.said[0])

    def test_a_lockfile_ahead_of_the_image_is_seeded_and_said_loudly(self):
        self.write(self.lock, LOCK + '[[package]]\nname = "new"\n')
        self.run_seed()
        self.assertTrue((self.qa / "cargo/registry/cache/serde.crate").is_file())
        self.assertTrue((self.qa / "build/debug/deps/libserde.rlib").is_file())
        self.assertEqual(self.said[0], "Dependencies: WARNING: Cargo.lock has moved past the image.")
        self.assertIn("fetches the crates that differ from the network", "\n".join(self.said))
        # An image that does not say what it was fetched for is not taken on trust.
        (self.seed / "lock.sha256").unlink()
        self.said.clear()
        self.run_seed(self.cache(self.root / "elsewhere"))
        self.assertIn("WARNING", self.said[0])

    def test_a_base_without_dependencies_leaves_cargo_to_fetch(self):
        chosen = seed_cache(self.root / "no-image", self.cache(), self.lock, self.said.append)
        self.assertEqual((chosen.sources, chosen.builds), (False, False))
        self.assertFalse(self.qa.exists())
        self.assertEqual(self.said, ["Dependencies: this base holds none; Cargo fetches what it needs."])

    def test_a_copy_cut_short_is_made_again(self):
        self.write(self.qa / "cargo.seeding/registry/cache/half.crate", "half")
        self.write(self.qa / "build.seeding/debug/deps/libhalf.rlib", "half")
        self.run_seed()
        self.assertNotIn("half", " ".join(self.files(self.qa)))
        self.assertTrue((self.qa / "cargo/registry/cache/serde.crate").is_file())
        # Cut short after the compiled crates went in: the sources still follow.
        other = self.cache(self.root / "again/target/qa-container")
        self.write(self.seed / "cache-path", f"{other.home.parent}\n")
        dependency_seed.seed_builds(self.seed, other)
        self.assertFalse(other.fetched())
        self.run_seed(other)
        self.assertTrue(other.fetched())


class CommandTests(unittest.TestCase):
    def run_main(self, environment, **patches):
        output, errors = io.StringIO(), io.StringIO()
        with patch.dict(os.environ, environment, clear=True), contextlib.ExitStack() as stack:
            for name, value in patches.items():
                stack.enter_context(patch(f"dependency_seed.{name}", value))
            stack.enter_context(contextlib.redirect_stdout(output))
            stack.enter_context(contextlib.redirect_stderr(errors))
            status = dependency_seed.main()
        return status, output.getvalue(), errors.getvalue()

    def test_it_needs_the_cache_cargo_is_given(self):
        status, _, errors = self.run_main({"CARGO_HOME": "/x"})
        self.assertEqual(status, 2)
        self.assertIn("CARGO_TARGET_DIR", errors)

    def test_a_copy_that_fails_leaves_an_empty_cache_for_cargo_to_fill(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "image/cargo/registry").mkdir(parents=True)
            cache = Cache(root / "cache/cargo", root / "cache/build")
            environment = {"CARGO_HOME": str(cache.home), "CARGO_TARGET_DIR": str(cache.target),
                           "PATH": os.environ["PATH"]}

            def full(command, **_):
                Path(command[-1]).mkdir(parents=True)
                raise subprocess.CalledProcessError(1, command)

            with patch("dependency_seed.subprocess.run", side_effect=full):
                status, _, errors = self.run_main(environment, SEED=root / "image")
            self.assertEqual(status, 0)
            self.assertIn("Cargo fetches what it needs", errors)
            self.assertFalse(cache.fetched())
            self.assertEqual([path.name for path in (root / "cache").iterdir()], [])


class ScriptTests(unittest.TestCase):
    SCRIPTS = Path(__file__).resolve().parent

    def test_every_script_that_compiles_in_the_image_seeds_its_cache_first(self):
        for name in ("qa.sh", "qa-parity.sh", "qa-drive.sh", "quality.sh"):
            script = (self.SCRIPTS / name).read_text(encoding="utf-8")
            with self.subTest(name):
                self.assertIn("python3 scripts/dependency_seed.py", script)
                compiles = re.search(r"\bcargo (?:test|build|clippy) ", script).start()
                self.assertLess(script.index("dependency_seed.py"), compiles)

    def test_formatting_and_a_shard_copy_nothing(self):
        quality = (self.SCRIPTS / "quality.sh").read_text(encoding="utf-8")
        self.assertIn("fmt:*|test:shard) ;;", quality)


if __name__ == "__main__":
    unittest.main()
