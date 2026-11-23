# Architecture Principles

This document records boundaries and constraints, not a frozen class hierarchy. Abstractions should be introduced where behavior varies, work crosses an asynchronous boundary, or a subsystem needs isolated tests.

## Principles

1. **Model product concepts, not widgets.** Navigation paths, locations, entries, selections, operations, and previews must not depend on a specific view instance.
2. **Keep the UI declarative.** Widgets render state and emit intent; they do not perform filesystem work directly.
3. **Make stale work harmless.** Navigation, peek, search, metadata, and preview requests carry cancellation or generation identity.
4. **Stream bounded results.** Large directories and searches arrive in batches with backpressure.
5. **Use capability boundaries.** Search, preview, themes, settings, and file operations expose the capabilities the application needs rather than leaking backend APIs.
6. **Prefer concrete code until variation is real.** Do not create a trait for every type. Extract a boundary when there is a second implementation, a test substitute, or a meaningful isolation requirement.
7. **Keep extensions outside the trusted core.** A future public extension system should use a versioned out-of-process or sandboxed protocol rather than Rust's unstable dynamic-library ABI.
8. **Preserve native paths.** Internal paths must not assume valid UTF-8.
9. **Make observability part of the design.** Slow requests, cancellation, operation failures, and provider errors should be traceable.

## Proposed layers

```text
UI
  Renders application state and sends user intents
        │
Application
  Navigation, selection, history, commands, orchestration
        │
Capabilities
  Files, operations, search, previews, themes, settings
        │
Adapters
  Local filesystem, desktop integration, tools, theme sources
```

Dependencies point inward. A filesystem adapter must not manipulate widgets, and a preview provider must not own navigation state.

## Core product models

- `Location`: a browsable destination with a stable identity
- `FileEntry`: native name/path, type, metadata availability, and capabilities
- `NavigationPath`: committed locations represented by Miller columns
- `PeekState`: temporary location, origin, request generation, and lifecycle
- `SelectionState`: active column, focused item, and multi-selection
- `ViewPreferences`: mode, density, type grouping, sorting, hidden files, and thumbnail policy
- `Operation`: queued file mutation with progress and final outcome
- `PreviewRequest` / `Preview`: bounded request and renderable result
- `SearchQuery` / `SearchResult`: explicit scope and streaming result
- `Theme`: validated semantic tokens with fallbacks

Models should distinguish “unknown/not loaded” from meaningful empty values.

### Dialogs and form controls

Action dialogs use `strata-components`' modal dialog primitives and `rune-kit` controls styled
through `strata-kit::Look` tokens. The shell's overlay and dialog slots own header alignment,
icon bezels, body and action spacing, focus treatment, backdrop blur, and semantic accent or
danger states; dialog-specific parts in `strata-app` supply content and behavior. The search
palette remains a specialized command interface because its query field and results are one
continuous keyboard surface. Settings remains a specialized navigable workspace rather than an
action dialog.

### Browser presentation modes

Browser presentations in `strata-app` (`columns`, `icons`, `list`) consume the same queued
`BrowserEvent` stream and send intents back to `strata_fs::app::Browser`. Columns, the
single-pane Icons grid, and the single-pane List table do not own independent filesystem or
navigation state. Reusable view components live in `strata-components` and receive their data
via accessors; `strata-app` adapts `Browser` snapshots into `listing::Column` rows and drives
view-specific selection, rename, marquee, and pointer policies.

Because `Browser` observers fire inside GLib's main-context dispatch while Rune owns the
application state, observers only push events into a queue (`crates/strata-app/src/events.rs`),
and parts apply those events during Rune's `poll` or update turn. `NavigationStarting` and
`ColumnReloading` are the two pre-mutation capture events: the outgoing directory, or a column
about to be cleared for a reload, is still readable when they arrive.

