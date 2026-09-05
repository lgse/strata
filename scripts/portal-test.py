#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise FileChooser v4, optionally on a private bus without installing a portal.

Requires PyGObject (Gio), dbus-daemon, and a graphical session. This client calls
Strata's backend directly; portal routing is tested separately with portal-test.html.
It only returns destinations: unlike the HTML SaveFile test, it never writes them.
"""

import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import uuid
import zlib

from gi.repository import Gio, GLib

BACKEND = "org.freedesktop.impl.portal.desktop.strata"
DESKTOP = "/org/freedesktop/portal/desktop"
INTERFACE = "org.freedesktop.impl.portal.FileChooser"


def fixture(folder):
    folder.mkdir(parents=True, exist_ok=True)
    for name in ("Documents", "Downloads", "Pictures", "Videos"):
        (folder / name).mkdir(exist_ok=True)
    (folder / "notes.txt").write_text("Strata portal test notes\n", encoding="utf-8")
    (folder / "readme.md").write_text("# Strata File Chooser\n", encoding="utf-8")
    (folder / "strata-portal-demo.txt").write_text("Existing file: test overwrite confirmation.\n", encoding="utf-8")
    def chunk(kind, data):
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))
    width, height = 480, 300
    pixels = b"".join(b"\0" + bytes(channel for x in range(width) for channel in
                        (80 + x * 100 // width, 130 + y * 90 // height, 220 - y * 60 // height))
                      for y in range(height))
    (folder / "sample.png").write_bytes(b"\x89PNG\r\n\x1a\n" +
        chunk(b"IHDR", struct.pack("!2I5B", width, height, 8, 2, 0, 0, 0)) +
        chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def request(connection, args, folder):
    options = {"current_folder": GLib.Variant("ay", os.fsencode(folder) + b"\0")}
    method = "OpenFile"
    if args.case == "multiple":
        options["multiple"] = GLib.Variant("b", True)
    if args.case == "directory":
        options["directory"] = GLib.Variant("b", True)
        options["accept_label"] = GLib.Variant("s", "Select Folder")
    if args.case in ("filters", "save"):
        options["filters"] = GLib.Variant("a(sa(us))", [
            ("Text files", [(0, "*.txt"), (0, "*.md")]),
            ("Images", [(1, "image/png"), (1, "image/jpeg")]),
            ("All files", [(0, "*")]),
        ])
    if args.case == "save":
        method = "SaveFile"
        options["current_name"] = GLib.Variant("s", "strata-portal-demo.txt")
    if args.case == "savefiles":
        method = "SaveFiles"
        options["files"] = GLib.Variant("aay", [b"notes.txt\0", b"new-file.txt\0"])
    if args.choices:
        options["choices"] = GLib.Variant("a(ssa(ss)s)", [
            ("encoding", "Encoding", [("utf8", "UTF-8"), ("latin1", "Latin-1")], "utf8"),
            ("compress", "Compress files", [], "false"),
        ])
    token = "strata_test_" + uuid.uuid4().hex
    handle = f"{DESKTOP}/request/strata_test/{token}"
    parameters = GLib.Variant("(osssa{sv})", (
        handle, "", "", f"Strata · {method} · {args.case}", options,
    ))
    loop = GLib.MainLoop()
    failed = []

    def completed(conn, result):
        try:
            response, values = conn.call_finish(result).unpack()
            print(json.dumps({"response": response, "results": values}, indent=2), flush=True)
        except GLib.Error as error:
            failed.append(str(error))
        finally:
            loop.quit()

    connection.call(BACKEND, DESKTOP, INTERFACE, method, parameters,
                    GLib.VariantType.new("(ua{sv})"), Gio.DBusCallFlags.NONE,
                    2**31 - 1, None, completed)
    if args.cancel_after is not None:
        def cancel():
            connection.call(BACKEND, handle, "org.freedesktop.impl.portal.Request", "Close",
                            None, None, Gio.DBusCallFlags.NONE, -1, None, None)
            return GLib.SOURCE_REMOVE
        GLib.timeout_add(max(1, int(args.cancel_after * 1000)), cancel)
    loop.run()
    if failed:
        raise RuntimeError(failed[0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", choices=["single", "multiple", "directory", "filters", "save", "savefiles"])
    parser.add_argument("--binary", type=Path, help="Launch this build with isolated D-Bus, settings, and cache")
    parser.add_argument("--folder", type=Path, help="Initial folder; defaults to a disposable sample fixture")
    parser.add_argument("--choices", action="store_true")
    parser.add_argument("--cancel-after", type=float, metavar="SECONDS")
    parser.add_argument("--view", choices=["columns", "icons", "list"], default="list")
    parser.add_argument("--theme", default="tokyo-night", help="Built-in theme for the isolated backend")
    parser.add_argument("--group-by-type", action="store_true")
    args = parser.parse_args()
    if not args.binary and (args.theme != "tokyo-night" or args.view != "list" or args.group_by_type):
        parser.error("Theme and view overrides require --binary; existing user settings are never modified")
    with tempfile.TemporaryDirectory(prefix="strata-portal-test-") as temporary:
        root = Path(temporary)
        folder = args.folder.resolve() if args.folder else root / "Test files"
        if not args.folder:
            fixture(folder)
        bus = backend = None
        try:
            if args.binary:
                env = os.environ.copy()
                for key, directory in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")]:
                    env[key] = str(root / directory)
                settings = root / "config/strata/settings.toml"
                settings.parent.mkdir(parents=True)
                settings.write_text(
                    f'mode = "theme"\ntheme = {json.dumps(args.theme)}\nbrowser_mode = {json.dumps(args.view)}\n'
                    f'group_by_type = {str(args.group_by_type).lower()}\n', encoding="utf-8",
                )
                if not args.folder:
                    (root / "config/user-dirs.dirs").write_text("".join(
                        f'XDG_{key}_DIR="{folder / name}"\n'
                        for key, name in (("DOCUMENTS", "Documents"), ("DOWNLOAD", "Downloads"),
                                          ("PICTURES", "Pictures"), ("VIDEOS", "Videos"))
                    ), encoding="utf-8")
                bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1"],
                                       stdout=subprocess.PIPE, text=True)
                address = bus.stdout.readline().strip()
                env["DBUS_SESSION_BUS_ADDRESS"] = address
                env["GIO_USE_VFS"] = "local"
                backend = subprocess.Popen([str(args.binary.resolve()), "--portal"], env=env)
            else:
                address = os.environ["DBUS_SESSION_BUS_ADDRESS"]
            connection = Gio.DBusConnection.new_for_address_sync(
                address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
                None, None,
            )
            if backend:
                deadline = time.monotonic() + 15
                while True:
                    owned = connection.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus",
                        "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (BACKEND,)),
                        None, Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
                    if owned:
                        break
                    if backend.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("The isolated Strata backend did not start")
                    time.sleep(.05)
            request(connection, args, folder)
        finally:
            for process in (backend, bus):
                if process:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
