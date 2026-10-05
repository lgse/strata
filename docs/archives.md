# Archives

## Extraction safety

Archive member paths are untrusted metadata. If an extractor joined a selected
folder with `../report.txt` unchanged, the filesystem would resolve the result
beside that folder instead of inside it. This archive path-traversal pattern is
also known as Zip Slip.

Strata resolves parent components without allowing them to walk above the
selected destination. For example, both `../report.txt` and
`folder/../report.txt` become `report.txt`. Extraction then continues with later
members instead of aborting the entire operation. Empty paths, absolute paths,
and Windows drive prefixes remain invalid.

The sanitized path still passes through the descriptor-relative destination
writer, which refuses symlink traversal. Normal conflict renaming also applies,
so two members that sanitize to `report.txt` become `report.txt` and
`report (2).txt` rather than overwriting one another.

## Extraction output

Strata writes members into a hidden `.strata-extraction-<id>` folder inside the
destination, created when the first member is written. It is visible during
extraction only when hidden files are shown. When the archive has been read,
Strata publishes the folder:

- A single top-level entry is moved up into the destination under its own
  name. It is renamed to `name (2)`, `name (3)` and so on only if the
  destination already has an entry with that name, including a symlink or
  special file, which is never followed or replaced.
- Several top-level entries stay together in a folder named after the archive
  without its extension, such as `photos/` for `photos.zip`. If that name is
  taken, the folder becomes `photos (1)/`, `photos (2)/` and so on. Entries
  inside the folder keep the archive's names, even when the destination
  already has an entry with the same name; only duplicates within the archive
  are renamed.
- If extraction fails or is cancelled after writing a file, everything written
  so far stays in the archive-named folder, even if it is a single entry. The
  error message ends with ``Extracted entries remain in `<folder>`.``, and the
  cancellation summary lists completed and pending entries inside that folder.
  If no file was written, any folders created along the way are removed and
  nothing is left behind.
- If extraction stops because a password is missing or incorrect, everything
  written so far is discarded, including members that were not encrypted.
  The password prompt extracts the whole archive again, so the retry lands
  under the archive's own name instead of `photos (1)/`. Damage in a member
  that is not encrypted is reported as damage, even when a password was
  given. If the output cannot be removed, the failure is reported like any
  other, with the reason and the folder that keeps the output, and no password
  prompt opens.

Publication never replaces an existing entry. If the filesystem lacks atomic
no-replace directory renames, folder publication fails and the error names the
hidden staging folder holding the output. Single files can use a no-clobber
hard-link/unlink fallback when supported.

Partial output from any other failed attempt stays in its own folder, so a
retry never merges into it. An archive with several top-level entries is
extracted into the next numbered folder; a single entry lands in the
destination under its own name, as usual.

`.tar.gz` archives are read to the end of the gzip stream, including every
gzip member that parallel compressors such as pigz write, so each member's
CRC32 and length trailer is verified. A mismatch or a truncated trailer is
reported as a damaged archive. Zero bytes after the last member, as tape
blocking or `dd` leave them, are padding rather than damage; any other
trailing bytes are reported as a damaged archive. Members already written are
kept as described above, but their contents are unverified.

## Links, permissions and times

Symbolic links are recreated with their stored targets, including absolute
targets and targets outside the archive, and are never followed by later
members: a member under a link name fails instead of writing through the link.
TAR names and hard-link targets retain their native bytes. Hard links become
links to the member of that name extracted earlier from the same archive,
following any conflict rename. A hard link to a member that
was not extracted, or that appears later, fails the extraction. FIFOs and
device nodes are refused. An existing symlink at a member's name, such as an
earlier link member with the same name, is skipped like a file: the member is
renamed to `name (2)`.

Each member gets its stored permission bits, masked by the umask, with setuid,
setgid and sticky bits removed, and its stored modification time. ZIP times
come from the Info-ZIP extended timestamp, then the NTFS field, then the DOS
time read as local time. Members without a stored mode keep the default
permissions. Folder permissions and times are applied only when extraction
completes, so partial output from a failed or cancelled extraction stays
writable. A folder listed more than once in the archive is restored once:
each of its permissions and time comes from the last entry that stores it.
Folders created only as parents of other members keep default
permissions. Owner, group, access times, extended attributes and ACLs are not
restored. RAR archives restore permissions and times, but their link members
still extract as regular files.

On filesystems that cannot store Unix permissions or times, such as FAT and
exFAT, those are skipped without an error, as `tar` and `unzip` do. Links
cannot be skipped: an archive with links fails there, naming the member, and
keeps the output written so far.

## Extraction targets

**Extract here** and **Extract to…** in the item context menu, the 10xer `; e`
and `; E` chords, and extraction on Enter or double-click apply only to a
regular file, or a symlink to one, with a local path and a recognised archive
extension. A folder named `photos.zip` opens like any other folder and offers
no Extract actions. FIFOs, sockets, devices, and broken links named like
archives are not extracted either; activation opens them externally and the
chords report "Not an archive".

If the item changes between opening the menu and running the action, the
operation layer checks the path again. A path that exists but is not a regular
file fails with ``Not an archive: `<name>` `` before any destination folder is
created.

Only a failure that the decoder reports as a missing or incorrect password
opens the password prompt, with "Invalid password" for an incorrect one. The
wording of an error message never decides it, so a member or archive name
containing "password" cannot open the prompt.

