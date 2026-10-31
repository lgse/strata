#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Probe filesystem semantics that Strata's file operations depend on.

Usage: fsprobe.py LABEL DIR [--other OTHER_DIR] [--backing SERVER_DIR] [--json OUT.jsonl]

--backing names the server-side directory that DIR is exported from (NFS/SMB/sshfs);
it enables the REMOTE probes, which create targets behind the client's back, and
lets the FLUSH probes count the flushes a share sends to the server's block device.

Creates a fresh subdirectory in DIR (and OTHER_DIR), runs each probe in its
own scratch directory, prints a table, and removes everything it created.
"""
import ctypes
import ctypes.util
import errno
import fcntl
import json
import os
import shutil
import stat
import sys
import tempfile
import time

libc = ctypes.CDLL(None, use_errno=True)
AT_FDCWD = -100
RENAME_NOREPLACE = 1
RENAME_EXCHANGE = 2
RENAME_WHITEOUT = 4
AT_EMPTY_PATH = 0x1000
AT_SYMLINK_FOLLOW = 0x400
FICLONE = 0x40049409
FS_IOC_FSGETXATTR = 0x801C581F
FS_IOC_GETFLAGS = 0x80086601
FS_XFLAG_CASEFOLD = 0x00040000
FS_XFLAG_CASENONPRESERVING = 0x00080000
FS_CASEFOLD_FL = 0x40000000

FS_MAGIC = {
    0x01021994: "tmpfs", 0x9123683E: "btrfs", 0xEF53: "ext2/3/4",
    0x65735546: "fuse", 0x794C7630: "overlayfs", 0x6969: "nfs",
    0xFE534D42: "smb2", 0xFF534D42: "cifs", 0x4D44: "msdos/vfat",
    0x2011BAB0: "exfat", 0x7366746E: "ntfs3", 0x5346544E: "ntfs",
    0x58465342: "xfs", 0x858458F6: "ramfs", 0xF2F52010: "f2fs",
    0x6A656A63: "fakeowner", 0x01021997: "v9fs", 0x73717368: "squashfs",
}


class Statfs(ctypes.Structure):
    _fields_ = [
        ("f_type", ctypes.c_long), ("f_bsize", ctypes.c_long),
        ("f_blocks", ctypes.c_ulong), ("f_bfree", ctypes.c_ulong),
        ("f_bavail", ctypes.c_ulong), ("f_files", ctypes.c_ulong),
        ("f_ffree", ctypes.c_ulong), ("f_fsid", ctypes.c_int * 2),
        ("f_namelen", ctypes.c_long), ("f_frsize", ctypes.c_long),
        ("f_flags", ctypes.c_long), ("f_spare", ctypes.c_long * 4),
    ]


def errname(code):
    return errno.errorcode.get(code, str(code))


def renameat2(old, new, flags):
    if libc.renameat2(AT_FDCWD, os.fsencode(old), AT_FDCWD, os.fsencode(new), flags) != 0:
        e = ctypes.get_errno()
        raise OSError(e, os.strerror(e))


def linkat_empty(fd, new):
    if libc.linkat(fd, b"", AT_FDCWD, os.fsencode(new), AT_EMPTY_PATH) != 0:
        e = ctypes.get_errno()
        raise OSError(e, os.strerror(e))


def fs_type(path):
    buf = Statfs()
    if libc.statfs(os.fsencode(path), ctypes.byref(buf)) != 0:
        return "?"
    magic = buf.f_type & 0xFFFFFFFF
    return f"{FS_MAGIC.get(magic, 'unknown')}(0x{magic:x}) namelen={buf.f_namelen}"


def touch(path, data=b"x"):
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
    os.write(fd, data)
    os.close(fd)


def read(path):
    with open(path, "rb") as f:
        return f.read()


class Probe:
    def __init__(self, root, other=None, backing=None):
        self.root = root
        self.other = other
        self.backing = backing
        self.results = []
        self.counter = 0

    def scratch(self, base=None):
        self.counter += 1
        path = os.path.join(base or self.root, f"p{self.counter:02d}")
        os.mkdir(path)
        return path

    def record(self, name, fn, base=None):
        d = self.scratch(base)
        try:
            detail = fn(d)
            result = "ok" + (f" ({detail})" if detail else "")
        except OSError as error:
            result = errname(error.errno) if error.errno else f"error {error}"
        except AssertionError as error:
            result = f"UNEXPECTED: {error}"
        except Exception as error:  # noqa: BLE001
            result = f"EXC {type(error).__name__}: {error}"
        self.results.append((name, result))

    def run(self):
        r = self.record

        def plain_rename_file(d):
            touch(f"{d}/a")
            os.rename(f"{d}/a", f"{d}/b")
        r("rename(file,new)", plain_rename_file)

        def noreplace_new(d):
            touch(f"{d}/a")
            renameat2(f"{d}/a", f"{d}/b", RENAME_NOREPLACE)
        r("renameat2(file,new,NOREPLACE)", noreplace_new)

        def noreplace_existing(d):
            touch(f"{d}/a", b"a")
            touch(f"{d}/b", b"b")
            try:
                renameat2(f"{d}/a", f"{d}/b", RENAME_NOREPLACE)
            except OSError as error:
                assert read(f"{d}/b") == b"b", "target clobbered"
                raise error
            raise AssertionError("existing target replaced")
        r("renameat2(file,existing,NOREPLACE)", noreplace_existing)

        def noreplace_dir(d):
            os.mkdir(f"{d}/a")
            touch(f"{d}/a/f")
            renameat2(f"{d}/a", f"{d}/b", RENAME_NOREPLACE)
        r("renameat2(dir,new,NOREPLACE)", noreplace_dir)

        def describe(path):
            try:
                st = os.lstat(path)
            except FileNotFoundError:
                return "MISSING"
            if stat.S_ISDIR(st.st_mode):
                return "dir[" + ",".join(sorted(os.listdir(path))) + "]"
            if stat.S_ISCHR(st.st_mode):
                return "chardev"
            return repr(read(path).decode())

        def exchange_files(d):
            touch(f"{d}/a", b"a")
            touch(f"{d}/b", b"b")
            renameat2(f"{d}/a", f"{d}/b", RENAME_EXCHANGE)
            state = f"a={describe(f'{d}/a')} b={describe(f'{d}/b')}"
            if read(f"{d}/b") == b"a" and describe(f"{d}/a") == "'b'":
                return f"swapped: {state}"
            raise AssertionError(f"returned 0 but NOT swapped: {state}; disk={sorted(os.listdir(d))}")
        r("renameat2(file,file,EXCHANGE)", exchange_files)

        def exchange_dirs(d):
            os.mkdir(f"{d}/a")
            touch(f"{d}/a/inA")
            os.mkdir(f"{d}/b")
            touch(f"{d}/b/inB")
            renameat2(f"{d}/a", f"{d}/b", RENAME_EXCHANGE)
            state = f"a={describe(f'{d}/a')} b={describe(f'{d}/b')}"
            if describe(f"{d}/a") == "dir[inB]" and describe(f"{d}/b") == "dir[inA]":
                return f"swapped: {state}"
            raise AssertionError(f"returned 0 but NOT swapped: {state}")
        r("renameat2(dir,dir,EXCHANGE)", exchange_dirs)

        def whiteout(d):
            touch(f"{d}/a")
            renameat2(f"{d}/a", f"{d}/b", RENAME_WHITEOUT)
            return f"returned 0; a={describe(f'{d}/a')} b={describe(f'{d}/b')}"
        r("renameat2(file,new,WHITEOUT)", whiteout)

        def rename_over_file(d):
            touch(f"{d}/a", b"a")
            touch(f"{d}/b", b"b")
            os.rename(f"{d}/a", f"{d}/b")
            assert read(f"{d}/b") == b"a"
        r("rename(file,existing file)", rename_over_file)

        def rename_dir_over_empty(d):
            os.mkdir(f"{d}/a")
            touch(f"{d}/a/f")
            os.mkdir(f"{d}/b")
            os.rename(f"{d}/a", f"{d}/b")
            assert os.path.exists(f"{d}/b/f")
        r("rename(dir,empty dir)", rename_dir_over_empty)

        def rename_dir_over_nonempty(d):
            os.mkdir(f"{d}/a")
            os.mkdir(f"{d}/b")
            touch(f"{d}/b/keep")
            os.rename(f"{d}/a", f"{d}/b")
        r("rename(dir,non-empty dir)", rename_dir_over_nonempty)

        def rename_dir_over_file(d):
            os.mkdir(f"{d}/a")
            touch(f"{d}/b")
            os.rename(f"{d}/a", f"{d}/b")
        r("rename(dir,file)", rename_dir_over_file)

        def rename_file_over_dir(d):
            touch(f"{d}/a")
            os.mkdir(f"{d}/b")
            os.rename(f"{d}/a", f"{d}/b")
        r("rename(file,dir)", rename_file_over_dir)

        def rename_into_self(d):
            os.mkdir(f"{d}/a")
            os.mkdir(f"{d}/a/sub")
            renameat2(f"{d}/a", f"{d}/a/sub/x", RENAME_NOREPLACE)
        r("renameat2(dir,own child,NOREPLACE)", rename_into_self)

        def placeholder_dir_publish(d):
            os.mkdir(f"{d}/stage")
            touch(f"{d}/stage/f")
            os.mkdir(f"{d}/target")
            os.rename(f"{d}/stage", f"{d}/target")
            assert os.path.exists(f"{d}/target/f") and not os.path.exists(f"{d}/stage")
        r("mkdir placeholder + rename(dir,placeholder)", placeholder_dir_publish)

        def placeholder_race(d):
            os.mkdir(f"{d}/stage")
            os.mkdir(f"{d}/target")
            touch(f"{d}/target/intruder")
            os.rename(f"{d}/stage", f"{d}/target")
        r("placeholder filled by racer → rename", placeholder_race)

        def link_file(d):
            touch(f"{d}/a")
            os.link(f"{d}/a", f"{d}/b")
            return f"nlink={os.lstat(f'{d}/a').st_nlink}"
        r("link(file,new)", link_file)

        def link_existing(d):
            touch(f"{d}/a", b"a")
            touch(f"{d}/b", b"b")
            os.link(f"{d}/a", f"{d}/b")
        r("link(file,existing)", link_existing)

        def link_publish_then_unlink(d):
            touch(f"{d}/stage", b"s")
            os.link(f"{d}/stage", f"{d}/target")
            os.unlink(f"{d}/stage")
            assert read(f"{d}/target") == b"s"
        r("link publish + unlink stage", link_publish_then_unlink)

        def link_dir(d):
            os.mkdir(f"{d}/a")
            os.link(f"{d}/a", f"{d}/b")
        r("link(dir,new)", link_dir)

        def symlink(d):
            os.symlink("target-does-not-exist", f"{d}/l")
            return f"readlink={os.readlink(f'{d}/l')}"
        r("symlink", symlink)

        def tmpfile(d):
            fd = os.open(d, os.O_TMPFILE | os.O_WRONLY, 0o600)
            try:
                os.write(fd, b"t")
                os.link(f"/proc/self/fd/{fd}", f"{d}/published", follow_symlinks=True)
            finally:
                os.close(fd)
            assert read(f"{d}/published") == b"t"
        r("O_TMPFILE + linkat(/proc/self/fd,FOLLOW)", tmpfile)

        def tmpfile_empty_path(d):
            fd = os.open(d, os.O_TMPFILE | os.O_WRONLY, 0o600)
            try:
                linkat_empty(fd, f"{d}/published")
            finally:
                os.close(fd)
        r("O_TMPFILE + linkat(AT_EMPTY_PATH)", tmpfile_empty_path)

        def proc_fd_link_named(d):
            touch(f"{d}/stage", b"s")
            fd = os.open(f"{d}/stage", os.O_RDONLY)
            try:
                os.link(f"/proc/self/fd/{fd}", f"{d}/published", follow_symlinks=True)
            finally:
                os.close(fd)
        r("linkat(/proc/self/fd of named file,FOLLOW)", proc_fd_link_named)

        def excl_over_dir(d):
            os.mkdir(f"{d}/x")
            fd = os.open(f"{d}/x", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
            os.close(fd)
            return "created over a directory"
        r("O_CREAT|O_EXCL over existing dir", excl_over_dir)

        def excl_over_dangling(d):
            os.symlink("missing", f"{d}/l")
            fd = os.open(f"{d}/l", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
            os.close(fd)
            return "created through a dangling symlink"
        r("O_CREAT|O_EXCL over dangling symlink", excl_over_dangling)

        def case_lookup(d):
            touch(f"{d}/Foo")
            try:
                os.lstat(f"{d}/foo")
                return "case-INSENSITIVE"
            except FileNotFoundError:
                return "case-sensitive"
        r("lookup other case", case_lookup)

        def casefold_flags(d):
            fd = os.open(d, os.O_RDONLY | os.O_DIRECTORY)
            out = []
            try:
                buf = bytearray(28)
                try:
                    fcntl.ioctl(fd, FS_IOC_FSGETXATTR, buf)
                    xflags = int.from_bytes(buf[:4], "little")
                    names = [n for n, bit in (("CASEFOLD", FS_XFLAG_CASEFOLD),
                                              ("CASENONPRESERVING", FS_XFLAG_CASENONPRESERVING)) if xflags & bit]
                    out.append("xflags=" + ("|".join(names) or "none"))
                except OSError as error:
                    out.append(f"FSGETXATTR {errname(error.errno)}")
                flags = bytearray(8)
                try:
                    fcntl.ioctl(fd, FS_IOC_GETFLAGS, flags)
                    value = int.from_bytes(flags[:4], "little")
                    out.append("GETFLAGS " + ("CASEFOLD_FL" if value & FS_CASEFOLD_FL else "no CASEFOLD_FL"))
                except OSError as error:
                    out.append(f"GETFLAGS {errname(error.errno)}")
            finally:
                os.close(fd)
            return ", ".join(out)
        r("casefold flags on dir (FSGETXATTR/GETFLAGS)", casefold_flags)

        def case_rename_plain(d):
            touch(f"{d}/Foo")
            os.rename(f"{d}/Foo", f"{d}/foo")
            return "listing=" + ",".join(sorted(os.listdir(d)))
        r("rename(Foo,foo)", case_rename_plain)

        def case_rename_noreplace(d):
            touch(f"{d}/Foo")
            renameat2(f"{d}/Foo", f"{d}/foo", RENAME_NOREPLACE)
            return "listing=" + ",".join(sorted(os.listdir(d)))
        r("renameat2(Foo,foo,NOREPLACE)", case_rename_noreplace)

        def ficlone(d):
            touch(f"{d}/a", b"clone-me" * 1024)
            src = os.open(f"{d}/a", os.O_RDONLY)
            dst = os.open(f"{d}/b", os.O_CREAT | os.O_WRONLY, 0o644)
            try:
                fcntl.ioctl(dst, FICLONE, src)
            finally:
                os.close(src)
                os.close(dst)
        r("FICLONE same dir", ficlone)

        def cfr(d):
            payload = b"y" * (1 << 20)
            touch(f"{d}/a", payload)
            src = os.open(f"{d}/a", os.O_RDONLY)
            dst = os.open(f"{d}/b", os.O_CREAT | os.O_WRONLY, 0o644)
            try:
                n = os.copy_file_range(src, dst, len(payload))
            finally:
                os.close(src)
                os.close(dst)
            return f"copied={n}"
        r("copy_file_range 1MiB", cfr)

        def chmod(d):
            touch(f"{d}/a")
            os.chmod(f"{d}/a", 0o600)
            m1 = stat.S_IMODE(os.lstat(f"{d}/a").st_mode)
            os.chmod(f"{d}/a", 0o755)
            m2 = stat.S_IMODE(os.lstat(f"{d}/a").st_mode)
            return f"0600->{m1:o} 0755->{m2:o}"
        r("chmod", chmod)

        def utime(d):
            touch(f"{d}/a")
            want = 1_600_000_001_123_456_789
            os.utime(f"{d}/a", ns=(want, want))
            got = os.lstat(f"{d}/a").st_mtime_ns
            return f"mtime delta_ns={got - want}"
        r("utime ns precision", utime)

        def xattr(d):
            touch(f"{d}/a")
            os.setxattr(f"{d}/a", "user.strata", b"v")
            return f"get={os.getxattr(f'{d}/a', 'user.strata')!r}"
        r("setxattr user.*", xattr)

        def inode_stable(d):
            touch(f"{d}/a")
            os.mkdir(f"{d}/sub")
            before = os.lstat(f"{d}/a")
            os.rename(f"{d}/a", f"{d}/sub/a")
            after = os.lstat(f"{d}/sub/a")
            return f"ino {'same' if before.st_ino == after.st_ino else 'CHANGED'}, dev {'same' if before.st_dev == after.st_dev else 'CHANGED'}"
        r("(dev,ino) across rename", inode_stable)

        def unlink_open(d):
            touch(f"{d}/a")
            fd = os.open(f"{d}/a", os.O_RDONLY)
            try:
                os.unlink(f"{d}/a")
                leftovers = os.listdir(d)
            finally:
                os.close(fd)
            after = os.listdir(d)
            time.sleep(2)
            return f"while-open={leftovers} after-close={after} after-2s={os.listdir(d)}"
        r("unlink open file", unlink_open)

        def rename_over_open(d):
            touch(f"{d}/a")
            touch(f"{d}/b")
            fd = os.open(f"{d}/b", os.O_RDONLY)
            try:
                os.rename(f"{d}/a", f"{d}/b")
                leftovers = sorted(os.listdir(d))
            finally:
                os.close(fd)
            after = sorted(os.listdir(d))
            time.sleep(2)
            return f"while-open={leftovers} after-close={after} after-2s={sorted(os.listdir(d))}"
        r("rename over open file", rename_over_open)

        def fat_chars(d):
            bad = []
            for ch in [":", "\\", "?", "*", "<", ">", "|", '"']:
                try:
                    touch(f"{d}/x{ch}y")
                except OSError as error:
                    bad.append(f"{ch}={errname(error.errno)}")
            for name in ["trail.", "trail ", "CON", "aux.txt"]:
                try:
                    touch(f"{d}/{name}")
                    listed = name in os.listdir(d)
                    if not listed:
                        bad.append(f"{name!r}=renamed-on-disk")
                except OSError as error:
                    bad.append(f"{name!r}={errname(error.errno)}")
            return "all accepted" if not bad else "rejected " + " ".join(bad)
        r("Windows-reserved names", fat_chars)

        def long_name(d):
            out = []
            for label, name in [("255B ascii", "a" * 255), ("256B ascii", "a" * 256),
                                ("85x3B utf8=255B", "€" * 85), ("120x3B utf8=360B", "€" * 120)]:
                try:
                    touch(f"{d}/{name}")
                    out.append(f"{label}:ok")
                except OSError as error:
                    out.append(f"{label}:{errname(error.errno)}")
            return " ".join(out)
        r("name length", long_name)

        def fsync_dir(d):
            fd = os.open(d, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        r("fsync(dir)", fsync_dir)

        def dir_nlink(d):
            os.mkdir(f"{d}/x")
            os.mkdir(f"{d}/x/1")
            return f"nlink={os.lstat(f'{d}/x').st_nlink}"
        r("dir st_nlink (2 + subdirs?)", dir_nlink)

        def rmdir_open_unlinked_child(d):
            os.mkdir(f"{d}/x")
            touch(f"{d}/x/f")
            fd = os.open(f"{d}/x/f", os.O_RDONLY)
            try:
                os.unlink(f"{d}/x/f")
                left = sorted(os.listdir(f"{d}/x"))
                try:
                    os.rmdir(f"{d}/x")
                    outcome = "rmdir ok"
                except OSError as error:
                    outcome = f"rmdir {errname(error.errno)}"
            finally:
                os.close(fd)
            return f"{outcome}, leftovers={left}"
        r("rmdir after unlinking an open child", rmdir_open_unlinked_child)

        def case_rename_two_step(d):
            touch(f"{d}/Foo")
            os.rename(f"{d}/Foo", f"{d}/.case-step")
            os.rename(f"{d}/.case-step", f"{d}/foo")
            return "listing=" + ",".join(sorted(os.listdir(d)))
        r("two-step rename Foo->tmp->foo", case_rename_two_step)

        def excl_other_case(d):
            touch(f"{d}/Foo")
            touch(f"{d}/foo")
            return "listing=" + ",".join(sorted(os.listdir(d)))
        r("O_EXCL create foo when Foo exists", excl_other_case)

        self.run_flush()

        if self.backing:
            self.run_remote()

        if self.other:
            self.run_cross()

    def backing_of(self, d):
        return os.path.join(self.backing, os.path.relpath(d, os.path.dirname(self.root)))

    def run_remote(self):
        """Create the target on the server's backing filesystem, behind the client."""
        r = self.record

        def remote(name, fn):
            def wrapped(d):
                return fn(d, self.backing_of(d))
            r(name, wrapped)

        def stage_negative(d, bd, data=b"remote"):
            touch(f"{d}/a", b"src")
            try:
                os.lstat(f"{d}/b")
            except FileNotFoundError:
                pass
            touch(f"{bd}/b", data)

        def outcome(bd):
            return "server copy " + ("KEPT" if read(f"{bd}/b") == b"remote" else "CLOBBERED")

        def r_noreplace(d, bd):
            stage_negative(d, bd)
            try:
                renameat2(f"{d}/a", f"{d}/b", RENAME_NOREPLACE)
            except OSError as error:
                return f"{errname(error.errno)}, {outcome(bd)}"
            return f"succeeded, {outcome(bd)}"
        remote("REMOTE renameat2(NOREPLACE) after server-side create", r_noreplace)

        def r_link(d, bd):
            stage_negative(d, bd)
            try:
                os.link(f"{d}/a", f"{d}/b")
            except OSError as error:
                return f"{errname(error.errno)}, {outcome(bd)}"
            return f"succeeded, {outcome(bd)}"
        remote("REMOTE link after server-side create", r_link)

        def r_excl(d, bd):
            stage_negative(d, bd)
            try:
                fd = os.open(f"{d}/b", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
                os.close(fd)
            except OSError as error:
                return f"{errname(error.errno)}, {outcome(bd)}"
            return f"succeeded, {outcome(bd)}"
        remote("REMOTE O_EXCL create after server-side create", r_excl)

        def r_mkdir_placeholder(d, bd):
            os.mkdir(f"{d}/stage")
            touch(f"{d}/stage/f")
            try:
                os.lstat(f"{d}/target")
            except FileNotFoundError:
                pass
            os.mkdir(f"{bd}/target")
            touch(f"{bd}/target/remote")
            try:
                os.mkdir(f"{d}/target")
            except OSError as error:
                return f"mkdir {errname(error.errno)}"
            return "mkdir succeeded over a server-side directory"
        remote("REMOTE mkdir placeholder after server-side mkdir", r_mkdir_placeholder)

    def block_device(self, d):
        """The block device DIR is stored on, else the one behind its server share."""
        for path in filter(None, [d, self.backing and self.backing_of(d)]):
            st = os.stat(path)
            node = f"/sys/dev/block/{os.major(st.st_dev)}:{os.minor(st.st_dev)}"
            if os.path.exists(node):
                name = os.path.basename(os.path.realpath(node))
                return (name if path == d else f"server {name}"), f"{node}/stat"
        return None

    def run_flush(self):
        """Count the device cache flushes each sync call sends after a little data is written."""
        rounds = 3

        def counters(stat_path):
            # Documentation/block/stat.rst: field 5 is write I/Os, 16 and 17 flush I/Os and ms.
            fields = [int(value) for value in read(stat_path).split()]
            if len(fields) < 17:
                raise AssertionError("the kernel reports no flush counters")
            return fields[4], fields[15], fields[16]

        def measure(sync):
            def probe(d):
                device = self.block_device(d)
                if device is None:
                    return "no block device"
                label, stat_path = device
                flushes, writes, ms = [], [], 0
                for n in range(rounds):
                    dir_fd = os.open(d, os.O_RDONLY | os.O_DIRECTORY)
                    fd = os.open(f"{d}/f{n}", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
                    try:
                        os.write(fd, os.urandom(16384))
                        before = counters(stat_path)
                        if sync is None:
                            os.close(fd)
                            fd = None
                        else:
                            sync(fd, dir_fd)
                        after = counters(stat_path)
                    finally:
                        if fd is not None:
                            os.close(fd)
                        os.close(dir_fd)
                    writes.append(str(after[0] - before[0]))
                    flushes.append(str(after[1] - before[1]))
                    ms += after[2] - before[2]
                return f"flushes {','.join(flushes)} ({ms} ms), writes {','.join(writes)} on {label}"
            return probe

        def syncfs(_fd, dir_fd):
            if libc.syncfs(dir_fd) != 0:
                e = ctypes.get_errno()
                raise OSError(e, os.strerror(e))

        self.record("FLUSH close, no sync", measure(None))
        self.record("FLUSH syncfs", measure(syncfs))
        self.record("FLUSH fsync(dir)", measure(lambda _fd, dir_fd: os.fsync(dir_fd)))
        self.record("FLUSH fdatasync(file)", measure(lambda fd, _dir_fd: os.fdatasync(fd)))

    def run_cross(self):
        r = self.record
        other = self.other

        def cross(name, fn):
            def wrapped(d):
                od = self.scratch(other)
                return fn(d, od)
            r(name, wrapped)

        def x_rename(d, od):
            touch(f"{d}/a")
            os.rename(f"{d}/a", f"{od}/a")
        cross("X rename(file) to OTHER", x_rename)

        def x_rename_noreplace(d, od):
            os.mkdir(f"{d}/a")
            renameat2(f"{d}/a", f"{od}/a", RENAME_NOREPLACE)
        cross("X renameat2(dir,NOREPLACE) to OTHER", x_rename_noreplace)

        def x_link(d, od):
            touch(f"{d}/a")
            os.link(f"{d}/a", f"{od}/a")
        cross("X link to OTHER", x_link)

        def x_ficlone(d, od):
            touch(f"{d}/a", b"clone-me" * 1024)
            src = os.open(f"{d}/a", os.O_RDONLY)
            dst = os.open(f"{od}/b", os.O_CREAT | os.O_WRONLY, 0o644)
            try:
                fcntl.ioctl(dst, FICLONE, src)
            finally:
                os.close(src)
                os.close(dst)
        cross("X FICLONE to OTHER", x_ficlone)

        def x_cfr(d, od):
            payload = b"y" * (1 << 20)
            touch(f"{d}/a", payload)
            src = os.open(f"{d}/a", os.O_RDONLY)
            dst = os.open(f"{od}/b", os.O_CREAT | os.O_WRONLY, 0o644)
            try:
                n = os.copy_file_range(src, dst, len(payload))
            finally:
                os.close(src)
                os.close(dst)
            return f"copied={n}"
        cross("X copy_file_range to OTHER", x_cfr)

        def x_dev(d, od):
            a, b = os.lstat(d), os.lstat(od)
            return f"st_dev {'same' if a.st_dev == b.st_dev else 'differs'} ({a.st_dev:#x} vs {b.st_dev:#x})"
        cross("X st_dev compare", x_dev)


def main():
    args = sys.argv[1:]
    out = None
    other = None
    if "--json" in args:
        i = args.index("--json")
        out = args[i + 1]
        del args[i:i + 2]
    backing = None
    if "--backing" in args:
        i = args.index("--backing")
        backing = args[i + 1]
        del args[i:i + 2]
    if "--other" in args:
        i = args.index("--other")
        other = args[i + 1]
        del args[i:i + 2]
    label, base = args
    root = tempfile.mkdtemp(prefix=".strata-fsprobe-", dir=base)
    other_root = tempfile.mkdtemp(prefix=".strata-fsprobe-", dir=other) if other else None
    probe = Probe(root, other_root, backing)
    try:
        probe.run()
    finally:
        for path in filter(None, [root, other_root]):
            shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                # SMB deferred close and NFS async silly-delete outlive close().
                time.sleep(3)
                shutil.rmtree(path, ignore_errors=True)
            if os.path.exists(path):
                print(f"WARNING: could not remove {path}", file=sys.stderr)
    header = f"== {label}: {base} [{fs_type(base)}]" + (f" OTHER={other} [{fs_type(other)}]" if other else "")
    print(header)
    width = max(len(n) for n, _ in probe.results)
    for name, result in probe.results:
        print(f"  {name:<{width}}  {result}")
    if out:
        with open(out, "a") as f:
            for name, result in probe.results:
                f.write(json.dumps({"label": label, "fs": fs_type(base), "op": name, "result": result}) + "\n")


if __name__ == "__main__":
    main()
