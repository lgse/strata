# SPDX-License-Identifier: MIT

import contextlib
import importlib.util
import io
from pathlib import Path
import re
import tempfile
import tomllib
import unittest

from image_inputs import IMAGE_INPUTS, REPOSITORY, crate_manifests

CONTAINER = REPOSITORY / "tests/container"
HASH = "0123456789abcdef"

ROOT = """
[workspace]
members = ["crates/*", "crates/shims/tests/support/peer"]

[package]
name = "app"

[[test]]
name = "qa"
path = "tests/qa.rs"
harness = false

[build-dependencies]
glib-build-tools = "1"
"""
MEMBERS = {
    "crates/app-core": '[package]\nname = "app-core"\nbuild = "codegen.rs"\n',
    "crates/shims": '[package]\nname = "shims"\n\n[lib]\nname = "shims_lib"\npath = "lib/mod.rs"\n',
    "crates/gallery": ('[package]\nname = "gallery"\nbuild = false\n\n[build-dependencies]\ncc = "1"\n\n'
                       '[[example]]\nname = "wiring"\npath = "examples/wiring/main.rs"\n\n'
                       '[[bin]]\nname = "gallery"\n\n[[bin]]\nname = "bless"\n\n[[bench]]\nname = "frames"\n'),
    "crates/shims/tests/support/peer": '[package]\nname = "peer"\n',
}


def stubber():
    spec = importlib.util.spec_from_file_location("stub_workspace", CONTAINER / "stub-workspace.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def stage():
    """The dependency stage of the image's recipe."""
    recipe = (CONTAINER / "Dockerfile").read_text(encoding="utf-8")
    return recipe[recipe.index("FROM toolchain AS dependencies"):]


class StubWorkspaceTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "Cargo.toml").write_text(ROOT, encoding="utf-8")
        for member, manifest in MEMBERS.items():
            (self.root / member).mkdir(parents=True)
            (self.root / member / "Cargo.toml").write_text(manifest, encoding="utf-8")
        # Another package's manifest in Cargo's cache, which is no member of the workspace.
        (self.root / "target/cargo/registry/src/serde").mkdir(parents=True)
        (self.root / "target/cargo/registry/src/serde/Cargo.toml").write_text('[package]\nname = "serde"\n')
        self.stub = stubber()

    def written(self):
        return {str(path.relative_to(self.root)): path.read_text() for path in self.root.rglob("*.rs")}

    def test_every_target_a_manifest_names_or_cargo_looks_for_gets_an_empty_file(self):
        self.assertEqual(self.stub.write_stubs(self.root),
                         ["app", "app-core", "gallery", "shims", "peer"])
        main = self.stub.MAIN
        self.assertEqual(self.written(), {
            "src/lib.rs": "", "src/main.rs": main, "tests/qa.rs": main, "build.rs": main,
            "crates/app-core/src/lib.rs": "", "crates/app-core/src/main.rs": main,
            "crates/app-core/codegen.rs": main,
            "crates/shims/lib/mod.rs": "", "crates/shims/src/main.rs": main,
            "crates/shims/tests/support/peer/src/lib.rs": "",
            "crates/shims/tests/support/peer/src/main.rs": main,
            "crates/gallery/src/lib.rs": "", "crates/gallery/src/main.rs": main,
            "crates/gallery/examples/wiring/main.rs": main, "crates/gallery/src/bin/bless.rs": main,
            "crates/gallery/benches/frames.rs": main,
        })

    def test_what_the_stubs_built_is_found_and_a_third_partys_files_are_not(self):
        target = self.root / "target/build"
        ours = [f"debug/deps/libapp_core-{HASH}.rlib", f"debug/deps/shims_lib-{HASH}.d",
                f"debug/deps/qa-{HASH}", f"debug/deps/bless-{HASH}", f"debug/.fingerprint/app-{HASH}",
                f"debug/build/app-core-{HASH}", "debug/app", "debug/libpeer.rlib", "debug/gallery.d",
                "debug/examples/wiring"]
        theirs = [f"debug/deps/libserde-{HASH}.rlib", f"debug/deps/application-{HASH}.d",
                  f"debug/.fingerprint/serde-{HASH}", "debug/.cargo-lock",
                  # A build script's own output, deeper than Cargo's artifacts.
                  f"debug/build/zstd-sys-{HASH}/out/app"]
        for name in ours + theirs:
            (target / name).parent.mkdir(parents=True, exist_ok=True)
            (target / name).write_text("")
        self.assertEqual(self.stub.leftovers(self.root, target), sorted(ours))
        for name in ours:
            (target / name).unlink()
        self.assertEqual(self.stub.leftovers(self.root, target), [])

    def test_the_command_fails_on_leftovers_and_on_a_tree_with_no_package(self):
        target = self.root / "target/build"
        (target / "debug/deps").mkdir(parents=True)
        with contextlib.redirect_stderr(io.StringIO()) as errors:
            self.assertEqual(self.stub.main(["leftovers", str(self.root), str(target)]), 0)
            (target / f"debug/deps/libapp-{HASH}.rlib").write_text("")
            self.assertEqual(self.stub.main(["leftovers", str(self.root), str(target)]), 1)
            self.assertIn(f"debug/deps/libapp-{HASH}.rlib", errors.getvalue())
            (self.root / "Cargo.toml").write_text("[workspace]\nmembers = []\n")
            self.assertEqual(self.stub.main(["packages", str(self.root)]), 1)


