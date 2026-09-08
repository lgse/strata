import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from e2e_bundle import image_key


REPOSITORY = Path(__file__).resolve().parents[1]


class ContainerRunnerTests(unittest.TestCase):
    def run_runner(self, engine_name="docker", binary=None, uid=None, workers="auto", extra_env=None):
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
                "'wayland': os.environ.get('WAYLAND_DISPLAY'), "
                "'notify': os.environ.get('NOTIFY_SOCKET')}) + '\\n')\n"
                "if sys.argv[1:3] == ['image', 'inspect']:\n"
                "    if '--format' in sys.argv: print(os.environ.get('MOCK_IMAGE_KEY', ''))\n"
                "    else: print(json.dumps([{'Id':'sha256:'+'a'*64, 'Os':'linux', 'Architecture':'amd64',\n"
                "         'Config':{'Labels':{'org.strata.e2e.inputs':os.environ['MOCK_IMAGE_KEY']},\n"
                "                   'Env':['RUSTUP_HOME=/opt/rustup']}}]))\n"
            )
            engine.chmod(0o755)
            environment = {
                **os.environ,
                "STRATA_CONTAINER_ENGINE": str(engine),
                "ENGINE_LOG": str(log),
                "MOCK_IMAGE_KEY": image_key(),
                "DISPLAY": ":0",
                "WAYLAND_DISPLAY": "wayland-0",
                "NOTIFY_SOCKET": "/run/user/1000/systemd/notify",
                "STRATA_E2E_UPDATE_BASELINES": "1",
                "STRATA_E2E_WORKERS": workers,
            }
            if uid is not None:
                identity = root / "id"
                identity.write_text(f"#!/bin/sh\necho {uid}\n")
                identity.chmod(0o755)
                environment["PATH"] = f"{root}:{os.environ.get('PATH', os.defpath)}"
            for name in ("STRATA_BINARY", "STRATA_E2E_IMAGE", "STRATA_E2E_BUNDLE"):
                environment.pop(name, None)
            if binary:
                environment["STRATA_BINARY"] = binary
            environment.update(extra_env or {})
            result = subprocess.run(
                [str(REPOSITORY / "scripts/e2e.sh"), "-k", "columns and baseline"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, calls

    def test_cached_base_is_verified_and_run_without_building_or_pulling(self):
        result, calls = self.run_runner()
        self.assertEqual(result.returncode, 0, result.stderr)
        inspect, run = [call["args"] for call in calls]
        self.assertEqual(inspect[:2], ["image", "inspect"])
        self.assertIn("sha256:" + "a" * 64, run)
        self.assertFalse(any(call["args"][0] in ("build", "pull") for call in calls))
        self.assertEqual(run[-2:], ["-k", "columns and baseline"])
        self.assertIn(f"type=bind,source={REPOSITORY},target=/workspace", run)
        self.assertIn("STRATA_E2E_UPDATE_BASELINES=1", run)
        self.assertIn("CARGO_TARGET_DIR=/workspace/target/e2e-container/build", run)
        self.assertIn("cargo build --locked --bin strata", " ".join(run))
        self.assertNotIn("--userns=keep-id", run)
        for call in calls:
            self.assertIsNone(call["display"])
            self.assertIsNone(call["wayland"])
            self.assertIsNone(call["notify"])

    def test_ci_explicitly_runs_the_engine_that_loaded_its_runtime(self):
        workflow = (REPOSITORY / ".github/workflows/ci.yml").read_text()
        loader = re.search(r"zstd -dc target/e2e-runtime/runtime.tar.zst \| (\w+) load", workflow)
        step = workflow.split("- name: Run the assigned scenarios without rebuilding or installing", 1)[1]
        step = step.split("- name:", 1)[0]
        runner = re.search(r"STRATA_CONTAINER_ENGINE: (\w+)", step)
        self.assertIsNotNone(loader)
        self.assertIsNotNone(runner)
        self.assertEqual(loader.group(1), runner.group(1))

    def test_worker_budget_is_forwarded_into_container(self):
        for workers in ("auto", "1", "8"):
            with self.subTest(workers=workers):
                result, calls = self.run_runner(workers=workers)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"STRATA_E2E_WORKERS={workers}", calls[1]["args"])

    def test_rootless_podman_preserves_checkout_ownership(self):
        result, calls = self.run_runner("podman")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--userns=keep-id", calls[1]["args"])
        self.assertIn("--passwd=false", calls[1]["args"])

    def test_non_default_uid_has_a_matching_image_account(self):
        result, calls = self.run_runner(uid=1001)
        self.assertEqual(result.returncode, 0, result.stderr)
        _, run = [call["args"] for call in calls]
        self.assertEqual(run[run.index("--user") + 1], "1001:1001")
        account = REPOSITORY / "target/e2e-container/accounts-1001-1001/passwd"
        self.assertIn("strata-e2e:x:1001:1001:", account.read_text())
        self.assertIn(f"type=bind,source={account},target=/etc/passwd,readonly", run)

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

    def test_preloaded_image_skips_the_container_build(self):
        result, calls = self.run_runner(extra_env={"STRATA_E2E_IMAGE": "pinned-runtime"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0]["args"][:2], ["image", "inspect"])
        self.assertEqual(calls[1]["args"][0], "run")
        self.assertIn("sha256:" + "a" * 64, calls[1]["args"])

    def test_ci_bundle_is_verified_before_the_engine_can_execute_it(self):
        from e2e_bundle import create, image_key

        with tempfile.TemporaryDirectory(dir=REPOSITORY) as directory:
            bundle = Path(directory)
            (bundle / "strata").write_bytes(b"container binary")
            (bundle / "plan.json").write_text("{}")
            commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY,
                                             text=True).strip()
            create(bundle, commit)
            env = {"STRATA_E2E_BUNDLE": str(bundle), "STRATA_E2E_IMAGE": "runtime",
                   "MOCK_IMAGE_KEY": image_key()}
            result, calls = self.run_runner(extra_env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[0]["args"][:2], ["image", "inspect"])
            self.assertIn(f"STRATA_E2E_BUNDLE=/workspace/{bundle.name}", calls[1]["args"])
            result, calls = self.run_runner(extra_env={**env, "MOCK_IMAGE_KEY": "wrong-toolkit"})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("pinned E2E inputs", result.stderr)
            self.assertEqual(len(calls), 1)
            (bundle / "strata").write_bytes(b"stale binary")
            result, calls = self.run_runner(extra_env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum mismatch", result.stderr)
            self.assertEqual(calls, [])

    def test_bundle_requires_an_image_and_cannot_escape_the_checkout(self):
        for env, message in [({"STRATA_E2E_BUNDLE": "/tmp"}, "runtime image"),
                             ({"STRATA_E2E_BUNDLE": "/tmp", "STRATA_E2E_IMAGE": "runtime"},
                              "inside the checkout")]:
            result, calls = self.run_runner(extra_env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)
            self.assertEqual(calls, [])

    def test_host_binary_is_rejected_before_build(self):
        result, calls = self.run_runner(binary="/tmp/host-strata")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("e2e-native.sh", result.stderr)
        self.assertEqual(calls, [])


class NativeRunnerTests(unittest.TestCase):
    def run_runner(self, *, current_requirements=True, binary=None, arguments=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            scripts.mkdir()
            runner = scripts / "e2e-native.sh"
            runner.write_text((REPOSITORY / "scripts/e2e-native.sh").read_text())
            suite = root / "tests/e2e"
            suite.mkdir(parents=True)
            (suite / "requirements.txt").write_text("pytest-xdist==3.8.0\n")
            venv = root / "venv"
            tools = venv / "bin"
            tools.mkdir(parents=True)
            (root / "target/debug").mkdir(parents=True)
            if current_requirements:
                (venv / "strata-requirements.txt").write_text((suite / "requirements.txt").read_text())
            for name in ("Xvfb", "dbus-daemon", "dbus-send", "import", "python3"):
                tool = tools / name
                tool.write_text("#!/bin/sh\nexit 0\n")
                tool.chmod(0o755)
            log = root / "calls.jsonl"
            for name in ("python", "pip", "cargo"):
                tool = tools / name
                tool.write_text(
                    f"#!{sys.executable}\nimport json, os, sys\n"
                    f"with open({str(log)!r}, 'a') as stream:\n"
                    "    stream.write(json.dumps({'tool': os.path.basename(sys.argv[0]), "
                    "'args': sys.argv[1:], 'binary': os.environ.get('STRATA_BINARY'), "
                    "'display': os.environ.get('DISPLAY'), 'wayland': os.environ.get('WAYLAND_DISPLAY')}) + '\\n')\n"
                )
                tool.chmod(0o755)
            environment = {**os.environ, "PATH": f"{tools}:{os.defpath}",
                           "STRATA_E2E_VENV": str(venv), "DISPLAY": ":0", "WAYLAND_DISPLAY": "wayland-0"}
            environment.pop("STRATA_BINARY", None)
            environment.pop("CARGO_TARGET_DIR", None)
            if binary:
                environment["STRATA_BINARY"] = binary
            result = subprocess.run(["bash", str(runner), *arguments], cwd=root,
                                    env=environment, capture_output=True, text=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            return result, calls, root

    def test_default_builds_once_before_parallel_pytest(self):
        result, calls, root = self.run_runner()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["tool"] for call in calls], ["cargo", "python"])
        invocation = calls[-1]
        self.assertEqual(invocation["binary"], str(root / "target/debug/strata"))
        self.assertEqual(invocation["args"][-4:], ["-n", "auto", "--dist=loadgroup", "--max-worker-restart=0"])
        self.assertIsNone(invocation["display"])
        self.assertIsNone(invocation["wayland"])

    def test_explicit_pytest_options_follow_defaults_and_binary_skips_build(self):
        result, calls, _ = self.run_runner(binary="/provided/strata", arguments=("-n", "0", "-k", "baseline"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["tool"] for call in calls], ["python"])
        self.assertEqual(calls[0]["args"][-4:], ["-n", "0", "-k", "baseline"])
        self.assertEqual(calls[0]["binary"], "/provided/strata")

    def test_existing_venv_is_updated_when_requirements_change(self):
        result, calls, _ = self.run_runner(current_requirements=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["tool"] for call in calls], ["pip", "cargo", "python"])


if __name__ == "__main__":
    unittest.main()
