#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run QA scenarios with Strata's UI thread under strace and list its file calls on user paths."""

from __future__ import annotations

import argparse
import bisect
import codecs
import datetime
import fnmatch
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
ALLOWLIST = REPOSITORY / "tests/qa/main-io-allowlist.toml"
RUNS = Path("target/main-io")
CONTAINER_CHECKOUT = "/workspace"
STACK_FRAMES_SHOWN = 8

USER, SETTINGS, UNCLASSIFIED, TOOLKIT = "user", "settings", "unclassified", "toolkit"
SEVERITY = (USER, SETTINGS, UNCLASSIFIED, TOOLKIT)
STARTUP, RUNNING = "startup", "running"
READ, WRITE, WATCH = "read", "write", "watch"

LINE = re.compile(r"^(?P<time>\d+\.\d+) (?P<call>\w+)\((?P<arguments>.*)\) += (?P<result>.*)$")
DESCRIPTOR = re.compile(r"^(?:-?\d+|AT_FDCWD)(?:<(?P<path>.*)>)?$")
DESCRIPTOR_NUMBER = re.compile(r"^(?:-?\d+|AT_FDCWD)$")
GVFS = re.compile(r"/run/user/\d+/gvfs(/|$)")
FRAME = re.compile(r"^(?P<object>.+?)\((?P<symbol>[^()]*)\) \[0x(?P<offset>[0-9a-f]+)\]$")
STALL = re.compile(r"UI thread (?:stalled \d+ ms .*|ran again after \d+ ms)")

PATHS = {
    **{call: (0,) for call in (
        "open", "creat", "stat", "lstat", "access", "truncate", "chdir", "mkdir", "rmdir",
        "unlink", "readlink", "chmod", "chown", "lchown", "statfs", "setxattr", "lsetxattr",
        "getxattr", "lgetxattr", "listxattr", "llistxattr", "removexattr", "lremovexattr",
        "utime", "utimes", "mknod", "chroot")},
    "rename": (0, 1),
    "link": (0, 1),
    "symlink": (1,),
    "inotify_add_watch": (1,),
}
AT_PATHS = {
    **{call: ((0, 1),) for call in (
        "openat", "openat2", "mkdirat", "mknodat", "fchownat", "futimesat", "newfstatat",
        "unlinkat", "readlinkat", "fchmodat", "fchmodat2", "faccessat", "faccessat2",
        "utimensat", "name_to_handle_at", "statx", "open_tree")},
    "renameat": ((0, 1), (2, 3)),
    "renameat2": ((0, 1), (2, 3)),
    "linkat": ((0, 1), (2, 3)),
    "symlinkat": ((1, 2),),
}
DESCRIPTORS = {"sendfile": (0, 1), "copy_file_range": (0, 2)}
IGNORED = {"execve", "getcwd"}
METADATA = {
    "stat", "lstat", "newfstatat", "statx", "access", "faccessat", "faccessat2", "readlink",
    "readlinkat", "statfs",
}
WRITES = {
    "creat", "mkdir", "mkdirat", "rmdir", "unlink", "unlinkat", "rename", "renameat",
    "renameat2", "link", "linkat", "symlink", "symlinkat", "truncate", "ftruncate",
    "fallocate", "write", "pwrite64", "pwritev", "fsync", "fdatasync", "syncfs", "chmod",
    "fchmod", "fchmodat", "fchmodat2", "chown", "fchown", "fchownat", "lchown", "utime",
    "utimes", "utimensat", "futimesat", "setxattr", "lsetxattr", "fsetxattr", "removexattr",
    "lremovexattr", "fremovexattr", "copy_file_range", "sendfile", "mknod", "mknodat",
}
WRITE_FLAGS = ("O_WRONLY", "O_RDWR", "O_CREAT", "O_TRUNC")
WATCHES = {"inotify_add_watch", "fanotify_mark"}

