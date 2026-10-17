# Filesystem compatibility

Strata works on local disks, removable media, network mounts (NFS, SMB/CIFS), FUSE filesystems
(sshfs, ntfs-3g, exfat-fuse, rclone, gvfsd-fuse), virtualization shares (9p, virtiofs, vboxsf)
and stacked filesystems (overlayfs, eCryptfs, mergerfs). They disagree about renames, links,
case, names and Trash. This document records what Strata guarantees on every mount, how local
file operations adapt to what a mount supports, and how to test that.

Strata reaches GVfs locations (`smb://`, `sftp://`, `mtp://`) through GIO, so most of their
semantics come from the GVfs backend. This document covers them only where it says so: the
[names](#names) they get by their address, the links they refuse, and how a native item moved to
one is deleted.

## Guarantees

- **Never replace a concurrent arrival.** An item that appears at a destination while Strata is
  working is never overwritten, with one narrow exception where `RENAME_NOREPLACE` is missing and
  the item cannot be hard-linked (on mounts without usable hard links, or for a file whose own link
  is refused), described in
  [The reserved-name step and its risk](#the-reserved-name-step-and-its-risk). Where only the
  client kernel or a FUSE daemon checks for an existing name, other clients are not covered; see
  the residual limits below. Replace only ever removes the item the user confirmed, identified by
  device, inode, type and, where the filesystem reports it, birth time.
- **Never remove a name on a guess.** A name is unlinked only after Strata has proven which inode
  it holds. When the outcome of a step cannot be proven, both names are kept and the operation
  reports that the result could not be confirmed.
- **Never delete what a copy does not hold.** A move that has to copy deletes a source entry only
  once the copy is flushed to the destination's device, and only while the entry is still what
  the copy read. Anything else stays where it was and is reported; see
  [Moves across filesystems](#moves-across-filesystems).
- **Show only complete copies.** A copy appears under its final name once it is complete, except
  on mounts where nothing can be placed; see [Publishing a copy](#publishing-a-copy).
- **Keep transfer work off the interface thread.** Native copying, placement, and paste/restore/delete
  planning run on workers. Some existing archive-name, undo and redo replay, and bookmark checks
  remain synchronous; removing those calls is tracked in [#1590](https://github.com/lgse/strata/issues/1590).
  An unresponsive mount can still block the window through those remaining checks.
- **Move metadata, not data.** Renames never copy file contents. Moves within one filesystem
  rename too, including on mounts with neither `RENAME_NOREPLACE` nor usable hard links. Moves
  copy only when the kernel returns `EXDEV` (across devices, bind mounts, btrfs subvolumes or
  overlayfs lower-layer folders), or where no step below can place the item: on a mount whose
  support a probe cannot settle, for a folder on a mount without `RENAME_NOREPLACE` that has hard
  links but cannot rename over an empty folder, or for a file on an SFTP server that refuses the
  reserved-name step's rename.

## Placing an item without replacing

Every local operation that puts an item at a new name — move, rename, undo, restore from Trash,
publishing a staged copy, setting an original aside for Replace or Merge and putting it back,
archive extraction and creation — goes through one routine, `placement::place` in
`src/adapters/local_operations/placement.rs`. It tries these steps in order and stops at the first
that applies:

| Step | Applies to | Operation | Why it cannot replace |
| --- | --- | --- | --- |
| 1 | everything | `renameat2(RENAME_NOREPLACE)` | the kernel refuses an existing target |
| 2 | files, symlinks, special files | `linkat`, then remove the old name once the new one reports the item's identity | creating a link fails if the name exists, on the server for NFS and SMB |
| 3 | folders | create an empty placeholder folder with `mkdirat`, then rename over it | a rename can only replace an empty folder; a filled or swapped placeholder fails with `ENOTEMPTY` or `ENOTDIR` |
| 4 | symlinks | create a symlink with the same target, then remove the old one | creating a symlink fails if the name exists |
| 5 | files, symlinks and special files that steps 2 and 4 cannot place, and folders that step 3 cannot place on mounts without usable hard links, once a probe has settled that `RENAME_NOREPLACE` is missing; folders whose name ntfs-3g would cut | the [reserved-name step](#the-reserved-name-step-and-its-risk) | a file is renamed only over Strata's own empty placeholder, checked just before, and a folder only to a name just found free; see the step's narrow window |
| 6 | — | copy, then delete | only for a cross-device move (`EXDEV`), a move or restore from Trash that no step above can place, or a staged copy whose stage no step can place, which is copied again from the stage |

`EEXIST` and `ENOTEMPTY` always mean the destination is taken. `EINVAL` from step 1 has three
possible meanings, and Strata never treats it as "copy instead":

- moving a folder into itself, which is checked before any fallback, or found from the fallback
  step's own `EINVAL` once `RENAME_NOREPLACE` is known to be missing;
- an invalid name on this filesystem (for example `:` on vfat), when `RENAME_NOREPLACE` is known to
  work here or the fallback step also returns `EINVAL`;
- `RENAME_NOREPLACE` unsupported, otherwise.

ntfs-3g renames over an existing entry by first linking it to `<name>.ntfs-3g-<ten digits>`, a
name it stores cut once it passes 255 UTF-16 units: the rename fails with `EINVAL` for a name of
237 units, and leaves an entry named after the cut for a longer one. There, and on a `fuseblk`
mount whose device udev cannot tell, steps 3 and 5 never rename over a name of 237 units or
more. A file is still linked by step 2, and one whose own link is refused is copied where step 6
applies. A folder skips step 3's placeholder and takes step 5's folder check: it is renamed once
the name is found free, so moves, renames, extraction and putting an original back still rename
it.

Residual limits:

- Where `RENAME_NOREPLACE` is missing and an item cannot be hard-linked, a program that overwrites
  a destination name just before Strata renames over its placeholder loses its write, and so does
  an item created at the name, even exclusively, after something else removed the placeholder in
  that window; see [The reserved-name step and its risk](#the-reserved-name-step-and-its-risk).
- Where the operation knows which item it moves (restore, Replace's source and the original it
  sets aside, putting an original back, a case-only rename), another item found at the source
  name is refused before anything moves. Another item found at the destination just after the
  move is left there and reported as unconfirmed: it may be one another program put there, so it
  is never moved back. An item swapped in at the source name between that check and the rename
  is therefore left at the destination too; where that is a hidden name Strata chose (an original
  set aside, a case-only rename's temporary name), the error names it.
- ext4, ntfs3, NFS and SMB give a freed inode number straight to the next item created. Where no
  birth time is reported (NFSv3, ntfs-3g, exfat-fuse), an item deleted and recreated at the same
  name with the same type passes for the original.
- Undo and restore find an item Strata moved to Trash (by Delete, Replace or Merge) by original
  path, device and inode number, without birth time. If another item trashed from the same path got
  that inode number, they can bring back that item instead; they never overwrite anything.
- On SMB2/3, gvfsd-fuse and FUSE daemons that ignore rename flags, the no-replace check is made
  by the client kernel, so it protects against local processes but not against another client
  creating the same name at the same moment.
- On rclone, s3fs and other FUSE daemons whose exclusive create is not enforced by the server, the
  reserved-name step's placeholder protects only against local processes.
- On a case-insensitive SMB share without POSIX extensions, the Linux cifs client turns server
  inode numbers off for the whole mount once a folder is looked up under a second spelling, as the
  check before a case-only folder rename does ("Autodisabling the use of server inode numbers" in
  the kernel log). Hard links then report different inode numbers, so Strata stops using them
  there, as on sshfs; `RENAME_NOREPLACE` still places items there. A paste never looks a name up
  under a second spelling there, as it numbers names that differ only in case before copying.
- After a lost reply while placing over a placeholder (to the placeholder's `mkdir` or exclusive
  create, or to the rename over it), an empty folder or file Strata created can remain at the
  destination name. Where the mount's type is known to keep inode numbers, a placeholder the rename
  left in place proves the rename never happened: Strata removes it and reports the error itself.
  On FUSE and vboxsf the moved item can report the placeholder's number, so nothing is removed
  there, nor on a mount of another type or one whose type cannot be read.

Renaming a folder in the lower layer of an overlayfs mount without `redirect_dir`, or moving
across bind mounts or btrfs subvolumes, returns `EXDEV`. Those moves copy, and the copy uses a
reflink where both sides share a btrfs or XFS filesystem.

### The reserved-name step and its risk

Some mounts offer neither `RENAME_NOREPLACE` nor hard links that keep the item's identity: sshfs
and other libfuse high-level daemons without `use_ino` that reject rename flags, vboxsf, rclone,
s3fs and libfuse2 builds of exfat-fuse. There, every operation that places an item can take this
step: moves, renames, undo, restore, staged copies, archive extraction and creation, and Replace
and Merge without Trash.

It runs only after the steps above give up, and only once a probe on Strata's own names, or a
definitive errno, has shown that the mount lacks `RENAME_NOREPLACE` and has settled whether hard
links work. Then:

- a file or special file takes it when hard links are missing or unusable on the mount, or
  refused for that one file, for example by `fs.protected_hardlinks`, and so does a symlink that
  step 4 cannot recreate either;
- a folder takes it only when the mount has no usable hard links and refuses to rename over the
  placeholder folder of step 3, or where ntfs-3g would cut its name when renaming it over that
  placeholder.

Mounts with `RENAME_NOREPLACE`, such as most local disks, removable media with kernel drivers and
SMB, never take it. Mounts without it whose hard links work, such as NFS, OpenZFS 2.1 and earlier,
ntfs-3g and 9p, take it only for a file whose own link is refused, and ntfs-3g also for a folder
named with 237 UTF-16 units or more. Where a probe cannot settle what a mount supports, the step
does not run.

For a file, Strata creates an empty placeholder at the destination with `O_CREAT | O_EXCL`, checks
that the name still holds that placeholder (same inode, a regular file, still empty), then renames
the item over it. For a folder, it checks that the name is free, then renames. If the rename
fails with an errno that shows it did not happen, the placeholder is removed only while the name
still holds it unchanged. After one that may hide a lost reply (`EIO`, `ETIMEDOUT`, `ENOENT` and
the like), nothing is removed: the item counts as placed only if its old name is gone and the new
one reports its identity, and otherwise the result is reported as unconfirmed. sshfs keeps a
name's inode number when a rename's reply is lost, so the placeholder's number there proves
nothing. The empty placeholder therefore stays at the destination name after an unconfirmed
rename, a check that cannot read the name, a lost reply to its creation, or if Strata stops before
the rename.

What it risks, and why that is low:

- **The item being placed is never at risk.** It moves in a single rename, so it is always at its
  old name or its new one. A failed step leaves it where it was; one whose reply was lost is
  reported as unconfirmed, with both names kept.
- **An item already at the destination is never touched.** The exclusive create fails, and the
  name counts as taken. A program that creates the name exclusively while the placeholder stands
  gets "already exists" and loses nothing.
- **The only loss is a concurrent overwrite.** A program that overwrites that exact name before
  the rename, by writing into the placeholder or renaming its own file over it, has its write
  replaced by the placed item. That program was itself overwriting whatever held the name, so the
  race only decides which of the two writes survives. An item created at the name after something
  else removed the placeholder in that window is replaced too, even if it was created exclusively.
- **The window is one rename call** for processes using the same mount, one network round trip on
  sshfs: a change they make before the check makes Strata treat the name as taken. A writer on the
  server or on another client shows only by a change of type or size, because vboxsf and sshfs
  keep the placeholder's inode number for whatever the name then holds, and sshfs may answer the
  check from its attribute cache; for such writers the window runs from the placeholder's creation
  to the rename. Removing the placeholder after a failed rename has the same check-then-unlink
  window. For comparison, GLib (and so Nautilus) always, and `mv -n` where `RENAME_NOREPLACE` is
  missing, check that the name is free and then rename: any item created at the name in between is
  replaced, even by a program that creates it exclusively.
- **A folder is renamed only to a name just found free.** A rename cannot put a folder over a file
  or a non-empty folder, and this mount refused to rename it over Strata's empty placeholder, so an
  item that arrives in between makes the rename fail. An `EPERM` refusal also leads here although
  it is not learned for the mount; if it was specific to the item, an empty folder that arrives in
  that instant could be replaced. ntfs-3g renames a folder over an empty one, so there an empty
  folder that arrives in that instant is replaced, and ntfs-3g leaves an entry named after the
  cut beside it.

Copying into a name created exclusively would protect an arrival completely, but it costs more than
it protects. It rewrites every byte, and a file written during the copy stays behind at the source.
A copy drops the owner, the birth time and, on these mounts, hard links, and needs room for a
second copy. An interrupted folder move is left split across two folders. Archive publication, and
Replace and Merge without Trash, would still be refused on these mounts.

## What a mount supports

Strata learns capabilities per mount instead of trusting the filesystem type. The cache lives in
`src/adapters/volume/mount_capabilities.rs`. It is keyed by the mount id from `statx`:
`STATX_MNT_ID_UNIQUE`, which is never reused, on Linux 6.8 and later, otherwise `STATX_MNT_ID` with
the device number. Entries are dropped whenever Strata's mount monitor reports a change.

- It records whether `RENAME_NOREPLACE`, `RENAME_EXCHANGE`, hard links, symbolic links and renaming
  over an empty folder work, and what the mount's type says of the [names](#names) it stores and of
  file sizes: FAT holds no file of 4 GiB or more, so a paste that selects one fails before copying
  anything.
- A capability is recorded only from a definitive answer (`ENOSYS`, `EOPNOTSUPP`, or a rejected
  exchange), from Strata's own placeholder, or from a probe on Strata's own temporary names
  (`.strata-probe-*`) in the destination. An `EINVAL` or `EPERM` from placing a user's item never
  marks a mount as unsupported, because it can also mean an invalid name or a protected file. Only
  a mount whose type does not show its name rules remembers which rules a refused name needed; see
  [Names a mount refuses anyway](#names-a-mount-refuses-anyway).
- An operation reads a mount's name rules, link support and file size limit once and keeps them
  for its whole run, so its planning and its copy agree even when a mount change drops the cache
  meanwhile.
- The cache orders the steps above, each safe on its own, so a stale entry can make an operation
  slower or refuse it. The exception is the
  [reserved-name step](#the-reserved-name-step-and-its-risk), which the cache alone enables: a
  stale entry can let any placement take it, with that step's narrow window.
- The common case costs nothing extra: step 1 always runs first, and the cache is read only after
  it fails.

## Replace and merge

Before copying anything, Replace chooses how to remove the original, in this order:

1. **Trash**, when the location supports it. Undo restores the original.
2. **Atomic exchange** (`RENAME_EXCHANGE`) on in-kernel local filesystems: ext4, XFS, btrfs, f2fs,
   tmpfs, bcachefs, OpenZFS 2.2 and later, and vfat on Linux 6.0 and later. It is never used on
   FUSE, where some daemons ignore the flag and overwrite the target. The filesystem type is read
   fresh with `fstatfs` for every decision.
3. **Set aside**: the original is placed at a hidden `.strata-replaced-<id>` name with the routine
   above, the new item is placed at the original name, and the set-aside original is removed by
   identity. If placing fails, the original is put back; if its name was taken meanwhile, it is
   kept and the error names it. The original's name is briefly absent, as it is with Trash. On
   mounts with neither `RENAME_NOREPLACE` nor usable hard links, these placements use the
   [reserved-name step](#the-reserved-name-step-and-its-risk) for files, and for folders where
   renaming over an empty folder is refused.
4. Otherwise Replace fails before any data is copied. This happens only where a probe cannot
   settle what the mount supports, or for a folder on a mount without `RENAME_NOREPLACE` that has
   hard links but cannot rename over an empty folder.

Copies are written into a hidden `.strata-replacement-<id>` stage before they are placed. A file
stage is created by the copy itself, exclusively, at a fresh name, so nothing is ever copied over
a stage. Where the original or a moved source will be deleted, the stage is flushed to disk first;
see [Durability](#durability).

When Replace is part of a move within one filesystem, the source itself is placed after the
original is out of the way, so nothing is copied unless no step can place it. Without Trash, a
replaced original is gone for good: the replacement is not added to Undo, because undoing it could
only delete the remaining copy. A replacing move keeps its Undo, which moves the item back.

Merge moves the files it overwrites to Trash. Without Trash, it sets them aside, never by
exchange: the originals go into one hidden `.strata-merge-<id>` folder inside the merge target, are
put back if the merge fails or is cancelled, and are removed once it succeeds and is flushed to
disk. To put an original back over a copy the merge already placed at its name, the merge deletes
that copy while it is still as placed; anything else at the name stays, and the original stays in
the folder, which the error names. An original whose name the merge left free, because the incoming item was a special file the destination
cannot store or a source that a merging move kept, is put back instead. It refuses to set a file
aside where Replace would. How the incoming items are placed is described under
[Merges](#merges).

## Copying

Every copy between two native paths goes through one engine, `copy_engine` in
`src/adapters/local_operations/copy_engine.rs`: pastes, Duplicate and Keep Both, moves across
filesystems, Replace's staging copy, merges, and restores from Trash where nothing can be placed.
It runs on worker threads, opens every entry through a descriptor beneath a pinned folder
(`openat2` with `RESOLVE_BENEATH` and no symlinks), and makes every call through the `FsOps` seam.
It never replaces an entry. Copies to or from GVfs locations stay on GIO, except that a native
item moved to one is deleted by Strata, as described under
[Moves across filesystems](#moves-across-filesystems).

### Publishing a copy

- **A file on a local POSIX filesystem** (ext2/3/4, XFS, btrfs, f2fs, tmpfs, bcachefs, ZFS,
  overlayfs) is written into an unnamed `O_TMPFILE` in the destination folder and linked to its
  name with an exclusive `linkat` once its data and metadata are complete; `AT_EMPTY_PATH` is
  tried first, then `/proc/self/fd`, which Linux before 6.10 needs. A crash leaves nothing
  behind. A name the mount refuses is linked under stricter rules first, as described under
  [Names a mount refuses anyway](#names-a-mount-refuses-anyway). If the link is still refused
  for any reason other than a taken name or a full disk, the bytes already written are copied on
  the same device into a hidden stage, without reading the source again. A refused name never
  turns `O_TMPFILE` off. SMB is left out although its client accepts `O_TMPFILE`: it fakes one
  with a visible `.__smbfile_tmp*` file and reports write errors only after the name is published.
  FUSE daemons that implement it are unprobed.
- **Elsewhere, and for folders,** the copy is built in a hidden `.strata-copy-*` stage in the
  destination folder and published with the [placement routine](#placing-an-item-without-replacing).
  A file stage is created exclusively, and its identity is read from its own descriptor. Replace
  builds its copy under `.strata-replacement-*` and places it itself; see
  [Replace and merge](#replace-and-merge).
- **Where nothing can be placed,** because the mount's learned capabilities rule out every step for
  a folder or publishing a stage returns "unsupported", entries are created exclusively at their
  final names. A copy that fails there removes only the entries it created, each identified by its
  inode, and the item's folder only once it is empty. A stage built before this was known is copied
  again from the stage, on the same device.
- **Keep Both and Duplicate** choose their number when the copy is published, so a name taken in
  the meantime moves the copy on to the next number, cut where needed to fit the mount's [length
  limit](#length). Moves within one filesystem try each numbered name with the placement routine.
  GVfs destinations are probed off the main thread, and the copy retries on "exists". After an
  unconfirmed publish, both names are kept and no other number is tried.

A crash or an unplugged device can leave a `.strata-copy-*` stage behind. It holds only a partial
copy whose source still exists, and is safe to delete; nothing removes it automatically.

### Data

| Step | When | Gives way to the next step |
| --- | --- | --- |
| `FICLONE`, one call | the file is not empty and the destination has not refused clones; on SMB only on kernels with the client's December 2023 fixes | on the errnos that mean it did nothing; `EOPNOTSUPP` and `ENOTTY` are remembered for the mount |
| `copy_file_range`, in chunks | the file reports a size, and both ends are on one device, or in one network family (NFS, SMB or FUSE) on a kernel with that family's fixes: NFS from 5.9, FUSE from 5.8, SMB from 6.7, 6.6.7 or 6.1.73 | on the errnos that mean "unsupported", from the current offset; `ENOSYS` is remembered for the mount |
| `pread` and `pwrite` | always | — |

- Chunks start at 1 MiB, double while a call takes under 100 ms, up to 64 MiB, and halve when one
  takes over 250 ms. On NFS they stop at 2 MiB, because a larger request becomes an asynchronous
  server-side COPY that costs about 3 s whatever its size. A call that takes over a second at
  2 MiB or less switches the rest of that copy to reads and writes. A file that reports a size of
  0, as virtual files on FUSE do, is read whole.
- `EIO`, `ENOSPC`, `EDQUOT`, `EFBIG` and lost connections fail the file. A clone that fails with
  `EIO` is not retried, since a fallback could read pages the cache has reverted.
- Progress and cancellation are checked between calls, and a file is copied to its end even if it
  grew after it was opened. A clone is a single call: on a hugely fragmented file it cannot be
  cancelled until it returns.

**Sparse files** keep their holes, as with `cp --sparse=auto`, when the file is at least 1 MiB with
fewer allocated blocks than its size, the source is ext2/3/4, XFS, btrfs, tmpfs or f2fs, and the
destination is not vfat, exFAT or ntfs3. Those fill a skipped hole with zeros in one call that
reports no progress and cannot be cancelled. ZFS and network sources are copied in full, because
some ZFS releases report holes over data not yet synced, also through NFS. The copy walks
`SEEK_DATA` and `SEEK_HOLE` up to the size seen at open, sets the copy's length, then copies
anything the file gained since. Skipped holes count as copied bytes, so progress reaches its
total. A clone keeps holes by itself. FUSE destinations get the walk too, since the daemon behind
them is unknown, and one may still write the zeros.

### Metadata

A file is created at 0600 and gets, after its data: its owner and group, set only for the ids that
differ; its extended attributes, ACLs last; its exact mode; and its times. The owner comes first
because `chown` clears file capabilities and the setuid and setgid bits. Extended attributes come
before the mode, so `user.*` attributes survive on read-only files and folders. A folder is made at
0700 and gets the same once its last child is done, so its times survive too.

- **Best effort.** Only `EIO`, `ESTALE` and lost connections fail the copy. Any other refusal is
  skipped without a report, so copies to removable media do not warn about permissions every time.
  A namespace the destination refuses with `EOPNOTSUPP` is remembered for the mount, and a name
  refused for lack of permission, such as `security.*` for a normal user, for the rest of the
  copy. Names that libattr's `xattr.conf` skips (`security.evm`, `trusted.SGI_*`, `xfsroot.*`,
  `afs.*`, `user.Beagle.*`) are never copied, nor are ntfs3's `$LXUID`, `$LXGID`, `$LXMOD` and
  `$LXDEV`, which would set owner and mode behind `fchown` and `fchmod`.
- **Special bits.** Setuid and setgid are dropped when the owner could not be kept, as `cp` does.
- **Setgid folders.** Inside a destination folder that hands its group to new items, everything
  copied keeps the inherited group, and copied folders keep the setgid bit.
- **Folders without owner access.** A folder whose mode would lock its owner out, such as 0555, is
  filled with owner access and recorded by its path beneath the copy, without holding a
  descriptor. It gets its exact mode in a last pass, deepest first, before the copy is published,
  so trees with thousands of read-only folders (`~/go/pkg/mod`) need no descriptor per folder. A
  stage that cannot be published gets owner access back, so it can be removed.
- **Times after close.** On NFSv4, closing a file written under a write delegation sets its times
  to the close time, so they are set again after close, through a read-only reopen, a write
  reopen, or by name. On SMB and FUSE, a forced stat after close catches times a writeback cache
  moved by 2 s or more, and sets them again.
- **Symbolic links** keep their times, except on FUSE, where sshfs would apply them to the link's
  target. Their owner is not set. Links a destination cannot hold are left out; see
  [Symbolic links the destination cannot hold](#symbolic-links-the-destination-cannot-hold).

Mount options decide the mode on vfat, exFAT, exfat-fuse, ntfs-3g and SMB without POSIX
extensions: `chmod` succeeds there without changing it, so a 0700 folder takes the mount's mode.
What the lab's mounts keep, with udisks' default options:

| Destination | Mode | Times | Extended attributes |
| --- | --- | --- | --- |
| ext4, tmpfs | exact | to the nanosecond | yes |
| ntfs3 | exact | 100 ns | yes |
| NFSv4.2 | exact | to the nanosecond, set again after close | `user.*` where the server supports them |
| NFSv3 | exact | to the nanosecond | no |
| SMB with POSIX extensions | exact | 100 ns | `user.*` |
| SMB without them | 0755; a read-only file shows 0555 | 100 ns | `user.*` |
| ntfs-3g | 0777 | 100 ns | `user.*` |
| sshfs | exact | whole seconds | no |
| vfat | files 0644, folders 0755; a read-only file stays read-only | modification 2 s, access only the date | no |
| exFAT (kernel) | 0755 | modification 10 ms, access 2 s | no |
| exfat-fuse | 0777 | whole seconds | no |

A normal user keeps only their own owner, and a group they belong to.

### Folders, names and case aliases

A folder's tree is copied by up to eight workers, one where either disk is rotational, on the queue
that deletions use. The queue runs depth first, so few folders are open at once. Entries are
created exclusively, so a taken name inside a folder the copy made means one of three things:

- **A case alias.** On case-insensitive destinations (vfat, exFAT, exfat-fuse, ntfs3 with `nocase`,
  SMB, and casefolded ext4, f2fs and tmpfs folders), two source names can be one destination name.
  The later name in byte order gets the next Keep Both number, so `Makefile` and `makefile` become
  `Makefile` and `makefile (1)`, and the completion report lists it. Only on FAT, exFAT and SMB
  without POSIX extensions, for names brought from another filesystem, are aliases numbered before
  copying, by their case-folded names (see [When names are fixed](#when-names-are-fixed));
  elsewhere only the destination's "exists" counts, which also covers 8.3 short names and Unicode
  normalization. A source sibling that already has the numbered name keeps it, and the alias moves
  on to `makefile (2)`.
- **The copy's own entry after a lost reply.** NFS `CREATE`, `MKDIR`, `SYMLINK` and `LINK` are not
  idempotent. An entry listed under exactly the requested name, still as fresh as the copy would
  have made it (an empty folder, an empty file with one link, or a symlink with the same target),
  and not made by another worker of the copy, is adopted.
- **Someone else's entry,** possible on SMB without POSIX extensions, where the 0700 stage is not
  private on the server. It is left alone, and the copy's entry is numbered.

So that numbering follows byte order, names that may collide are claimed by the folder's job, one
after another, before their contents are written. ext2/3/4, XFS, btrfs, tmpfs and f2fs folders
without the casefold flag never collide, so there each file is simply created by its own job.
Elsewhere, only names that fold to the same key (compatibility forms decomposed, case folded,
trailing dots and spaces dropped) are claimed; a name that looks like a FAT short name
(`PROGRA~1`) or is not valid UTF-8 makes the whole folder claim. A claim is a `mknodat`, which opens
no descriptor, because vfat mounted with `flush` waits 100 ms at every close of a written file.
Where `mknodat` is refused, and always on SMB, it is an exclusive read-only create: on SMB, names
made by `mknodat` and looked up by name while other files were being written stalled the folder's
publication for Samba's 35 s lease-break timeout.

Names an item brings from another filesystem are fixed by the destination's rules before its
folder's names are claimed; see [Names](#names).

### Hard links

Names of one file within one copied item stay one file where the destination keeps hard links, as
with `cp -a` and cross-filesystem `mv`. Only the first name is copied. Once every file of the item
is written, each other name is linked to that copy exclusively, before folders get their final
modes. Whether the destination keeps links comes from the mount's learned capabilities, or from a
probe on Strata's own `.strata-probe-*` names in the destination, never inside the copy. Links
that report their own inode number, as on sshfs, count as missing. Where a link is refused, that
name and every later one are copied from the first copy, on the same device.

Names outside the copied item are copied separately, and so, in a merge, are names that land in
different merged folders. SMB shares without POSIX extensions report one link for each file a
folder listing shows, and sshfs for every file, so their hard links are copied as separate files.

### Special files

Named pipes inside a copied folder are recreated with their owner, mode and times; their extended
attributes are not copied, since reading them would open the pipe. Where the destination refuses
pipes (vfat, exFAT and sshfs with `EPERM`, exfat-fuse with `EIO`, SMB without POSIX extensions
with `EOPNOTSUPP`), they are left out and reported, as sockets and device files always are. A
selected pipe is recreated where the destination allows it. Selected pipes and symlinks are first
built in private stages so a concurrent arrival at the final name cannot be mistaken for the copy.
Where no placement primitive can publish a pipe, the pipe is created exclusively but has no proven
destination identity: a move keeps its source and destructive undo retains the copy. A symlink that
publication or a merge places by making it again at its name, as on sshfs, stands for its copy only
if it points where the staged link did. A failed unprovable creation may also remain rather than
risk removing an arrival. A selected pipe the destination refuses, a selected socket or device file,
and a pipe that Replace would stage fail with "it is not a regular file, directory, or symbolic
link".

### Symbolic links the destination cannot hold

vfat, exFAT and exfat-fuse have no symbolic links, so links copied there are left out without
asking the mount. A mount that refuses one with `ENOSYS` or `EOPNOTSUPP`, as SMB without POSIX
extensions does, is not asked again until it changes, and a GVfs location that answers "not
supported", as `smb://` and `mtp://` do, is not asked again for the rest of the item. Where names
must be UTF-8, as on every mount the [names table](#the-rules-each-destination-gets) does not give
any bytes, a link whose target is not valid UTF-8 is left out without asking the mount: SMB with
POSIX extensions would store each malformed byte as `?` without an error, and NTFS refuses it. A
mount that refuses one link, with `EPERM` while its name is free, or its target with
`EINVAL`, `EILSEQ` or `EFBIG`, as ntfs3 refuses an overlong target, leaves out that link only; a
link whose name the mount refuses is first tried under stricter names, as described under [Names a
mount refuses anyway](#names-a-mount-refuses-anyway). A taken name stays a taken name. Every link
left out is reported, and the rest of the copy goes on. A merge leaves an item at a link's name
where the destination is known to hold no links; where it learns so only when it copies the link,
that item was already moved out of the way, as any item a merge replaces.

A selected link is left out the same way, and the paste goes on with the next item. An extraction
leaves out link members the same way, with any hard link to one. A Replace by a link leaves the
original as it is. A move keeps the source of each link it left out, with the folders that hold it,
also where GIO copies the item, and lists them as left behind. Undo leaves those items out, and the
report says so and names them.

Link targets are copied and extracted byte for byte, so a link pointing at an entry whose name was
fixed, such as `latest` pointing at `2026-10-14T10:00`, no longer finds it at the destination.
Targets are otherwise not checked against the destination's rules, and some mounts change or
refuse them: ntfs3 reads a `\` in a target back as `/`; sshfs, with `contain_symlinks` on by
default and not shown in mountinfo, creates absolute targets and targets holding `..` but cannot
read them back; and SMB with POSIX extensions refuses a target longer than 1003 UTF-16 units with
`ENOSPC`, which fails the copy as a full disk would, and one of 4,095 bytes with `ENAMETOOLONG`,
which fails it as a name too long.

### Merges

Merge first moves the originals it replaces out of the way, as described in
[Replace and merge](#replace-and-merge). The incoming items are then built in a hidden
`.strata-copy-*` stage inside each merged folder, made when the first of them arrives, with the
naming rules above. Once the folder's items are complete, they are placed one by one in byte order
with the placement routine:

- A name another program took in the meantime gives the incoming item the next number, reported as
  taken. This includes a file that meets a folder.
- Where nothing can be placed, a file is copied out of the stage at its name, and new folders are
  copied in place in the live folder from the start.
- After an unconfirmed placement, the stage is kept, since it may hold the item, and the merge
  fails.

Placed folders then get their exact modes, and the empty stage is removed. Nothing is written into
a live folder except by an exclusive rename, or an exclusive create where nothing can be placed, so
a stopped merge leaves no partial file under a real name. Items already placed stay, and only they
are reported, except a copy over an original set aside without Trash, which gives way to the
original. Undoing a merge removes items under the names they were saved as. Each merged folder
is opened without following symlinks and pinned, so a folder swapped for a link is not followed,
while mount points inside the target are.

### Moves across filesystems

A move within one filesystem is a rename. Across filesystems, or where nothing can place the item,
Strata copies the item and records for each source entry what the copy read: its identity, link
count, size, and modification and change times. It records names only and holds no descriptors.
After the destination is flushed (see [Durability](#durability)), each source entry is deleted only
if it is still what the copy read:

- **Torn files.** A file that changes while it is read is copied once more: a stat after the copy
  shows another size or other times, or the copy's own reads found the file's end away from its
  size, which is all a rewrite within one tick of a coarse clock may leave. A file that reports a
  size of 0 before and after, as virtual files on FUSE do, is not held to its end. If it changes
  again, its copy is removed and the source is kept. A selected file then fails with "changed while
  it was being copied", and nothing is published.
- **The check.** A forced stat (`AT_STATX_FORCE_SYNC`, which revalidates NFS and SMB) must show the
  same type, size and modification time and, where inode numbers are stable, the same inode, change
  time and link count. Removing one name of a hard-linked file changes the other names' change time
  and link count, so Strata counts the names it removed itself, across every item of a paste, a
  restore or an undo, and accepts only those link counts. Right after removing a name, it reads the
  file's change time through another name of it in the same item, and the file's later names must
  show that change time. Before Linux 6.13, a change within the clock tick of that removal shows
  the same change time. A name in another item (two selected or restored items that share a
  file) is accepted with any change time once Strata removed one of the file's names, as long as
  its size and modification time still match.
- **SMB change times.** The SMB client answers a stat from the attributes it cached for a file
  (`acregmax`, or `actimeo` in the mount's options, one second by default, and longer for a file
  this machine keeps reading), which can hold a change time from before a change reached the
  server, such as the times the server sets once the client's deferred close of a file written
  there reaches it. So a move from an SMB mount records each source entry with a forced stat too,
  one more round trip per entry, and moving or undoing after a paste there, or after reading the
  files, does not keep unchanged entries as changed. Elsewhere, and on SMB mounts with `actimeo=0`,
  which cache nothing, the record takes no extra round trip.
- **Relaxed identity.** FUSE daemons without `use_ino`, vfat and exFAT, and cifs mounted with
  `noserverino` give an entry a new inode number once the kernel evicts it. There, the type and the
  birth time, where both stats report a non-zero one, stand in for the inode, and the change time is
  compared only for files with one link.
- **Racily clean files.** Where timestamps are coarse, as on every filesystem before Linux 6.13, a
  same-size rewrite can leave a file's times as they were. Such files are compared by content; see
  [Racily clean files](#racily-clean-files).
- **Folders** are opened and checked like entries, and removed once empty. A folder that gained
  entries, or another folder put at its name, stays.
- **The copy itself.** The move carries the identities of the entries it created, rather than
  adopting whatever is at a destination name afterwards, and the size and modification time it
  left each file and symbolic link with, as a stat reports them after the close and after times a
  close reset (NFSv4 write delegations, FUSE writeback caches) are set again. Before deleting
  sources it checks the published root and copied descendants; a merge also checks every merged
  ancestor and the actual numbered placement names. If any of these entries is missing, has another
  identity, or is a file or link of another size or modification time, as one rewritten in place or
  another file given its freed inode number within its birth time's tick (as on ntfs3), nothing of
  that item is deleted. A fallback that copies out of a stage records the newly created copy's
  identities, not the discarded stage's.

Where the destination keeps inode numbers, its entries must keep the exact device, inode, type and
available birth time. Where it numbers an evicted entry afresh (FUSE daemons without `use_ino`,
vfat and exFAT, cifs with `noserverino`), the type and the birth time, where both stats report one,
stand in for the inode as they do for sources, and a file or symbolic link must also keep its
recorded size and modification time, and a file its contents where those times cannot tell. The
kernel's exFAT driver sets a folder's birth time again whenever an entry is renamed into or within
it, as a merge places entries, so there a folder of the copy is known by its type alone and its
entries are checked one by one. An entry renumbered after a rename or a cache eviction then still
proves the copy, and the move deletes its sources.

What stays is reported by cause: changed during the move, could not be deleted (with the error, as
in a read-only folder), a special file or symbolic link the destination cannot store, or an item
whose copy was moved away. An entry already gone is not reported, since a move leaves exactly that.
A moved item with anything left behind is left out of Undo, and the report says so, naming up to
eight such items and counting the rest. Undoing a move gives back names that were numbered as
aliases, where the original name is free.

A native item moved to a GVfs destination is recorded before GIO copies it, then deleted the same
way. Nothing can be flushed there and nothing is compared by content, and a file that changes
during GIO's copy stays at the source while its torn copy stays at the destination. Items on GVfs
locations are copied and deleted by GIO. One moved to a native destination whose flush needs each
file synced, such as SMB, has its files synced before the flush.

### Racily clean files

As in Git's "racily clean" check: where a filesystem's timestamps are coarse, a file rewritten with
the same size within one tick keeps its modification time, and FAT keeps no change time at all, so
no stat can show the rewrite. On those mounts, a file whose modification time falls within one
tick of when the copy read it is copied with reads and writes while a SHA-256 digest of the bytes
is taken. Before the move deletes it, Strata waits until a rewrite would have moved its time, reads
the source again, up to one byte past the size it copied, and keeps it as changed if it differs.
It is deleted only if its name still holds the file it compared. Its copy is read back too, without
waiting, and must hold the same bytes: a file put in the copy's place within the tick can match
the copy's identity, size and times. Otherwise the source stays, reported as an item whose copy was
moved away. As the copy takes the source's modification time, a file modified within the window of
the destination, by the same table, is digested too, and only its copy read back, even where the
source's own times need no comparison.

| Source | Window after the modification time |
| --- | --- |
| vfat, msdos, exFAT; exfat-fuse; `fuseblk` mounts whose format udev cannot tell | 2 s (exFAT stores 10 ms, but gets FAT's window) |
| HFS, HFS+ | 1 s |
| sshfs | 3 s: the 1 s tick, plus 2 s for a server clock behind ours |
| any other file whose modification time has no fraction of a second, as on FAT-backed shares, ext2/3 with small inodes, rclone and gvfsd-fuse | 2 s |
| any other source, except ext4, XFS, btrfs and tmpfs on Linux 6.13 and later | none, only the 100 ms below |

- Each window gets 100 ms more for the coarse clock the kernel stamps times with, which advances
  once per tick (up to 10 ms). Files modified longer ago are not read again.
- From Linux 6.13, ext4, XFS, btrfs and tmpfs stamp times with multigrain timestamps: a change
  right after a stat gets a finer time than the one the stat saw, so a rewrite always shows, and
  those mounts keep the stat check alone. Other filesystems, such as f2fs, ntfs3 and ramfs, and
  overlayfs, which does not show which filesystem stamps its files, are compared on every kernel,
  as are NFS, SMB and FUSE mounts, whose server or daemon stamps the times. The kernel release is
  read once, and the family comes from the mount already looked up for each file.
- The wait is at most the window, so moving a file just saved on a FAT stick takes up to about 2 s
  longer, and one saved within the last 100 ms elsewhere up to 100 ms longer.
- A file dated ahead of the clock, on a mount this machine stamps (FAT, `fuseblk`, HFS), cannot
  have been rewritten unnoticed until the clock reaches its time. While the clock is before it, it
  is deleted without being read again; after, it is compared like the others.
- If the clock was set back to before the copy read a file, the file is kept as changed.

### Durability

Before Strata deletes anything that only a copy holds, it flushes the copy to the destination's
device. That covers the source of a move that copied, an original that Replace exchanged or set
aside, a merge's set-aside originals, an archive's replaced original, and the Trash copy after a
restore copy. The flush goes through a folder opened before the copy wrote, because since Linux 5.8
`syncfs` reports only the writeback errors that follow its descriptor's open; a folder others may
only write into is replaced by the nearest readable folder above it on the same mount. Plain
copies never sync, but every close is checked, which catches the write errors NFS, SMB and FUSE
report there.

| Destination | Flush | Why |
| --- | --- | --- |
| ext2/3/4, XFS, btrfs, f2fs, tmpfs, bcachefs, ZFS, overlayfs, ntfs3, NFS | `syncfs` | `sync_fs` flushes the device; NFS commits to the server |
| vfat, msdos, exFAT | `syncfs`, then `fsync` of the folder | `sync_fs` writes back without flushing the device's cache, which `fsync`, of folders too, does |
| ntfs-3g, exfat-fuse | `syncfs`, then `fsync` of the folder | without `FUSE_SYNCFS`, `syncfs` writes the daemon's device pages back but flushes nothing; both daemons sync and flush their whole device for a folder's `fsync` |
| other FUSE daemons, and `fuseblk` mounts udev cannot tell | `fdatasync` of each written file before it is closed, then `fsync` of the folder | a file's `fsync` reaches the daemon; the folder's covers names published after the files were synced |
| SMB | `fdatasync` of each written file | only a file's `fsync` reaches the server, as an SMB FLUSH |
| any other | `fdatasync` of each written file, then `syncfs` | |

- ntfs-3g and exfat-fuse are recognised by the format udev records for the mount's device
  (`ID_FS_TYPE` in `/run/udev/data/b<major>:<minor>`), read off the main thread and cached with
  the mount's capabilities. The same record gives them, and FAT served by FUSE, their
  [name rules](#the-rules-each-destination-gets).
- A removable device gets a folder `fsync` besides. Before Linux 5.8, every file is synced too,
  since `syncfs` reports no writeback errors there.
- If the flush fails, nothing behind it is deleted, both copies stay, and the error says the device
  could not finish writing.

**Batches.** A paste collects what it owes per destination filesystem and flushes once per batch:
at 256 items, at 1 GiB copied, 5 s after the batch's first item (checked as items finish), and at
the end of the paste, including on cancel or failure. Sources are deleted in batches, and each item
is recorded as moved, for Undo, once its source is deleted. Cancelling stops copying, then
completes the deletions of the batch, whose copies are on disk. Between local folders, an item the
cancel cuts off is removed from the destination and counted as not attempted; a merge keeps what it
placed and counts as failed. A crash before a batch closes leaves both copies. The progress dock
shows "Writing to device…" during a final flush, except on SMB, whose files were synced as they were
written. A merge also flushes the filesystems mounted beneath its target that it wrote to. Replace
syncs only where its original or its source will be deleted: for moves, without Trash, and when a
move to Trash fails after the copy was staged. Undoing moves is batched the same way; a restore copy
flushes once per item.

**Parallel moves.** A paste of more than eight native files into a native folder, none of them in
conflict, copies or moves up to eight files at a time, one where either disk is rotational. A FAT,
exFAT or NTFS destination whose device does not say counts as rotational. Each move tries a rename
first; one that has to copy joins the batches.

### Restoring from Trash

Restore places an item with the placement routine. Only where no step can place it on its
filesystem does it copy the item, in place at its final names, pinned to the Trash item it checked,
with the best-effort metadata above. The copy is flushed, then the Trash copy is deleted entry by
entry, as a move deletes its source. Anything changed or uncopyable stays in the Trash with its
`.trashinfo`, and is reported as left in the Trash. A failed restore copy removes only the entries
it created. A cancel during the copy removes them too and counts the item as not attempted, left in
the Trash with its `.trashinfo`; if they cannot be removed, the item counts as failed. A cancelled
restore or merge undo ends as cancelled, even when its last item finished. A restore across
filesystems is refused.

Undoing a merge, or a copy or an archive that replaced its original through the Trash, deletes what
the operation put at the original's path, then restores the original. As it places an item there,
the operation records it as a [move](#moves-across-filesystems) records its sources, with forced
stats on every mount and the contents of files within a tick of coarse times. Eligibility for that
record comes from known created identities for the whole tree and, for regular files, size and
modification-time baselines captured before publication. A concurrent arrival, replaced descendant,
or detectable in-place edit between publication and recording therefore cannot become deletion
evidence. Placement and hard-link creation change change times and link counts, so those fields are
captured after placement rather than compared to the pre-publication baseline. The size/time
baseline is not a full content reread: a same-size edit hidden by timestamp resolution, or a writer
restoring the old modification time before recording, can escape it. The later coarse-time digest
cannot retroactively prove which bytes were published. It reads such a file back only until the
tick passes, as any later rewrite changes its times. An item has no record if a cancel stops its
recording, or if it would take the records of its paste or archive past 100,000 entries (some 20 MB
kept in the undo history); one that does takes none of them. The undo first compares all of it by
the same rules, and deletes it only if every entry is still as recorded and no folder holds an
entry added since. Otherwise all of it stays, and the original stays in the Trash with its
`.trashinfo`, reported as left there because what replaced it may have changed. An entry that
changes between the comparison and its deletion still stays, though entries deleted before it are
gone. Without a record, an original is restored only to an empty path. A merge undo deletes its
copy before it restores the original, so an item cut off after it deleted anything counts as
failed; one cut off while it compares counts as not attempted. A path it cannot check fails with
that error and the original stays in the Trash.

A merge undo moves what the merge created to the Trash, and so does undoing the items copied
beside it in the same paste. Where there is no Trash, it deletes them by the same rules instead. A
copy merge records what it creates, under the name it was saved as and within the paste's 100,000
entries, only where an original could not go to the Trash, or where it overwrote nothing and the
location offers no Trash; a paste with a merge records each item it copies beside it where the
destination offers no Trash. The undo deletes an item only if all of it is unchanged. A folder of
the item that the copy left without owner access, such as 0555, gets it while its entries are
deleted, and its mode back if it stays; the folder the item sits in is never changed. An item that
changed, or has no record (one on a GVfs location, a GIO copy without native creation provenance,
an entry whose client-reported identity changed during publication, or one created where the Trash
worked during the paste), stays whole and fails with "It was kept, as it may have changed since it was copied";
another undo does not try it again. One that changes after part of it was deleted fails with "The
folder is not empty", and stays to undo. An item cut off after it deleted anything counts as
failed, and one cut off while it compares as not attempted.

As in a move, anything that changes an entry's change time counts as a change, even with its
contents untouched, and an undo may come long after the operation. Renaming the item away and
back, deleting it to the Trash and undoing that, changing its permissions or adding a hard link to
it, as some backup tools do, all keep it. So does replacing it twice and undoing both, as undoing
the second puts the first back by a rename.

### The completion report

After a transfer or an archive extraction, Strata reports what it could not do as asked:

- names the destination doesn't allow or that are too long for it, names it took for another (case
  aliases), and names taken during a merge, an extraction or an undo, each with the name the item
  was saved as, and an item saved under several names in turn once, from its first name to its
  last;
- special files and symbolic links left out of a copy, and symbolic links left out of an extraction;
- source entries a move left behind, entries a restore left in the Trash, originals an undo left
  there because what replaced them may have changed, and names an undo could not give back, with
  the reason.

One short renamed line is shown as a notice. More are listed in an information dialog, up to eight
lines and a count of the rest. Anything left out or left behind makes it a "Completed with errors"
warning. When the transfer failed or was cancelled, or a restore or merge undo completed with
errors, the dialog of that outcome lists the whole report under it instead, in the same sections.
Keep Both numbers, which the user asked for, and metadata a destination cannot store are not
reported.

### Limits

- **Coarse times.** vfat keeps no change time and a 2 s modification time, exFAT 10 ms, sshfs whole
  seconds, and the kernel's coarse clock advances once per tick. Moves compare racily clean files
  by content; without that, a same-size rewrite within one tick would pass. On NFS, SMB and FUSE
  mounts other than sshfs, a server or daemon whose clock runs more than 100 ms behind ours can
  still hide a same-size rewrite of a file it stamped within one of its ticks.
- **exFAT folders.** The kernel's exFAT driver sets a folder's birth time again whenever an entry
  is renamed into or within it. A move from exFAT keeps a whole source folder another program did
  that to, instead of only the changed entry. A move onto exFAT knows the folders of its copy by
  their type alone, so where another program renames a merged folder away and makes another at its
  name, the move still deletes its sources, whose copies stay in the renamed folder.
- **Relaxed identity.** Where a source has no stable inode numbers and reports no birth time
  (sshfs, exfat-fuse, ntfs-3g), a different file with the same type, size and times put at a source
  name passes for the original, unless it is racily clean and compared. The same goes for a move's
  copy on such a destination before its sources are deleted: a different file or link of its type,
  size and modification time, as another copy of the source would have, passes for it. On kernel
  exFAT and vfat, whose birth times count in 10 ms ticks, a copy replaced within its birth time's
  tick by a file of its size and modification time passes as well, and the move deletes its source.
  A copy recognises its own entries, and a selected folder it is about to read, by the mount's
  rule: by inode number where the mount keeps them, by birth time on vfat and exFAT, and by type
  elsewhere (its claimed files also by being empty), so an entry the kernel renumbered after
  eviction does not stop the copy. A hidden stage the kernel renumbered can still be left behind.
- **Modes that cannot be stored.** On vfat, exFAT, exfat-fuse, ntfs-3g and SMB without POSIX
  extensions, the mount options decide modes, and nothing reports what was lost. On SMB with POSIX
  extensions, a file opened right after `chmod 0444` reports 0755, so any copier copies 0755.
- **Alias order.** Aliases are numbered in byte order where names are claimed. They are numbered
  in the order their copies finish where the destination takes two names for one that Strata's
  fold keeps apart, and on XFS's deprecated `ascii-ci` format, which no casefold flag shows. In a
  merge, a name another program takes at the same moment is decided by timing.
- **sshfs.** The 3 s window covers a server clock up to 2 s behind; a server further behind can hide
  a rewrite. sshfs answers stats from its attribute cache (`cache_timeout`, 20 s by default), so a
  change made on the server can stay invisible to both stat checks for longer than the window; the
  content comparison reads from the server.
- **Times after close.** The check on SMB and FUSE tolerates 2 s, so a daemon that moves times by
  less is not corrected. On NFSv4, setting times again costs one more open per file.
- **GVfs destinations.** A native item moved to one is not flushed and not compared by content, and
  a torn copy stays there while its source is kept. Sources on GVfs locations are deleted by GIO,
  without a check of each entry, except the links it left out. A selected link moved from a GVfs
  location is left out only where the destination is known to hold no links.
- **SMB sources.** Opening two names of one hard-linked file at once fails with `EINVAL` on the
  kernel client, so Strata opens one name of a file at a time when it reads from SMB, whatever link
  count the share reports. After this machine writes a file on a share, opening another name of it
  fails the same way, for any program, until the client's deferred close expires (`closetimeo` in
  the mount's options, one second by default). Strata retries an open of an SMB source that fails
  with `EINVAL`, backing off, for that time plus half a second but at most ten seconds, and
  cancelling stops the wait at once. Opens still refused after that, as while the writer keeps the
  file open, fail with "Could not copy". Other errors and other filesystems are not retried.
- **SMB read-only folders.** Without POSIX extensions, the client decides a folder's read-only
  attribute from cached attributes, so write access given back right after it was taken does
  nothing until the folder is revalidated. Strata revalidates before giving it back; other programs
  may not. Folder access times come back as the modification time.
- **ntfs-3g and exfat-fuse** are recognised only from the host's udev records. Without them, as in
  containers, they keep per-file syncs and the 2 s comparison window, and get Windows name rules
  that leave `aux.c` and `con.txt` alone until a refusal teaches more. Their batched flush relies on
  the daemon's `FUSE_FSYNCDIR` syncing the whole device, as both daemons do; the kernel reports
  success for a daemon that does not implement it.
- **vfat with `flush`** waits 100 ms at every close of a written file, which bounds copies of many
  small files.

## Names

Mounts disagree about which names they store, and many change or refuse a name without a clear
error: vfat drops a trailing dot, SMB stores `:` as a private-use character that other clients
show, and exfat-fuse can corrupt its volume on a malformed name. Strata takes each mount's rules
from what the mount shows, fixes the names an item brings from another filesystem before anything
is created, reports every fix, and refuses a name the user types that the location would store as
another.

### The rules each destination gets

A folder takes the rules of the mount behind it, found through symbolic links and nested mounts,
off the main thread: the mount's type and superblock options from mountinfo, the device format udev
records for a `fuseblk` mount, and `f_namelen` from `statfs`, which only filesystems that count
bytes mean (0 means unknown, and other values are clamped to 64 to 4,095 bytes). The rules are
cached with the mount's [capabilities](#what-a-mount-supports), so a remount, such as ntfs3's with
`windows_names`, applies from the next operation. GVfs locations get the rules of their address.

| Destination | Characters | Trailing dots and spaces | Device names | Length |
| --- | --- | --- | --- | --- |
| ext2/3/4, XFS, btrfs, f2fs, tmpfs, bcachefs, ZFS, overlayfs, NFS | any bytes | kept | kept | `f_namelen` bytes |
| vfat, msdos, exFAT; a FUSE daemon on a device udev records as FAT or exFAT, such as exfat-fuse | Windows, UTF-8 | dropped | bare | 255 UTF-16 units |
| ntfs3, and ntfs on Linux 7.1 and later | Windows, UTF-8 | dropped | bare; all with `windows_names` | 255 units |
| ntfs-3g, on a device udev records as NTFS | Windows, UTF-8 | dropped | all | 255 units |
| a `fuseblk` mount whose device udev cannot tell | Windows, UTF-8 | dropped | bare | 255 units |
| SMB without POSIX extensions | Windows, mapped, UTF-8; nothing above U+FFFF with `nocase` | dropped | all | `f_namelen` bytes and 255 units; folders 213 bytes |
| SMB with POSIX extensions | UTF-8 | kept | kept | `f_namelen` bytes; folders 213 bytes |
| HFS+ | UTF-8 | kept | kept | 255 units |
| sshfs, other FUSE daemons, 9p, virtiofs, vboxsf, ceph, eCryptfs, unknown types | any bytes | kept | kept | `f_namelen` bytes |
| `smb://` | Windows, UTF-8 | dropped | all | 255 bytes and units; folders 213 bytes |
| `mtp://` | Windows, UTF-8 | dropped, with leading spaces | all | 255 bytes and 254 units |
| `sftp://` | any bytes | kept | kept | 255 bytes |
| other GVfs locations | any bytes | kept | kept | none |

- **Windows:** `< > : " \ | ? *` and U+0001 to U+001F become `_`; U+007F stays.
- **UTF-8:** each malformed sequence becomes one `_`, so `caf\xe9.txt` becomes `caf_.txt`. Most of
  these mounts refuse such names, SMB with POSIX extensions stores each byte as `?`, and exfat-fuse
  corrupts its volume on some of them. With `nocase`, SMB refuses every character above U+FFFF, so
  each becomes `_` there (`a😀b` becomes `a_b`), as on vfat mounted with `iocharset=utf8` and
  without `utf8`. vfat and exFAT mounted with `iocharset=ascii` and without `utf8` get `_` for
  every character beyond ASCII.
- **Mapped:** with `mapposix`, the cifs client's default, the client stores the characters Windows
  refuses, and trailing dots and spaces, as the private-use characters U+F001 to U+F029, which
  other clients show, and reads those back as what they stand for, so a name that really holds
  U+F022 would list as `x:y`. Strata sends neither: the characters become `_` by the Windows rule,
  and so do those private-use characters, except U+F026, which the client reads back unchanged.
  With `mapchars`, the same holds for U+F02A, U+F03A, U+F03C, U+F03E, U+F03F and U+F07C.
- **Trailing dots and spaces** are dropped, repeatedly (`notes. .` becomes `notes`), and a name
  left empty becomes `_`. MTP also drops leading spaces, except before a dot, where that would hide
  the item.
- **Device names** get `_` after the device word: `CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`,
  `LPT0` to `LPT9`, `COM¹` to `COM³`, `LPT¹` to `LPT³`, `CONIN$` and `CONOUT$`, in any case and
  with any spaces after the word. **Bare** is the Windows 11 rule: a whole name (`CON` becomes
  `CON_`, `aux` becomes `aux_`) and `NUL` before an extension (`nul.txt` becomes `nul_.txt`), while
  `aux.c` and `con.txt` keep their names, so a source tree copied to a stick still builds. **All**
  also covers a device word before the first dot (`con.txt` becomes `con_.txt`, `PRN.tar.gz`
  becomes `PRN_.tar.gz`). FAT, exFAT and ntfs3 store device names, and Windows 11 opens all but the
  bare ones. ntfs-3g and ntfs3 with `windows_names` refuse them, and Samba lists them under 8.3
  aliases (`CON` as `CSHOFG~F`). udisks mounts ntfs-3g with `windows_names`, which its mountinfo
  does not show, and ntfs3 without it.

Under `/run/user/$UID/gvfs`, the GVfs bridge, where one FUSE mount serves every GVfs location, the
folder of each location takes the rules of its address: `smb-share:` those of `smb://`, `mtp:`
those of `mtp://`. A folder there whose location cannot be read gets the rules of `mtp://` with
SMB's limit for folders.

### Length

A name fits when its UTF-8 bytes and its UTF-16 units both fit, wherever each is counted. FAT,
exFAT, NTFS and HFS+ count 255 UTF-16 units whatever `f_namelen` says (1530 on vfat and exFAT), so
a name of 255 CJK characters (765 bytes) fits there, where ext4 takes 85. SMB counts both the
server's bytes and the 255 units the protocol carries. Samba 4.24 first creates a folder as
`.::TMPNAME:D:<pid>%<random>:<name>`, which takes up to 42 of the server's 255 bytes, so folder
names on SMB are held to 213 bytes. That costs other servers at
most 42 bytes on folder names of 214 to 255 bytes; other Samba versions and Windows servers are
unverified. MTP takes 255 bytes and 254 units: past that, libmtp sends an empty name instead of
failing, which phones refuse or hang on.

A name that is too long is cut at the end of its stem. A file keeps its extension, the last one or
`.tar` with up to four letters or digits after it (`.tar.gz`, `.tar.zst`), where that takes at most
half the limit; otherwise the whole name is cut, as a folder's always is. The cut moves back to the
start of the grapheme cluster it falls in, by at most eight characters, so flags, emoji sequences,
accented letters, Hangul syllables and, with Pango 1.54 or later, consonants joined by a virama
stay whole; a longer cluster is cut inside. The stem keeps at least its first character, so `x.txt`
never becomes the hidden `.txt`, and no marker is added: the name stays a prefix of the original.
Trailing dots and spaces and device names are then checked again. Every number Strata adds to a
name, for Keep Both, aliases, names taken meanwhile, new files and folders, archive members and new
archives, cuts the stem the same way to fit ` (n)`, and `.tar` stays before ` (1).gz`.

### When names are fixed

Names are fixed where an item brings them onto another filesystem: in a copy or a move across
filesystems, a merge, drag and drop, Send to, and the redo of such a move. Inside a copied tree,
each folder decides by its own device. A duplicate, a move within one filesystem and a restore from
Trash keep every name, so `CON` duplicated on a stick stays `CON`. Under the GVfs bridge, the
folder of each location counts as a filesystem of its own, and what its refusals teach stays with
it, so a song moved between two folders of one phone keeps its name. A folder merged into takes the
rules of its own mount, and whether names are fixed is decided again, only where it or the folder
merged into it is on another device than its parent: a bind mount of the same filesystem shares its
rules.

A folder's names are planned before its first entry is created, and the plan depends only on the
folder's entries. A name that needs no change keeps it, even where a fixed name would equal it.
Fixed names that equal another name are numbered, in the byte order of the names they replace, so
`a:b` beside `a_b` in a folder copied to a stick becomes `a_b (1)`. FAT, exFAT and SMB without
POSIX extensions (shares the kernel mounts, and `smb://`) also take names that differ only in case
for one, as Windows and Samba do by default, so there a kept name that equals one sorting before it
in bytes is numbered as an alias, among a paste's top-level items too (`Readme.txt` beside
`README.txt`); a share set to compare case still gets those numbers. Elsewhere, a top-level item
whose name, looked up once before it is copied, leads to an earlier item of the same paste saved
under another spelling, as in casefolded folders for `makefile` after `Makefile`, is numbered and
reported as an alias too. Any other item at the name, including one that took the earlier item's
place since, keeps it taken. In a merge, a planned name equal to an existing item replaces that
item like any other same-named one. A paste's top-level items from different folders that share a
name are numbered on every destination, the later one in the paste's order, before anything is
copied, and reported as names already taken: `notes.txt` from two folders becomes `notes.txt` and
`notes (1).txt`, also in a move within one filesystem. An item copied into its own folder is
numbered only as it is copied, so a same-named item from another folder meets the original there
and is asked about like any other conflict. A paste copies the items whose names it planned first,
then duplicates, then the items a conflict was resolved for, so no item numbered as it is copied
takes a planned name, and a duplicate is copied before Replace can remove its original.
The completion report lists every fixed name with the name it was saved as, and says which were
shortened.

An extraction gives its member names, and the folder it names after the archive, the rules of the
destination, numbers members whose names became one in archive order, and leaves out the links the
destination cannot hold; see [Extraction output](archives.md#extraction-output).

Undoing a move fixes only names the original mount cannot hold at all, such as malformed UTF-8, and
then gives the entries inside the item the names they had, deepest first, also where the item came
from a GVfs location onto a local disk or the other way round. An entry whose name another item
took meanwhile keeps the name it was saved as and is reported as taken; one whose name the location
now refuses is reported as fixed again; any other failure is listed with the name the entry kept
and the error. An entry no longer at the name it was saved as, because it was deleted or renamed
since, is passed over without a report. Redo fixes the names again.

### The conflict prompt

Before a paste asks about conflicts, it works out, off the GTK thread, the name each item will get,
and looks for an existing item, a broken link included, under that name. Where that name is
another, the prompt says so first ("“a:b” will be saved as “a_b” in this location."), so copying
`a:b` onto a stick that holds `a_b` offers Replace or Keep Both instead of failing with "already
exists". Replace, Merge and Keep Both act on that name, and Keep Both is offered for moves too.
Where a folder from another filesystem would be merged into a destination whose rules change
characters, the prompt also says that items whose names change overwrite items with the new names.
Where the destination is slow to answer, the transfer shows "Preparing items…" with Cancel and no
item count yet, docked for a copy as its progress will be, and its progress then takes that row's
place at once; the same paste repeated meanwhile is ignored. The prompt asks nothing of a selected
link the destination is known not to hold, which the paste leaves out.

### Names a mount refuses anyway

A mount may still refuse a name: a call that creates it fails with `EINVAL`, `EILSEQ`, `EBADMSG`,
`ENAMETOOLONG`, or `ENOENT` in a folder that exists, or placing a staged copy fails with an error
the placement routine puts down to the name. The entry is then tried again under the name its
folder's rules give, then under each of the Windows rules that changes it on its own (UTF-8, the
characters Windows refuses, trailing dots and spaces, all device names, 255 bytes and UTF-16
units), and then under those rules together, fewest first, skipping names already tried. The first
name the mount takes is used, numbered where it is taken, and reported as fixed, for a Keep Both
copy from the item's own name to the numbered one.

A mount whose type does not show its rules (sshfs and other FUSE daemons, 9p, virtiofs, vboxsf,
ceph, eCryptfs, a `fuseblk` device udev cannot tell, unknown types, and the GVfs bridge's folders
other than SMB and MTP ones) remembers the rules that worked until it changes. Later names brought
onto it are fixed by them before they are copied, and names the user types there are checked
against them; names undo gives back are not. A mount that refused only `x.` learns trailing dots
alone and keeps `a:b`. Any other mount tries each refused name again on its own, so one odd folder
never puts a whole disk under Windows rules. Undo tries entries inside an item again, but gives the
item itself back under its own name or fails. An extraction tries each refused member name again,
links and hard links included, and also a name whose lookup fails with `EINVAL`, `EILSEQ`,
`EBADMSG` or `ENAMETOOLONG`, as sshfs fails some. Copies through GIO do not try again. Where no
rule helps, the item fails with "The name isn’t valid at this location", or "The name is too long";
an extraction names the member.

### Names the user types

Names the user types are never fixed. Renaming an item, creating a file or folder by name, or
naming a new archive is refused before anything changes where the folder's rules, including any a
mount has learned from refusals, would store the name as another, and the message names what the
location cannot hold: the characters, each quoted and given by code point where it shows nothing of
its own (`U+0009`, `U+0301`), a leading space (MTP) or a trailing space or period, a device name
Windows reserves, or the length. The 10xer create and rename prompts check the name off the GTK
thread before they close and stay open with that message. A rename is held to the length of a file,
since Samba's shorter limit for folders applies only to the folders it creates. Names Strata
chooses itself, such as "new folder", are not checked.

### Limits of the name rules

- **Paths.** Only names are checked. An item deep enough fails where its whole path is too long:
  SMB and sshfs refuse paths of about 4 KB, since every request carries the whole path from the
  share or the server's root, and Windows' 260-character `MAX_PATH`, which Linux drivers do not
  enforce, is not predicted.
- **Case and normalization.** Strata plans case aliases only on FAT, exFAT and SMB without POSIX
  extensions, and elsewhere leaves them to the destination's "exists" (see [Folders, names and
  case aliases](#folders-names-and-case-aliases)). NTFS keeps `Makefile` and `makefile` apart on
  Linux, though Windows takes them for one name. No lab mount normalizes Unicode, except that NFC
  and NFD forms are one name in casefolded ext4 folders, and Strata never normalizes a name.
- **Windows 10** cannot open files such as `aux.c` or `con.txt`, which FAT, exFAT and ntfs3 keep
  under the Windows 11 rule.
- **Legacy code pages are not supported.** vfat mounted without `utf8` and with an `iocharset`
  other than `utf8` or `ascii` stores each byte of a UTF-8 name as a code-page character, and SMB
  with an `iocharset` other than UTF-8 writes `?` for what it cannot convert, both without an
  error. The kernel's exFAT with a code page other than UTF-8 or ASCII refuses what it cannot
  convert, which no rule fixes, so the item fails. udisks mounts vfat with `utf8` and exFAT with
  `iocharset=utf8`.
- **msdos** keeps 8.3 names and refuses names that start with a dot unless mounted with
  `dotsOK=yes`, so Strata's hidden `.strata-*` names fail there, and so does every copy that needs
  a stage. Only an explicit `-t msdos` mounts it.
- **HFS+** stores names decomposed and turns characters it cannot map into `?`, without an error.
- **GVfs.** Rules come from the address alone, not from the share or device behind it, and a copy
  through GIO fails on a name the backend refuses, with the backend's error.

## Rename

Local renames go through the same routine instead of GIO's check-then-rename.

- **Case-only renames** (`notes.txt` to `Notes.txt`) on case-insensitive folders — vfat, exFAT,
  SMB shares, casefolded ext4, f2fs and tmpfs folders, ntfs3 with `nocase` — are done in two steps
  through a hidden `.strata-rename-<id>` name, with rollback. A plain rename is a silent no-op
  there, and a no-replace rename reports the name as taken. The SMB client caches the lookup of
  the new spelling for about a second, so it is revalidated before the second step. exfat-fuse
  gives each spelling its own inode number, so there the new spelling counts as the item itself
  only when the folder lists no such name, the item's own name is the only one that matches it
  ignoring case, and type, size and modification time agree.
- On mounts with neither `RENAME_NOREPLACE` nor usable hard links, renaming a file takes the
  [reserved-name step](#the-reserved-name-step-and-its-risk) like any other placement, and so does
  renaming a folder where renaming over an empty folder is refused.
- A rename and its undo never copy. Where no step can place the item, the rename is refused.
- A typed name that the location would store as another is refused before anything changes; see
  [Names the user types](#names-the-user-types).

Other locations are renamed through GIO. GLib (2.88.3, and its main branch in October 2026) calls a
location's rename without checking that its backend has one, and the stand-in GIO uses for an
address no backend serves, as `smb://` is without a running GVfs, has none, so that call would end
the process. Strata checks first and reports the backend missing instead.

## Trash

GIO decides where Trash is available. It reports Trash as unsupported for:

- tmpfs, overlayfs, eCryptfs, gvfsd-fuse and other mounts GLib treats as system-internal;
- mounts whose mountinfo root is not `/`, which includes bind mounts and btrfs `subvol=` mounts,
  and nested btrfs subvolumes that have no mount entry;
- mounts with `x-gvfs-notrash`;
- mount roots where `.Trash-$uid` cannot be created or would be owned by another user, which is
  common on NFS (root squash, read-only export roots), CIFS mounted with `uid=` and sshfs without
  idmap;
- bind mounts of the home filesystem, where the move into the home Trash fails with `EXDEV`.

There, Delete offers permanent deletion and Replace and Merge use the order above.

GLib 2.86.0 to 2.88.4 can report success without trashing anything on FUSE filesystems that do not
report entry types (GLib issue #4060). Strata therefore checks that a trashed local item is gone
before reporting success or recording Undo.

## Hidden names

| Name | Purpose | After a crash |
| --- | --- | --- |
| `.strata-replacement-*` | Replace's staging copy; after an exchange, the swapped-out original until it is deleted | **may be the user's original** |
| `.strata-copy-*` | a copy being built: a new item, or a merge's incoming items inside the merged folder | partial copy; safe to delete |
| `.strata-compression-*` | archive being written; after an exchange, the replaced archive | **may be the user's original** |
| `.strata-replaced-*` | original set aside by Replace | **may be the only copy of the user's item** |
| `.strata-merge-*` | originals set aside by Merge | **may hold the only copies** |
| `.strata-rename-*` | middle step of a case-only rename | **is the user's item** |
| `.strata-extraction-*` | archive extraction staging | extracted output; see [Archives](archives.md) |
| `.strata-probe-*` | capability probe | empty; safe to delete |

The prefixes live in `placement.rs`. Names that may hold user data are never deleted or renamed automatically.

## Filesystem notes

Rename and link support decides which placement step runs. "Rejects flags" means
`renameat2` returns `EINVAL` for both `RENAME_NOREPLACE` and `RENAME_EXCHANGE`. The names each
filesystem stores are under [Names](#names).

| Filesystem | No-replace | Exchange | Hard links | Notes |
| --- | --- | --- | --- | --- |
| ext4, XFS, btrfs, f2fs, tmpfs, bcachefs | yes | yes | yes | btrfs renames across subvolumes return `EXDEV`; reflinks still work |
| OpenZFS 2.2 and later | yes | yes | yes | the loaded module version counts, not the userspace tools |
| OpenZFS 2.1 and earlier (Ubuntu 22.04, Debian 12) | rejects flags | rejects flags | yes | |
| vfat | yes | Linux 6.0 and later | no | case-insensitive; no symlinks; trailing dots are dropped |
| exFAT (kernel) | yes | no | no | case-insensitive; no symlinks |
| ntfs3, ntfs (Linux 7.1 and later) | yes | no | yes | case-sensitive unless mounted with `nocase` |
| ntfs-3g, libfuse2 daemons | rejects flags | rejects flags | ntfs-3g: yes | the kernel refuses rename flags below FUSE protocol 7.23; ntfs-3g never gets a rename over a name of 237 UTF-16 units or more (see [Placing an item without replacing](#placing-an-item-without-replacing)) |
| exfat-fuse (libfuse3) | enforced by the kernel only | ignored: overwrites | no | built on libfuse2, it rejects flags, and files take the [reserved-name step](#the-reserved-name-step-and-its-risk) |
| NFS v3 and v4 | rejects flags | rejects flags | yes | unlinking an open file leaves `.nfsXXXX` until it is closed |
| SMB/CIFS | client-side only | no | server-dependent | usually case-insensitive; open files may block rename and delete |
| sshfs | rejects flags | rejects flags | not used | each link reports its own inode number; reports "exists" as `EPERM`; files take the [reserved-name step](#the-reserved-name-step-and-its-risk); without the server's `posix-rename@openssh.com` extension its rename over the placeholder fails with `EPERM`, so moved, restored and staged files are copied, and file renames, archive publication and setting a file aside fail |
| 9p (WSL2, QEMU), ceph, eCryptfs | rejects flags | rejects flags | yes | |
| vboxsf, rclone, s3fs | rejects flags | rejects flags | no | files take the [reserved-name step](#the-reserved-name-step-and-its-risk); vboxsf refuses to rename over an empty folder, so its folders take that step too |
| mergerfs | rejects flags | rejects flags | within a branch | |
| overlayfs | yes | yes | yes | lower-layer folders cannot be renamed without `redirect_dir` |

libfuse high-level daemons without `use_ino`, sshfs among them, give each hard link its own inode
number, so a link cannot prove that it holds the item. Once a probe on Strata's own names confirms
this, Strata stops using links on that mount, and files take the
[reserved-name step](#the-reserved-name-step-and-its-risk). A link to a user's item made before
that confirmation is removed by the identity it reported right after linking, if its type,
permissions, size and modification time match the item's, and the file then takes that step.
Otherwise, or without that confirmation, the new name may be a racer's item, so both names are
kept and the result is reported as unconfirmed.

Archive extraction never sets a symbolic link's time on FUSE, where sshfs applies it to the target.

## Testing

### Unit tests

Unit tests inject each quirk deterministically through the syscall seam in
`src/adapters/local_operations/fs_ops.rs`. A test installs a scope over its temporary directory:

```rust
let _quirks = FsQuirks::install(root.path(), Quirks::without_rename_flags());
```

Quirks emulate rejected rename flags (NFS, ntfs-3g), missing hard links (vfat, vboxsf), sshfs
collisions reported as `EPERM`, gvfsd-fuse refusing to rename over an empty folder, case-insensitive
lookups, invalid names, unsupported or lying Trash, reused inode numbers, hard links with their own
inode numbers, missing birth times, lost replies (on sshfs, a lost rename reply leaves the target
name reporting its old inode number) and racing processes, before or after a call.

For copies they also emulate a mount's family and superblock options as mountinfo reports them, the
device format udev records, refused `O_TMPFILE` and its links, refused clones, a `copy_file_range`
that ends early, short reads and writes, times reset at close (NFSv4.2 delegations, FUSE writeback
caches), refused `chown`, foreign groups, extended attributes refused by prefix, injected or listed
with ACLs first, refused `mknodat` of files and pipes, NFS silly renames, files that report no size,
reported modes, change times that equal the modification time (FAT), folder birth times that renames
reset (exFAT), times stamped within one tick of a coarse clock (kernels without multigrain
timestamps), single link counts (sshfs), entries renumbered after eviction (never one a descriptor
holds), SMB's refusal to open two names of one file at once or, during its deferred close, another
name of a file just written, its stale read-only folders, change times answered from its attribute
cache, server inode numbers turned off after a collision and creation times Samba makes up, and
writeback errors that `syncfs` reports. Tests can also pretend to run on another kernel release, freeze the clock
that the racily clean check, the wait for SMB's deferred close and the emulated attribute cache
read, and set a paste's batch limits and clock. The seam also asserts that no call runs on the main
context's thread.

For names they emulate the errno each lab mount returned for a name it refuses, names stored or
listed as others (vfat dropping trailing dots, SMB reading private-use characters back and listing
device names under 8.3 aliases, `?` for malformed bytes on SMB with POSIX extensions), length
limits in bytes and for SMB folders, `f_namelen`, refused symbolic links, the cut temporary name
ntfs-3g renames over an entry through, and a filesystem mounted inside another with a device of
its own. Presets for the lab's mounts (`Quirks::vfat()`, `exfat()`, `exfat_fuse()`, `ntfs3()`,
`ntfs3_windows_names()`, `ntfs_3g()`, `cifs()`, `cifs_nocase()`, `cifs_posix()`, `sshfs()`) carry
each mount's mountinfo line and udev format, so the name rules come from the real derivation, and
their refusals are written from the lab's results, not from Strata's rules. Run them with:

```bash
./scripts/test-headless.py local_operations::
./scripts/test-headless.py volume::
```

### Real filesystems

`scripts/fslab/fslab.sh` is a manual lab that CI never runs. It starts one privileged Docker
container that mounts real kernel filesystems over loopback, then runs syscall probes or
`scripts/test-headless.py` with the test fixtures placed on each mount.

| Mount | Filesystem |
| --- | --- |
| `tmpfs`, `ext4` | controls |
| `ext4-casefold` | ext4 with a `chattr +F` folder |
| `vfat`, `exfat`, `ntfs3` | kernel drivers with udisks' default options |
| `exfat-fuse`, `ntfs-3g` | FUSE drivers |
| `nfs42`, `nfs3` | kernel NFS server and client over loopback |
| `cifs`, `cifs-nocase`, `cifs-posix` | Samba with the kernel SMB 3.1.1 client |
| `sshfs` | sshfs speaking SFTP to a loopback `sftp-server` that runs as the test user, in a mount namespace without the client mount |

```sh
scripts/fslab/fslab.sh up                              # build if needed, start, mount everything
scripts/fslab/fslab.sh probe [MOUNT...]                # capability matrix in target/fslab/probe/
scripts/fslab/fslab.sh test FILTER [MOUNT...]          # the working tree, including uncommitted changes
scripts/fslab/fslab.sh test --rev REV FILTER [MOUNT...]
scripts/fslab/fslab.sh down                            # always run when finished
```

`test` sets `STRATA_TEST_TMPDIR` to a directory on each mount. `test-headless.py` passes that
absolute path to the tests as `TMPDIR`, which `tempfile::tempdir()` honours; nothing else is
inherited. Results, logs and leftovers per mount go to `target/fslab/runs/`. The summary flags a
mount on which the kernel cifs client turned server inode numbers off during the run, because
failures after that point can be artefacts of that mount state.

Unit tests layer quirks on a fully featured filesystem, so on a limited mount some of them fail by
design: a test expecting a plain rename sees the ladder link instead on NFS, fixtures with
symlinks, FIFOs or Unix permissions cannot exist on vfat, a racer keyed to a step the mount never
takes (an `O_TMPFILE` link, a hole, a `syncfs`, an exchange) never runs, and a time with a fraction
of a second is rounded away on vfat, exfat-fuse and sshfs. Those three report the modification
time as the change time, so tests that land the SMB client's deferred closes fail there after two
seconds. A fixture's name can be one the mount refuses (`a:b` on vfat), and a preset can expect a
name stored that the real mount refuses, folds or lists as another (Samba's 8.3 aliases, long SMB
folder names). On NFS a file renamed over while open reads as `.nfsXXXX`, and on casefolded
folders `/proc` shows files the test process created as `name (deleted)`, so quirk patterns by
name miss them. The container has no udev database, so
exfat-fuse and ntfs-3g are not recognised there: they keep their per-file syncs, and get the name
rules of a `fuseblk` device udev cannot tell, which learn from refusals (ntfs-3g refuses device
names). Read failures against a run of the same filter on `origin/main`.

A FUSE daemon gets every name a preset over it passes on, and exfat-fuse stores overlong forms
and encoded surrogates, writing an overlong `/` into its volume as a real `/`, which `fsck.exfat`
then finds corrupt. The tests that send such names, through a preset that stores any bytes or as
fixtures, skip each row with a name that is not valid UTF-8 on a FUSE base and say so on stderr
("skipped on a FUSE base"); a new test that sends one must call `skips_malformed_name_on_fuse` the
same way. Fixtures with other malformed names, which the daemons refuse, fail there instead. Run
`down` and `up`, which format fresh images, before checking names or the volume on exfat-fuse.

Requirements and safety:

- Docker on amd64, as a non-root user in the `docker` group, and the host modules `loop`, `vfat`,
  `exfat`, `ntfs3`, `nfsd`, `nfs`, `cifs` and `fuse`. The image builds on the verified E2E base
  from `scripts/e2e_base.py`.
- The container is privileged: it loads host modules and attaches loop devices in the host
  kernel. It has no network; its NFS, SMB and SFTP servers listen only on its own loopback.
  Crates are fetched first by a separate unprivileged container, and the build runs offline.
- Images live on a container-private tmpfs and are attached blank, so the host's udev never sees a
  filesystem. `up`, `probe` and `test` check this. Only udiskie ignores the images' directory, so
  stop other automounters (GNOME Disks, KDE) while the lab is up.
- The NFS exports and the sshfs server's folder are container-private tmpfs mounts, not loop
  devices. After full runs, Linux 7.2 kept a reference to the NFSv4.2 export after nfsd had
  stopped, with no client, thread or process left, which pinned the export's loop device and its
  memory on the host until reboot. A leaked tmpfs export pins only its few remaining files. tmpfs
  has no UUID, so each export sets `fsid=`; it reports birth times, which NFSv4.2 passes on.
- `down` releases everything that is free and detaches only loop devices still backed by the
  lab's own images, deleting each image once its device is gone. If a mount or loop device stays
  busy, `down` lists what holds it and keeps the container running with that mount, its server and
  its loop device; run `down` again once those are released. `down --force` unmounts lazily
  instead, which can keep loop devices attached on the host, or a tmpfs export's memory, until
  reboot.
- The container runs its own copy of `scripts/fslab`, refreshed by every `fslab.sh` command, so
  switching branches or rebasing while the lab is up cannot break `down`.
- The NFS, SMB and SFTP servers share the kernel with their clients, so there is no latency,
  reconnect or multi-client race. ZFS, 9p, virtiofs, eCryptfs and FUSE network filesystems other
  than sshfs are not covered.

Never point tests at real user shares or removable drives.

The historical [lab measurements](https://github.com/lgse/strata/pull/1615#issuecomment-6208891751)
are retained with the pull request. They use memory-backed loop devices and loopback servers, not
real USB latency or independent network clients.