class RecipeTests(unittest.TestCase):
    def test_every_file_the_recipe_copies_from_its_directory_is_an_image_input(self):
        recipe = (CONTAINER / "Dockerfile").read_text(encoding="utf-8")
        copied = set(re.findall(r"^COPY (tests/container/\S+)", recipe, re.MULTILINE))
        self.assertIn("tests/container/stub-workspace.py", copied)
        self.assertEqual(copied | {"tests/container/Dockerfile"}, set(IMAGE_INPUTS))

    def test_every_image_input_is_where_ci_looks_for_a_changed_environment(self):
        # ci.yml asks `git diff -- tests/container` whether to build a candidate, and
        # test-image.yml publishes on `tests/container/**`.
        for name in IMAGE_INPUTS:
            self.assertTrue(name.startswith("tests/container/"), name)

    def test_the_build_context_and_the_dependency_key_hold_every_workspace_manifest(self):
        root = tomllib.loads((REPOSITORY / "Cargo.toml").read_text(encoding="utf-8"))
        members = {path for pattern in root["workspace"]["members"]
                   for path in REPOSITORY.glob(f"{pattern}/Cargo.toml")}
        self.assertTrue(any(len(path.relative_to(REPOSITORY).parts) > 3 for path in members),
                        "no member below crates/*: the case that broke the publisher")
        self.assertEqual(members - set(crate_manifests()), set())
        ignored = (REPOSITORY / ".dockerignore").read_text(encoding="utf-8").splitlines()
        self.assertIn("!crates/**/Cargo.toml", ignored)
        self.assertEqual(sorted(stubber().package_names(REPOSITORY)),
                         sorted(tomllib.loads(path.read_text())["package"]["name"]
                                for path in members | {REPOSITORY / "Cargo.toml"}))

    def test_the_stage_builds_what_the_qa_scripts_build_where_they_keep_it(self):
        recipe = stage()
        qa = (REPOSITORY / "scripts/qa.sh").read_text(encoding="utf-8")
        parity = (REPOSITORY / "scripts/qa-parity.sh").read_text(encoding="utf-8")
        gtk = "cargo test --locked --features qa,invariant-checks --test qa"
        rune = "cargo build --locked -p strata-rune --features qa"
        self.assertIn(gtk, qa)
        self.assertIn(rune, parity)
        self.assertIn(f"{gtk} --no-run", recipe)
        self.assertIn(rune, recipe)
        container = (REPOSITORY / "scripts/lib/qa-container.sh").read_text(encoding="utf-8")
        cache = re.search(r"STRATA_TEST_CARGO_CACHE:-\$repository/(target/[a-z-]+)\}", container).group(1)
        self.assertIn(f'--env "CARGO_HOME=/workspace/$cargo_cache/cargo"', container)
        self.assertIn(f'--env "CARGO_TARGET_DIR=/workspace/$cargo_cache/build"', container)
        self.assertIn(f"CARGO_HOME=/workspace/{cache}/cargo", recipe)
        self.assertIn(f"CARGO_TARGET_DIR=/workspace/{cache}/build", recipe)
        self.assertIn(f"/workspace/{cache} > /opt/strata-dependencies/cache-path", recipe)
        for variable in ("CARGO_PROFILE_DEV_DEBUG=0", "CARGO_INCREMENTAL=0"):
            self.assertIn(variable, recipe)
            self.assertIn(variable, container)

    def test_the_stage_never_builds_both_lanes_in_one_cargo_run_and_removes_its_stubs(self):
        recipe = stage()
        self.assertNotIn("--workspace", recipe)
        self.assertNotIn("--all-features", recipe)
        self.assertLess(recipe.index("stub-workspace.py stub"), recipe.index("cargo fetch"))
        self.assertLess(recipe.index("cargo clean"), recipe.index("stub-workspace.py leftovers"))
        self.assertLess(recipe.index("stub-workspace.py leftovers"), recipe.index("mv "))

    def test_the_cargo_home_a_run_copies_is_not_cleaned_by_cargo(self):
        # Cargo deletes the crates a command did not use for a month, which a base outlives.
        recipe = stage()
        self.assertIn('auto-clean-frequency = "never"', recipe)
        self.assertLess(recipe.index("auto-clean-frequency"), recipe.index("mv "))


if __name__ == "__main__":
    unittest.main()
