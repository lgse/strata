import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPOSITORY = Path(__file__).resolve().parents[1]


class ContainerRunnerTests(unittest.TestCase):
    def run_runner(self, engine_name="docker", binary=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = root / engine_name
            log = root / "calls.jsonl"
            engine.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "with open(os.environ['ENGINE_LOG'], 'a') as stream:\n"
                "    stream.write(json.dumps({'args': sys.argv[1:], "
                "'display': os.environ.get('DISPLAY'), "
                "'wayland': os.environ.get('WAYLAND_DISPLAY')}) + '\\n')\n"
            )
            engine.chmod(0o755)
            environment = {
                **os.environ,
                "STRATA_CONTAINER_ENGINE": str(engine),
                "ENGINE_LOG": str(log),
                "DISPLAY": ":0",
                "WAYLAND_DISPLAY": "wayland-0",
                "STRATA_E2E_UPDATE_BASELINES": "1",
            }
            environment.pop("STRATA_BINARY", None)
            if binary:
                environment["STRATA_BINARY"] = binary
            result = subprocess.run(
                [str(REPOSITORY / "scripts/e2e.sh"), "-k", "columns and baseline"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, calls

    def test_build_and_run_share_image_and_forward_arguments(self):
        result, calls = self.run_runner()
        self.assertEqual(result.returncode, 0, result.stderr)
        build, run = [call["args"] for call in calls]
        self.assertEqual(build[0], "build")
        image = build[build.index("--tag") + 1]
        self.assertIn(image, run)
        self.assertEqual(run[-2:], ["-k", "columns and baseline"])
        self.assertIn(f"type=bind,source={REPOSITORY},target=/workspace", run)
        self.assertIn("STRATA_E2E_UPDATE_BASELINES=1", run)
        self.assertIn("CARGO_TARGET_DIR=/workspace/target/e2e-container/build", run)
        self.assertIn("cargo build --locked --bin strata", " ".join(run))
        self.assertNotIn("--userns=keep-id", run)
        for call in calls:
            self.assertIsNone(call["display"])
            self.assertIsNone(call["wayland"])

    def test_rootless_podman_preserves_checkout_ownership(self):
        result, calls = self.run_runner("podman")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--userns=keep-id", calls[1]["args"])

    def test_failed_container_build_cannot_run_stale_binary(self):
        _, calls = self.run_runner()
        arguments = calls[1]["args"]
        command = arguments[arguments.index("bash"):]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / "bin"
            tools.mkdir()
            for name, status in [("cargo", 42), ("pkg-config", 0), ("rustc", 0)]:
                tool = tools / name
                tool.write_text(f"#!/bin/sh\nexit {status}\n")
                tool.chmod(0o755)
            scripts = root / "scripts"
            scripts.mkdir()
            runner = scripts / "e2e-native.sh"
            runner.write_text("#!/bin/sh\ntouch stale-binary-ran\n")
            runner.chmod(0o755)
            result = subprocess.run(
                command,
                cwd=root,
                env={
                    **os.environ,
                    "PATH": f"{tools}:{os.defpath}",
                    "HOME": str(root / "home"),
                    "CARGO_HOME": str(root / "cargo"),
                    "CARGO_TARGET_DIR": str(root / "target"),
                },
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 42)
            self.assertFalse((root / "stale-binary-ran").exists())

    def test_host_binary_is_rejected_before_build(self):
        result, calls = self.run_runner(binary="/tmp/host-strata")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("e2e-native.sh", result.stderr)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
