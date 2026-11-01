# SPDX-License-Identifier: MIT
import json
import os
from pathlib import Path
import selectors
from string import Formatter
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from gi.repository import Gio, GLib

from provider import KDEConnect, MESSAGES, Provider, Snapshot, Unavailable, Uncertain, tr

PROVIDER = Path(__file__).resolve().parents[1]


class FakeKDE:
    def __init__(self, devices=None):
        self.devices = devices if devices is not None else {"phone1": "Pixel"}
        self.owner = ":1.20"
        self.submissions = []
        self.failure = None
        self.unavailable = False

    def discover(self, deadline):
        if self.unavailable:
            raise Unavailable("Service unavailable")
        return Snapshot(self.owner, dict(self.devices))

    def submit(self, owner, device, uris, deadline):
        if self.failure:
            raise self.failure
        self.submissions.append((owner, device, uris))


class MenuTests(unittest.TestCase):
    def setUp(self):
        language = patch.dict(os.environ, {"LC_ALL": "C.UTF-8"})
        language.start()
        self.addCleanup(language.stop)
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.file = self.root / 'report "冬" $HOME;`x`\n.txt'
        self.file.write_text("test document")
        self.backend = FakeKDE()
        self.now = 10.0
        self.provider = Provider(self.backend, clock=lambda: self.now)
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

    def test_no_devices_and_missing_service_hide_action(self):
        self.backend.devices = {}
        self.assertEqual(self.menu(), [])
        self.backend.unavailable = True
        self.assertEqual(self.menu(), [])

    def test_single_device_is_a_send_to_destination(self):
        actions = self.menu()
        self.assertEqual(len(actions), 1)
        self.assertEqual(actions[0]["label"], "Pixel")
        self.assertEqual(actions[0]["placement"], "send-to")
        self.assertNotIn("children", actions[0])
        reply = self.activate(actions[0])
        self.assertEqual(reply["outcome"], {"status": "accepted", "accepted": 1, "total": 1})
        self.assertEqual(reply["feedback"], "notice")
        self.assertEqual(self.backend.submissions, [(":1.20", "phone1", [self.file.as_uri()])])
        self.assertNotIn("delivered", reply["message"].lower())

    def test_each_device_is_a_destination_routed_to_that_device(self):
        self.backend.devices["tablet2"] = "Tablet"
        leaves = self.menu()
        self.assertEqual([a["label"] for a in leaves], ["Pixel", "Tablet"])
        self.assertEqual({a["placement"] for a in leaves}, {"send-to"})
        self.assertEqual(self.backend.submissions, [])
        tablet = leaves[1]
        self.activate(tablet)
        self.assertEqual(self.backend.submissions[0][1], "tablet2")

    def test_batch_is_one_submission_with_literal_file_uris(self):
        other = self.root / "another %23?.pdf"
        other.write_bytes(b"pdf fixture")
        paths = [str(self.file), str(other)]
        reply = self.activate(self.menu(paths)[0], paths)
        self.assertEqual(reply["outcome"]["total"], 2)
        self.assertEqual(len(self.backend.submissions), 1)
        self.assertEqual(self.backend.submissions[0][2], [self.file.as_uri(), other.as_uri()])
        self.assertIn("%0A", self.backend.submissions[0][2][0])
        self.assertIn("%25", self.backend.submissions[0][2][1])

    def test_disconnect_never_redirects_to_remaining_device(self):
        self.backend.devices["tablet2"] = "Tablet"
        phone = self.menu()[0]
        del self.backend.devices["phone1"]
        reply = self.activate(phone)
        self.assertEqual(reply["outcome"]["status"], "rejected")
        self.assertEqual(self.backend.submissions, [])
        self.assertEqual([a["label"] for a in self.menu()], ["Tablet"])

    def test_service_restart_rejects_stale_target(self):
        leaf = self.menu()[0]
        self.backend.owner = ":1.21"
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.backend.submissions, [])

    def test_selection_change_rejects_entire_batch(self):
        leaf = self.menu()[0]
        other = self.root / "other"
        other.write_text("other")
        self.assertEqual(self.activate(leaf, [str(other)])["outcome"]["status"], "rejected")
        self.assertEqual(self.backend.submissions, [])

    def test_replaced_changed_removed_and_symlinked_file_rejected(self):
        for operation in ("replace", "edit", "remove", "symlink"):
            with self.subTest(operation=operation):
                self.file.unlink(missing_ok=True)
                self.file.write_text("original")
                leaf = self.menu()[0]
                if operation == "edit":
                    self.file.write_text("edited content")
                else:
                    self.file.unlink()
                    if operation == "replace":
                        self.file.write_text("replacement")
                    elif operation == "symlink":
                        self.file.symlink_to("/etc/hosts")
                self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.backend.submissions, [])

    def test_mixed_folder_symlink_and_background_selections_hide_actions(self):
        link = self.root / "link"
        link.symlink_to(self.file)
        for paths in ([str(self.root)], [str(self.file), str(self.root)], [str(link)]):
            with self.subTest(paths=paths):
                self.assertEqual(self.menu(paths), [])
        reply = self.provider.handle(self.request(background=True))
        self.assertEqual(reply["actions"], [])

    def test_duplicate_names_have_distinct_labels_and_targets(self):
        self.backend.devices = {"phone1": "Phone", "phone2": "Phone"}
        leaves = self.menu()
        self.assertNotEqual(leaves[0]["label"], leaves[1]["label"])
        self.activate(leaves[1])
        self.assertEqual(self.backend.submissions[0][1], "phone2")

    def test_unusual_device_names_remain_valid_protocol_labels(self):
        for name in ("\n\x1b\t", "冬" * 200, "A\x00B", "normal"):
            with self.subTest(name=name):
                self.backend.devices = {"phone1": name}
                leaf = self.menu()[0]
                self.assertLessEqual(len(leaf["label"].encode()), 128)
                self.assertFalse(any(ord(c) < 32 or 127 <= ord(c) < 160 for c in leaf["label"]))

    def test_localized_menus_bound_names_and_preserve_routing(self):
        self.backend.devices = {"phone1": "冬" * 90}
        for language in MESSAGES:
            with self.subTest(language=language), patch.dict(os.environ, {"LC_ALL": language}):
                leaf = self.menu()[0]
                self.assertLessEqual(len(leaf["label"].encode()), 128)
                self.assertEqual(self.activate(leaf)["outcome"]["status"], "accepted")
                self.assertEqual(self.backend.submissions[-1][1], "phone1")
        for locale, expected in (
            ("fr_FR.UTF-8", "KDE Connect n’a pas répondu à temps."),
            ("zh_CN.UTF-8", "KDE Connect 未及时响应。"),
            ("zh_TW.UTF-8", "KDE Connect 未及時回應。"),
            ("zh_HK.UTF-8", "KDE Connect 未及時回應。"),
        ):
            with self.subTest(locale=locale), patch.dict(os.environ, {"LC_ALL": locale}):
                self.assertEqual(tr("KDE Connect did not respond in time."), expected)
        with patch.dict(os.environ, {"LC_ALL": "unknown"}):
            self.assertEqual(tr("KDE Connect did not respond in time."),
                             "KDE Connect did not respond in time.")

    def test_translations_keep_complete_messages_and_named_placeholders(self):
        formatter = Formatter()
        for language, messages in MESSAGES.items():
            self.assertEqual(messages.keys(), MESSAGES["en"].keys())
            for original, translated in messages.items():
                with self.subTest(language=language, message=original):
                    fields = lambda text: {key for _, key, _, _ in formatter.parse(text) if key}
                    self.assertEqual(fields(translated), fields(original))
                    self.assertTrue(translated.strip())
                    translated.format(device="Pixel", count=2)

    def test_menu_refresh_preserves_ticket_until_consumed(self):
        leaf = self.menu()[0]
        self.assertEqual(self.menu()[0]["context"], leaf["context"])
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "accepted")
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        fresh = self.menu()[0]
        self.assertNotEqual(fresh["context"], leaf["context"])
        self.assertEqual(self.activate(fresh)["outcome"]["status"], "accepted")

    def test_expired_tampered_and_invalidated_tickets_cannot_send(self):
        leaf = self.menu()[0]
        self.assertEqual(self.activate({**leaf, "context": "forged"})["outcome"]["status"], "rejected")
        self.now += 121
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        leaf = self.menu()[0]
        event = self.provider.invalidate()
        self.assertEqual(event["revision"], 1)
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")
        self.assertEqual(self.backend.submissions, [])

    def test_unknown_submission_never_retried_or_reported_as_success(self):
        leaf = self.menu()[0]
        self.backend.failure = Uncertain("Check the phone before retrying")
        reply = self.activate(leaf)
        self.assertEqual(reply["outcome"], {"status": "unknown", "accepted": 0, "total": 1})
        self.assertNotIn("feedback", reply)
        self.assertEqual(self.activate(leaf)["outcome"]["status"], "rejected")

    def test_known_submission_failure_reports_rejection(self):
        leaf = self.menu()[0]
        self.backend.failure = Unavailable("Sharing disabled")
        reply = self.activate(leaf)
        self.assertEqual(reply["outcome"]["status"], "rejected")
        self.assertEqual(reply["message"], "Sharing disabled")

    def test_device_count_limit_never_produces_truncated_menu(self):
        self.backend.devices = {f"d{n}": f"Device {n}" for n in range(63)}
        self.assertEqual(len(self.menu()), 63)
        self.backend.devices["extra"] = "Extra"
        self.assertEqual(self.menu(), [])

    def test_invalid_paths_and_versions_never_submit(self):
        for changes in ({"paths": []}, {"paths": ["relative"]}, {"paths": ["/bad\0path"]},
                        {"paths": ["/bad\udcff"]}, {"paths": [str(self.file)] * 201},
                        {"version": 2}, {"background": "false"}):
            with self.subTest(changes=changes):
                reply = self.provider.handle({**self.request(), **changes})
                self.assertEqual(reply["error"], "invalid-request")
        self.assertEqual(self.backend.submissions, [])


