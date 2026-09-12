# SPDX-License-Identifier: MIT

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

from e2e_bundle import REPOSITORY, image_key


class QualityRunnerTests(unittest.TestCase):
    def run_runner(self, phase="all", key=None, engine_name="podman", extra_env=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "engine.jsonl"
            engine = root / engine_name
            engine.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                f"with open({str(log)!r}, 'a') as f: f.write(json.dumps(dict(args=sys.argv[1:], "
                "display=os.getenv('DISPLAY'), wayland=os.getenv('WAYLAND_DISPLAY'), "
                "bus=os.getenv('DBUS_SESSION_BUS_ADDRESS'))) + '\\n')\n"
                "if sys.argv[1:3] == ['image', 'inspect']:\n"
                f" print(json.dumps([{{'Id': 'sha256:'+'a'*64, 'Os': 'linux', 'Architecture': 'amd64', "
                f"'Config': {{'Labels': {{'org.strata.e2e.inputs': {(image_key() if key is None else key)!r}}}, "
                "'Env': ['RUSTUP_HOME=/opt/rustup']}}]))\n"
            )
            engine.chmod(0o755)
            result = subprocess.run([str(REPOSITORY / "scripts/quality.sh"), phase],
                                    env={**os.environ, "STRATA_CONTAINER_ENGINE": str(engine),
                                         "STRATA_QUALITY_IMAGE": "fixture", "DISPLAY": ":0",
                                         "WAYLAND_DISPLAY": "wayland-0", "DBUS_SESSION_BUS_ADDRESS": "unix:path=/desktop",
                                         **(extra_env or {})}, capture_output=True, text=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, calls

    def test_phases_use_verified_immutable_image_without_building(self):
        for phase in ("all", "fmt", "clippy", "test"):
            with self.subTest(phase=phase):
                result, calls = self.run_runner(phase)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual([call["args"][0] for call in calls], ["image", "run"])
                args = calls[-1]["args"]
                self.assertIn("sha256:" + "a" * 64, args)
                self.assertEqual(args[-1], phase)
                self.assertIn("--userns=keep-id", args)
                self.assertIn("CARGO_TARGET_DIR=/workspace/target/quality-container/build", args)
                self.assertNotIn("/workspace/target/e2e-container/build", " ".join(args))
                for call in calls:
                    self.assertIsNone(call["display"])
                    self.assertIsNone(call["wayland"])
                    self.assertIsNone(call["bus"])

    def test_shard_handoff_is_forwarded_without_changing_public_phases(self):
        result, calls = self.run_runner("test", extra_env={
            "STRATA_QUALITY_TASK": "shard", "STRATA_QUALITY_SHARD": "0"})
        self.assertEqual(result.returncode, 0, result.stderr)
        args = calls[-1]["args"]
        self.assertIn("STRATA_QUALITY_TASK", args)
        self.assertIn("STRATA_QUALITY_SHARD", args)
        self.assertTrue(any(arg.startswith("STRATA_QUALITY_COMMIT=") for arg in args))
        self.assertEqual(args[-1], "test")

    def test_workflow_matrix_and_required_gate_match_the_plan(self):
        from quality_ci import SHARDS
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        shard = workflow.split("\n  quality-shard:", 1)[1].split("\n  quality:", 1)[0]
        self.assertIn(f"shard: {list(range(SHARDS))}", shard)
        self.assertIn("fail-fast: false", shard)
        self.assertIn("STRATA_QUALITY_TASK: shard", shard)
        self.assertNotIn("cargo test", shard)
        gate = workflow.split("\n  quality:\n", 1)[1].split("\n  e2e-build:", 1)[0]
        self.assertIn("name: Format, lint, and test", gate)
        self.assertIn("needs: [quality-build, quality-shard]", gate)
        self.assertIn("always()", gate)
        self.assertIn('test "$BUILD_RESULT" = success && test "$SHARD_RESULT" = success', gate)
        self.assertIn("python3 scripts/quality_ci.py verify", gate)

    def test_invalid_provenance_never_executes_the_image(self):
        result, calls = self.run_runner(key="wrong")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)

    def test_invalid_phase_is_rejected_before_engine_access(self):
        result, calls = self.run_runner("skip-tests")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])

    def test_docker_does_not_receive_podman_only_options(self):
        result, calls = self.run_runner(engine_name="docker")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--userns=keep-id", calls[-1]["args"])

    def test_quality_checkout_retains_base_commits_beyond_the_merge_parents(self):
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        build = workflow.split("\n  quality-build:", 1)[1].split("\n  quality-shard:", 1)[0]
        checkout = build.split("uses: actions/checkout@", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("fetch-depth: 0", checkout)

    def test_ci_builds_only_deliberate_unpublished_recipe_changes(self):
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        step = workflow.split("- name: Resolve the shared environment or build an explicit recipe update", 1)[1]
        script = textwrap.dedent(step.split("run: |\n", 1)[1].split("\n      - name:", 1)[0])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = {
                "python3": 'case "$*" in *"e2e_images.py environment"*) [ "$PUBLISHED" = 1 ] || exit 1; echo environment_image=ghcr.io/example/build@sha256:abc;; *"e2e_base.py build"*) echo build >> "$CALL_LOG"; echo sha256:abc;; *) echo inputs;; esac\n',
                "docker": 'echo "$*" >> "$CALL_LOG"\n',
                "git": 'case "$1" in cat-file) exit "$BASE_STATUS";; diff) exit "$DIFF_STATUS";; esac\n',
            }
            for name, content in tools.items():
                tool = root / name
                tool.write_text("#!/bin/sh\n" + content)
                tool.chmod(0o755)
            for published, diff, base, expected in ((1, 0, 0, "pull"), (0, 1, 0, "build"),
                                                     (0, 0, 0, "failure"), (0, 1, 1, "failure"),
                                                     (0, 2, 0, "failure")):
                with self.subTest(published=published, diff=diff, base=base):
                    log = root / "calls"
                    log.write_text("")
                    result = subprocess.run(["bash", "-eu", "-c", script], capture_output=True, text=True,
                        env={**os.environ, "PATH": f"{root}:{os.defpath}", "PUBLISHED": str(published),
                             "DIFF_STATUS": str(diff), "BASE_STATUS": str(base), "BASE_REF": "HEAD^",
                             "GITHUB_OUTPUT": str(root / "outputs"), "CALL_LOG": str(log)})
                    if expected == "failure":
                        self.assertNotEqual(result.returncode, 0)
                        self.assertEqual(log.read_text(), "")
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertTrue(log.read_text().startswith(expected))

    def test_real_inner_shell_propagates_failures_and_requires_private_gtk(self):
        _, calls = self.run_runner()
        args = calls[-1]["args"]
        command = args[args.index("bash"):]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, content in {
                "rustc": "exit 0\n",
                "cargo": 'echo "$*" >> "$CALL_LOG"\ncase "$1" in "$FAIL_PHASE") exit 42;; esac\n',
                "xvfb-run": 'echo "xvfb $*" >> "$CALL_LOG"\nshift\nexec "$@"\n',
                "dbus-run-session": 'echo "private-dbus $*" >> "$CALL_LOG"\nshift\nexec "$@"\n',
            }.items():
                tool = root / name
                tool.write_text("#!/bin/sh\n" + content)
                tool.chmod(0o755)
            for failure in ("fmt", "clippy", "test", "none"):
                log = root / f"{failure}.log"
                result = subprocess.run(command, env={**os.environ, "PATH": f"{root}:{os.defpath}",
                    "HOME": str(root), "CARGO_HOME": str(root / "cargo-home"),
                    "CALL_LOG": str(log), "FAIL_PHASE": failure}, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0 if failure == "none" else 42, result.stderr)
                text = log.read_text()
                if failure == "fmt":
                    self.assertNotIn("clippy", text)
                if failure in ("fmt", "clippy"):
                    self.assertNotIn("xvfb", text)
                else:
                    self.assertIn("xvfb -a dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11", text)
                    self.assertIn("GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1", text)
                    self.assertIn("test --locked --all-targets --all-features", text)


if __name__ == "__main__":
    unittest.main()
