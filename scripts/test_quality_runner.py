# SPDX-License-Identifier: MIT

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

from image_inputs import REPOSITORY, image_key


class QualityRunnerTests(unittest.TestCase):
    def run_runner(self, phase="all", key=None, engine_name="podman", extra_env=None, options=()):
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
                f"'Config': {{'Labels': {{'org.strata.test.inputs': {(image_key() if key is None else key)!r}}}, "
                "'Env': ['RUSTUP_HOME=/opt/rustup']}}]))\n"
            )
            engine.chmod(0o755)
            result = subprocess.run([str(REPOSITORY / "scripts/quality.sh"), phase, *options],
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
                self.assertIn("--pids-limit=-1", args)
                self.assertIn("CARGO_TARGET_DIR=/workspace/target/quality-container/build", args)
                self.assertNotIn("/workspace/target/qa-container/build", " ".join(args))
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
        tmp = args[args.index("--tmpfs") + 1].split(":", 1)
        self.assertEqual(tmp[0], "/tmp")
        self.assertIn("exec", tmp[1].split(","))
        self.assertEqual(args[-1], "test")

    def test_workflow_matrix_and_required_gate_match_the_plan(self):
        from quality_ci import SHARDS
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        shard = workflow.split("\n  quality-shard:", 1)[1].split("\n  quality:", 1)[0]
        self.assertIn(f"shard: {list(range(SHARDS))}", shard)
        self.assertIn("fail-fast: false", shard)
        self.assertIn("STRATA_QUALITY_TASK: shard", shard)
        self.assertIn("run: ./scripts/quality.sh test --jobs 3", shard)
        self.assertNotIn("cargo test", shard)
        gate = workflow.split("\n  quality:\n", 1)[1].split("\n  release-scripts:", 1)[0]
        self.assertIn("name: Format, lint, and test", gate)
        self.assertIn("needs: [quality-build, quality-shard]", gate)
        self.assertIn("always()", gate)
        self.assertIn('test "$BUILD_RESULT" = success && test "$SHARD_RESULT" = success', gate)
        self.assertIn("python3 scripts/quality_ci.py verify", gate)

    def test_post_merge_skips_test_execution_but_keeps_trusted_cache_build(self):
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("\njobs:\n", 1)[1], re.M | re.S))
        for name in ("quality-shard", "quality", "release-scripts"):
            with self.subTest(job=name):
                condition = re.search(r"^    if: (.+)$", jobs[name], re.M).group(1)
                self.assertIn("github.event_name != 'push'", condition)
        build = jobs["quality-build"]
        condition = re.search(r"^    if: (.+)$", build, re.M).group(1)
        self.assertNotIn("github.event_name != 'push'", condition)
        self.assertIn("STRATA_QUALITY_TASK: build", build)
        self.assertIn("if: github.event_name == 'push' && github.ref == 'refs/heads/main'", build)
        packaging = (REPOSITORY / ".github/workflows/packaging.yml").read_text()
        renderer = packaging.split("- name: Test the package renderer\n", 1)[1].split("\n      - name:", 1)[0]
        self.assertIn("if: ${{ github.event_name != 'push' }}", renderer)

    def test_invalid_provenance_never_executes_the_image(self):
        result, calls = self.run_runner(key="wrong")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)

    def test_test_options_reach_the_container_and_bad_ones_never_start_it(self):
        for phase, options in (("test", ("--jobs", "3")), ("all", ("--refresh-timings", "--jobs", "12"))):
            with self.subTest(phase=phase, options=options):
                result, calls = self.run_runner(phase, options=options)
                self.assertEqual(result.returncode, 0, result.stderr)
                forwarded = [arg.replace("--jobs", "--lanes") for arg in options]
                self.assertEqual(calls[-1]["args"][-1 - len(options):], [phase, *forwarded])
        result, calls = self.run_runner("test", options=("--jobs", "3"), extra_env={
            "STRATA_QUALITY_TASK": "shard", "STRATA_QUALITY_SHARD": "0"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1]["args"][-3:], ["test", "--lanes", "3"])
        for phase, options, task in (("test", ("--jobs",), "test"), ("test", ("--jobs", "0"), "test"),
                                     ("test", ("--jobs", "two"), "test"), ("fmt", ("--jobs", "2"), "test"),
                                     ("clippy", ("--refresh-timings",), "test"), ("test", ("--filter", "ui::"), "test"),
                                     ("test", ("--refresh-timings",), "shard"), ("test", ("--jobs", "3"), "build")):
            with self.subTest(phase=phase, options=options, task=task):
                result, calls = self.run_runner(phase, options=options, extra_env={"STRATA_QUALITY_TASK": task})
                self.assertEqual(result.returncode, 2)
                self.assertEqual(calls, [])

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
                "python3": 'case "$*" in *"image_registry.py environment"*) [ "$PUBLISHED" = 1 ] || exit 1; echo environment_image=ghcr.io/example/build@sha256:abc;; *"image_base.py build"*) echo build >> "$CALL_LOG"; echo sha256:abc;; *) echo inputs;; esac\n',
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

    def test_real_inner_shell_propagates_failures_and_forwards_test_options(self):
        _, calls = self.run_runner(options=("--jobs", "5"))
        args = calls[-1]["args"]
        command = args[args.index("bash"):]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, content in {
                "rustc": "exit 0\n",
                "cargo": 'echo "$*" >> "$CALL_LOG"\ncase "$1" in "$FAIL_PHASE") exit 42;; esac\n',
                "xvfb-run": 'echo "xvfb $*" >> "$CALL_LOG"\nwhile [ "$#" -gt 0 ]; do\ncase "$1" in -a) shift;; -s) shift 2;; *) break;; esac\ndone\nexec "$@"\n',
                "dbus-run-session": 'echo "private-dbus $*" >> "$CALL_LOG"\nshift\nexec "$@"\n',
                "python3": 'echo "python3 $*" >> "$CALL_LOG"\ncase "$FAIL_PHASE" in test) exit 42;; esac\n',
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
                    self.assertNotIn("python3", text)
                else:
                    # Local shards start their own private displays; see test_quality_ci.
                    self.assertIn("python3 scripts/quality_ci.py local --lanes 5", text)
                    self.assertNotIn("xvfb", text)
                    self.assertNotIn("test --locked", text)


if __name__ == "__main__":
    unittest.main()