class FakeConnection:
    def __init__(self):
        self.calls = []
        self.subscriptions = []
        self.failure = None

    def is_closed(self):
        return False

    def signal_subscribe(self, *args):
        self.subscriptions.append(args)

    def call_sync(self, destination, path, interface, method, args, signature, flags, timeout, cancel):
        self.calls.append((destination, path, interface, method, args.unpack(), signature.dup_string(), flags, timeout))
        if self.failure:
            raise self.failure
        replies = {
            "GetNameOwner": GLib.Variant("(s)", (":1.2",)),
            "deviceNames": GLib.Variant("(a{ss})", ({"phone1": "Pixel", "other": "No share"},)),
            "hasPlugin": GLib.Variant("(b)", (path.endswith("phone1"),)),
            "shareUrls": GLib.Variant("()", ()),
        }
        return replies[method]


class DBusContractTests(unittest.TestCase):
    def setUp(self):
        self.connection = FakeConnection()
        self.backend = KDEConnect()
        self.backend.connection = self.connection

    def test_discovery_filters_paired_reachable_and_enabled_share_plugin(self):
        result = self.backend.discover(time.monotonic() + 5)
        self.assertEqual(result, Snapshot(":1.2", {"phone1": "Pixel"}))
        self.assertEqual(self.connection.calls[1][4], (True, True))
        self.assertEqual(self.connection.calls[2][4], ("kdeconnect_share",))
        self.assertEqual(self.connection.calls[1][0], ":1.2")
        self.assertTrue(all(0 < call[-1] <= 1500 for call in self.connection.calls))

    def test_batch_submission_pins_owner_and_preserves_uri_list(self):
        uris = ["file:///tmp/a%20b", "file:///tmp/%E5%86%AC%0A"]
        self.backend.submit(":1.2", "phone1", uris, time.monotonic() + 5)
        call = self.connection.calls[0]
        self.assertEqual(call[:5], (":1.2", "/modules/kdeconnect/devices/phone1/share",
                         "org.kde.kdeconnect.device.share", "shareUrls", (uris,)))

    def test_no_bus_reply_after_submission_is_uncertain(self):
        self.connection.failure = GLib.Error.new_literal(Gio.io_error_quark(), "timeout", Gio.IOErrorEnum.TIMED_OUT)
        with self.assertRaises(Uncertain):
            self.backend.submit(":1.2", "phone1", ["file:///tmp/test"], time.monotonic() + 5)

    def test_unknown_object_before_dispatch_is_rejected(self):
        self.connection.failure = Gio.DBusError.new_for_dbus_error(
            "org.freedesktop.DBus.Error.UnknownObject", "Plugin missing")
        with self.assertRaises(Unavailable):
            self.backend.submit(":1.2", "phone1", ["file:///tmp/test"], time.monotonic() + 5)

    def test_elapsed_deadline_never_dispatches(self):
        with self.assertRaises(Unavailable):
            self.backend.submit(":1.2", "phone1", ["file:///tmp/test"], time.monotonic() - 1)
        self.assertEqual(self.connection.calls, [])

    def test_device_and_service_signals_invalidate_live_menu(self):
        provider = Provider(FakeKDE())
        events = []
        backend = KDEConnect(lambda: events.append(provider.invalidate()))
        with patch.object(Gio, "bus_get_sync", return_value=self.connection):
            backend.connect()
        for member in ("reachableChanged", "pairStateChanged", "nameChanged", "pluginsChanged"):
            backend.service_signal(None, ":1.2", "/modules/kdeconnect/devices/phone1",
                                   "org.kde.kdeconnect.device", member, None)
        self.connection.subscriptions[1][-1](None)
        self.assertEqual([e["revision"] for e in events], [1, 2, 3, 4, 5])
        backend.service_signal(None, ":1.2", "/modules/kdeconnect/devices/phone1",
                               "org.kde.kdeconnect.device", "batteryChargeChanged", None)
        self.assertEqual(len(events), 5)


