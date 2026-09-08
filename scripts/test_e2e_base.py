# SPDX-License-Identifier: GPL-3.0-or-later

import unittest
from unittest.mock import Mock, patch

from e2e_base import build_base, ensure_base, verify_image
from e2e_bundle import image_key


def valid_image():
    return {"Id": "sha256:" + "a" * 64, "Os": "linux", "Architecture": "amd64",
            "Config": {"Labels": {"org.strata.e2e.inputs": image_key()},
                       "Env": ["RUSTUP_HOME=/opt/rustup"]}}


class LocalBaseTests(unittest.TestCase):
    def test_cached_base_requires_no_pull_or_build(self):
        with patch("e2e_base.inspect", return_value=valid_image()), \
             patch("e2e_base.subprocess.run") as run:
            self.assertEqual(ensure_base("podman"), valid_image()["Id"])
        run.assert_not_called()

    def test_application_manifest_changes_do_not_force_an_environment_refresh(self):
        with patch("e2e_base.inspect", return_value=valid_image()), \
             patch("e2e_base.dependency_key", side_effect=AssertionError("must reuse the environment")), \
             patch("e2e_base.subprocess.run") as run:
            ensure_base("podman")
        run.assert_not_called()

    def test_cached_registry_reference_is_reused_without_network(self):
        with patch("e2e_base.inspect", side_effect=[None, valid_image()]), \
             patch("e2e_base.subprocess.run", return_value=Mock(returncode=0)) as run:
            ensure_base("podman")
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][:2], ["podman", "tag"])

    def test_missing_base_is_pulled_verified_and_recorded_locally(self):
        with patch("e2e_base.inspect", side_effect=[None, None, valid_image()]), \
             patch("e2e_base.subprocess.run", return_value=Mock(returncode=0)) as run, \
             patch("builtins.print"):
            ensure_base("podman")
        self.assertEqual([call.args[0][1] for call in run.call_args_list], ["pull", "tag"])
        self.assertIn("-1001-1001", run.call_args_list[0].args[0][-1])
        self.assertEqual(run.call_args_list[1].args[0][2], valid_image()["Id"])

    def test_missing_publication_never_silently_builds_from_ubuntu(self):
        with patch("e2e_base.inspect", return_value=None), \
             patch("e2e_base.subprocess.run", return_value=Mock(returncode=1)) as run, \
             patch("builtins.print"), self.assertRaisesRegex(ValueError, "explicit local environment build"):
            ensure_base("podman")
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0][1], "pull")

    def test_wrong_local_provenance_is_rejected_without_replacing_the_image(self):
        image = valid_image()
        image["Config"]["Labels"]["org.strata.e2e.inputs"] = "wrong"
        with patch("e2e_base.inspect", return_value=image), patch("e2e_base.subprocess.run") as run, \
             self.assertRaisesRegex(ValueError, "input labels"):
            ensure_base("podman")
        run.assert_not_called()

    def test_runtime_only_images_cannot_be_used_to_compile_the_application(self):
        image = valid_image()
        image["Config"]["Env"] = []
        with self.assertRaisesRegex(ValueError, "runtime-only"):
            verify_image(image, image_key())

    def test_explicit_build_uses_pinned_recipe_and_current_account_ids(self):
        with patch("e2e_base.os.getuid", return_value=1234), patch("e2e_base.os.getgid", return_value=5678), \
             patch("e2e_base.subprocess.run", return_value=Mock(returncode=0)) as run, \
             patch("e2e_base.inspect", return_value=valid_image()):
            build_base("podman")
        command = run.call_args.args[0]
        self.assertEqual(command[:2], ["podman", "build"])
        self.assertIn("E2E_UID=1234", command)
        self.assertIn("E2E_GID=5678", command)
        self.assertIn("org.strata.e2e.inputs=" + image_key(), command)
        self.assertIn("toolchain", command)


if __name__ == "__main__":
    unittest.main()