SETTINGS_PLACES = (
    ("config", "strata"), ("state", "strata"), ("cache", "strata"), ("data", "strata"),
    ("cache", "thumbnails"), ("config", "gtk-3.0/bookmarks"), ("config", "xdg-desktop-portal"),
    ("data", "xdg-desktop-portal"), ("data", "dbus-1/services"),
    ("home", ".local/state/omarchy"), ("home", ".config/omarchy"), ("runtime", "strata"),
    ("install", ""),
)
TOOLKIT_PLACES = (
    ("config", "gtk-4.0"), ("config", "gtk-3.0/settings.ini"), ("config", "fontconfig"),
    ("config", "dconf"), ("config", "user-dirs.dirs"), ("config", "mimeapps.list*"),
    ("config", "*-mimeapps.list"), ("config", "pulse"), ("config", "alsa"),
    ("cache", "fontconfig"), ("cache", "mesa_shader_cache"), ("cache", "mesa_shader_cache_db"),
    ("cache", "radv_builtin_shaders"),
    ("data", "icons"), ("data", "fonts"), ("data", "themes"), ("data", "mime"),
    ("data", "applications"), ("data", "recently-used.xbel*"),
    ("data", "glib-2.0"), ("data", "pixmaps"),
    ("home", ".fonts"), ("home", ".fonts.conf"), ("home", ".fontconfig"), ("home", ".icons"),
    ("home", ".local/share/icons"), ("home", ".local/share/fonts"), ("home", ".themes"),
    ("home", ".Xdefaults*"), ("home", ".Xresources"), ("home", ".Xauthority"),
    ("home", ".XCompose"), ("home", ".drirc"), ("home", ".asoundrc*"), ("home", ".pulse*"),
    ("home", ".config/pulse"), ("home", ".config/alsa"), ("runtime", "*"),
    ("environment", "strata.log"), ("environment", "portal.log"),
)
SYSTEM_PREFIXES = (
    "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/proc", "/sys", "/dev", "/run", "/var",
    "/opt", "/.flatpak-info",
)
TOOLKIT_IN_TMP = ("/tmp/.X11-unix", "/tmp/.ICE-unix")
USER_PREFIXES = ("/run/media", "/media", "/mnt", "/tmp")
PLACEHOLDERS = (
    "fixture", "install", "home", "config", "data", "cache", "state", "runtime", "environment",
)
SHARED_ROOTS = ("config", "data", "cache", "state", "runtime", "environment")


@dataclass(frozen=True)
class Allowed:
    reason: str
    category: str
    paths: tuple[str, ...] = ()
    calls: tuple[str, ...] = ()
    access: str | None = None
    phase: str | None = None

    def allows(self, call: "Call") -> bool:
        return (
            call.category == self.category
            and (not self.paths or all(
                any(fnmatch.fnmatchcase(path, pattern) for pattern in self.paths)
                for path in call.paths))
            and (not self.calls or call.name in self.calls)
            and self.access in (None, call.access)
            and self.phase in (None, call.phase)
        )


def load_allowlist(path: Path) -> list[Allowed]:
    with path.open("rb") as stream:
        document = tomllib.load(stream)
    entries = []
    for number, entry in enumerate(document.get("exception", []), start=1):
        unknown = set(entry) - {"reason", "category", "paths", "calls", "access", "phase"}
        if unknown:
            raise ValueError(f"allowlist entry {number}: unknown keys {sorted(unknown)}")
        if not str(entry.get("reason", "")).strip():
            raise ValueError(f"allowlist entry {number}: each exception needs its documented reason")
        category = entry.get("category")
        if category == USER:
            raise ValueError(f"allowlist entry {number}: user paths can never be allowed")
        if category not in (SETTINGS, UNCLASSIFIED):
            raise ValueError(f"allowlist entry {number}: unknown category {category!r}")
        if entry.get("access") not in (None, READ, WRITE, WATCH):
            raise ValueError(f"allowlist entry {number}: access must be read, write or watch")
        if entry.get("phase") not in (None, STARTUP, RUNNING):
            raise ValueError(f"allowlist entry {number}: phase must be startup or running")
        entries.append(Allowed(
            reason=entry["reason"], category=category, paths=tuple(entry.get("paths", ())),
            calls=tuple(entry.get("calls", ())), access=entry.get("access"),
            phase=entry.get("phase"),
        ))
    return entries


@dataclass(frozen=True)
class Call:
    scenario: str
    name: str
    access: str
    phase: str
    paths: tuple[str, ...]
    category: str
    line: str
    stack: tuple[str, ...] = ()


