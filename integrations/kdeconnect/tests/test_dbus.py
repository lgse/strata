# SPDX-License-Identifier: MIT
"""Real Gio calls and provider framing against a disposable D-Bus daemon."""

import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time
import unittest

SOURCE = Path(__file__).resolve().parents[1]


class PrivateBusTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="strata-kde-")
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        config = self.root / "bus.conf"
        config.write_text('''<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>
<policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>''')
        self.bus = self.spawn(["dbus-daemon", "--nofork", "--print-address=1", "--config-file=" + str(config)], stdout=subprocess.PIPE)
        self.assertTrue(select.select([self.bus.stdout], [], [], 5)[0], "private bus startup")
        address = self.bus.stdout.readline().decode().strip()
        self.env = {"PATH": os.defpath, "LANG": "C.UTF-8", "DBUS_SESSION_BUS_ADDRESS": address}
        self.control = self.root / "control.json"
        self.record = self.root / "record.jsonl"
        self.ready = self.root / "ready"
        self.devices = {}
        self.update()
        self.start_service()
        self.provider = self.spawn([sys.executable, str(SOURCE / "provider.py")], env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.paths = [self.root / 'report 冬 $HOME;`x`\n.txt', self.root / 'a %23?.pdf']
        for path in self.paths:
            path.write_text("disposable fixture")
        self.sequence = 0
        self.assertEqual(self.request()["actions"], [])

    def spawn(self, command, **kwargs):
        process = subprocess.Popen(command, bufsize=0, **kwargs)
        def stop():
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream:
                    stream.close()
        self.addCleanup(stop)
        return process

    def start_service(self):
        self.ready.unlink(missing_ok=True)
        self.service = self.spawn([sys.executable, str(SOURCE / "tests/mock_service.py"),
                                   str(self.control), str(self.record), str(self.ready)], env=self.env)
        deadline = time.monotonic() + 5
        while not self.ready.exists() and time.monotonic() < deadline:
            self.assertIsNone(self.service.poll(), "mock service exited")
            time.sleep(.02)
        self.assertTrue(self.ready.exists(), "mock service name acquisition")

    def update(self, **extra):
        temporary = self.control.with_suffix(".tmp")
        temporary.write_text(json.dumps({"devices": self.devices, **extra}))
        temporary.replace(self.control)

    def read(self):
        self.assertTrue(select.select([self.provider.stdout], [], [], 7)[0], "provider response")
        return json.loads(self.provider.stdout.readline())

    def request(self, method="menu", leaf=None):
        self.sequence += 1
        request = {"version": 1, "id": self.sequence, "method": method,
                   "paths": [str(p) for p in self.paths], "background": False}
        if leaf:
            request.update(action=leaf["id"], context=leaf["context"])
        self.provider.stdin.write((json.dumps(request) + "\n").encode())
        self.provider.stdin.flush()
        while True:
            reply = self.read()
            if reply.get("id") == self.sequence:
                return reply

    def await_invalidation(self):
        while self.read().get("event") != "invalidate":
            pass

    def submissions(self):
        return [json.loads(line) for line in self.record.read_text().splitlines()] if self.record.exists() else []

    def test_live_zero_one_multiple_and_disconnect_preserve_exact_routing(self):
        self.assertEqual(self.request()["actions"], [])
        self.devices = {"phone1": {"name": "Pixel"}, "unpaired": {"name": "Ignore", "paired": False},
                        "offline": {"name": "Ignore", "reachable": False}, "disabled": {"name": "Ignore", "share": False}}
        self.update()
        self.await_invalidation()
        leaf = self.request()["actions"][0]
        self.assertEqual((leaf["label"], leaf["placement"]), ("Pixel", "send-to"))
        self.assertEqual(self.request("activate", leaf)["outcome"]["status"], "accepted")
        self.assertEqual(self.submissions(), [{"device": "phone1", "uris": [p.as_uri() for p in self.paths],
                                               "owner": self.ready.read_text()}])
        self.devices["tablet2"] = {"name": "Tablet"}
        self.update()
        self.await_invalidation()
        leaves = self.request()["actions"]
        self.assertEqual([leaf["label"] for leaf in leaves], ["Pixel", "Tablet"])
        self.assertEqual(self.request("activate", leaves[1])["outcome"]["status"], "accepted")
        self.assertEqual(self.submissions()[-1]["device"], "tablet2")
        self.devices["tablet2"]["reachable"] = False
        self.update()
        self.await_invalidation()
        self.assertEqual(self.request("activate", leaves[1])["outcome"]["status"], "rejected")
        self.assertEqual(len(self.submissions()), 2)
        self.assertEqual([leaf["label"] for leaf in self.request()["actions"]], ["Pixel"])

    def test_service_restart_never_reuses_old_target_or_activation(self):
        self.devices = {"phone1": {"name": "Pixel"}}
        self.update()
        self.await_invalidation()
        leaf = self.request()["actions"][0]
        old_owner = self.ready.read_text()
        self.service.terminate()
        self.service.wait(timeout=5)
        self.start_service()
        self.assertNotEqual(self.ready.read_text(), old_owner)
        self.assertEqual(self.request("activate", leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.submissions(), [])

    def test_remote_uncertain_reply_is_not_retried(self):
        self.devices = {"phone1": {"name": "Pixel"}}
        self.update(error="org.freedesktop.DBus.Error.Failed")
        self.await_invalidation()
        leaf = self.request()["actions"][0]
        reply = self.request("activate", leaf)
        self.assertEqual(reply["outcome"]["status"], "unknown")
        self.assertNotIn("feedback", reply)
        self.assertEqual(self.request("activate", leaf)["outcome"]["status"], "rejected")
        self.assertEqual(len(self.submissions()), 1)


if __name__ == "__main__":
    unittest.main()
