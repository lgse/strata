# Trash restore safety and testing

Trash metadata is untrusted. Strata resolves the physical GVfs item, validates
its destination against the mount hosting the item, and asks for confirmation
of the full destination before restoring. Undo uses the same validator without
an additional confirmation. Relative volume paths resolve against the volume
topdir, including the shared `.Trash/$uid` layout.

Restore does not cross bind-mount or subvolume boundaries. Planning rejects a
nested mount before showing confirmation, and execution retains the kernel's
`openat2` confinement check against concurrent changes.

The move goes through the placement routine described in
[Filesystem compatibility](filesystem-compatibility.md#placing-an-item-without-replacing)
and never replaces an item already at the destination. It uses an atomic
`RENAME_NOREPLACE` where the filesystem offers one. Where it does not (some NFS
and FUSE configurations), a file is hard-linked to the destination and its
trash name is removed only once the link is proven, and a folder is renamed
over an empty placeholder that Strata has just created. Where hard links are
unusable too, as on sshfs and vboxsf, the restore takes the
[reserved-name step](filesystem-compatibility.md#the-reserved-name-step-and-its-risk),
which has one narrow window. Only where no step can place the item, for example
where a probe cannot settle what the mount supports, is it copied through the
[copy engine](filesystem-compatibility.md#restoring-from-trash): each entry is
created exclusively at its final name with the metadata the destination keeps,
and the copy is flushed to disk before the trash copy is deleted entry by entry.
Entries that changed meanwhile, or that the destination cannot store, stay in
the Trash with the item's trash metadata and are reported. If something already
exists at the destination, or the outcome cannot be confirmed, the payload and
its trash metadata remain intact.

Deleting or trashing a folder in Strata removes its sidebar pin and pins inside
it from GTK's shared bookmarks file and every open sidebar. Only confirmed
deletions remove pins; failed and unattempted items keep theirs. Unavailable
drives and network locations stay pinned. Restoring a folder, including Undo,
does not restore its former pins; pin it again if needed.

Pin cleanup runs in the deletion provider as one bookmarks update when the
deletion or restore run ends, however it ends, so it does not depend on the
originating window staying open. Pins identify paths; deleting a symlink does
not remove pins to its target. If updating bookmarks fails, Strata reports that
separately and does not revert a completed deletion. Cleanup is not guaranteed
if the process exits before the provider confirms the deletion, or before the
settings writer saves it (quitting waits up to 10 seconds for queued saves).

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
./scripts/test-headless.py ui::window::bookmarks::tests
./scripts/test-headless.py adapters::bookmarks::tests
./scripts/test-headless.py adapters::local_operations::tests::deletion
./scripts/test-headless.py ui::browser::trash::tests
./scripts/test-headless.py adapters::gio_location::tests
./scripts/test-headless.py without_trash_support
```

The tests create their own temporary payloads and metadata. They cover:

- shared-trash topdir resolution and exclusion of the entire shared trash tree;
- crafted cross-volume paths, parent traversal, and symlink escapes;
- nested mounts on the same filesystem and raw non-UTF-8 mountinfo;
- destinations appearing immediately before rename, unsupported atomic rename
  errors, occupied directories, and dangling destination symlinks;
- preservation of the payload and metadata on failure, and metadata cleanup on
  success;
- restore copies where nothing can be placed: read-only items keep their
  metadata, a failed copy removes only what it created, the trash copy is
  deleted only once the copy is flushed, and changed or uncopyable entries stay
  in the Trash with their metadata
  (`adapters::local_operations::tests::restore_safety`);
- mixed valid/invalid selections, full destination presentation, cancellation,
  and missing modal hosts;
- bounded concurrent lookups, result ordering, and dropping active lookups when
  the batch is cancelled. A filesystem call already blocked inside the kernel
  may still take time to return; cancellation does not kill that system call.
- `trash:///` entries whose names are not valid UTF-8 keep GIO's percent-encoded
  URI and their name bytes, so the Trash listing includes them
  (`adapters::gio_location::tests`, `adapters::local_files::tests::browse`,
  `services::file_source::tests`);
- the explained, Cancel-first permanent confirmation when Trash is unsupported,
  both before a Trash attempt (the listing reports no Trash support) and after
  one fails, in the foreground, docked, and after a mixed failure, with local or
  network wording (`ui::browser::trash::tests`);
- FUSE trash refusals: the error classifier and mount-health facts, the
  per-mount record keyed by mount ID, and removal of only new, empty trash
  folders (`adapters::trash_support::tests`); the retryable result on FUSE but
  not on a local disk (`adapters::local_operations::tests::deletion`); and the
  up-front confirmation after a recorded refusal until Reload
  (`ui::window::tests::keyboard_dispatch::file_commands`).

Run the real keyboard and drag/drop regressions in the pinned QA container:

```bash
./scripts/qa.sh cross-volume-enter-choice cross-volume-plain-copy
./scripts/qa.sh delete-without-trash-focuses-cancel tenxer-d-without-trash-confirms
```

These verify Copy, Move, and Cancel with real keyboard input in Columns, and
that the chosen action agrees with the resulting files. The default
cross-device strategy is **Always Ask**. An unresolved volume lookup uses the
configured cross-device policy and is described as unresolved,
not as a proven device difference. Explicit modifier overrides remain supported.
The two without-trash scenarios delete from a `/dev/shm` folder, which GIO
reports as unable to trash, and checks that Delete and 10xer **d** explain the
missing Trash, focus Cancel, and delete only after an explicit confirmation.

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
4. Test a filesystem without atomic no-replace renames, such as NFS or sshfs.
   Restore must succeed without replacing an item created at the destination
   meanwhile. Where nothing can be placed, the item is copied: modes, times and
   extended attributes must survive where the destination stores them, and an
   entry changed in the Trash during the restore must stay there with its
   metadata and be reported.
5. Check long paths and large selections: destinations must remain readable and
   scrollable, with a cancellable loading indicator during lookup.
6. Exercise Undo and DnD modifiers, cursor feedback, and folder/background/sidebar
   targets in every view. A mixed-parent drop skips individual no-ops and still
   transfers the valid sources.
7. In a session running `gvfsd`, trash a file whose name is not valid UTF-8
   (for example the byte `\xe9`). The Trash view must list it as
   `(invalid encoding)`, count it in the footer, enable Empty Trash, and show
   its percent-encoded `trash:///` URI in the path bar and Properties. Restore
   it and check that the original name bytes return; trash it again and delete
   it permanently. The pinned test container sets `GIO_USE_VFS=local`, so it
   cannot browse `trash:///` and does not cover this step.
8. On a writable FUSE mount that cannot hold a Trash (for example sshfs to a
   server root you can write, or an rclone remote without VFS cache), press
   Delete. Expect the Cancel-first confirmation naming the mount and its type,
   no `.Trash-$uid` left at the mount root, an up-front confirmation on the next
   Delete, and Move to Trash hidden until Reload. A disconnected sshfs mount
   must show the raw error, not "doesn't support Trash". The test container has
   no FUSE, so it does not cover this step.

Dispose of the entire dedicated test environment afterward. No fixture utility
in this repository modifies or cleans the user's real Trash.