@dataclass
class Launch:
    meta: dict
    roots: dict[str, str] = field(init=False)
    data_dirs: list[str] = field(init=False)
    harness: list[str] = field(init=False)

    def __post_init__(self) -> None:
        roots = {name: self.meta[name] for name in PLACEHOLDERS if self.meta.get(name)}
        binary = self.meta.get("binary")
        if binary:
            roots["install"] = os.path.dirname(os.path.dirname(binary))
        self.roots = {name: os.path.normpath(root) for name, root in roots.items()}
        private = [self.roots[root] for root in ("fixture", "home") if root in self.roots]
        self.data_dirs = [
            os.path.normpath(folder) for folder in self.meta.get("data_dirs", ())
            if folder.startswith("/") and not any(
                within(folder, root) or within(root, folder) for root in private)
        ]
        self.harness = [os.path.normpath(path) for path in self.meta.get("harness", ())]

    def placeholder(self, path: str) -> str:
        best = None
        for name, root in self.roots.items():
            if within(path, root) and (best is None or len(root) > len(self.roots[best])):
                best = name
        if best is None:
            return path
        return "{" + best + "}" + path[len(self.roots[best]):]

    def classify(self, path: str, call: str = "") -> str:
        if not path.startswith("/"):
            return TOOLKIT
        if self.below("runtime", "gvfs", path) or GVFS.match(path):
            return USER
        for places, category in ((SETTINGS_PLACES, SETTINGS), (TOOLKIT_PLACES, TOOLKIT)):
            if any(self.below(root, place, path) for root, place in places):
                return category
        shared = [self.roots[root] for root in SHARED_ROOTS if root in self.roots]
        if path in shared or (call in METADATA and any(within(root, path) for root in shared)):
            return TOOLKIT
        if any(within(path, folder) for folder in self.data_dirs + self.harness):
            return TOOLKIT
        if call in METADATA:
            for places, category in ((SETTINGS_PLACES, SETTINGS), (TOOLKIT_PLACES, TOOLKIT)):
                if any(self.above(root, place, path) for root, place in places):
                    return category
        if any(self.below(root, place, path)
               for root, place in (
                   ("fixture", ""), ("home", ""), ("data", "Trash"), ("data", "gtksourceview-5"),
                   ("cache", "gstreamer-1.0"), ("data", "gstreamer-1.0"))):
            return USER
        if self.below("environment", "", path):
            return UNCLASSIFIED
        if any(within(path, prefix) for prefix in TOOLKIT_IN_TMP):
            return TOOLKIT
        if (any(within(path, prefix) for prefix in USER_PREFIXES)
                or path.startswith("/dev/shm/strata-")):
            return USER
        if path == "/" or any(within(path, prefix) for prefix in SYSTEM_PREFIXES):
            return TOOLKIT
        return UNCLASSIFIED

    def below(self, root: str, place: str, path: str) -> bool:
        if root not in self.roots or not within(path, self.roots[root]):
            return False
        relative = os.path.relpath(path, self.roots[root])
        return not place or any(
            fnmatch.fnmatchcase(relative, pattern) for pattern in (place, place + "/*"))

    def above(self, root: str, place: str, path: str) -> bool:
        if root not in self.roots:
            return False
        target = os.path.normpath(os.path.join(self.roots[root], place.split("*", 1)[0]))
        return path != target and within(target, path)


def within(path: str, root: str) -> bool:
    return path == root or path.startswith(root.rstrip("/") + "/")


def split_arguments(text: str) -> list[str]:
    parts, current, depth, quoted, escaped, descriptor = [], [], 0, False, False, False
    for character in text:
        if descriptor:
            current.append(character)
            descriptor = character != ">"
            continue
        if quoted:
            current.append(character)
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                quoted = False
            continue
        if character == '"':
            quoted = True
        elif character == "<" and DESCRIPTOR_NUMBER.match("".join(current).strip()):
            descriptor = True
        elif character in "([{<":
            depth += 1
        elif character in ")]}>":
            depth -= 1
        elif character == "," and depth == 0:
            parts.append("".join(current).strip())
            current = []
            continue
        current.append(character)
    parts.append("".join(current).strip())
    return parts


def unescaped(text: str) -> str:
    return codecs.escape_decode(text.encode())[0].decode("utf-8", "surrogateescape")


def string_argument(token: str) -> str | None:
    match = re.match(r'^"((?:[^"\\]|\\.)*)"', token)
    if not match:
        return None
    return unescaped(match.group(1))


def descriptor_path(token: str) -> str | None:
    match = DESCRIPTOR.match(token)
    if not match or match.group("path") is None:
        return None
    return unescaped(match.group("path")).removesuffix(" (deleted)")


def resolve(folder: str | None, name: str | None, cwd: str) -> str | None:
    if name is None or name == "":
        return folder
    if name.startswith("/"):
        return os.path.normpath(name)
    if name.startswith("resource:"):
        return name
    return os.path.normpath(os.path.join(folder or cwd, name))


def call_paths(name: str, arguments: list[str], cwd: str) -> list[str]:
    def argument(index: int) -> str:
        return arguments[index] if index < len(arguments) else ""

    if name in PATHS:
        paths = [resolve(None, string_argument(argument(index)), cwd) for index in PATHS[name]]
    elif name in AT_PATHS:
        paths = [
            resolve(descriptor_path(argument(folder)), string_argument(argument(path)), cwd)
            for folder, path in AT_PATHS[name]
        ]
    else:
        paths = [descriptor_path(argument(index)) for index in DESCRIPTORS.get(name, (0,))]
    return [path for path in paths if path]


