# SPDX-License-Identifier: MIT
import json
import os
from pathlib import Path
import selectors
from string import Formatter
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from provider import ACTION, MESSAGES, LocalSend, Provider, Unavailable, resolve, tr

PROVIDER = Path(__file__).resolve().parents[1]


class FakeLauncher:
    def __init__(self):
        self.path = "/usr/bin/localsend"
        self.opened = []
        self.failure = None

    def executable(self):
        return self.path

    def open(self, executable, paths):
        if self.failure:
            raise self.failure
        self.opened.append((executable, paths))

    def reap(self):
        pass


class ProviderTests(unittest.TestCase):
    def setUp(self):
        language = patch.dict(os.environ, {"LC_ALL": "C.UTF-8"})
        language.start()
        self.addCleanup(language.stop)
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.file = self.root / 'report "冬" $HOME;`x`\n.txt'
        self.file.write_text("test document")
        self.folder = self.root / "-photos"
        self.folder.mkdir()
        self.launcher = FakeLauncher()
        self.now = 10.0
        self.provider = Provider(self.launcher, clock=lambda: self.now)
        self.sequence = 0

    def request(self, method="menu", paths=None, **extra):
        self.sequence += 1
        return {"version": 1, "id": self.sequence, "method": method,
                "paths": paths if paths is not None else [str(self.file)],
                "background": False, **extra}

    def menu(self, paths=None):
        return self.provider.handle(self.request(paths=paths))["actions"]

    def activate(self, leaf, paths=None, **extra):
        request = self.request("activate", paths, action=leaf["id"], context=leaf["context"])
        return self.provider.handle({**request, **extra})

    def test_selection_opens_once_in_localsend_with_literal_paths(self):
        paths = [str(self.file), str(self.folder)]
        actions = self.menu(paths)
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["label"], "LocalSend")
        self.assertEqual(actions[0]["placement"], "send-to-entry")
        reply = self.activate(actions[0], paths)
        self.assertEqual(reply["outcome"], {"status": "accepted", "accepted": 2, "total": 2})
        self.assertEqual(reply["feedback"], "none")
        self.assertEqual(self.launcher.opened, [("/usr/bin/localsend", paths)])

    def test_missing_app_hides_action_and_rejects_open_menu(self):
        leaf = self.menu()[0]
        self.launcher.path = None
        self.assertEqual(self.menu(), [])
        reply = self.activate(leaf)
        self.assertEqual(reply["outcome"]["status"], "rejected")
        self.assertEqual(reply["message"], "LocalSend is not installed in a system directory.")
        self.assertEqual(self.launcher.opened, [])

    def test_symlinks_missing_paths_and_background_hide_action(self):
        link = self.root / "link"
        link.symlink_to(self.file)
        for paths in ([str(link)], [str(self.file), str(link)], [str(self.root / "gone")]):
            with self.subTest(paths=paths):
                self.assertEqual(self.menu(paths), [])
        reply = self.provider.handle(self.request(background=True))
        self.assertEqual(reply["actions"], [])

    def test_replaced_edited_removed_and_symlinked_items_are_rejected(self):
        for operation in ("replace", "edit", "remove", "symlink"):
            with self.subTest(operation=operation):
                self.file.unlink(missing_ok=True)
                self.file.write_text("original")
                leaf = self.menu()[0]
                if operation == "edit":
                    self.file.write_text("edited document")
                else:
                    self.file.unlink()
                if operation == "replace":
                    self.file.write_text("replacement")
                elif operation == "symlink":
                    self.file.symlink_to("/etc/hosts")
                self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.launcher.opened, [])

    def test_selection_change_rejects_entire_batch(self):
        leaf = self.menu()[0]
        self.assertEqual(self.activate(leaf, [str(self.folder)])["outcome"]["status"], "rejected")
        self.assertEqual(self.launcher.opened, [])

    def test_menu_refresh_preserves_ticket_until_consumed(self):
        leaf = self.menu()[0]
        self.assertEqual(self.menu()[0]["context"], leaf["context"])
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "accepted")
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        fresh = self.menu()[0]
        self.assertNotEqual(fresh["context"], leaf["context"])
        self.assertEqual(self.activate(fresh)["outcome"]["status"], "accepted")
        self.assertEqual(len(self.launcher.opened), 2)

    def test_expired_forged_and_misdirected_tickets_cannot_open(self):
        leaf = self.menu()[0]
        self.assertEqual(self.activate({**leaf, "context": "forged"})["outcome"]["status"], "rejected")
        self.assertEqual(self.activate({**leaf, "id": "other"})["outcome"]["status"], "rejected")
        self.assertEqual(self.activate(leaf, background=True)["outcome"]["status"], "rejected")
        self.now += 121
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.launcher.opened, [])

    def test_launch_failure_is_rejected_and_consumes_the_ticket(self):
        leaf = self.menu()[0]
        self.launcher.failure = Unavailable("LocalSend could not be started.")
        reply = self.activate(leaf)
        self.assertEqual(reply["outcome"], {"status": "rejected", "accepted": 0, "total": 1})
        self.assertEqual(reply["message"], "LocalSend could not be started.")
        self.assertNotIn("feedback", reply)
        self.launcher.failure = None
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")

    def test_unreadable_item_is_rejected(self):
        leaf = self.menu()[0]
        with patch("provider.os.access", return_value=False):
            reply = self.activate(leaf)
        self.assertEqual(reply["message"], "One or more selected items cannot be read.")
        self.assertEqual(self.launcher.opened, [])

    def test_localized_messages(self):
        message = "LocalSend could not be started."
        for locale, expected in (
            ("fr_FR.UTF-8", "Impossible de démarrer LocalSend."),
            ("pt.UTF-8", "Não foi possível iniciar o LocalSend."),
            ("zh_HK.UTF-8", "無法啟動 LocalSend。"),
            ("unknown", message),
        ):
            with self.subTest(locale=locale), patch.dict(os.environ, {"LC_ALL": locale}):
                self.assertEqual(tr(message), expected)

    def test_translations_keep_complete_messages_and_named_placeholders(self):
        formatter = Formatter()
        fields = lambda text: {key for _, key, _, _ in formatter.parse(text) if key}
        for language, messages in MESSAGES.items():
            self.assertEqual(messages.keys(), MESSAGES["en"].keys())
            for original, translated in messages.items():
                with self.subTest(language=language, message=original):
                    self.assertEqual(fields(translated), fields(original))
                    self.assertTrue(translated.strip())
                    translated.format(count=2)

    def test_invalid_requests_never_open(self):
        for changes in ({"paths": []}, {"paths": ["relative"]}, {"paths": ["/bad\0path"]},
                        {"paths": ["/bad\udcff"]}, {"paths": [str(self.file)] * 201},
                        {"version": 2}, {"background": "false"}, {"id": -1}):
            with self.subTest(changes=changes):
                reply = self.provider.handle({**self.request(), **changes})
                self.assertEqual(reply["error"], "invalid-request")
        reply = self.provider.handle(self.request("rename"))
        self.assertEqual(reply["error"], "unsupported-method")
        self.assertEqual(self.launcher.opened, [])


