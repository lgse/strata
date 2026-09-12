#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise locally built release artifacts; never use execution to validate downloads."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

from release_bundle import digest, inspect_elf

ROOT = Path(__file__).resolve().parents[1]


def elf_sections(path):
    with path.open("rb") as stream:
        header = stream.read(64)
        assert header[:6] == b"\x7fELF\x02\x01"
        offset, = struct.unpack_from("<Q", header, 40)
        size, count, strings = struct.unpack_from("<HHH", header, 58)
        assert size == 64 and 0 < count < 1024 and strings < count
        stream.seek(offset)
        sections = [struct.unpack("<IIQQQQIIQQ", stream.read(size)) for _ in range(count)]
        names_section = sections[strings]
        assert names_section[5] < 65536
        stream.seek(names_section[4])
        names = stream.read(names_section[5])
        return {names[section[0]:].split(b"\0", 1)[0]: (section[4], section[5]) for section in sections}


def verify_debug(binary, symbols):
    offset, size = elf_sections(binary)[b".gnu_debuglink"]
    assert size < 1024
    with binary.open("rb") as stream:
        stream.seek(offset)
        data = stream.read(size)
    name = data.split(b"\0", 1)[0].decode()
    assert name == symbols.name
    crc = 0
    with symbols.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            crc = zlib.crc32(block, crc)
    assert int.from_bytes(data[-4:], "little") == crc
    assert elf_sections(symbols)[b".debug_line"][1] > 0


def verify(directory):
    manifest = json.loads((directory / "bundle.json").read_text())
    for name, expected in manifest["files"].items():
        assert not Path(name).is_absolute() and ".." not in Path(name).parts
        path = directory / name
        assert path.is_relative_to(directory) and not path.is_symlink()
        assert digest(path) == expected, name
    assert manifest["files"]["UnRAR.txt"] == digest(ROOT / "data/licenses/UnRAR.txt")
    assert manifest["files"]["THIRD_PARTY_LICENSES.md"] == digest(ROOT / "THIRD_PARTY_LICENSES.md")
    for name in ("strata", "strata-media-helper"):
        inspect_elf(directory / name, manifest["target"])
        assert os.access(directory / name, os.X_OK)
        symbols = directory.with_name((directory.name if name == "strata" else directory.name.replace("strata-", "strata-media-helper-", 1)) + ".debug")
        verify_debug(directory / name, symbols)
    return manifest


def install(archive, manifest, launcher, expected=0):
    result = subprocess.run(["/bin/bash", "-c", 'source "$1"; install_bundle "$2" "$3" "$4" "$5"',
        "release-gate", str(ROOT / "install.sh"), str(archive), manifest["release_tag"][1:], manifest["target"], str(launcher)],
        env={"PATH": "/usr/bin:/bin", "STRATA_INSTALLER_TESTING": "1"}, text=True, capture_output=True, check=False)
    assert (result.returncode == 0) == (expected == 0), result.stderr
    return result


def disk_full(root, archive, old_archive, new, old, previous, bundle):
    mount = root / "limited storage"
    mount.mkdir()
    size = sum(path.stat().st_size for path in previous.rglob("*") if path.is_file())
    size += sum(path.stat().st_size for path in bundle.rglob("*") if path.is_file()) // 2 + 2 * 1024 * 1024
    command = ["bwrap", "--unshare-net", "--unshare-pid", "--die-with-parent", "--tmpfs", "/", "--chmod", "0755", "/",
        "--proc", "/proc", "--dev", "/dev", "--symlink", "usr/bin", "/bin", "--symlink", "usr/lib", "/lib"]
    if Path("/lib64").exists():
        command += ["--symlink", "usr/lib64", "/lib64"]
    for path in (Path("/usr"), Path("/etc"), ROOT, root.parent, archive.parent, old_archive.parent):
        command += ["--ro-bind", str(path), str(path)]
    command += ["--size", str(size), "--tmpfs", str(mount), "--chmod", "0700", str(mount), "/bin/bash", "-c",
        'set -eu; source "$1"; install_bundle "$2" "$3" "$4" "$5/strata"; before=$(readlink -f "$5/strata"); '
        'if install_bundle "$6" "$7" "$4" "$5/strata" 2>"$5/error"; then exit 1; fi; '
        'grep -q "No space left on device" "$5/error"; test "$(readlink -f "$5/strata")" = "$before"; test -x "$before"',
        "disk-full", str(ROOT / "install.sh"), str(old_archive), old["release_tag"][1:], old["target"], str(mount),
        str(archive), new["release_tag"][1:]]
    result = subprocess.run(command, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C", "STRATA_INSTALLER_TESTING": "1"},
        text=True, capture_output=True, check=False, timeout=60)
    assert result.returncode == 0, result.stderr


