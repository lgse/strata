# Cross-volume trash restore fixture

Manual test harness for Strata issue [#478](https://github.com/lgse/strata/issues/478): restoring items that GVfs stored in a volume trash directory (`.Trash-$UID` on a distinct mount), and refusing restores whose `Path=` / `trash::orig-path` points at a different volume.

GVfs names volume-trash entries after their escaped physical path (`trash:///%5Crun%5Cmedia%5C…`) and writes **relative** `Path=` values. Strata must resolve the physical `files/` entry through `standard::target-uri`, then only move the item if the destination is on the same volume as that entry.

This script plants labeled items into the session trash so you can exercise that in the running app. It does not hard-code a machine, user, or disk: home comes from `HOME` / `XDG_DATA_HOME`, extra volumes are passed with `--volume` or auto-detected under `/run/media/$USER` and `/media/$USER`, and reject orig-paths that need a third filesystem use the process temp dir (`TMPDIR`, else `/tmp`).

## Prerequisites

- `gio` (GLib)
- PyGObject (`gi.repository.Gio`) so the script can list `trash:///` after planting
- A **writable extra mount** that can host `.Trash-$UID` (typically removable media)
- That mount must be a different filesystem from home (`st_dev` / GIO `id::filesystem`)

`gio trash` will not use volume trash on system-internal mounts such as tmpfs. It also cannot create `.Trash-$UID` on a root-owned mountpoint. Those cases are covered by crafted `Path=` values, not by trashing files from those locations.

## Quick start

From the repository root:

```bash
./scripts/trash-restore-fixture/trash_restore_fixture.py plant --volume /run/media/$USER/<disk>
```

If exactly one writable extra volume is mounted under `/run/media/$USER` or `/media/$USER`, `--volume` can be omitted and that mount is used. If none or several are found, pass `--volume` explicitly.

Open **Trash** in Strata. Items are prefixed `478-` by default (overridable with `--prefix`).

1. Restore one `RESTORE` item and confirm it returns to the destination in the dialog.
2. Restore one `REJECT` item and confirm Strata refuses (nothing appears in the reject sinks).
3. Multi-select a mix of `RESTORE` and `REJECT`.
4. Restore both `RESTORE-collision.txt` rows; the confirm dialog destinations distinguish the extra-volume copy from the home copy.

A markdown summary (destinations, GVfs orig-paths, on-disk `.trashinfo`) is written to `<tmp-dir>/<label>-trash-fixture.md`.

```bash
./scripts/trash-restore-fixture/trash_restore_fixture.py status --volume /run/media/$USER/<disk>
./scripts/trash-restore-fixture/trash_restore_fixture.py clean --volume /run/media/$USER/<disk>
```

Use the same `--prefix`, `--label`, and `--volume` on `status` and `clean` that you used for `plant`.

## What gets planted

Names below use the default prefix `478` and label `strata-478`. Substitute whatever you passed to `--prefix` / `--label`.

| Kind | Expect | Notes |
| --- | --- | --- |
| `<prefix>-RESTORE-volume-file.txt` | Restore | Authentic GVfs volume trash, relative `Path=` |
| `<prefix>-RESTORE-volume spaces.txt` | Restore | Spaces; GVfs percent-encodes `Path=` |
| `<prefix>-RESTORE-nested.txt` | Restore | Nested relative path; parent dirs are recreated |
| `<prefix>-RESTORE-album/` | Restore | Directory on the extra volume |
| `<prefix>-RESTORE-collision.txt` (volume) | Restore | Same display name as the home copy; must land on the extra volume |
| `<prefix>-RESTORE-home-file.txt` | Restore | Control: home trash → home |
| `<prefix>-RESTORE-collision.txt` (home) | Restore | Must land under `~/<label>-home/restore-here/` |
| `<prefix>-REJECT-volume-to-home.txt` | Reject | Volume `files/`, `Path=` on home |
| `<prefix>-REJECT-volume-to-tmp.txt` | Reject | Volume `files/`, `Path=` in `--tmp-dir` |
| `<prefix>-REJECT-volume-symlink.txt` | Reject | Relative `Path=` through a symlink from the volume into home |
| `<prefix>-REJECT-volume-dotdot.txt` | Reject | Percent-encoded `../` trying to leave the volume topdir |
| `<prefix>-REJECT-home-to-volume.txt` | Reject | Home `files/`, `Path=` on the extra volume |
| `<prefix>-REJECT-home-to-tmp.txt` | Reject | Home `files/`, `Path=` in `--tmp-dir` |
| `<prefix>-MISSING-PARENT.txt` | Lookup may succeed; move should fail | Same-volume dest whose parent directory does not exist |

Successful restores land in:

- `<volume>/<label>/restore-here/`
- `~/<label>-home/restore-here/`

Rejects must not create files in:

- `~/<label>-home/reject-sink/`
- `<volume>/<label>/reject-sink/`
- `~/<label>-home-sink/`
- `<tmp-dir>/<prefix>-REJECT-*`

Unrelated trash on the extra volume is left alone.

After planting reject items, the script restarts `gvfsd-trash` so `trash::orig-path` is re-read from the rewritten `.trashinfo` files. Pass `--no-restart-gvfs` if you need to skip that.

## Commands and flags

```text
plant    create trash entries and a manifest
status   list fixture items in trash and GVfs
clean    remove fixture trash items and test directories
```

Shared:

| Flag | Default | Meaning |
| --- | --- | --- |
| `--volume PATH` | auto-detect if unique | Extra filesystem that can host `.Trash-$UID` |
| `--prefix STR` | `478` | Item name prefix |
| `--label STR` | `strata-478` | Directory name on the volume and under home |
| `--home-dir PATH` | `~/<label>-home` | Home-side restore tree |
| `--home-sink PATH` | `~/<label>-home-sink` | Symlink-escape target on home |
| `--tmp-dir PATH` | process temp dir | Directory used for off-volume reject orig-paths |
| `--manifest PATH` | `<tmp-dir>/<label>-trash-fixture.md` | Plant summary |

`plant` only:

| Flag | Meaning |
| --- | --- |
| `--only {all,restore,reject}` | Plant only same-volume restores, only cross-volume rejects, or both |
| `--force` | Replace an existing fixture with the same prefix/label |
| `--restart-gvfs` / `--no-restart-gvfs` | Restart `gvfsd-trash` after rewriting `Path=` (default: yes) |
| `--dry-run` | Print planned items without writing trash |

`clean` only:

| Flag | Meaning |
| --- | --- |
| `--keep-dirs` | Leave restore/reject directories on disk |

Examples:

```bash
# Preview without touching trash (still needs a resolvable extra volume)
./scripts/trash-restore-fixture/trash_restore_fixture.py plant \
  --volume /run/media/$USER/<disk> --dry-run

# Same-volume restores only
./scripts/trash-restore-fixture/trash_restore_fixture.py plant \
  --volume /run/media/$USER/<disk> --only restore

# Another disk and label so two fixtures can coexist
./scripts/trash-restore-fixture/trash_restore_fixture.py plant \
  --volume /mnt/usb --prefix demo --label strata-demo

# Replace a fixture with the same prefix/label
./scripts/trash-restore-fixture/trash_restore_fixture.py plant \
  --volume /run/media/$USER/<disk> --force
```

## Tests

Helpers (path encoding, flags, rewrite targets) are unit-tested without touching session trash:

```bash
python3 scripts/trash-restore-fixture/test_trash_restore_fixture.py
```

CI's `python3 -m unittest discover -s scripts -p 'test_*.py'` loads the same tests through `scripts/test_trash_restore_fixture.py`.
