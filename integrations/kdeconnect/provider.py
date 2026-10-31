#!/usr/bin/python3
# SPDX-License-Identifier: MIT
"""Device-aware KDE Connect menus for Strata's external provider protocol."""

from collections import Counter, OrderedDict
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import sys
import time
import unicodedata

SERVICE = "org.kde.kdeconnect"
ROOT = "/modules/kdeconnect"
MAX_FRAME = 1024 * 1024
REQUEST_SECONDS = 5.0

MESSAGES = json.loads(Path(__file__).with_name("messages.json").read_text(encoding="utf-8"))


def tr(message, **values):
    # Providers inherit the system locale, independently of Strata's preference.
    language = next((os.environ[k] for k in ("LC_ALL", "LC_MESSAGES", "LANG") if os.environ.get(k)), "en")
    language = language.split(".")[0].split("@")[0].replace("_", "-")
    language = {"pt": "pt-BR", "zh": "zh-CN", "zh-HK": "zh-TW", "zh-MO": "zh-TW"}.get(language, language)
    messages = MESSAGES.get(language, MESSAGES.get(language.split("-")[0], MESSAGES["en"]))
    return messages.get(message, message).format(**values)


class Unavailable(Exception):
    """Discovery or submission is known not to have succeeded."""


class Uncertain(Exception):
    """A submission may have reached KDE Connect; never retry automatically."""


@dataclass(frozen=True)
class Snapshot:
    owner: str
    devices: dict


@dataclass
class Ticket:
    token: str
    owner: str
    device: str
    paths: tuple
    identities: tuple
    expires: float


def label(text, limit=90):
    clean = "".join(c for c in text if unicodedata.category(c) != "Cc").strip()
    return (clean or tr("Device")).encode("utf-8")[:limit].decode("utf-8", "ignore")


def action_id(device):
    return "send-" + hashlib.sha256(device.encode("utf-8")).hexdigest()[:48]


def file_identities(paths):
    result = []
    for path in paths:
        s = os.stat(path, follow_symlinks=False)
        if not stat.S_ISREG(s.st_mode):
            raise ValueError(tr("Select regular files, without folders or symbolic links."))
        result.append((s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns))
    return tuple(result)