def access_of(name: str, arguments: list[str]) -> str:
    if name in WATCHES:
        return WATCH
    if name in WRITES:
        return WRITE
    if name in ("open", "openat", "openat2"):
        flags = " ".join(arguments[1:3])
        if any(flag in flags for flag in WRITE_FLAGS):
            return WRITE
    return READ


def read_launch(directory: Path) -> tuple[list[Call], list[str]]:
    launch = Launch(json.loads((directory / "meta.json").read_text()))
    ready = launch.meta.get("ready") or float("inf")
    cwd = launch.meta.get("fixture", "/")
    calls: list[Call] = []
    frames: list[str] | None = None
    trace = directory / "trace"
    lines = trace.read_text(errors="surrogateescape").splitlines() if trace.exists() else []
    for line in lines + [""]:
        if line.startswith(" > "):
            if frames is not None:
                frames.append(line[3:])
            continue
        if frames is not None:
            calls[-1] = replace(calls[-1], stack=tuple(frames))
            frames = None
        match = LINE.match(line)
        if not match or match.group("call") in IGNORED:
            continue
        name = match.group("call")
        arguments = split_arguments(match.group("arguments"))
        paths = call_paths(name, arguments, cwd)
        if not paths:
            continue
        category = min((launch.classify(path, name) for path in paths), key=SEVERITY.index)
        if category == TOOLKIT:
            continue
        calls.append(Call(
            scenario=launch.meta.get("scenario", directory.parent.name),
            name=name,
            access=access_of(name, arguments),
            phase=STARTUP if float(match.group("time")) < ready else RUNNING,
            paths=tuple(launch.placeholder(path) for path in paths),
            category=category,
            line=line[:400],
        ))
        frames = []
    log = directory / "strata.log"
    stalls = STALL.findall(log.read_text(errors="replace")) if log.exists() else []
    return calls, stalls


class Symbols:
    def __init__(self) -> None:
        self.binaries: dict[str, tuple[list, list[int], list[str]]] = {}

    def symbol(self, frame: str) -> str:
        match = FRAME.match(frame)
        if not match:
            return ""
        if match.group("symbol") not in ("", "+0x0"):
            return match.group("symbol")
        segments, addresses, names = self.binary(match.group("object"))
        offset = int(match.group("offset"), 16)
        segment = next((segment for segment in segments
                        if segment[0] <= offset < segment[0] + segment[2]), None)
        if segment is None:
            return ""
        address = offset - segment[0] + segment[1]
        index = bisect.bisect_right(addresses, address) - 1
        return f"{names[index]}+{address - addresses[index]:#x}" if index >= 0 else ""

    def binary(self, path: str) -> tuple[list, list[int], list[str]]:
        if path not in self.binaries:
            self.binaries[path] = read_symbols(host_path(path))
        return self.binaries[path]


def host_path(path: str) -> Path:
    if path.startswith(CONTAINER_CHECKOUT + "/"):
        return REPOSITORY / path[len(CONTAINER_CHECKOUT) + 1:]
    return Path(path)