## Archive creation

Strata chooses compression according to the output container, not just the
selected filename extension.

| Output | Payload encoding | Password support |
| --- | --- | --- |
| ZIP | Stored for known already-compressed members; DEFLATE level 6 for other files | AES-256 for either method; filenames remain visible |
| 7Z | Copy for known already-compressed members; LZMA2 level 6 for other files | AES-256 for both methods, plus encrypted headers |
| TAR | No compression | None |
| TAR.GZ | One gzip member containing the entire TAR stream; stored DEFLATE blocks when all regular-file payloads are known already-compressed, otherwise default gzip compression | None |
| RAR | Extraction only | No RAR creation |

ZIP and 7Z can mix methods in one archive. 7Z currently writes independent,
non-solid members: one file cannot borrow another file's compression dictionary.
LZMA2 streams on the compression worker rather than buffering 64 MiB jobs for
additional codec threads. Its dictionary is capped at the member's size (minimum
4 KiB, maximum the level-6 default of 8 MiB). This avoids oversized per-file setup
and keeps worker lifetime tied to the operation. It does not parallelize large
individual members.

Gzip has no per-file method field. A mixed TAR.GZ therefore compresses *all* TAR
bytes, including any already-compressed members. The all-compressed fast path
still emits a valid gzip header, DEFLATE stream, checksum and trailer, not raw
TAR or concatenated per-file gzip streams. It trades a little size overhead,
including uncompressed TAR metadata/padding, for avoiding recompression. TAR
retains its existing sparse-file and symlink handling. Choose ZIP or 7Z when
per-member compression selection matters for mixed inputs.

## Classification and limits

The shared, case-insensitive extension heuristic is in
`src/adapters/local_operations/archive/compression.rs`. It covers common compressed
media, compressed archives and compressed document/package formats. Compound
names such as `.tar.gz` and aliases such as `.tgz` are recognized. Raw containers
such as `.tar`, `.bmp`, `.wav`, `.avi` and `.iso`, extensionless files and unknown
extensions retain compression.

Extensions are hints, not content validation: an MP4/MOV/MKV container can contain
uncompressed streams, a PDF can contain plain data, and even a ZIP can contain
stored members. Such exceptions may yield a larger output. Strata does not read
all payloads to estimate compressibility. The existing preflight walk opens
entries to count members and classify names; payloads are streamed once during
encoding. Directory recursion remains descriptor-relative and symlinks are not
followed. Sources are reopened for encoding, so concurrent source edits can
change the usefulness of the preflight gzip choice, but not the output format.

ZIP preserves UTF-8 symlink targets; TAR preserves native symlink targets. 7Z
creation rejects symlinks rather than following or silently replacing them.
Every format records each member's modification time and Unix mode. ZIP stores
the time as local DOS time, clamped to 1980–2107, plus an Info-ZIP extended
timestamp; TAR records symlink times too; 7Z uses the p7zip Unix-mode
attribute.

## Existing archive names

When the requested archive already exists, the creation prompt offers Cancel,
Keep Both, and Replace. Keep Both publishes the next available numbered name,
such as `archive (1).zip` or `archive (1).tar.gz`, and selects that new archive.
It skips existing files, directories, and symlinks, including names that appear
while encoding is running. The completed staging file is retried atomically at
publication; the sources are not compressed again for each collision. Replace
retains its existing overwrite behavior.

## Feedback and cancellation

Archive operations show `Preparing…` while counting members, then `Compressing…`
(or `Processing archive…` for extraction) and completed-file counts. A large
member can take time before the completed count advances. The activity indicator
continues animating during that work and finalization; it is not a byte-level
progress estimate.

Copying, compression, deletion, and drive formatting use independent progress
cards at the bottom right, without a progress modal or a restore action. Browsing
continues while the jobs run. Cards show the full filename/status and destination
(or the drive being formatted), wrapping long text instead of truncating it.
Moves, extraction, and undo/redo keep their foreground presentation.

The X cancels only its own copying, compression, or deletion job. Once requested,
it is disabled while the worker stops; device-writing warnings remain in that
card, without a separate banner or a Return to browser action. Do not unplug a
drive while cancellation or writes are pending. Formatting cannot be cancelled
once started: its card keeps the unplug warning visible and offers Close only
after completion. Normal progress and pending cancellation use neutral styling,
not error borders or text.

Successful cards show an operation-complete title, a clickable **Complete**
status, and `100%`. Their bar becomes a five-second auto-dismiss countdown.
Hovering or focusing a card pauses the remaining time; leaving resumes it rather
than restarting the countdown. Pin a completed card to keep it until dismissed.
**Complete** or X dismisses only that notification, never another operation.

Finish or cancel running operations before closing their window. Completed
cards are notifications, not active jobs, and do not block window closure.

Cancellation is cooperative. Encoded output and TAR's input-to-encoder writes
check cancellation, as do 7Z source reads and existing ZIP copy chunks. Errors
from these checks become cancellation results, not corruption/password errors.
The UI waits for the worker to return before removing progress and showing the
cancellation summary. Compression staging is discarded rather than published,
and replacing an existing archive leaves that archive intact on cancellation.
Cancelled extraction keeps the members already written inside the
archive-named folder described in [Extraction output](#extraction-output).
Individual blocking filesystem calls and codec calls still have to return;
cancellation is not an immediate thread kill.