def write_program(path, body):
    path.write_text(f"#!{sys.executable}\nimport json, os, sys, time\n{body}\n")
    path.chmod(0o755)


class LaunchTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.program = self.root / "localsend"
        self.record = self.root / "record.json"

    def test_handover_runs_in_its_own_session_with_exact_arguments(self):
        write_program(self.program, f"json.dump({{'argv': sys.argv[1:], 'sid': os.getsid(0)}}, open({str(self.record)!r}, 'w'))")
        paths = ["/tmp/a b\n.txt", "/tmp/--hidden"]
        LocalSend(lambda: str(self.program)).open(str(self.program), paths)
        record = json.loads(self.record.read_text())
        self.assertEqual(record["argv"], paths)
        self.assertNotEqual(record["sid"], os.getsid(0))

    def test_first_instance_keeps_running_and_is_reaped_later(self):
        write_program(self.program, "time.sleep(float(sys.argv[1]))")
        launcher = LocalSend(lambda: str(self.program), startup=0.1)
        launcher.open(str(self.program), ["0.4"])
        self.assertEqual(len(launcher.children), 1)
        self.addCleanup(lambda: [c.kill() for c in launcher.children])
        launcher.children[0].wait(5)
        launcher.reap()
        self.assertEqual(launcher.children, [])

    def test_failed_start_is_unavailable(self):
        write_program(self.program, "sys.exit(3)")
        with self.assertRaises(Unavailable):
            LocalSend(lambda: str(self.program)).open(str(self.program), ["/tmp/x"])
        with self.assertRaises(Unavailable):
            LocalSend(lambda: None).open(str(self.root / "missing"), ["/tmp/x"])


class ResolveTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.bin = self.root / "usr/bin"
        self.package = self.root / "opt/localsend"
        for directory in (self.bin, self.package):
            directory.mkdir(parents=True)
        for directory in (self.root / "usr", self.bin, self.root / "opt", self.package):
            directory.chmod(0o755)
        self.target = self.package / "localsend_app"
        write_program(self.target, "")
        (self.bin / "localsend").symlink_to("../../opt/localsend/localsend_app")
        self.uid = os.stat(self.root).st_uid

    def find(self, names=("localsend", "localsend_app"), trust=(), owner=None):
        owner = self.uid if owner is None else owner
        return resolve(names, [str(self.bin)], [str(t) for t in trust], str(self.root),
                       lambda s: s.st_uid == owner)

    def test_target_in_a_trust_root_runs_through_its_link(self):
        self.assertEqual(self.find(trust=[self.package]), str(self.bin / "localsend"))

    def test_owned_unwritable_target_runs_directly(self):
        self.assertEqual(self.find(), str(self.target))

    def test_foreign_owner_or_writable_directory_is_refused(self):
        self.assertIsNone(self.find(owner=self.uid + 1))
        self.package.chmod(0o775)
        self.assertIsNone(self.find())
        self.package.chmod(0o755)
        self.target.chmod(0o757)
        self.assertIsNone(self.find())

    def test_alternative_name_is_found_when_the_first_is_missing(self):
        (self.bin / "localsend").unlink()
        (self.bin / "localsend_app").symlink_to(self.target)
        self.assertEqual(self.find(), str(self.target))
        self.assertIsNone(self.find(names=("localsend",)))


class WireTests(unittest.TestCase):
    def test_fragmented_and_batched_requests_flush_and_exit_on_eof(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "file.txt"
            path.write_text("fixture")
            program = Path(scratch) / "localsend"
            write_program(program, "print('noise'); print('noise', file=sys.stderr)")
            process = subprocess.Popen(
                [sys.executable, "-c",
                 "import sys; from provider import LocalSend, main; "
                 f"main(LocalSend(lambda: {str(program)!r}))"],
                cwd=PROVIDER, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            self.addCleanup(lambda: process.poll() is None and process.kill())
            request = {"version": 1, "id": 1, "method": "menu", "paths": [str(path)], "background": False}
            wire = (json.dumps(request) + "\n").encode()
            process.stdin.write(wire[:20])
            process.stdin.flush()
            process.stdin.write(wire[20:])
            process.stdin.flush()
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(4), "Provider failed to flush the menu reply")
            leaf = json.loads(process.stdout.readline())["actions"][0]
            self.assertEqual(leaf["id"], ACTION)
            activate = {**request, "id": 2, "method": "activate", "action": leaf["id"], "context": leaf["context"]}
            query = {**request, "id": 3, "method": "query"}
            process.stdin.write((json.dumps(activate) + "\n" + json.dumps(query) + "\n").encode())
            process.stdin.flush()
            process.stdin.close()
            process.stdin = None
            remaining, errors = process.communicate(timeout=5)
            replies = [json.loads(line) for line in remaining.splitlines()]
            self.assertEqual([r["id"] for r in replies], [2, 3])
            self.assertEqual(replies[0]["outcome"]["status"], "accepted")
            self.assertEqual(replies[1]["decorations"], [])
            self.assertEqual(process.returncode, 0, errors.decode())
            self.assertEqual(errors, b"")

    def test_oversized_frame_ends_the_session(self):
        result = subprocess.run(
            [sys.executable, str(PROVIDER / "provider.py")],
            input=b"x" * (1024 * 1024 + 1), capture_output=True, timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