Pointer intent and marquee selection are shared across Columns, Icons, and List through
`strata-components` (`collection`) and `strata-app` (`pointer`, `clicks`, `dnd`). List file
drags start only on the icon or rendered filename; unused Name space, row padding, and metadata
columns remain marquee origins. Columns and Icons treat unselected whitespace and gutters as
marquee origins while allowing selected rows/cards to initiate drags; Alt-drag forces a marquee
from an item in any mode. Plain presses on pane background or blank chrome clear the selection,
Ctrl/Shift presses preserve it for union/range selection, and presses on controls, drag handles,
or scrollbars never start a marquee. Returning a folder drag to that same folder does not
spring-open it; other destination folders retain hover navigation.

### Collection behavior and lifetime boundaries

The browser and open/save chooser share the same collection interaction model and search
pipeline, with chooser-specific restrictions enforced by policy in `strata-app::chooser`:

- Normal Columns, List, and Icons resolve displayed rows through `strata-app::listing`
  (including List grouping and column sorting). Basenames are never identity; every action
  resolves by `Location` / `EntryId`.
- Inline filter and recursive search keep their worker/receiver, query generation, debouncing,
  and stale-result rejection off the UI thread. Changing query text clears stale selection so
  Enter opens the highlighted top match unless the user explicitly selects another result.
- Inline rename and new-entry creation share validation, stem selection, and lifecycle cleanup
  across Columns, List, and Icons (`strata-app::rename` and `strata-app::new_item`). Navigating
  away, unbinding a row, or tearing down a view cancels active edits cleanly.
- Fast-scroll thumbnail and metadata admission prioritizes visible and overscan rows, cancels
  stale requests, and refreshes bindings without resetting active rename editors. Completed file
  operations invalidate affected folder counts across browsers through `file_activity`.

### Shell and application map

