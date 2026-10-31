#!/usr/bin/python3
# SPDX-License-Identifier: MIT
"""Hands selected files to the LocalSend app for Strata's external provider protocol."""

from collections import OrderedDict
from dataclasses import dataclass
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import time

# The package names the program `localsend` or, in the upstream .deb, `localsend_app`.
NAMES = ("localsend", "localsend_app")
# Strata's trusted command directories; see src/trusted_command.rs.
SEARCH_ROOTS = (
    "/run/wrappers/bin", "/usr/bin", "/usr/sbin", "/bin", "/sbin",
    "/run/current-system/sw/bin", "/run/current-system/profile/bin",
)
TRUST_ROOTS = ("/usr/bin", "/usr/sbin", "/bin", "/sbin", "/run/wrappers/bin", "/nix/store", "/gnu/store")
ACTION = "open"
MAX_FRAME = 1024 * 1024
STARTUP_SECONDS = 1.5
TICKET_SECONDS = 120

MESSAGES = json.loads(Path(__file__).with_name("messages.json").read_text(encoding="utf-8"))


def tr(message, **values):
    # Providers inherit the system locale, independently of Strata's preference.
    language = next((os.environ[k] for k in ("LC_ALL", "LC_MESSAGES", "LANG") if os.environ.get(k)), "en")
    language = language.split(".")[0].split("@")[0].replace("_", "-")
    language = {"pt": "pt-BR", "zh": "zh-CN", "zh-HK": "zh-TW", "zh-MO": "zh-TW"}.get(language, language)
    messages = MESSAGES.get(language, MESSAGES.get(language.split("-")[0], MESSAGES["en"]))
    return messages.get(message, message).format(**values)


class Unavailable(Exception):
    """LocalSend is known not to have received the selection."""


@dataclass
class Ticket:
    token: str
    paths: tuple
    identities: tuple
    expires: float


def owned_up_to(path, top, trusted_owner):
    path = Path(path)
    if not path.is_relative_to(top):
        return False
    for directory in (path, *path.parents):
        if not directory.is_relative_to(top):
            break
        try:
            s = os.stat(directory)
        except OSError:
            return False
        if not trusted_owner(s) or s.st_mode & 0o022:
            return False
    return True


def resolve(names=NAMES, search_roots=SEARCH_ROOTS, trust_roots=TRUST_ROOTS, top="/",
            trusted_owner=lambda s: s.st_uid == 0):
    """Finds LocalSend the way Strata checks the requirement. A target outside the
    trust roots, such as /opt/localsend/localsend, is returned itself, so the
    unchecked links leading to it are never executed."""
    trusted = [os.path.realpath(root) for root in trust_roots]
    for name in names:
        candidates = [os.path.join(d, name) for d in search_roots if os.path.isfile(os.path.join(d, name))]
        for candidate in candidates:
            target = os.path.realpath(candidate)
            if any(Path(target).is_relative_to(root) for root in trusted):
                return candidate
        for candidate in candidates:
            target = os.path.realpath(candidate)
            if owned_up_to(target, top, trusted_owner):
                return target
    return None


def identities(paths):
    result = []
    for path in paths:
        s = os.stat(path, follow_symlinks=False)
        if not (stat.S_ISREG(s.st_mode) or stat.S_ISDIR(s.st_mode)):
            raise ValueError("not a regular file or folder")
        result.append((s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns))
    return tuple(result)


def readable(path):
    mode = os.R_OK | (os.X_OK if os.path.isdir(path) else 0)
    return os.access(path, mode)


class LocalSend:
    """Starts the app with the paths as arguments. A running instance takes them
    over and the new process exits; otherwise the new process is the app."""

    def __init__(self, find=resolve, startup=STARTUP_SECONDS):
        self.find = find
        self.startup = startup
        self.children = []

    def executable(self):
        return self.find()

    def reap(self):
        self.children = [child for child in self.children if child.poll() is None]

    def open(self, executable, paths):
        self.reap()
        try:
            # A new session keeps the app alive when Strata stops this provider's process group.
            child = subprocess.Popen(
                [executable, *paths], cwd="/", stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
            )
        except OSError as error:
            raise Unavailable(tr("LocalSend could not be started.")) from error
        try:
            code = child.wait(self.startup)
        except subprocess.TimeoutExpired:
            self.children.append(child)
            return
        if code != 0:
            raise Unavailable(tr("LocalSend closed before it opened the selection."))


class Provider:
    def __init__(self, launcher, clock=time.monotonic):
        self.launcher = launcher
        self.clock = clock
        self.tickets = OrderedDict()

    def ticket(self, paths, found):
        now = self.clock()
        key = tuple(paths)
        t = self.tickets.get(key)
        if t is None or t.identities != found or t.expires < now:
            t = Ticket(secrets.token_hex(24), key, found, now + TICKET_SECONDS)
            self.tickets[key] = t
        self.tickets.move_to_end(key)
        while len(self.tickets) > 256:
            self.tickets.popitem(last=False)
        return t

    def menu(self, paths, background):
        if background:
            return []
        try:
            found = identities(paths)
        except (OSError, ValueError):
            return []
        if self.launcher.executable() is None:
            return []
        t = self.ticket(paths, found)
        # A product name, not translated; Strata draws it as the whole Send to… entry.
        return [{"id": ACTION, "label": "LocalSend", "placement": "send-to-entry", "context": t.token}]

    def activate(self, request):
        paths = request["paths"]
        total = len(paths)

        def result(status, message):
            return {"message": message, "outcome": {
                "status": status, "accepted": total if status == "accepted" else 0, "total": total,
            }, **({"feedback": "none"} if status == "accepted" else {})}

        t = self.tickets.get(tuple(paths))
        if (t is None or t.token != request.get("context") or request.get("background")
                or request.get("action") != ACTION or t.expires < self.clock()):
            return result("rejected", tr("This LocalSend menu expired. Reopen the right-click menu."))
        # Consume before launching so a lost reply cannot open the selection twice.
        del self.tickets[t.paths]
        try:
            if identities(paths) != t.identities:
                raise Unavailable(tr("The selected items changed. Reopen the right-click menu."))
            if not all(readable(path) for path in paths):
                raise Unavailable(tr("One or more selected items cannot be read."))
            executable = self.launcher.executable()
            if executable is None:
                raise Unavailable(tr("LocalSend is not installed in a system directory."))
            self.launcher.open(executable, list(paths))
        except Unavailable as error:
            return result("rejected", str(error))
        except (OSError, ValueError):
            return result("rejected", tr("The selected items are no longer available."))
        return result("accepted", tr("Items opened in LocalSend: {count}. Choose a device there to send them.",
                                     count=total))

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
        method = request.get("method")
        if method == "query":
            return {**reply, "decorations": []}
        if method == "menu":
            return {**reply, "actions": self.menu(paths, request["background"])}
        if method == "activate":
            return {**reply, **self.activate(request)}
        return {**reply, "error": "unsupported-method", "message": tr("Unsupported provider method.")}


def main(launcher=None):
    launcher = launcher if launcher is not None else LocalSend()
    provider = Provider(launcher)
    stdin = sys.stdin.buffer
    try:
        while True:
            frame = stdin.readline(MAX_FRAME + 1)
            if not frame.endswith(b"\n"):
                return
            try:
                request = json.loads(frame)
                if not isinstance(request, dict):
                    raise ValueError("Expected an object")
            except (ValueError, UnicodeError):
                return
            print(json.dumps(provider.handle(request), ensure_ascii=True), flush=True)
            launcher.reap()
    except (BrokenPipeError, KeyboardInterrupt):
        pass


if __name__ == "__main__":
    main()
