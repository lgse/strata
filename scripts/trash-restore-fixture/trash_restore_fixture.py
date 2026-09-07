#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Plant labeled trash entries for manual cross-volume restore testing.

Creates authentic GVfs volume-trash items (relative Path=, escaped trash:///
names) plus crafted orig-paths that point at another volume. Open Trash in
Strata and restore the prefixed items.

Examples:

  ./scripts/trash-restore-fixture/trash_restore_fixture.py plant --volume /run/media/$USER/<disk>
  ./scripts/trash-restore-fixture/trash_restore_fixture.py status
  ./scripts/trash-restore-fixture/trash_restore_fixture.py clean
  ./scripts/trash-restore-fixture/trash_restore_fixture.py plant --volume /mnt/usb --prefix demo --label strata-demo --force
"""

from __future__ import annotations

import argparse
import datetime as dt
import getpass
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
from dataclasses import dataclass
from pathlib import Path


class FixtureError(Exception):
    pass


@dataclass(frozen=True)
class CaseSpec:
    key: str
    expect: str
    side: str
    kind: str
    group: str
    notes: str
    rewrite: str | None = None


@dataclass(frozen=True)
class Case:
    title: str
    expect: str
    destination: Path
    info: Path
    notes: str


@dataclass(frozen=True)
class Config:
    uid: int
    prefix: str
    label: str
    volume: Path | None
    home: Path
    home_dir: Path
    home_sink: Path
    home_trash: Path
    tmp_dir: Path
    manifest: Path
    only: str
    restart_gvfs: bool

    def name(self, key: str) -> str:
        return f"{self.prefix}-{key}"

    @property
    def markers(self) -> tuple[str, ...]:
        return fixture_markers(self.prefix, self.label)

    @property
    def volume_dir(self) -> Path:
        return self.require_volume() / self.label

    @property
    def volume_trash(self) -> Path:
        return self.require_volume() / f".Trash-{self.uid}"

    def require_volume(self) -> Path:
        if self.volume is None:
            raise FixtureError("pass --volume")
        return self.volume


SPECS: tuple[CaseSpec, ...] = (
    CaseSpec(
        "RESTORE-volume-file.txt",
        "RESTORE",
        "volume",
        "file",
        "restore",
        "Authentic GVfs volume trash with relative Path=.",
    ),
    CaseSpec(
        "RESTORE-volume spaces.txt",
        "RESTORE",
        "volume",
        "file",
        "restore",
        "Spaces in the name; GVfs percent-encodes Path=.",
    ),
    CaseSpec(
        "RESTORE-nested.txt",
        "RESTORE",
        "volume",
        "nested",
        "restore",
        "Nested relative Path=. Parent dirs are recreated so restore can succeed.",
    ),
    CaseSpec(
        "RESTORE-album",
        "RESTORE",
        "volume",
        "dir",
        "restore",
        "Directory trashed from the extra volume.",
    ),
    CaseSpec(
        "RESTORE-collision.txt",
        "RESTORE",
        "volume",
        "file",
        "restore",
        "Same display name as the home-trash collision item; must return to the extra volume.",
    ),
    CaseSpec(
        "RESTORE-home-file.txt",
        "RESTORE",
        "home",
        "file",
        "restore",
        "Control: home trash restoring onto the home volume.",
    ),
    CaseSpec(
        "RESTORE-collision.txt",
        "RESTORE",
        "home",
        "file",
        "restore",
        "Same display name as the volume collision item; must return to home.",
    ),
    CaseSpec(
        "REJECT-volume-to-home.txt",
        "REJECT",
        "volume",
        "file",
        "reject",
        "Volume trash files/, Path= points at home.",
        "home-abs",
    ),
    CaseSpec(
        "REJECT-volume-to-tmp.txt",
        "REJECT",
        "volume",
        "file",
        "reject",
        "Volume trash files/, Path= points at --tmp-dir.",
        "tmp-abs",
    ),
    CaseSpec(
        "REJECT-volume-symlink.txt",
        "REJECT",
        "volume",
        "file",
        "reject",
        "Relative Path= walks through a symlink from the extra volume into home.",
        "symlink-rel",
    ),
    CaseSpec(
        "REJECT-volume-dotdot.txt",
        "REJECT",
        "volume",
        "file",
        "reject",
        "Percent-encoded ../ Path= that tries to leave the volume topdir.",
        "dotdot",
    ),
    CaseSpec(
        "REJECT-home-to-volume.txt",
        "REJECT",
        "home",
        "file",
        "reject",
        "Home trash files/, Path= points at the extra volume.",
        "volume-abs",
    ),
    CaseSpec(
        "REJECT-home-to-tmp.txt",
        "REJECT",
        "home",
        "file",
        "reject",
        "Home trash files/, Path= points at --tmp-dir.",
        "tmp-abs",
    ),
    CaseSpec(
        "MISSING-PARENT.txt",
        "LOOKUP-OK / MOVE-FAIL",
        "volume",
        "file",
        "reject",
        "Same-volume orig-path whose parent directory does not exist.",
        "missing-parent",
    ),
)


def fixture_markers(prefix: str, label: str) -> tuple[str, ...]:
    markers = [f"{prefix}-", label]
    return tuple(dict.fromkeys(markers))


def is_fixture_entry(name: str, body: str, markers: tuple[str, ...]) -> bool:
    stem = name.removesuffix(".trashinfo")
    haystacks = (stem, body)
    return any(marker in haystack for marker in markers for haystack in haystacks)


def encode_trashinfo_path(path: str) -> str:
    encoded = []
    for part in path.split("/"):
        if part == "..":
            encoded.append("%2e%2e")
        else:
            encoded.append(urllib.parse.quote(part, safe="-_.~"))
    return "/".join(encoded)


def parent_escape_path(topdir: Path, destination: Path) -> str:
    """Relative Path= from the volume topdir that walks to destination via .."""
    if not destination.is_absolute():
        raise FixtureError("escape destination must be absolute")
    ups = max(len(topdir.parts) - 1, 1)
    rest = destination.parts[1:]
    return encode_trashinfo_path("/".join([".."] * ups + list(rest)))


def selected_specs(only: str) -> tuple[CaseSpec, ...]:
    if only == "all":
        return SPECS
    return tuple(spec for spec in SPECS if spec.group == only)


def spec_title(spec: CaseSpec, specs: tuple[CaseSpec, ...]) -> str:
    if sum(1 for other in specs if other.key == spec.key) > 1:
        return f"{spec.key} ({spec.side} copy)"
    return spec.key


def media_roots(user: str | None = None) -> list[Path]:
    name = user or getpass.getuser()
    return [Path("/run/media") / name, Path("/media") / name]


def volume_can_host_trash(volume: Path, uid: int) -> bool:
    trash = volume / f".Trash-{uid}"
    if trash.is_dir():
        return os.access(trash, os.W_OK)
    return os.access(volume, os.W_OK)


def discover_volumes(
    home: Path,
    uid: int,
    roots: list[Path] | None = None,
) -> list[Path]:
    try:
        home_dev = home.stat().st_dev
    except OSError as error:
        raise FixtureError(f"cannot stat home {home}: {error}") from error
    found: list[Path] = []
    for root in roots if roots is not None else media_roots():
        if not root.is_dir():
            continue
        try:
            children = sorted(root.iterdir())
        except OSError:
            continue
        for child in children:
            try:
                if not child.is_dir() or child.stat().st_dev == home_dev:
                    continue
            except OSError:
                continue
            if volume_can_host_trash(child, uid):
                found.append(child)
    return found


def xdg_data_home(home: Path) -> Path:
    override = os.environ.get("XDG_DATA_HOME")
    if override:
        return Path(override)
    return home / ".local/share"


def validate_identity(prefix: str, label: str) -> None:
    for name, value in (("prefix", prefix), ("label", label)):
        if not value or value in (".", "..") or "/" in value or "\\" in value:
            raise FixtureError(f"invalid {name}: {value!r}")
    if len(prefix) < 2:
        raise FixtureError("prefix must be at least 2 characters")


def add_identity_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--volume",
        type=Path,
        help="extra filesystem that can host .Trash-$UID (required for plant if more than one is mounted)",
    )
    parser.add_argument(
        "--prefix",
        default="478",
        help="item name prefix (default: 478)",
    )
    parser.add_argument(
        "--label",
        default="strata-478",
        help="directory name on the volume and under home (default: strata-478)",
    )
    parser.add_argument("--home-dir", type=Path, help="home-side restore tree (default: ~/<label>-home)")
    parser.add_argument(
        "--home-sink",
        type=Path,
        help="symlink escape target on home (default: ~/<label>-home-sink)",
    )
    parser.add_argument(
        "--tmp-dir",
        type=Path,
        help="directory used for off-volume reject orig-paths (default: the process temp dir)",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="where to write the plant summary (default: <tmp-dir>/<label>-trash-fixture.md)",
    )


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = parser.add_subparsers(dest="command", required=True)

    plant = sub.add_parser("plant", help="create trash entries and a manifest")
    add_identity_args(plant)
    plant.add_argument(
        "--only",
        choices=("all", "restore", "reject"),
        default="all",
        help="plant only same-volume restores, only cross-volume rejects, or both",
    )
    plant.add_argument(
        "--force",
        action="store_true",
        help="remove an existing fixture with the same prefix/label before planting",
    )
    plant.add_argument(
        "--restart-gvfs",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="restart gvfsd-trash after rewriting Path= so trash:/// orig-paths update (default: yes)",
    )
    plant.add_argument(
        "--dry-run",
        action="store_true",
        help="print the planned items without writing trash",
    )

    clean = sub.add_parser("clean", help="remove fixture trash items and test directories")
    add_identity_args(clean)
    clean.add_argument(
        "--keep-dirs",
        action="store_true",
        help="leave restore/reject directories on disk",
    )

    status = sub.add_parser("status", help="list fixture items in trash and GVfs")
    add_identity_args(status)

    return parser.parse_args(argv)


def resolve_volume(
    requested: Path | None,
    home: Path,
    uid: int,
    *,
    required: bool,
) -> Path | None:
    if requested is not None:
        volume = requested.expanduser().resolve()
        if not volume.is_dir():
            raise FixtureError(f"volume is not a directory: {volume}")
        if not volume_can_host_trash(volume, uid):
            raise FixtureError(f"cannot create or write .Trash-{uid} on {volume}")
        try:
            if volume.stat().st_dev == home.stat().st_dev:
                raise FixtureError(
                    f"{volume} is on the same filesystem as {home}; pick a distinct mount"
                )
        except OSError as error:
            raise FixtureError(f"cannot stat {volume}: {error}") from error
        return volume
    candidates = discover_volumes(home, uid)
    if not required:
        return None
    if len(candidates) == 1:
        return candidates[0]
    if not candidates:
        raise FixtureError(
            "no writable extra volume found under /run/media/$USER or /media/$USER; pass --volume"
        )
    listing = "\n".join(f"  {path}" for path in candidates)
    raise FixtureError(f"multiple extra volumes found; pass --volume:\n{listing}")


def config_from_args(
    args: argparse.Namespace,
    *,
    home: Path | None = None,
    uid: int | None = None,
    require_volume: bool = True,
) -> Config:
    validate_identity(args.prefix, args.label)
    home = (home or Path.home()).expanduser().resolve()
    uid = os.getuid() if uid is None else uid
    volume = resolve_volume(
        args.volume,
        home,
        uid,
        required=require_volume,
    )
    home_dir = (
        args.home_dir.expanduser().resolve()
        if args.home_dir
        else home / f"{args.label}-home"
    )
    home_sink = (
        args.home_sink.expanduser().resolve()
        if args.home_sink
        else home / f"{args.label}-home-sink"
    )
    tmp_dir = (
        args.tmp_dir.expanduser().resolve()
        if args.tmp_dir
        else Path(tempfile.gettempdir()).resolve()
    )
    manifest = (
        args.manifest.expanduser().resolve()
        if args.manifest
        else tmp_dir / f"{args.label}-trash-fixture.md"
    )
    return Config(
        uid=uid,
        prefix=args.prefix,
        label=args.label,
        volume=volume,
        home=home,
        home_dir=home_dir,
        home_sink=home_sink,
        home_trash=xdg_data_home(home) / "Trash",
        tmp_dir=tmp_dir,
        manifest=manifest,
        only=getattr(args, "only", "all"),
        restart_gvfs=getattr(args, "restart_gvfs", True),
    )


def source_path(cfg: Config, spec: CaseSpec) -> Path:
    name = cfg.name(spec.key)
    root = cfg.volume_dir if spec.side == "volume" else cfg.home_dir
    if spec.rewrite:
        return root / name
    if spec.kind == "nested":
        return root / "restore-here" / "nested" / "deep" / name
    return root / "restore-here" / name


def destination_path(cfg: Config, spec: CaseSpec) -> Path:
    name = cfg.name(spec.key)
    if spec.rewrite == "home-abs":
        return cfg.home_dir / "reject-sink" / name
    if spec.rewrite == "tmp-abs":
        return cfg.tmp_dir / name
    if spec.rewrite == "volume-abs":
        return cfg.volume_dir / "reject-sink" / name
    if spec.rewrite == "symlink-rel":
        return cfg.home_sink / name
    if spec.rewrite == "dotdot":
        return cfg.home_dir / "reject-sink" / name
    if spec.rewrite == "missing-parent":
        return cfg.volume_dir / "restore-here" / "gone" / name
    return source_path(cfg, spec)


def rewrite_value(cfg: Config, spec: CaseSpec) -> str | None:
    dest = destination_path(cfg, spec)
    if spec.rewrite in {"home-abs", "tmp-abs", "volume-abs"}:
        return str(dest)
    if spec.rewrite == "symlink-rel":
        return f"{cfg.label}/link-home/{cfg.name(spec.key)}"
    if spec.rewrite == "dotdot":
        return parent_escape_path(cfg.require_volume(), dest)
    if spec.rewrite == "missing-parent":
        return f"{cfg.label}/restore-here/gone/{cfg.name(spec.key)}"
    return None


def write_file(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")


def payload(spec: CaseSpec, name: str) -> str:
    if spec.key == "RESTORE-collision.txt" and spec.side == "volume":
        return "collision FROM VOLUME\n"
    if spec.key == "RESTORE-collision.txt" and spec.side == "home":
        return "collision FROM HOME\n"
    return f"{name} payload\n"


def ensure_keep(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    keep = path / ".keep-empty"
    if not keep.exists():
        keep.write_text(
            "This directory should stay empty except restored RESTORE items.\n",
            encoding="utf-8",
        )


def prepare_trees(cfg: Config, specs: tuple[CaseSpec, ...]) -> None:
    groups = {spec.group for spec in specs}
    cfg.volume_dir.mkdir(parents=True, exist_ok=True)
    cfg.home_dir.mkdir(parents=True, exist_ok=True)
    if "restore" in groups:
        ensure_keep(cfg.volume_dir / "restore-here")
        ensure_keep(cfg.home_dir / "restore-here")
    if "reject" in groups:
        ensure_keep(cfg.volume_dir / "reject-sink")
        ensure_keep(cfg.home_dir / "reject-sink")
        ensure_keep(cfg.home_sink)
        link = cfg.volume_dir / "link-home"
        if link.exists() or link.is_symlink():
            link.unlink()
        link.symlink_to(cfg.home_sink)


def newest_info(trash_root: Path, before: set[str]) -> Path:
    info = trash_root / "info"
    added: set[str] = set()
    for _ in range(40):
        now = {path.name for path in info.glob("*.trashinfo")}
        added = now - before
        if len(added) == 1:
            return info / added.pop()
        time.sleep(0.05)
    raise FixtureError(
        f"could not find new trashinfo in {info} (added={sorted(added) or 'none'})"
    )


def trash_root_for(path: Path, cfg: Config) -> Path:
    resolved = path.resolve()
    if resolved.is_relative_to(cfg.require_volume().resolve()):
        return cfg.volume_trash
    return cfg.home_trash


def gio_trash(path: Path, cfg: Config) -> Path:
    trash_root = trash_root_for(path, cfg)
    for directory in (trash_root, trash_root / "files", trash_root / "info"):
        directory.mkdir(mode=0o700, exist_ok=True)
    before = {entry.name for entry in (trash_root / "info").glob("*.trashinfo")}
    try:
        subprocess.run(
            ["gio", "trash", str(path)],
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError as error:
        raise FixtureError("gio is not installed") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        raise FixtureError(f"gio trash failed for {path}: {detail or error}") from error
    return newest_info(trash_root, before)


def rewrite_path(info_path: Path, new_path: str) -> None:
    lines = info_path.read_text(encoding="utf-8").splitlines()
    out = []
    replaced = False
    for line in lines:
        if line.startswith("Path="):
            out.append(f"Path={new_path}")
            replaced = True
        else:
            out.append(line)
    if not replaced:
        out.append(f"Path={new_path}")
    info_path.write_text("\n".join(out) + "\n", encoding="utf-8")


def restart_gvfsd_trash() -> None:
    subprocess.run(["pkill", "-x", "gvfsd-trash"], check=False)
    time.sleep(0.3)
    subprocess.run(
        ["gio", "list", "trash:///"],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(0.2)


def list_gvfs_items(markers: tuple[str, ...]) -> list[dict[str, str]]:
    try:
        from gi.repository import Gio
    except ImportError:
        return []
    trash = Gio.File.new_for_uri("trash:///")
    enumerator = trash.enumerate_children(
        "standard::name,standard::display-name,standard::target-uri,trash::orig-path",
        Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,
        None,
    )
    rows: list[dict[str, str]] = []
    while True:
        info = enumerator.next_file(None)
        if info is None:
            break
        display = info.get_display_name() or ""
        name = info.get_name() or ""
        orig = info.get_attribute_byte_string("trash::orig-path") or ""
        target = info.get_attribute_string("standard::target-uri") or ""
        blob = " ".join((display, name, orig, target))
        if not any(marker in blob for marker in markers):
            continue
        rows.append(
            {
                "display": display,
                "name": name,
                "orig": orig,
                "target": target,
            }
        )
    return rows


def fixture_present(cfg: Config) -> bool:
    for trash in (cfg.volume_trash, cfg.home_trash):
        info = trash / "info"
        if not info.is_dir():
            continue
        for path in info.glob("*.trashinfo"):
            try:
                body = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                body = ""
            if is_fixture_entry(path.name, body, cfg.markers):
                return True
    return False


def create_source(cfg: Config, spec: CaseSpec) -> Path:
    path = source_path(cfg, spec)
    name = cfg.name(spec.key)
    if spec.kind == "dir":
        path.mkdir(parents=True, exist_ok=True)
        write_file(path / "cover.txt", "album cover\n")
        write_file(path / "track.txt", "album track\n")
        return path
    write_file(path, payload(spec, name))
    return path


def plant_spec(cfg: Config, spec: CaseSpec, specs: tuple[CaseSpec, ...]) -> Case:
    created = create_source(cfg, spec)
    info = gio_trash(created, cfg)
    if spec.kind == "nested":
        created.parent.mkdir(parents=True, exist_ok=True)
    rewritten = rewrite_value(cfg, spec)
    if rewritten is not None:
        rewrite_path(info, rewritten)
    return Case(
        title=spec_title(spec, specs),
        expect=spec.expect,
        destination=destination_path(cfg, spec),
        info=info,
        notes=spec.notes,
    )


def write_manifest(
    cfg: Config,
    cases: list[Case],
    gvfs_rows: list[dict[str, str]],
    stamp: str,
) -> None:
    def cell(value: str) -> str:
        return value.replace("|", "\\|")

    identity = [
        f"--prefix {cfg.prefix}",
        f"--label {cfg.label}",
        f"--volume {cfg.volume}",
    ]
    clean_cmd = (
        "./scripts/trash-restore-fixture/trash_restore_fixture.py clean "
        + " ".join(identity)
    )
    lines = [
        f"# Strata trash restore fixture ({cfg.label})",
        "",
        f"Created {stamp} on uid {cfg.uid}.",
        "",
        f"Open **Trash** in Strata. Items are prefixed `{cfg.prefix}-`.",
        "Restore one at a time first, then try a mixed RESTORE + REJECT multi-select.",
        "",
        "## Expected results",
        "",
        "| Item | Expect | Destination | Notes |",
        "| --- | --- | --- | --- |",
    ]
    for case in cases:
        lines.append(
            f"| `{cell(case.title)}` | **{case.expect}** | `{cell(str(case.destination))}` | {cell(case.notes)} |"
        )
    lines += [
        "",
        "Rejects must not create files in:",
        "",
        f"- `{cfg.home_dir / 'reject-sink'}`",
        f"- `{cfg.volume_dir / 'reject-sink'}`",
        f"- `{cfg.home_sink}`",
        f"- `{cfg.tmp_dir / (cfg.prefix + '-REJECT-*')}`",
        "",
        "Successful restores land under:",
        "",
        f"- `{cfg.volume_dir / 'restore-here'}`",
        f"- `{cfg.home_dir / 'restore-here'}`",
        "",
        "Unrelated trash on the volume is left alone.",
        "",
        "## GVfs view",
        "",
        "| Display name | orig-path | target-uri | trash:/// name |",
        "| --- | --- | --- | --- |",
    ]
    if gvfs_rows:
        for row in gvfs_rows:
            lines.append(
                f"| `{cell(row['display'])}` | `{cell(row['orig'])}` | `{cell(row['target'])}` | `{cell(row['name'])}` |"
            )
    else:
        lines.append("| _(none listed)_ | | | |")
    lines += ["", "## On-disk trashinfo", ""]
    for case in cases:
        body = case.info.read_text(encoding="utf-8").strip().replace("\n", " | ")
        lines.append(f"- `{case.info}`: `{body}`")
    lines += [
        "",
        "## Cleanup",
        "",
        "```",
        clean_cmd,
        "```",
        "",
        "GVfs cannot trash on system-internal mounts such as `/tmp`, and cannot",
        "create `.Trash-$UID` on a root-owned mountpoint. Pass a writable extra",
        "volume with `--volume`.",
        "",
    ]
    cfg.manifest.parent.mkdir(parents=True, exist_ok=True)
    cfg.manifest.write_text("\n".join(lines), encoding="utf-8")


def print_plan(cfg: Config) -> None:
    specs = selected_specs(cfg.only)
    print(f"volume:     {cfg.volume}")
    print(f"volume dir: {cfg.volume_dir}")
    print(f"home dir:   {cfg.home_dir}")
    print(f"home sink:  {cfg.home_sink}")
    print(f"manifest:   {cfg.manifest}")
    print(f"only:       {cfg.only}")
    for spec in specs:
        title = spec_title(spec, specs)
        dest = destination_path(cfg, spec)
        rewritten = rewrite_value(cfg, spec)
        extra = f" Path= {rewritten}" if rewritten else ""
        print(f"  [{spec.expect}] {title} -> {dest}{extra}")


def plant(cfg: Config, *, force: bool, dry_run: bool) -> int:
    specs = selected_specs(cfg.only)
    if dry_run:
        print_plan(cfg)
        return 0
    if fixture_present(cfg) and not force:
        raise FixtureError(
            f"a {cfg.prefix}/{cfg.label} fixture is already in trash; pass --force to replace it"
        )
    if force:
        clean(cfg, keep_dirs=False, quiet=True)
    prepare_trees(cfg, specs)
    cases = [plant_spec(cfg, spec, specs) for spec in specs]
    if cfg.restart_gvfs and any(spec.rewrite for spec in specs):
        restart_gvfsd_trash()
    else:
        time.sleep(0.2)
    rows = list_gvfs_items(cfg.markers)
    stamp = dt.datetime.now().strftime("%Y-%m-%dT%H:%M:%S")
    write_manifest(cfg, cases, rows, stamp)
    print(f"Planted {len(cases)} labeled trash items.")
    print(f"GVfs currently lists {len(rows)} matching entries.")
    print(f"Manifest: {cfg.manifest}")
    for case in cases:
        print(f"  [{case.expect}] {case.title}")
    return 0


def clean_trash_root(trash_root: Path, markers: tuple[str, ...]) -> int:
    info = trash_root / "info"
    files = trash_root / "files"
    if not info.is_dir():
        return 0
    removed = 0
    for info_path in list(info.glob("*.trashinfo")):
        try:
            body = info_path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            body = ""
        if not is_fixture_entry(info_path.name, body, markers):
            continue
        stem = info_path.name[: -len(".trashinfo")]
        target = files / stem
        if target.is_dir() and not target.is_symlink():
            shutil.rmtree(target, ignore_errors=True)
        else:
            target.unlink(missing_ok=True)
        info_path.unlink(missing_ok=True)
        removed += 1
    return removed


def extra_volumes(cfg: Config) -> list[Path]:
    volumes = discover_volumes(cfg.home, cfg.uid)
    if cfg.volume is not None and cfg.volume not in volumes:
        volumes.append(cfg.volume)
    return volumes


def clean(cfg: Config, *, keep_dirs: bool, quiet: bool = False) -> int:
    removed = clean_trash_root(cfg.home_trash, cfg.markers)
    volumes = extra_volumes(cfg)
    for volume in volumes:
        removed += clean_trash_root(volume / f".Trash-{cfg.uid}", cfg.markers)
    paths = [
        cfg.tmp_dir / cfg.name("REJECT-volume-to-tmp.txt"),
        cfg.tmp_dir / cfg.name("REJECT-home-to-tmp.txt"),
        cfg.manifest,
    ]
    if not keep_dirs:
        paths.extend((cfg.home_dir, cfg.home_sink))
        paths.extend(volume / cfg.label for volume in volumes)
    for path in paths:
        if path.is_symlink() or path.is_file():
            path.unlink(missing_ok=True)
        elif path.is_dir():
            shutil.rmtree(path, ignore_errors=True)
    if not quiet:
        print(f"Removed {removed} fixture trashinfo entries.")
    return 0


def cases_from_disk(cfg: Config) -> list[Case]:
    specs = selected_specs("all")
    cases: list[Case] = []
    volume_trashes = [cfg.volume_trash] if cfg.volume is not None else [
        volume / f".Trash-{cfg.uid}" for volume in extra_volumes(cfg)
    ]
    for spec in specs:
        name = cfg.name(spec.key)
        trashes = volume_trashes if spec.side == "volume" else [cfg.home_trash]
        for trash in trashes:
            info = trash / "info" / f"{name}.trashinfo"
            if not info.exists():
                continue
            try:
                destination = destination_path(cfg, spec)
            except FixtureError:
                destination = info
            cases.append(
                Case(
                    title=spec_title(spec, specs),
                    expect=spec.expect,
                    destination=destination,
                    info=info,
                    notes=spec.notes,
                )
            )
            break
    return cases


def status(cfg: Config) -> int:
    cases = cases_from_disk(cfg)
    rows = list_gvfs_items(cfg.markers)
    if cfg.volume is not None:
        print(f"volume:   {cfg.volume}")
    else:
        found = extra_volumes(cfg)
        print(
            f"volume:   {', '.join(str(path) for path in found) or '(none found)'}"
        )
    print(f"prefix:   {cfg.prefix}")
    print(f"label:    {cfg.label}")
    print(f"on disk:  {len(cases)}")
    print(f"in gvfs:  {len(rows)}")
    for case in cases:
        print(f"  [{case.expect}] {case.title} -> {case.destination}")
    if rows:
        print("gvfs orig-paths:")
        for row in rows:
            print(f"  {row['display']}: {row['orig']}")
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    args = parse_args(argv)
    try:
        cfg = config_from_args(args, require_volume=args.command == "plant")
        if args.command == "plant":
            return plant(
                cfg,
                force=args.force,
                dry_run=args.dry_run,
            )
        if args.command == "clean":
            return clean(cfg, keep_dirs=args.keep_dirs)
        return status(cfg)
    except FixtureError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