class WireTests(unittest.TestCase):
    def test_fragmented_and_batched_requests_flush_and_exit_on_eof(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "file.txt"
            path.write_text("fixture")
            process = subprocess.Popen(
                [sys.executable, "-c", "import sys; sys.path.insert(0, 'tests'); from provider import main; from test_provider import FakeKDE; main(FakeKDE())"],
                cwd=PROVIDER, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                env={**os.environ, "DISPLAY": "", "WAYLAND_DISPLAY": "",
                     "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent-strata-test-bus"},
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
            reply = json.loads(process.stdout.readline())
            leaf = reply["actions"][0]
            activate = {**request, "id": 2, "method": "activate", "action": leaf["id"], "context": leaf["context"]}
            query = {**request, "id": 3, "method": "query"}
            process.stdin.write((json.dumps(activate) + "\n" + json.dumps(query) + "\n").encode())
            process.stdin.flush()
            process.stdin.close()
            process.stdin = None
            remaining, errors = process.communicate(timeout=5)
            replies = [json.loads(line) for line in remaining.splitlines()]
            self.assertEqual(replies[0]["outcome"]["status"], "accepted")
            self.assertEqual(replies[0]["feedback"], "notice")
            self.assertEqual(replies[1]["decorations"], [])
            self.assertEqual(process.returncode, 0, errors.decode())
            self.assertEqual(errors, b"")

    def test_missing_desktop_bus_keeps_process_alive_and_menu_empty(self):
        request = {"version": 1, "id": 1, "method": "menu", "paths": [__file__], "background": False}
        result = subprocess.run(
            [sys.executable, str(PROVIDER / "provider.py")],
            input=(json.dumps(request) + "\n").encode(), capture_output=True, timeout=5,
            env={**os.environ, "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent-strata-test-bus"},
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(json.loads(result.stdout)["actions"], [])


if __name__ == "__main__":
    unittest.main()