`crates/strata-app/src/shell.rs` composes the window from eight named slots (tabs, location bar,
sidebar, view, drawer, footer, overlays, and dialogs). Parts implement `slots::Part` and
register in `crates/strata-app/src/lib.rs` (see [Workspace crates](workspace.md#parts-of-the-rune-interface)).

| Responsibility | Owner (`crates/strata-app/src/`) |
| --- | --- |
| Shell composition, event loop, slot contract | `shell.rs`, `slots.rs`, `events.rs`, `host/` |
| Browser views and row/column listing model | `listing.rs`, `columns/`, `icons/`, `list/`, `pane.rs`, `pane_header/` |
| Pointer intent, clicks, marquee, drag-and-drop | `pointer.rs`, `clicks.rs`, `dnd/`, `drop_dialog/` |
| Hover peek lifecycle and placement | `peek.rs` |
| Inline rename and new-entry workflows | `rename/`, `new_item/` |
| Location editing, breadcrumbs, mount auth | `location/`, `mount_prompts/`, `connect_server/` |
| Context menus, popovers, chooser restrictions | `menus.rs`, `popover/`, `chooser/` |
| Clipboard/cut intent and file-manager formats | `clipboard/` |
| Transfers, conflicts, archives, undo/redo | `transfer.rs`, `conflict_dialog/`, `archive.rs`, `compress/`, `trash/`, `undo/` |
| Properties, permissions, item customization | `properties/`, `customize/` |
| Previews, thumbnails, metadata fills | `preview/`, `thumbnails/`, `fills/`, `details/` |
| Settings dialog, themes, transparency, shortcuts | `settings/`, `settings_dialog/`, `appearance/`, `theme.rs`, `shortcuts/` |
| Sidebar, tabs, search palette, filter, footer | `sidebar/`, `tab_strip/`, `palette/`, `filter/`, `footer/` |
| 10xer mode, key routing, and GPU effects | `tenxer/`, `tenxer_fx/`, `window_keys/`, `fx/` |

New File and New Folder allocate real entries through `app::Browser` and the operation provider.
`crates/strata-fs/src/adapters/local_operations/create_entry.rs` shares atomic conflict retries
for files and directories and closes an empty file before reporting its creation. `EntryCreated`
carries the allocated location, which selects the real row and starts a normal inline rename.
A pending request identity prevents late focus after cancellation or navigation. There are no
temporary creation rows; both new and existing items use the same validation and rename path.

Every local operation that puts an item at a name goes through `placement::place` in
`adapters/local_operations/placement.rs`, a no-clobber ladder that adapts to what the mount
supports. Its syscalls go through the `FsOps` seam in `fs_ops.rs`, and what each mount supports is
cached in `adapters/volume/mount_capabilities.rs`. See
[Filesystem compatibility](filesystem-compatibility.md).

The names a mount stores are derived from mountinfo, udev and `statfs` in
`adapters/volume/names.rs` and `mounts.rs`, and cached with its capabilities;
`adapters/local_operations/mount_profile.rs` reads them once per operation, so its planner and
workers agree. Its sibling `names.rs` fixes, plans and numbers names by those rules for copies,
moves, merges and extraction; `transfer_targets.rs` works out, off the main thread, the name each
pasted item will get and whether something has it, for the conflict prompt; and `typed_names.rs`
refuses a typed name the location would store as another.

Copies between native paths, including moves across filesystems, merges, Replace stages and
restore copies, go through `adapters/local_operations/copy_engine.rs`. It copies through
descriptors on worker threads, on the queue `work_queue.rs` shares with deletions, and records
what a move read so that only unchanged sources are deleted. `owed.rs` batches the deletions a
paste owes behind one durability barrier per destination filesystem, and the completion report
reaches the UI as `OperationEvent::ItemsReported`. Replacement/merge, deletion, restore/undo and
progress orchestration live in their adjacent `replacement`, `deletion`, `restore` and `progress`
modules. `native_fs` owns shared descriptor primitives, the per-mount identity rule (`compare`)
and `Pinned`, the `O_PATH` hold for items nothing else proves; the syscall seam composes
`FsEnvironment` for Trash, mount-table, udev and `/proc` holder queries without a second injection
registry. Identities that outlive an operation (Trash items, saved views of trashed items, a
folder's load stamp) are `services/item_identity.rs`'s `ItemIdentity`, which carries its mount's
traits and compares without I/O.

Every mutating `OperationProvider` entry, Empty Trash and a drive's unmount hold an
`ActiveOperation` (`adapters/local_operations/release.rs`) until their final event. It registers
the operation in `services/file_activity.rs`, the in-memory registry of Strata's own reads and
running operations, and releases what the operation changes: it fences those paths against new
reads, stops the registered reads under them (previews, thumbnails, sandboxes, metadata probes and
background listings) and waits for them within a budget. Readers register at the funnels they
already share, and preview drawers and thumbnails follow the registry's notices on the main thread.
`leftovers.rs` removes folders that hold only other programs' silly-rename leftovers, and
`holders.rs` names the program that still has a file open; see
[Files that are still open](filesystem-compatibility.md#files-that-are-still-open).

Every hidden name Strata creates in a user's folder, and every one earlier builds created, is in
the pure table `model/hidden_names.rs`, which adapters take their names from and which classifies a
listed name by its whole form. `app/browser/leftovers.rs` classifies a settled column's names in
memory, hides them while `file_activity` shows an operation in that folder, and emits
`LeftoversChanged` for the pane button and the once-per-session dock card. The review dialog
in `strata-app` reads the items through `OperationProvider::review_leftovers`, and
`adapters/local_operations/leftovers.rs` restores originals (`recover`, behind the same
`ActiveOperation` as every operation) and sweeps stale probes; see
[Leftovers after a crash or unplug](filesystem-compatibility.md#leftovers-after-a-crash-or-unplug).

Filesystem work for Trash lives in `adapters/trash.rs`. Measurement shares one entry/time budget
across root and descendant batches; depth truncation and unreadable descendants remain branch-local.
Deleting Trash streams its own batches, independently of any incomplete measurement. Native path
and GIO URI conversion lives in `adapters/gio_location.rs`, shared by files, operations, preview and
browser presentation. It preserves native bytes, keeps GLib's normalized percent-encoding (so names
that are not valid UTF-8 and encoded slashes survive), and sanitizes credentials on inbound GIO
locations. `Location::display_name` names headings, breadcrumbs, tab labels and the window title.
GVfs reports "/" as the leaf name of a mount root, even an SMB share's, so such a root is named
from its URI: the percent-decoded last path segment, or the host at a server root. Only the shown
name is decoded; the URI used for I/O is unchanged.

Local archive operations live under `adapters/local_operations/archive/`:

| Responsibility | Owner |
| --- | --- |
| Operation entry points, worker lifecycle and progress events | `archive.rs` in the parent directory |
| Staged publication, source traversal and compression writers | `compression.rs` |
| Per-operation extraction state, copying, cleanup, size preflight, hard-link resolution, deferred directory metadata and outcomes | `extraction.rs` |
| Confined destination writes, link creation, mode and time restoration, path validation, staging folder lifecycle, publication, member names by the destination's rules and conflict naming | `destination.rs` |
| ZIP, TAR/gzip and 7z member enumeration, passwords and decoder errors | `decoders.rs` |

Every decoder feeds one `ExtractionSession` per operation. The session has no codec or widget
API dependencies; decoders lend it member streams and the archive name, and provide
already-known pending names on cancellation. The session writes members into a hidden staging
folder under the destination and publishes it in `finish` for every outcome: a single root moves
up verbatim, several roots are renamed to the archive stem, and failed or cancelled output stays
in the archive-named folder unless it holds only directories.
A password failure instead discards the staging folder, since the retry extracts everything
again. Decoders report it as `ArchiveError::PasswordRequired` or `IncorrectPassword`, and its kind
travels as `PasswordFailure` on `OperationEvent::Failed` and `BrowserEvent::OperationFailed`;
that kind alone decides the password retry, never the message text. Only a member that is
encrypted and was given a password reports malformed data as a possible wrong password. Staging
that cannot be discarded turns the failure into an ordinary one.
A directory member repeated in the archive is restored once, each field from the last entry
that stores it.
Member identity tracking stays
inside each decoder rather than assuming unique names or matching header/callback order. The
session validates pending names and maps them through the folders it made, fixing the rest by the
destination's rules, without filesystem probes or name reservations; final leaf conflicts remain
unknown until a member is attempted. Sequential formats do not scan unread content to complete
that list.
Before writing, the session checks claimed uncompressed size against destination free space from
`fstatvfs` on the pinned root, and it refuses a member whose extracted size does not match the
size declared by the archive header. ZIP and 7z advertise a total up front, so an oversized
archive is refused before any member is written; TAR streams check each member as it arrives, so
extraction stops at the free-space boundary and members already written stay inside the
archive-named folder, which the failure message names. The
guarantee is that extraction never exceeds the free space observed when the session opened;
it does not model per-file overhead such as block rounding or inodes. Filesystems that report no
capacity (`f_blocks == 0`, as FUSE mounts without `statfs` do) skip the free-space checks and
keep only the declared-size match. The `zip` crate does not bound inflated output by the header
size itself, so that match is the control that stops a ZIP member lying about its size.

Decoders pass symlinks, TAR hard links and each member's mode and modification time through
`MemberContent` and `MemberMetadata`, and refuse FIFOs and device nodes. Restoring metadata is
best effort: `EPERM`, `EOPNOTSUPP` and `EINVAL` from filesystems without Unix permissions or times
are ignored in `destination.rs`, which makes those calls through the `FsOps` seam. TAR extraction
preserves native path bytes where the destination stores them, and hard-link target identity
through fixed names. The sandboxed RAR helper streams
`STRRAR03` records carrying each member's mode and
time (a RAR 5 FILETIME, or the DOS local time of older formats, which only the parent can
convert in the user's zone); RAR links are not yet extracted as links. Error records and failed
member trailers carry a failure kind, so a missing or incorrect password reaches the parent
without parsing text. Format
libraries remain behind the adapter boundary. See [archive creation](archives.md) for
container-specific encoding, classification and cancellation behavior. Archive unit tests sit in each module's
adjacent `tests.rs`; provider-level tests remain in `archive/tests.rs`, with shared test-only builders
in `fixtures.rs`.

Feature unit tests sit beside their implementations in adjacent `tests.rs` modules. Headless
workspace tests run through `mise run test` on Rune's virtual clock; compositor, D-Bus, and
windowed tests run in private headless sway sessions via `mise run test-platform`; and end-to-end
QA scenarios run in the pinned container via `./scripts/qa.sh` (see [testing.md](testing.md)).

Code on the UI thread makes no file call, except those listed under [Off the interface
thread](filesystem-compatibility.md#off-the-interface-thread). `strata_core::ui_thread` marks that
thread, and the syscall seam, `FsEnvironment`, the probes, `MountTable::current()` and the settings
writer's file calls assert that they run elsewhere; only named, scoped exceptions (the reads that
build the first window, the exit flush) let them through. The interface asks instead:
`adapters/local_operations/probes.rs` for presence, folder checks, writable destinations and
typed-path listings; `adapters/volume/mount_watch.rs`, a thread that keeps the mount table published
for readers that do no I/O; `adapters/volume/mount_lanes.rs`, which bounds the folder loads, probes,
monitors and other requests it admits to a mount that stops answering;
`adapters/local_files/monitors.rs`, the one worker that creates and cancels file monitors;
`adapters/gio_close.rs`, which closes the enumerators and streams the interface lets go of in the
background (GLib still closes a GVfs stream whose open succeeded just as it was cancelled); and `storage/writer.rs`, the background writer for Strata's own settings and state. See
[Unresponsive mounts](filesystem-compatibility.md#unresponsive-mounts) and the [main-thread I/O
audit](testing.md#main-thread-io-audit).

This separation is not a redesign of operation policy. Alternate renderers, staged
publication/metadata orchestration, native transfer security and the settings workspace should be
refactored independently of browser composition. Investigation and scope decisions are recorded in
[issue #397](https://github.com/lgse/strata/issues/397).

### Browser directory-event routing

`app/browser/loading.rs` dispatches provider events through an owned open-load target:
native batches stage for sorting/publication, while remote batches keep the first-batch
and coalesced-tail paths. Completion carries truncation and both filesystem capabilities
together. Requests outside the open-load gate retain their existing peek/failure handling;
stale work is still checked against the owning directory or peek request.

A load's `DirectoryEvent::Stamp` arrives before its first batch: it requests the column's change
monitor, which `directory_changes.rs` checks once it is live, and becomes the column's baseline for
`app/browser/freshness.rs` when the listing is shown. That scheduler checks the visible folders that
other writers can change, or that have no live monitor, through `FileSource::check_stamps`, one
background job per mount, and re-lists a moved one through a background request that keeps the
listing if it fails or falls short. A check that finds the folder gone or replaced feeds the same
departure check as a monitor's removal report. Its inputs come from the interface: focus,
visibility and power saver from window presence tracking in `strata-app`, and the "Watch network
folders" preference. It also follows each column's mount and emits `ColumnNotResponding` for the
notice. See
[Changes made elsewhere](filesystem-compatibility.md#changes-made-elsewhere).

`loading/metadata.rs` separates full-sort fills, applied by location, from viewport fills,
validated against directory identity and row-position/location tokens. Metadata chunks
never complete a sort; `MetadataFinished` retains that responsibility. Both modules release
state and routing borrows before synchronous observer dispatch, allowing observers to
navigate safely. Sorting, metadata scheduling, and cancellation remain in the browser
controller; event routing does not change those policies.

A folder's own directory monitor reports its deletion, its rename, and its move alike, as
the removal of the folder itself. Inotify follows the folder rather than its path, so it
reports nothing when an ancestor is renamed or moved and goes on to report later changes
under the old path; the local adapter therefore looks the folder up at its path, on its
mount's lane, after each batch of changes, and reports its removal when the path holds no
folder or one that the identity recorded by the load that asked for the monitor proves to
be another. On mounts that renumber folders nothing proves that, so there a folder put
back at the old path keeps receiving the moved folder's changes until the folder is opened
again, since a reload keeps the column's monitor.
A column whose parent column is its parent folder leaves that report to the parent
column's monitor, which sees the departure as a change to one of its entries, and checks
only that the first column of that chain is still where it was, since no open monitor sees
a rename above it; any other column, including the depth-0 folder, is checked in
`directory_changes.rs`. A folder that validates again (deleted and recreated) reloads in
place, which renews its monitor. A missing depth-0 folder is looked up by the identity its
load's change stamp recorded off the main thread (an `ItemIdentity`), compared by the
mount's [identity rule](filesystem-compatibility.md#identity): a birth time keeps a new
folder that reuses the inode from passing for a rename, and on mounts that renumber folders
without reporting their birth times (sshfs, exFAT) nothing proves a folder, so it counts as
departed. The lookup (`find_by_identity`) searches the nearest existing ancestor: its
folders when the parent remains, else the same path below each of its folders, which finds
a folder whose ancestor was renamed in place. Found there, the folder was renamed in place,
itself or with that ancestor: the search indexes are rebased and the open columns relocate
(`ColumnsRelocated`), as for an in-app rename. Not found, with an ancestor that cannot be
listed, or before its identity is known, it departed: the nearest
existing ancestor that the source allows is restored without a history entry, or the
folder shows its read error when there is none. Backends without identities treat every
rename as a departure. A folder or ancestor that cannot be reached (unmounted, offline, or
not permitted) stays as it is. A monitor can lose the removal report, when a burst of
changes turns its batch into a rescan or when a reload supersedes the check, so a failed
reload of a depth-0 folder that loaded before runs the same check; it leaves the read
error in place unless the folder is confirmed gone. A departure that arrives during a
deletion, restoration, or transfer waits with the operation's other changes, which apply
first.

### Browser staged publication

`app/browser/publication.rs` owns staged row publication on the same `Browser`, not a
second controller. A `PublicationPlan` captures request identity, row count, selection,
and terminal payload; only the separate progress cursor advances while tails stream.
Sort paths capture the plan before notifying preference observers.

Inline publication, idle completion, and synchronous draining share selection/terminal
dispatch. Selection follows the final rows; a load's metadata retry follows its load
completion event. Idle tails reject superseded directory requests and respect both the
captured total and current model length. Draining instead publishes the entire remaining
current model before mutations that require convergence. Borrows end before synchronous
observer calls, including cancellation from a tail observer.

Inline thresholds, prefix/chunk sizes, idle priority, and the cooperative time budget
are unchanged. Queue ownership and cancellation/truncation remain on `Browser`; sort
workers, remote coalescing, and operation callbacks are separate responsibilities.

### Window composition and sidebar

`crates/strata-app/src/startup/` and `crates/strata-app/src/host/` own application startup:
claiming the UI thread, loading saved preferences and themes, mounting the shell and its parts,
starting the embedded GLib loop, and scheduling initial navigation, portal integration, and due
update checks.

`crates/strata-app/src/sidebar/` assembles the sidebar and connects its preferences, browser
events, and device monitors. Standard places, pinned folders, and device rows are rendered as
separate sections, with the chooser's local-only filter preserved. Bookmark storage lives in
`crates/strata-fs/src/adapters/bookmarks.rs`: every pin, unpin, reorder, and deletion cleanup
is one job on the background settings writer (`storage::writer`) that reads the shared
`gtk-3.0/bookmarks` file, applies the change, and saves it atomically, retrying up to twice if
another program changed the file concurrently. Every open sidebar adopts the updated pins.

Folder pinning and pin reordering work through dedicated drop gaps between pinned rows (including
an empty section). Internal folder drags carry a provider-matched location snapshot without
filesystem probes; external folder drops validate directory eligibility asynchronously with a
two-second deadline, reject Trash locations and web links before probing, and require external
sources to offer Copy so completing a drop never asks another application to delete originals.

### Window keyboard routing

`crates/strata-app/src/window_keys/` owns window-level keyboard routing, reading presses through
`strata_desktop::keys` (with `xkbcommon` layout fallback) and matching catalog bindings (see
[keyboard shortcuts](keyboard-shortcuts.md)). Its ordered stages preserve shortcut precedence:
modal and inline-editing ownership, transient dismissal, window/file commands, focus traversal,
and item/directory navigation. Editable text fields keep standard text-editing shortcuts (copy,
cut, paste, select-all, undo, redo) including under non-Latin keyboard layouts.

When [10xer mode](10xer-mode.md) is on (`crates/strata-app/src/tenxer/`), 10xer key stages run
ahead of the default pipeline, hiding window/pane chrome and keeping keyboard focus inside the
browser panes. The file chooser applies a restricted chooser policy so window-wide actions
(global search, clipboard, undo, Open With, custom actions) stay unavailable while choosing files.
`crates/strata-app/src/shortcuts/` supplies the F1 shortcut reference and keyboard settings
presentation.

## Capability boundaries

### File source

Enumerates locations, retrieves metadata, watches changes, identifies folders across renames, and reports supported actions. Every load reports the folder's change stamp (`DirectoryEvent::Stamp`) before its first batch and a digest of the listed names before it finishes; `probe_stamp` and `check_stamps` read a stamp again on request, `check_stamps` as background work grouped per mount, and `responsiveness` says, without I/O, whether a location's mount still answers. Begin with local files. Avoid designing a universal remote filesystem API before a second backend exists.

### Operation service

Owns mutations, progress, cancellation, conflicts, and partial outcomes. UI code submits commands and observes operation state.

### Search provider

Streams scoped results and supports cancellation. Current-list filtering can remain in the application model; recursive filename and content search are providers.

### Preview registry

Chooses providers by content type and declared priority. Every provider receives byte/time/dimension budgets and returns either a preview, an unsupported result, or a contained failure.

### Theme source

Produces semantic tokens. Omarchy, generic system appearance, and user files are sources feeding one validated theme model.

### Settings store

Loads and saves a versioned schema through XDG-standard locations. Unknown keys are tolerated, defaults are centralized, and migrations are explicit.

Application preferences use immediate-and-live bindings rather than separate startup and Settings-page initializers. See [Preference lifecycle](preferences.md) for consumer scopes, binding ownership, and required regression coverage.

## Customization model

Start with stable data-driven customization:

- Semantic color tokens
- Typography tokens for interface and monospace preview text
- Density, spacing, radius, and animation tokens
- Keybinding configuration
- Search exclusions
- Preview enablement and limits

Internally, search, preview, and theme implementations should be registries so built-in providers remain modular. This does **not** require exposing an unsafe public plugin ABI in the first release.

When third-party extensions are justified, prefer a versioned message protocol with explicit capabilities and permissions. This permits extensions written in multiple languages and allows isolation from the main process.

### Custom actions

User-authored context-menu actions follow that direction without an ABI:
`model::action` parses and validates the portable `action.toml` contract,
`services::actions` owns the cached catalog and matching, `adapters::local_actions`
reads, validates, writes, imports, and exports action directories, and
`adapters::local_jobs` starts one invocation as a child process. `services::jobs`
owns the queue, per-item iteration, progress, cancellation bookkeeping, bounded
logs, and history, with the Jobs UI in `strata-app` observing it.

Definitions are data, not code, until an action is invoked: opening a menu only
matches declarative rules. Invocations receive their inputs through files and the
environment rather than through a shell or an interpolated command line, and the
registry, job service, and runner keep filesystem, process, and widget
responsibilities apart. Actions are trusted local programs; this boundary is
organizational, not a security sandbox. See [Custom actions](custom-actions.md).

## Workspace organization

```text
src/
└── main.rs               # binary entrypoint (`strata`)
crates/
├── strata-core/          # models, localization, storage, logging, UI-thread guard
├── strata-preview/       # preview sandbox launcher, helpers, document parsers
├── strata-fs/            # file source, operations, trash, volumes, search, browser state
├── strata-desktop/       # portal, D-Bus, updater, settings, tabs session, key catalog
├── strata-qa/            # toolkit-free `--qa` protocol and session engine
├── strata-platform/      # Rune Linux shims (Wayland/X11, D-Bus, GLib loop, PCM audio)
├── strata-kit/           # theme mapping, icons, fonts, transparency, overrides.toml
├── strata-components/    # pure accessor-driven Rune UI components
├── strata-gallery/       # component showcase scenes, goldens, integration demos
├── strata-app/           # Strata shell, slots, parts, hosts, QA backend
└── strata-test-support/  # shared test helpers
```

See [Workspace crates](workspace.md) for layering and `deny.toml` enforcement.

## Decision records

Significant decisions should be captured as short ADRs under `docs/adr/`, including context, decision, consequences, and status. Appropriate subjects include extension isolation, configuration format, preview sandboxing, and indexed search.