def runtime(binary, output, *extra):
    command = [sys.executable, str(ROOT / "docs/evidence/850/runtime_evidence.py"),
               "--binary", str(binary), "--output", str(output), *extra]
    with output.with_suffix(".log").open("w") as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=300)
    result = json.loads((output / "runtime.json").read_text())
    assert result["completed"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--previous", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cycles", type=int, default=100)
    parser.add_argument("--legacy", type=Path)
    parser.add_argument("--rust-installed", type=Path)
    args = parser.parse_args()
    assert os.geteuid() != 0, "run installation/GUI checks as the unprivileged installation owner"
    bundle, previous, output = args.bundle.resolve(), args.previous.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    new, old = verify(bundle), verify(previous)
    assert new["target"] == old["target"]
    assert (new["release_tag"], new["source_commit"]) != (old["release_tag"], old["source_commit"])
    results = dict(target=new["target"], current=new["source_commit"], previous=old["source_commit"], completed=False)
    archive, old_archive = bundle.with_name(bundle.name + ".tar.gz"), previous.with_name(previous.name + ".tar.gz")
    try:
        if args.rust_installed:
            assert digest(args.rust_installed) == new["files"]["strata"]
            results["native_rust_updater"] = runtime(args.rust_installed, output / "native-rust-updater")
        if args.legacy:
            for tag in ("v0.4.0", "v0.16.0"):
                binary = args.legacy / tag / "strata"
                assert digest(binary) == new["files"]["strata"]
                assert not binary.with_name("strata-media-helper").exists()
                results[f"published_installer_{tag}"] = runtime(binary, output / f"legacy-{tag}")
        with tempfile.TemporaryDirectory(prefix="installed-", dir=output) as temporary:
            root = Path(temporary)
            disk_full(root, archive, old_archive, new, old, previous, bundle)
            results["disk_full_preserved_old"] = True
            launcher = root / "custom install % path/bin/strata"
            install(old_archive, old, launcher)
            old_path = launcher.resolve()
            store = launcher.parent / ".strata-bundles"
            with (store / "install.lock").open("r+b") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                install(archive, new, launcher, expected=1)
                assert launcher.resolve() == old_path
                fcntl.flock(lock, fcntl.LOCK_UN)
            results["competing_install_preserved_old"] = True
            versions = store / "versions"
            versions.chmod(0o500)
            try:
                install(archive, new, launcher, expected=1)
                assert launcher.resolve() == old_path
            finally:
                versions.chmod(0o755)
            results["permission_failure_preserved_old"] = True
            results["running_old_instance"] = runtime(launcher, output / "running-old-video",
                "--activate-archive", str(archive), "--launcher", str(launcher))
            new_path = launcher.resolve()
            assert new_path != old_path and old_path.is_file()
            assert digest(new_path.with_name("strata-media-helper")) == new["files"]["strata-media-helper"]
            results["installed"] = runtime(launcher, output / "installed-video", "--cycles", str(args.cycles), "--pause-release")
            results["pulse_audio"] = runtime(launcher, output / "private-pulse", "--audio-fixture", "audio", "--private-pulse")
            install(old_archive, old, launcher)
            assert launcher.resolve() == old_path
            assert digest(launcher.resolve().with_name("strata-media-helper")) == old["files"]["strata-media-helper"]
            results["rollback"] = runtime(launcher, output / "rollback-video")
            install(archive, new, launcher)
            assert launcher.resolve() == new_path
            results["forward_after_rollback"] = True
            # The published one-file extraction families discard the sidecar. Exercise
            # their filesystem result with the exact finalized UI, offline, not a stub ELF.
            legacy = root / "legacy/bin/strata"
            legacy.parent.mkdir(parents=True)
            shutil.copy2(bundle / "strata", legacy)
            cache, home = root / "legacy/cache", root / "legacy/home"
            cache.mkdir(); home.mkdir()
            environment = {"PATH": "/usr/bin:/bin", "HOME": str(home), "XDG_CACHE_HOME": str(cache)}
            for _ in range(2):
                subprocess.run([str(legacy), "--repair-media-helper"], env=environment, check=True, timeout=30,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            recovered = list(cache.glob("strata-media-recovery/*/strata-media-helper"))
            assert len(recovered) == 1 and digest(recovered[0]) == new["files"]["strata-media-helper"]
            results["offline_single_file_recovery"] = runtime(legacy, output / "single-file-video")
            shutil.copy2(previous / "strata-media-helper", legacy.with_name("strata-media-helper"))
            results["mismatched_sidecar_recovery"] = runtime(legacy, output / "mismatched-sidecar-video")
        results["completed"] = True
    finally:
        (output / "release-gate.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
