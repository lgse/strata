# SPDX-License-Identifier: MIT

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "trace_main_io", Path(__file__).with_name("trace-main-io.py")
)
AUDIT = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = AUDIT
SPEC.loader.exec_module(AUDIT)

ENVIRONMENT = "/tmp/strata-qa-home-x"
FIXTURE = "/tmp/strata-qa-fixture-x"
RUNTIME = "/tmp/strata-qa-runtime"
SHADERS = "/tmp/strata-qa-shaders-x"
LOG = "/tmp/strata-qa-x/strata.log"


class MainThreadAuditTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.allowlist = self.root / "allowlist.toml"

    def launch(self, trace, log=""):
        directory = self.root / "run" / "scenario" / "launch-1"
        directory.mkdir(parents=True)
        meta = {
            "scenario": "example-scenario/columns",
            "fixture": FIXTURE,
            "environment": ENVIRONMENT,
            "home": f"{ENVIRONMENT}/home",
            "config": f"{ENVIRONMENT}/config",
            "data": f"{ENVIRONMENT}/data",
            "cache": f"{ENVIRONMENT}/cache",
            "state": f"{ENVIRONMENT}/state",
            "runtime": RUNTIME,
            "data_dirs": [f"{ENVIRONMENT}/data-dirs", "/usr/share"],
            "binary": "/workspace/target/build/debug/strata",
            "harness": [SHADERS, LOG],
            "started": 100.0,
            "ready": 200.0,
        }
        (directory / "meta.json").write_text(json.dumps(meta))
        (directory / "trace").write_text("".join(line + "\n" for line in trace))
        (directory / "strata.log").write_text(log)
        return self.root / "run"

    def violations(self, run):
        report = AUDIT.analyze(run, AUDIT.load_allowlist(AUDIT.ALLOWLIST))
        return report, sorted(
            (entry["category"], entry["phase"], entry["call"], entry["access"], tuple(entry["paths"]))
            for entry in report.as_json()
        )

    def test_only_user_and_settings_calls_outside_documented_exceptions_are_violations(self):
        cwd = f"AT_FDCWD<{FIXTURE}>"
        run = self.launch([
            f'150.0 openat({cwd}, "{ENVIRONMENT}/config/strata/settings.toml", O_RDONLY|O_CLOEXEC) = 3<{ENVIRONMENT}/config/strata/settings.toml>',
            f'150.1 statx({cwd}, "{ENVIRONMENT}/home", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = 0',
            f'150.2 newfstatat({cwd}, "/tmp", {{st_mode=S_IFDIR|0777, ...}}, 0) = 0',
            f'150.3 readlink("/workspace/target", 0x7ffe, 1023) = -1 EINVAL (Invalid argument)',
            f'150.4 inotify_add_watch(3<anon_inode:inotify>, "{ENVIRONMENT}/home", IN_MODIFY) = 4',
            f'150.5 inotify_add_watch(3<anon_inode:inotify>, "{ENVIRONMENT}/config/strata", IN_MODIFY) = 6',
            f'150.6 statfs("{ENVIRONMENT}/home", {{f_type=TMPFS_MAGIC, ...}}) = 0',
            f'150.7 statx({cwd}, "{ENVIRONMENT}/home/.local/state/omarchy", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = -1 ENOENT (No such file or directory)',
            f'150.8 mkdir("{ENVIRONMENT}/cache/strata/fonts", 0777) = 0',
            f'201.0 openat({cwd}, "/usr/share/icons/index.theme", O_RDONLY) = 7</usr/share/icons/index.theme>',
            f'201.1 read(5<anon_inode:[eventfd]>, ""..., 8) = 8',
            f'201.2 statx({cwd}, "{FIXTURE}/new file (1)", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = -1 ENOENT (No such file or directory)',
            f'201.3 openat(9<{FIXTURE}/Documents>, "notes.txt", O_RDONLY|O_CLOEXEC) = 10<{FIXTURE}/Documents/notes.txt>',
            f'201.4 close(10<{FIXTURE}/Documents/notes.txt>) = 0',
            f'201.5 openat({cwd}, "{ENVIRONMENT}/config/strata/actions", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)',
            f'201.6 openat({cwd}, "{ENVIRONMENT}/state/strata/tabs.toml", O_WRONLY|O_CREAT|O_TRUNC, 0644) = 11<{ENVIRONMENT}/state/strata/tabs.toml>',
            f'201.7 renameat2({cwd}, "{FIXTURE}/a", {cwd}, "{FIXTURE}/b", RENAME_NOREPLACE) = 0',
            f'201.8 newfstatat({cwd}, "resource:///org/gnome/gtksourceview/styles/", 0x7ffe, 0) = -1 ENOENT (No such file or directory)',
            f'201.9 getdents64(12<{ENVIRONMENT}/data/Trash/files>, 0x55 /* 2 entries */, 32768) = 48',
            f'202.0 write(1<{ENVIRONMENT}/strata.log>, ""..., 120) = 120',
            f'202.0 write(2<{ENVIRONMENT}/portal.log>, ""..., 96) = 96',
            f'202.1 mkdir("{RUNTIME}/strata/actions/7-0", 0700) = 0',
            f'202.2 openat({cwd}, "{RUNTIME}/pulse/native", O_RDONLY) = -1 ENOENT (No such file or directory)',
            f'202.3 openat({cwd}, "{ENVIRONMENT}/data-dirs/applications/mimeinfo.cache", O_RDONLY) = 6<{ENVIRONMENT}/data-dirs/applications/mimeinfo.cache>',
            f'202.4 inotify_add_watch(3<anon_inode:inotify>, "{ENVIRONMENT}/config", IN_MODIFY) = 5',
            f'202.5 access("{ENVIRONMENT}/cache/strata/fonts/2.304/JetBrainsMono.ttf", R_OK) = 0',
            f'202.6 openat({cwd}, "{ENVIRONMENT}/cache/strata/fonts/2.304/.strata-write-7-0.tmp", O_WRONLY|O_CREAT|O_EXCL, 0600) = 13<{ENVIRONMENT}/cache/strata/fonts/2.304/.strata-write-7-0.tmp>',
            f'202.7 access("{ENVIRONMENT}/record-files", X_OK) = 0',
            f'202.8 newfstatat({cwd}, "{ENVIRONMENT}/record-files", {{st_mode=S_IFREG|0755, ...}}, 0) = 0',
            f'202.9 openat({cwd}, "{ENVIRONMENT}/record-files", O_RDONLY) = 14<{ENVIRONMENT}/record-files>',
            f'203.0 statx({cwd}, "{RUNTIME}/gvfs/sftp:host=nas/report.txt", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = 0',
            f'203.1 read(15<{FIXTURE}/notes (draft.txt>, ""..., 8192) = 12',
            f'203.2 openat(16<{FIXTURE}/a [b>, "child", O_RDONLY|O_CLOEXEC) = -1 ENOENT (No such file or directory)',
            f'203.3 fstat(17<{FIXTURE}/h \\303\\251>, {{st_mode=S_IFREG|0644, ...}}) = 0',
            f'203.4 statx({cwd}, "{FIXTURE}/h \\303\\251", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = 0',
            f'203.5 close(18<{FIXTURE}/say \\"hi\\" {{\\76>) = 0',
            f'203.6 openat({cwd}, "{ENVIRONMENT}/data/gtksourceview-5/language-specs", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)',
            f'203.7 mkdir("{ENVIRONMENT}/cache/gstreamer-1.0", 0777) = 0',
            f'203.8 openat({cwd}, "{SHADERS}/mesa_shader_cache/ab", O_RDONLY) = 19<{SHADERS}/mesa_shader_cache/ab>',
            f'203.9 fstat(2<{LOG}>, {{st_mode=S_IFREG|0644, ...}}) = 0',
        ])
        _, violations = self.violations(run)
        self.assertEqual(violations, sorted([
            ("user", "startup", "inotify_add_watch", "watch", ("{home}",)),
            ("settings", "startup", "inotify_add_watch", "watch", ("{config}/strata",)),
            ("settings", "startup", "statfs", "read", ("{home}",)),
            ("user", "running", "statx", "read", ("{fixture}/new file (1)",)),
            ("user", "running", "openat", "read", ("{fixture}/Documents/notes.txt",)),
            ("user", "running", "close", "read", ("{fixture}/Documents/notes.txt",)),
            ("user", "running", "renameat2", "write", ("{fixture}/a", "{fixture}/b")),
            ("user", "running", "getdents64", "read", ("{data}/Trash/files",)),
            ("settings", "running", "openat", "read", ("{config}/strata/actions",)),
            ("settings", "running", "openat", "write", ("{state}/strata/tabs.toml",)),
            ("settings", "running", "mkdir", "write", ("{runtime}/strata/actions/7-0",)),
            ("settings", "running", "openat", "write", ("{cache}/strata/fonts/2.304/.strata-write-7-0.tmp",)),
            ("unclassified", "running", "openat", "read", ("{environment}/record-files",)),
            ("user", "running", "statx", "read", ("{runtime}/gvfs/sftp:host=nas/report.txt",)),
            ("user", "running", "read", "read", ("{fixture}/notes (draft.txt",)),
            ("user", "running", "openat", "read", ("{fixture}/a [b/child",)),
            ("user", "running", "fstat", "read", ("{fixture}/h \u00e9",)),
            ("user", "running", "statx", "read", ("{fixture}/h \u00e9",)),
            ("user", "running", "close", "read", ('{fixture}/say "hi" {>',)),
            ("user", "running", "openat", "read", ("{data}/gtksourceview-5/language-specs",)),
            ("user", "running", "mkdir", "write", ("{cache}/gstreamer-1.0",)),
        ]))

    def test_stack_frames_and_watchdog_stalls_are_reported_with_the_call(self):
        tools = self.root / "bin"
        tools.mkdir()
        for name, output in (
            ("readelf", "  LOAD 0x001000 0x0000000000002000 0x0000000000002000 0x005000 0x005000 R E 0x1000"),
            ("nm", "0000000000003000 t strata::ui::window::open_checked\n"
                   "0000000000004000 t <strata::ui::browser::Pane>::reload"),
        ):
            tool = tools / name
            tool.write_text(f"#!/bin/sh\ncat <<'EOF'\n{output}\nEOF\n")
            tool.chmod(0o755)
        run = self.launch([
            f'201.0 statx(AT_FDCWD<{FIXTURE}>, "{FIXTURE}/x", AT_STATX_SYNC_AS_STAT, STATX_ALL, 0x7ffe) = 0',
            " > /usr/lib/libc.so.6(statx+0xe) [0xf00e]",
            " > /workspace/target/build/debug/strata(strata::ui::browser::naming::free_name+0x41) [0x1041]",
            " > /workspace/target/build/debug/strata(+0x0) [0x2010]",
            " > /workspace/target/build/debug/strata(+0x0) [0x3020]",
            " > /usr/lib/libglib-2.0.so.0(g_main_context_dispatch+0x10) [0x2010]",
        ], log="WARN UI thread stalled 250 ms in statx(\"/tmp/x\") wchan=-\nWARN UI thread ran again after 300 ms\n")
        with patch.dict(os.environ, {"PATH": f"{tools}:{os.environ.get('PATH', os.defpath)}"}):
            report, _ = self.violations(run)
        [entry] = report.as_json()
        self.assertEqual(entry["stacks"], [(
            "strata::ui::browser::naming::free_name+0x41",
            "strata::ui::window::open_checked+0x10",
            "<strata::ui::browser::Pane>::reload+0x20",
        )])
        self.assertEqual(report.stalls, {"example-scenario/columns": [
            'UI thread stalled 250 ms in statx("/tmp/x") wchan=-',
            "UI thread ran again after 300 ms",
        ]})

    def test_the_allowlist_holds_only_documented_exceptions_outside_user_paths(self):
        for entry, message in [
            ('reason = "x"\ncategory = "user"\npaths = ["{fixture}/*"]', "user paths"),
            ('category = "settings"', "documented reason"),
            ('reason = "x"\ncategory = "settings"\nphase = "later"', "phase"),
        ]:
            with self.subTest(message=message):
                self.allowlist.write_text(f"[[exception]]\n{entry}\n")
                with self.assertRaisesRegex(ValueError, message):
                    AUDIT.load_allowlist(self.allowlist)
        self.assertTrue(AUDIT.load_allowlist(AUDIT.ALLOWLIST))


if __name__ == "__main__":
    unittest.main()
