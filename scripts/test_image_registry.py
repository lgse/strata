# SPDX-License-Identifier: MIT

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

from image_inputs import IMAGE_INPUTS, dependency_key
from image_registry import check_image_labels, main, references, resolve


class ImageIdentityTests(unittest.TestCase):
    def test_tags_bind_platform_accounts_and_inputs_without_latest(self):
        refs = references("Example/Strata", "a" * 64, "b" * 64, 1001, 1001)
        self.assertTrue(all(ref.startswith("ghcr.io/example/strata-test-image:") for ref in refs.values()))
        self.assertTrue(all(ref.endswith("-linux-amd64-1001-1001") for ref in refs.values()))
        self.assertIn("b" * 64, refs["build"])
        self.assertIn("a" * 64, refs["environment_image"])
        self.assertIn("a" * 64, refs["environment_cache"])
        self.assertTrue(all(len(ref.split(":")[-1]) <= 128 for ref in refs.values()))
        self.assertNotEqual(refs, references("Example/Strata", "a" * 64, "b" * 64, 1000, 1000))

    def test_rejects_invalid_names_and_non_digest_input_keys(self):
        for repository, key in (("../strata", "a" * 64), ("example/strata;command", "a" * 64),
                                ("example/strata", "latest")):
            with self.subTest(repository=repository, key=key), self.assertRaises(ValueError):
                references(repository, key, "b" * 64, 1001, 1001)

    def test_dependency_identity_changes_only_with_environment_or_dependency_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = (*IMAGE_INPUTS, "Cargo.toml", "Cargo.lock", ".dockerignore")
            for name in files:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            original = dependency_key(root)
            (root / "application.rs").write_text("changed application")
            self.assertEqual(dependency_key(root), original)
            for name in files:
                with self.subTest(name=name):
                    (root / name).write_text(name + "changed")
                    self.assertNotEqual(dependency_key(root), original)
                    (root / name).write_text(name)


class RegistryDiscoveryTests(unittest.TestCase):
    def test_pins_the_top_level_manifest_digest_not_an_arbitrary_platform(self):
        manifest = {"digest": "sha256:" + "a" * 64,
                    "manifests": [{"digest": "sha256:" + "b" * 64}]}
        result = Mock(returncode=0, stdout=json.dumps(manifest))
        with patch("image_registry.subprocess.run", return_value=result) as run:
            pinned = resolve("ghcr.io/example/strata-test-image:inputs")
        self.assertEqual(pinned, "ghcr.io/example/strata-test-image@sha256:" + "a" * 64)
        self.assertEqual(run.call_args.kwargs["timeout"], 20)
        self.assertNotIn("shell", run.call_args.kwargs)

    def test_unavailable_is_a_cache_miss_but_malformed_success_fails_closed(self):
        with patch("image_registry.subprocess.run", return_value=Mock(returncode=1)):
            self.assertIsNone(resolve("ghcr.io/example/build:missing"))
        for manifest in ({}, [], {"digest": "latest"}, {"digest": "sha256:bad\ncache_from=other"}):
            with self.subTest(manifest=manifest), \
                 patch("image_registry.subprocess.run", return_value=Mock(returncode=0, stdout=json.dumps(manifest))), \
                 self.assertRaises(ValueError):
                resolve("ghcr.io/example/build:bad")

    def test_base_configuration_is_read_by_digest_before_accepting_its_labels(self):
        labels = {"org.strata.test.inputs": "a" * 64}
        image = {"os": "linux", "architecture": "amd64", "config": {"Labels": labels}}
        results = [Mock(returncode=0, stdout=json.dumps({"digest": "sha256:" + "b" * 64})),
                   Mock(returncode=0, stdout=json.dumps({"linux/amd64": image}))]
        with patch("image_registry.subprocess.run", side_effect=results) as run:
            pinned = resolve("ghcr.io/example/build:inputs", expected_labels=labels)
        self.assertIn(pinned, run.call_args.args[0])
        self.assertIn("{{json .Image}}", run.call_args.args[0])

    def test_wrong_or_missing_input_labels_and_platforms_are_rejected(self):
        labels = {"org.strata.test.inputs": "a" * 64}
        correct = {"os": "linux", "architecture": "amd64", "config": {"Labels": labels}}
        check_image_labels(correct, labels)
        check_image_labels({"linux/amd64": correct}, labels)
        for image in ([], {}, {**correct, "architecture": "arm64"}, {**correct, "config": {}},
                      {**correct, "config": {"Labels": {"org.strata.test.inputs": "wrong"}}}):
            with self.subTest(image=image), self.assertRaises(ValueError):
                check_image_labels(image, labels)

    def test_publisher_checks_require_the_published_image_with_its_labels(self):
        for command, labels in (("environment", {"org.strata.test.inputs"}),
                                ("build", {"org.strata.test.inputs", "org.strata.test.dependencies"})):
            output, error = io.StringIO(), io.StringIO()
            with self.subTest(command=command), \
                 patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/strata"}), \
                 patch("sys.argv", ["image_registry.py", command]), \
                 patch("image_registry.resolve", return_value=None) as lookup, \
                 redirect_stdout(output), redirect_stderr(error), self.assertRaises(SystemExit) as exited:
                main()
            self.assertEqual(set(lookup.call_args.kwargs["expected_labels"]), labels)
            self.assertEqual(exited.exception.code, 1)
            self.assertEqual(output.getvalue(), "")
            self.assertIn("unavailable anonymously", error.getvalue())

    def test_registry_timeout_fails_the_publisher_check(self):
        error = io.StringIO()
        with patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/strata"}), \
             patch("sys.argv", ["image_registry.py", "build"]), \
             patch("image_registry.resolve", side_effect=subprocess.TimeoutExpired("inspect", 20)), \
             redirect_stderr(error), self.assertRaises(SystemExit) as exited:
            main()
        self.assertEqual(exited.exception.code, 1)

if __name__ == "__main__":
    unittest.main()
