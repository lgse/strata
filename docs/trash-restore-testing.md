# Trash restore safety and testing

Trash metadata is untrusted. Strata resolves the physical GVfs item, validates
its destination against the mount hosting the item, and asks for confirmation
of the full destination before restoring. Undo uses the same validator without
an additional confirmation. Relative volume paths resolve against the volume
topdir, including the shared `.Trash/$uid` layout.

Restore does not cross bind-mount or subvolume boundaries. Planning rejects a
nested mount before showing confirmation, and execution retains the kernel's
`openat2` confinement check against concurrent changes.

The move requires an atomic `RENAME_NOREPLACE`. Filesystems that reject this
operation (including some NFS and FUSE configurations) receive an explicit error;
the payload and its trash metadata remain intact. Checking for an absent
filename and then doing an ordinary rename is not safe: a concurrent writer
could create that filename between the two operations. Users can instead copy
the item to a destination they explicitly choose.

## Automated regressions

Run the full Rust suite with isolated GTK, disposable preferences, and required
cross-device coverage:

```bash
./scripts/test-headless.py
```

`/dev/shm` must be writable and on a different filesystem from the temporary
directory. `STRATA_REQUIRE_DEVICE_TESTS=1` is also set by CI; lack of a genuine
cross-device boundary fails instead of producing a vacuous pass.

Focused selections can be run on the same infrastructure:

```bash
./scripts/test-headless.py restore
./scripts/test-headless.py volume
```

The tests create their own temporary payloads and metadata. They cover:

- shared-trash topdir resolution and exclusion of the entire shared trash tree;
- crafted cross-volume paths, parent traversal, and symlink escapes;
- nested mounts on the same filesystem and raw non-UTF-8 mountinfo;
- destinations appearing immediately before rename, unsupported atomic rename
  errors, occupied directories, and dangling destination symlinks;
- preservation of the payload and metadata on failure, and metadata cleanup on
  success;
- mixed valid/invalid selections, full destination presentation, cancellation,
  and missing modal hosts;
- bounded concurrent lookups, result ordering, and dropping active lookups when
  the batch is cancelled. A filesystem call already blocked inside the kernel
  may still take time to return; cancellation does not kill that system call.

Run the real keyboard and drag/drop regressions in the pinned E2E container:

```bash
./scripts/e2e.sh -k cross_volume
```

These verify Copy, Move, and Cancel with real keyboard input in every view, and
that the chosen action agrees with the resulting files. The default
cross-device strategy is **Always Ask**. An unresolved volume lookup uses the
configured cross-device policy and is described as unresolved,
not as a proven device difference. Explicit modifier overrides remain supported.

## Manual acceptance

Use a disposable desktop session and expendable test volume, not the live
user's Trash. Never identify fixture ownership by substring matching filenames
or `.trashinfo` contents, and never run a blanket cleanup scan of other mounts.

1. Trash and restore files and directories on the test volume. Include spaces,
   nested parents, a sticky shared `.Trash/$uid`, and duplicate display names
   with different original destinations.
2. Check that confirmation shows the full destination. Cancel during lookup and
   during confirmation; neither may move files or delete metadata.
3. Change the original destination between confirmation and execution, and
   create a destination collision. Restore must fail without overwriting data.
4. Test an unsupported atomic-rename filesystem. Expect the explicit safety
   error and preserved payload/metadata, not an unflagged-rename fallback.
5. Check long paths and large selections: destinations must remain readable and
   scrollable, with a cancellable loading indicator during lookup.
6. Exercise Undo and DnD modifiers, cursor feedback, and folder/background/sidebar
   targets in every view. A mixed-parent drop skips individual no-ops and still
   transfers the valid sources.

Dispose of the entire dedicated test environment afterward. No fixture utility
in this repository modifies or cleans the user's real Trash.