class KDEConnect:
    """Bounded D-Bus calls, pinned to a service's unique owner during sending."""

    def __init__(self, changed=lambda: None):
        from gi.repository import Gio, GLib

        self.Gio, self.GLib = Gio, GLib
        self.changed = changed
        self.connection = None

    def connect(self):
        if self.connection is not None and not self.connection.is_closed():
            return self.connection
        c = self.Gio.bus_get_sync(self.Gio.BusType.SESSION, None)
        self.connection = c
        c.signal_subscribe(
            SERVICE, None, None, None, None, self.Gio.DBusSignalFlags.NONE,
            self.service_signal,
        )
        c.signal_subscribe(
            "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
            "/org/freedesktop/DBus", SERVICE, self.Gio.DBusSignalFlags.NONE,
            lambda *args: self.changed(),
        )
        return c

    def service_signal(self, connection, sender, path, interface, member, parameters):
        if member in {
            "deviceListChanged", "deviceAdded", "deviceRemoved", "reachableChanged",
            "pairStateChanged", "nameChanged", "pluginsChanged", "PropertiesChanged",
        }:
            self.changed()

    def call(self, owner, path, interface, method, args, result_type, deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise Unavailable(tr("KDE Connect did not respond in time."))
        return self.connect().call_sync(
            owner, path, interface, method, args,
            self.GLib.VariantType.new(result_type),
            self.Gio.DBusCallFlags.NO_AUTO_START,
            max(1, min(1500, int(remaining * 1000))), None,
        ).unpack()

    def discover(self, deadline):
        try:
            owner = self.call(
                "org.freedesktop.DBus", "/org/freedesktop/DBus",
                "org.freedesktop.DBus", "GetNameOwner",
                self.GLib.Variant("(s)", (SERVICE,)), "(s)", deadline,
            )[0]
            devices = self.call(
                owner, ROOT, SERVICE + ".daemon", "deviceNames",
                self.GLib.Variant("(bb)", (True, True)), "(a{ss})", deadline,
            )[0]
            if len(devices) > 63:
                raise Unavailable(tr("Too many devices to fit in Strata's device menu."))
            eligible = {}
            for device, name in devices.items():
                if not device or not all(c.isascii() and (c.isalnum() or c == "_") for c in device):
                    continue
                if self.call(
                    owner, ROOT + "/devices/" + device, SERVICE + ".device",
                    "hasPlugin", self.GLib.Variant("(s)", ("kdeconnect_share",)),
                    "(b)", deadline,
                )[0]:
                    eligible[device] = name
            return Snapshot(owner, eligible)
        except self.GLib.Error as error:
            raise Unavailable(tr("KDE Connect is unavailable or the device list changed.")) from error

    def submit(self, owner, device, uris, deadline):
        try:
            self.call(
                owner, ROOT + "/devices/" + device + "/share", SERVICE + ".device.share",
                "shareUrls", self.GLib.Variant("(as)", (uris,)), "()", deadline,
            )
        except self.GLib.Error as error:
            remote = self.Gio.DBusError.get_remote_error(error)
            if remote in {
                "org.freedesktop.DBus.Error.UnknownMethod",
                "org.freedesktop.DBus.Error.UnknownObject",
                "org.freedesktop.DBus.Error.UnknownInterface",
                "org.freedesktop.DBus.Error.ServiceUnknown",
                "org.freedesktop.DBus.Error.NameHasNoOwner",
                "org.freedesktop.DBus.Error.InvalidArgs",
                "org.freedesktop.DBus.Error.AccessDenied",
            }:
                raise Unavailable(tr("The device is unavailable or file sharing was disabled.")) from error
            raise Uncertain(
                tr("KDE Connect did not confirm the submission. Check your device before retrying.")
            ) from error


class Provider:
    def __init__(self, backend, clock=time.monotonic):
        self.backend = backend
        self.clock = clock
        self.revision = 0
        self.tickets = OrderedDict()

    def invalidate(self):
        self.revision += 1
        self.tickets.clear()
        return {"version": 1, "event": "invalidate", "revision": self.revision}

    def ticket(self, snapshot, device, paths, identities):
        now = self.clock()
        key = (snapshot.owner, device, tuple(paths))
        t = self.tickets.get(key)
        if t is None or t.identities != identities or t.expires < now:
            t = Ticket(secrets.token_hex(24), snapshot.owner, device, tuple(paths), identities, now + 120)
            self.tickets[key] = t
        self.tickets.move_to_end(key)
        while len(self.tickets) > 256:
            self.tickets.popitem(last=False)
        return t

    def menu(self, paths, background, deadline):
        if background:
            return []
        try:
            identities = file_identities(paths)
            snapshot = self.backend.discover(deadline)
        except (OSError, ValueError, Unavailable):
            return []
        if not snapshot.devices or len(snapshot.devices) > 63:
            return []
        names = {device: label(name) for device, name in snapshot.devices.items()}
        counts = Counter(names.values())
        actions = []
        for device in sorted(names, key=lambda d: (names[d].casefold(), d)):
            name = names[device]
            if counts[name] > 1:
                name += " (" + hashlib.sha256(device.encode()).hexdigest()[:6] + ")"
            t = self.ticket(snapshot, device, paths, identities)
            actions.append({"id": action_id(device), "label": name, "placement": "send-to",
                            "context": t.token})
        return actions

    def activate(self, request, deadline):
        paths = request["paths"]
        total = len(paths)

        def result(status, message):
            return {"message": message, "outcome": {
                "status": status, "accepted": total if status == "accepted" else 0, "total": total,
            }, **({"feedback": "notice"} if status == "accepted" else {})}

        t = next((t for t in self.tickets.values() if t.token == request.get("context")), None)
        if (t is None or request.get("background") or tuple(paths) != t.paths
                or request.get("action") != action_id(t.device) or t.expires < self.clock()):
            return result("rejected", tr("This device menu expired. Reopen the right-click menu."))
        # Consume before submission; a lost reply must not cause an automatic duplicate send.
        self.tickets.pop((t.owner, t.device, t.paths), None)
        try:
            if file_identities(paths) != t.identities:
                raise Unavailable(tr("The selected files changed. Reopen the right-click menu."))
            if not all(os.access(path, os.R_OK) for path in paths):
                raise Unavailable(tr("One or more selected files cannot be read."))
            snapshot = self.backend.discover(deadline)
            if snapshot.owner != t.owner or t.device not in snapshot.devices:
                raise Unavailable(tr("The selected device disconnected or file sharing was disabled."))
            if file_identities(paths) != t.identities:
                raise Unavailable(tr("The selected files changed. Reopen the right-click menu."))
            self.backend.submit(t.owner, t.device, [Path(p).as_uri() for p in paths], deadline)
        except Uncertain as error:
            return result("unknown", str(error))
        except (OSError, ValueError, Unavailable) as error:
            message = str(error) if isinstance(error, Unavailable) else tr("The selected files are no longer available.")
            return result("rejected", message)
        return result(
            "accepted", tr("Files submitted to {device}: {count}. KDE Connect handles transfer progress and results.",
               device=label(snapshot.devices[t.device]), count=total),
        )

    def handle(self, request):
        reply = {"version": 1, "id": request.get("id", 0)}
        paths = request.get("paths")
        try:
            valid_paths = (isinstance(paths, list) and 1 <= len(paths) <= 200
                and all(isinstance(p, str) and p.startswith("/") and "\0" not in p
                        and len(p.encode("utf-8")) <= 16384 for p in paths))
        except UnicodeError:
            valid_paths = False
        if (request.get("version") != 1 or not valid_paths
                or type(request.get("id")) is not int or request["id"] < 0
                or type(request.get("background")) is not bool):
            return {**reply, "error": "invalid-request", "message": tr("Invalid provider request.")}
        deadline = time.monotonic() + REQUEST_SECONDS
        method = request.get("method")
        if method == "query":
            return {**reply, "decorations": [], "revision": self.revision}
        if method == "menu":
            actions = self.menu(paths, request["background"], deadline)
            return {**reply, "actions": actions, "revision": self.revision}
        if method == "activate":
            return {**reply, **self.activate(request, deadline)}
        return {**reply, "error": "unsupported-method", "message": tr("Unsupported provider method.")}


def main(backend=None):
    from gi.repository import GLib

    loop = GLib.MainLoop()

    def emit(value):
        print(json.dumps(value, ensure_ascii=True), flush=True)

    def changed():
        emit(provider.invalidate())

    provider = Provider(backend if backend is not None else KDEConnect(changed))
    buffer = bytearray()
    os.set_blocking(sys.stdin.fileno(), False)

    def incoming(fd, condition):
        try:
            chunk = os.read(fd, 65536)
        except BlockingIOError:
            return True
        if not chunk:
            loop.quit()
            return False
        buffer.extend(chunk)
        while b"\n" in buffer:
            end = buffer.index(b"\n")
            if end > MAX_FRAME:
                loop.quit()
                return False
            frame = bytes(buffer[:end])
            del buffer[:end + 1]
            try:
                request = json.loads(frame)
                if not isinstance(request, dict):
                    raise ValueError("Expected an object")
            except (ValueError, UnicodeError):
                loop.quit()
                return False
            emit(provider.handle(request))
        if len(buffer) > MAX_FRAME:
            loop.quit()
            return False
        return True

    GLib.io_add_watch(sys.stdin.fileno(), GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, incoming)
    try:
        loop.run()
    except (BrokenPipeError, KeyboardInterrupt):
        pass


if __name__ == "__main__":
    main()