def read_symbols(path: Path) -> tuple[list, list[int], list[str]]:
    try:
        headers = subprocess.run(["readelf", "-lW", str(path)], capture_output=True,
                                 text=True, check=True).stdout
        table = subprocess.run(["nm", "-C", "-n", "--defined-only", str(path)],
                               capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return [], [], []
    segments = [
        (int(fields[1], 16), int(fields[2], 16), int(fields[4], 16))
        for fields in (line.split() for line in headers.splitlines())
        if fields[:1] == ["LOAD"]
    ]
    addresses, names = [], []
    for line in table.splitlines():
        fields = line.split(" ", 2)
        if len(fields) == 3 and fields[1] in ("t", "T", "w", "W"):
            addresses.append(int(fields[0], 16))
            names.append(fields[2])
    return segments, addresses, names


SYMBOLS = Symbols()


def strata_frames(stack: tuple[str, ...]) -> list[str]:
    symbols = (SYMBOLS.symbol(frame) for frame in stack)
    return [symbol for symbol in symbols
            if symbol.startswith(("strata::", "<strata::"))][:STACK_FRAMES_SHOWN]


@dataclass
class Report:
    launches: int = 0
    violations: dict[tuple, list[Call]] = field(default_factory=dict)
    allowed: dict[str, int] = field(default_factory=dict)
    stalls: dict[str, list[str]] = field(default_factory=dict)

    def as_text(self) -> str:
        lines = [f"Traced launches: {self.launches}"]
        lines.append(f"Violations: {len(self.violations)} distinct, "
                     f"{sum(len(calls) for calls in self.violations.values())} calls")
        for key in sorted(self.violations, key=lambda key: (SEVERITY.index(key[0]), key[1:])):
            category, phase, name, access, paths = key
            calls = self.violations[key]
            scenarios = sorted({call.scenario for call in calls})
            lines.append(f"- [{category}] {phase} {name} ({access}) {', '.join(paths)}: "
                         f"{len(calls)} calls in {len(scenarios)} scenarios")
            lines.extend(f"    {scenario}" for scenario in scenarios[:3])
            lines.append(f"    e.g. {calls[0].line}")
            stacks = {tuple(strata_frames(call.stack)) for call in calls} - {()}
            for stack in sorted(stacks)[:3]:
                lines.append("    from:")
                lines.extend(f"      {frame}" for frame in stack)
        if self.allowed:
            lines.append("Allowed by documented exceptions:")
            lines.extend(f"- {reason}: {count} calls"
                         for reason, count in sorted(self.allowed.items()))
        if self.stalls:
            lines.append("UI stalls logged by the watchdog:")
            for scenario, stalls in sorted(self.stalls.items()):
                lines.append(f"- {scenario}")
                lines.extend(f"    {stall}" for stall in stalls)
        return "\n".join(lines) + "\n"

    def as_json(self) -> list[dict]:
        return [
            {"category": key[0], "phase": key[1], "call": key[2], "access": key[3],
             "paths": list(key[4]), "calls": len(calls),
             "scenarios": sorted({call.scenario for call in calls}), "example": calls[0].line,
             "stacks": sorted({tuple(strata_frames(call.stack)) for call in calls} - {()})}
            for key, calls in self.violations.items()
        ]


def analyze(run: Path, allowlist: list[Allowed]) -> Report:
    report = Report()
    for meta in sorted(run.glob("*/launch-*/meta.json")):
        report.launches += 1
        calls, stalls = read_launch(meta.parent)
        if stalls:
            scenario = json.loads(meta.read_text()).get("scenario", "")
            report.stalls.setdefault(scenario, []).extend(stalls)
        for call in calls:
            exception = next((entry for entry in allowlist if entry.allows(call)), None)
            if exception is not None:
                report.allowed[exception.reason] = report.allowed.get(exception.reason, 0) + 1
                continue
            key = (call.category, call.phase, call.name, call.access, call.paths)
            report.violations.setdefault(key, []).append(call)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analyze", type=Path, metavar="RUN",
                        help="classify an earlier run's traces instead of running scenarios")
    parser.add_argument("--allowlist", type=Path, default=ALLOWLIST)
    parser.add_argument("--stall-ms", type=int, metavar="MS",
                        help="also start Strata's UI stall watchdog with this limit")
    parser.add_argument("--stacks", action="store_true",
                        help="record the stack of every traced call (slow; use on a selection)")
    arguments, qa_arguments = parser.parse_known_args(argv)
    if qa_arguments[:1] == ["--"]:
        qa_arguments = qa_arguments[1:]
    try:
        allowlist = load_allowlist(arguments.allowlist)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        parser.exit(2, f"main-thread I/O audit: {error}\n")

    status = 0
    if arguments.analyze is not None:
        run = arguments.analyze
    else:
        stamp = datetime.datetime.now(datetime.UTC).strftime("%Y%m%dT%H%M%SZ")
        relative = RUNS / stamp
        run = REPOSITORY / relative
        run.mkdir(parents=True)
        (run / "options.json").write_text(json.dumps({"stacks": arguments.stacks}) + "\n")
        environment = {**os.environ, "STRATA_QA_MAIN_IO": str(relative)}
        if arguments.stall_ms:
            environment["STRATA_UI_STALL_MS"] = str(arguments.stall_ms)
        status = subprocess.run(
            [str(REPOSITORY / "scripts/qa.sh"), *qa_arguments], env=environment, check=False
        ).returncode
    report = analyze(run, allowlist)
    text = report.as_text()
    (run / "report.txt").write_text(text)
    (run / "violations.json").write_text(json.dumps(report.as_json(), indent=2) + "\n")
    sys.stdout.write(text)
    print(f"Traces and report: {run}")
    if report.violations:
        return 1
    return status


if __name__ == "__main__":
    sys.exit(main())
