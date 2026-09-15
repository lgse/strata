# Archive creation

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
Compression selection does not relax these restrictions or change staged
publication, conflict handling, or permissions.

## Existing archive names

When the requested archive already exists, the creation prompt offers Cancel,
Keep Both, and Replace. Keep Both publishes the next available numbered name,
such as `archive (1).zip` or `archive (1).tar.gz`, and selects that new archive.
It skips existing files, directories, and symlinks, including names that appear
while encoding is running. The completed staging file is retried atomically at
publication; the sources are not compressed again for each collision. Replace
retains its existing overwrite behavior.

## Feedback and cancellation

Archive operations show `Preparing…` immediately, then completed-file counts.
An activity spinner remains visible while the current file is processed and
while the archive is finalized; it is not a byte-level progress estimate.

Cancellation is cooperative. Encoded output and TAR's input-to-encoder writes
check cancellation, as do 7Z source reads and existing ZIP copy chunks. Errors
from these checks become cancellation results, not corruption/password errors.
The UI waits for the worker to return before removing progress and showing the
cancellation summary. Staging is discarded rather than published, and replacing
an existing archive leaves that archive intact on cancellation. Individual
blocking filesystem calls and codec calls still have to return; cancellation
is not an immediate thread kill.
