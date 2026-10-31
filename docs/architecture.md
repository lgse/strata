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

Action dialogs use the modal shell and themed form controls in `ui/controls.rs`. The shell owns
header alignment, icon bezels, body and action spacing, focus treatment, and semantic accent or
danger states; dialog-specific code supplies only content and behavior. The search palette remains a
specialized command interface because its query field and results are one continuous keyboard
surface. Settings remains a specialized navigable workspace rather than an action dialog. Native
platform choosers, such as GTK's color dialog, are also kept native. `style.css` themes the color
chooser's buttons and editor controls with selectors on GTK's `window.dialog.colorchooser` and
`colorchooser` nodes that reuse the action-dialog and form-control rules, instead of wrapping the
chooser in the modal shell.

### Browser presentation modes

Browser presentations consume the same `BrowserEvent` stream and send intents back to the same
application controller. Columns, the single-pane Icons grid, and the single-pane List must not
own independent filesystem or navigation state. Mode-specific widget construction and interaction
policy live behind the UI presentation boundary (`ui/browser_modes.rs`); shared operations stay in
the application layer. A future mode should therefore add a renderer rather than add mode checks to
filesystem, navigation, or operation code.

`ui/browser_modes/events.rs` applies alternate-mode events on the same `ModeViews`.
Structural events rebuild the active presentation; row, loading, and selection handlers
keep their effects separate. Only panes belonging to the active mode and event depth
receive incremental updates. Shared browser effects in `ui/browser/events.rs` still run
before alternate-mode dispatch. Browser notifications remain synchronous: pane/query
updates must release `ModeViews` borrows before notifying observers. Load completion
returns a selection-restoration action; the caller releases `ModeViews` before
applying it, with the saved position already taken out of `PaneNavigation`. Footer
and selection observers therefore see restored state without dropped events.
`NavigationStarting` and `ColumnReloading` are the two pre-mutation capture events:
the outgoing directory, or a column about to be cleared for a reload, is still
readable when they arrive.

Pane helpers share string-model splicing, but authoritative entry borrows end before GTK
notifications. Reload detaches selection/filter models without detaching the collection
views; completion or failure reconnects them. Teardown retains its stronger detachment.
Busy insertions/publication, replacement, and splices keep their distinct count/spinner
rules. Selection restoration captures existing pane focus before applying the selection
and preserves explicit focus requests and empty-selection behavior. Renderer construction,
rename, pointer policy, and preference ownership remain separate responsibilities.

`ui/browser_modes/list_factory.rs` owns List item setup, binding, and thumbnail
cancellation on unbind. Its context retains the existing shared column widths, click
controls, source-position mapping, and weak browser ownership. A typed row view names
widget parts without changing their layout. An owned binding snapshot resolves the
source entry before updating GTK or requesting metadata.

Fast-scroll binds update labels/accessibility and admit viewport-prioritized thumbnails
and metadata without waiting for scrolling to stop. Each viewport coalesces a follow-up
admission pass after a frame, outside layout, so a final scroll cannot strand work
classified against old allocations. Icons reserves a font-sized details line even when
empty; metadata truncates within the caption width instead of resizing the grid.
Cut styling, tooltips and date
bindings refresh outside layout, for visible/overscan items, once scrolling has been still for
25 ms and at least two frames.
These presentation refreshes never resubmit file work or reset a name label or active
rename editor. Identical active thumbnail/metadata requests are reused. Completed
file operations invalidate affected folder counts across browsers through the shared
activity notification, distinguishing direct writes from recursive merges. Folder
bindings remain registered after a successful fill and resolve shifted positions
through the shared metadata index. Only live viewport targets are recounted;
parent listings and unrelated metadata work stay intact. Missing
bindings retain the existing fallback path. Pane assembly, headers, grouping/filtering, and Icons factories
remain in the composition module rather than changing alongside this lifecycle boundary.

Pointer intent is shared through `ui/pointer.rs` and `ui/marquee.rs`. In all three modes,
thumbnail slots, rendered row text/metadata, and Icons' caption region are item drag targets.
Columns view treats the whole visible `.file-row`, including unused label allocation and row
padding, as a drag origin. List uses content-only hits throughout its Name column, including
row padding: unused space starts a marquee, while filenames and icons still drag files.
List metadata columns retain row dragging. Row drag/drop controllers stay on the
application-owned row so they can coexist with GTK's native list-item selection gesture.
Icons keeps content-only drag behavior. Marquee ownership mirrors these policies: Columns
uses allocated bounds; List applies its Name-column policy in each mapped row's coordinates;
Icons keeps `hits_item_content` and treats card gutters as marquee origins. Pane background
and surrounding chrome remain marquee origins, and Alt-drag can force a marquee from an item
in any mode. Both paths use GTK's configured drag threshold. Within collection viewports,
marquees claim the sequence only after that threshold, leaving simple clicks and
modifier-clicks intact. Chrome-origin drags transfer focus to a visible item in the target
collection after crossing the threshold, giving active selection feedback without scrolling
to an old cursor. Sidebar origins also switch browser input ownership to the pointer, so
keyboard actions use the selected pane rather than stale hover state. A plain press on pane
background or on blank chrome beside the panes (sidebar, pane headers, the strip beside the
last column) clears the selection immediately; Ctrl/Shift presses keep it so the marquee can
union or range from it, and marquee releases never clear. Returning to an inactive column then
selects its first visible entry. A preview that is showing something (content or its
placeholder) is a drag-only origin: a plain click there leaves the selection alone and only a
drag past the threshold changes it, while the empty reserved preview slot clears like the strip
beside the columns. Presses on controls, including disabled ones and drag handles such as the
preview header, never start a marquee, and neither do presses on a viewport's own
scrollbars, including overlay scrollbars drawn over blank row space. Click
activation and automatic preview wait for release and reject cancelled gestures, drag motion,
and recycled items. Marquees anchor and cache mapped item geometry in scroll-content
coordinates, independent of native GtkScrollable or GtkViewport layout. Edge and wheel scrolling
refresh selection after layout/paint, even without pointer motion; unmapped rows cannot
overwrite cached geometry with stale allocations. The visible band stays clipped to the
viewport, while earlier off-screen hits remain selected. Release completes pending
layout-dependent selection before disconnecting the frame handler.

### Collection behavior and lifetime boundaries

The browser and open/save chooser use the same collection behavior, not a universal widget:

| Owner | Responsibility and intentional differences |
| --- | --- |
| `ui/collection_interaction.rs` | Display-position pointer transitions (Ctrl toggle, Shift range, plain selection/drag-group preservation) and press/release/cancel ownership. Modified sequences are claimed only on release so marquee can participate. |
| `ui/search_session.rs` | Root, hidden-file and recursive-scope inputs; worker/receiver, query intent, generation, bounded event draining, cancellation, status/coverage delivery, pruning the hits a worker proves gone, and starting a confined source's search only once a worker places its root inside the boundary. It knows neither widgets nor acceptance/navigation policy. |
| `ui/inline_search.rs` | Single-pane search composition and transactional consumer selection publication. `inline_search/collection.rs` owns stable result objects and displayed-order lookup; `presentation.rs` builds and binds typed row/card parts. |
| `ui/enter_target.rs` | Which result Enter in a filter field opens: the top one until the user picks another after the query last changed. Outlines it while the field has focus, keeps it scrolled into view until the user scrolls, and tells the result surfaces when a new query drops the old selection. A selection or focus change counts as the pick unless the surface reports it as its own write. Off in 10xer. |
| `ui/browser/columns/search.rs` | Native Columns result reconciliation. Its result vector is authoritative; the string model is only a label projection, updated in lockstep under selection suppression. Columns keeps its multi-depth navigation and native collection. |
| `ui/collection_edit.rs` | A rename lease on an entry identity and typed editor/display handles: validation, submission signals, cancellation, focus, reveal tick and cleanup. It has no `ViewState` or `browser_modes` dependency. |
| Browser/chooser adapters | Activation, single-click preview/navigation, selection cardinality, filename/preview updates, context commands and filesystem-operation coordination. `browser/inline_edit.rs` retains pending-operation identities, optimistic labels, refresh/error handling and operation safety. |

Normal Columns resolves displayed positions through `ViewMap`; normal List/Icons use
`SourceIndexMap` and the current view model (including List grouping). Filtered Columns
resolves **all** result actions through its authoritative result vector. Single-pane results
resolve activation, selection, drag, context and edit targets through the sorted GTK model;
the path/rank map only supplies ordering to its sorter. Basenames are never identity.

Single-pane publication captures selected/focused paths before reconciliation and restores
selection by identity while the query stays the same; a changed query text publishes an empty
selection in both single-pane results and Columns search results, so Enter in the field falls
back to the top result that `ui/enter_target.rs` outlines. A filter that narrows a listing
without a native path in place keeps that listing's own selection; the new query only stops it
counting as the pick. Restoring focus never reselects an explicitly deselected entry.
If the last selected result disappears, it retains the nearest previous
selection slot; Columns intentionally leaves the selection empty when its selected result
vanishes. Columns uses per-column hidden-file preferences and does not display a separate
coverage/status row; single-pane search uses browser preferences and shows partial coverage or
empty/indexing status. Nonlocal locations keep their native, nonrecursive filter fallback.
The search worker drops hits whose files are proven gone before it publishes them, and pruning
after a file operation asks the probe module, so neither surface checks a hit on the interface
thread; any answer but a proven absence keeps the hit.
The shared session preserves these adapter policies rather than normalizing them.

During single-pane reconciliation, consumer selection callbacks are suppressed until model,
selection, focus and bindings agree. Identity-preserving/no-op updates do not announce transient
index changes. Callback lists and payloads are snapshots, with no callback-list borrow held
across delivery. Reentrant result updates/query dismissal are queued (latest publication wins)
until the current delivery completes. GTK's internal model notifications remain synchronous;
consumers subscribe to the collection publication surface, not raw model signals.

Presentations register `EditWidgets` at construction and bind their entry identity before
handing out an `EditTarget`. Behavior never finds an editor by CSS class or sibling traversal.
An unchanged binding preserves a draft; removing/rebinding/unbinding a row intentionally
cancels the lease without submission. Commit, Escape, view teardown and owner destruction
retire handlers and reveal ticks. Focus-leave handlers are disconnected immediately, but their
controller is detached on a weak-widget idle callback: mutating GTK's controller list inside
a Tab focus walk is unsafe. Deferred focus checks the active editor and does not reselect text.
Presentation-specific reveal/cleanup hooks retain Columns' constrained editor and List's row
reveal without putting those geometries in the edit controller.

The view owns its filter binding and search session. Detaching a pane/column disconnects the
entry, cancels a pending debounce, removes the poll source and drops the search handle/receiver.
Query intent changes reject old results before debounce completes; root/scope/hidden-input
changes or an explicit restart create a new generation. Polling holds a weak session reference,
and delivers without borrowing session state so callbacks may cancel/restart safely. Filter
scope preference bindings remain widget-anchored. Factory unbind cancels thumbnail requests
and edits; teardown releases typed bound-widget records. Collection detachment retires deferred
scroll work; presentation ticks are widget-owned and stop with their widget. These rules also
apply when mode changes recreate a pane or a chooser closes.

Native keyboard navigation and marquee geometry remain presentation adapters over the same
selection models. They are deliberately not replaced with another navigation engine. Columns'
source/depth anchor, slow-click rename and release activation, List metadata/grouping, Icons'
content hit testing and chooser restrictions remain explicit policies. Tests retain distinct
pointer, keyboard, lifecycle and filesystem-effect routes rather than replacing them with a
cross-mode launch smoke test. See [GUI validation](testing.md) and
[live preferences](preferences.md).

### Browser implementation map

`ui/browser.rs` is the composition root and stable `BrowserView` command facade. Private feature
modules under `ui/browser/` implement methods on the same `ViewState`; splitting a feature into a
file does not give it a second controller or a separate selection model. Imports name the owning
module explicitly. Re-exports retain the shared entry points used by alternate modes and the chooser.

| Responsibility | Owner |
| --- | --- |
| Exhaustive event dispatch and shared effects | `events.rs` |
| Miller column assembly, publication helpers, sizing | `columns.rs` |
| Miller row factory, binding, pointer and drag interactions | `columns/rows.rs` |
| Collection filtering, position mapping, scrolling and selection | `collection.rs` |
| Entry encoding, matching, labels, icons and metadata presentation | `entry.rs` |
| Pane actions and loading presentation | `pane_header.rs`, `presentation.rs` |
| Hover peek lifecycle and placement | `peek.rs` |
| Inline rename and new-entry workflows | `inline_edit.rs` |
| Location editing, breadcrumbs and mount authentication | `location.rs` |
| Selection-aware menus and restricted chooser menus | `context_menu.rs`, `chooser_context.rs` |
| Clipboard/cut intent and drag data | `clipboard.rs` |
| Transfers, destination search and archive dialogs | `transfer.rs`, `destination.rs`, `archive.rs` |
| Progress and Trash confirmation/cancellation | `progress.rs`, `trash.rs` |
| Properties, permissions and item customization | `properties.rs`, `customization.rs` |
| Display paths and desktop launching | `paths.rs`, `desktop.rs` |

Generic modal hosting, animation and dismissal live in `ui/modal.rs`, not in a browser feature.
`ModalHost` discovers the window overlay and enables its optional blur; existing dismissal owns
unblurring and must leave it enabled while another visible modal remains. Dialog-specific cancel,
close, backdrop and submission policies remain with the dialog.

New File and New Folder allocate real entries through `app::Browser` and the operation provider.
`adapters/local_operations/create_entry.rs` shares atomic conflict retries for files and directories
and closes an empty file before reporting its creation. `EntryCreated` carries the allocated
location, which `inline_edit.rs` uses to select the real row and start a normal rename. A pending
request identity prevents late focus after cancellation or navigation. There are no temporary
creation rows; both new and existing items use the same validation and deferred rename dispatch
outside GTK's focus walk.

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
(`ui/browser/leftovers.rs`) reads the items through `OperationProvider::review_leftovers`, and
`adapters/local_operations/leftovers.rs` restores originals (`recover`, behind the same
`ActiveOperation` as every operation) and sweeps stale probes; see
[Leftovers after a crash or unplug](filesystem-compatibility.md#leftovers-after-a-crash-or-unplug).

Filesystem work for Trash lives in `adapters/trash.rs`. Measurement shares one entry/time budget
across root and descendant batches; depth truncation and unreadable descendants remain branch-local.
Deleting Trash streams its own batches, independently of any incomplete measurement. Native path
and GIO URI conversion lives in `adapters/gio_location.rs`, shared by files, operations, preview and
browser presentation. It preserves native bytes, keeps GLib's normalized percent-encoding (so names
that are not valid UTF-8 and encoded slashes survive), and sanitizes credentials on inbound GIO
locations.

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

Feature unit tests sit beside their implementations. Cross-feature browser tests remain in
`ui/browser/tests/`; GTK tests that need independent initialization can use
`test_support::gtk_test`, which launches a subprocess with disposable XDG directories. Set
`STRATA_REQUIRE_GTK_TESTS=1` when exercising those tests on a display to make unavailable GTK a
failure rather than a skip.

Code on the GTK main thread makes no file call, except those listed under [Off the interface
thread](filesystem-compatibility.md#off-the-interface-thread). `util/ui_thread.rs` marks that
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
visibility and power saver from `ui/presence.rs`, which follows each window, and the "Watch network
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
the removal of the folder itself. A column whose parent column is its parent folder leaves
that report to the parent column's monitor, which sees the departure as a change to one of
its entries; any other column, including the depth-0 folder, is checked in
`directory_changes.rs`. A folder that validates again (deleted and recreated) reloads in
place, which renews its monitor. A missing depth-0 folder is looked up in its parent by
the identity its load's change stamp recorded off the main thread (an `ItemIdentity`),
compared by the mount's [identity rule](filesystem-compatibility.md#identity): a birth time
keeps a new folder that reuses the inode from passing for a rename, and on mounts that
renumber folders without reporting their birth times (sshfs, exFAT) nothing proves a
folder, so it counts as departed. Found there
(`find_by_identity`), the folder was renamed in place: the search indexes are rebased and
the open columns relocate (`ColumnsRelocated`), as for an in-app rename. Not found, with a
parent that cannot be listed, or before its identity is known, it departed: the nearest
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

### Window composition

`ui/window.rs::present_target` owns the startup sequence: prepare theme/styles, compose
and bind the window, arm first-paint work and destruction cleanup, present, then schedule
initial navigation, portal integration, and the due update check. Reveal selection is
queued before navigation. Sidebar discovery remains deferred until after the first paint.

`ui/window/composition.rs` coordinates the window's components. Its private `layout`
module assembles the header, sidebar/browser/preview splits, and live shortcut footer;
`input` installs pointer history and edit-cancellation gestures. `search` shares one
toggle/dismissal path between the header button and window action, reading preferences
at dispatch. `settings` owns update notices and a single lazily created Settings layer
per window; both Settings entry points reuse it and the process-wide install guard.
Preferences take effect before Settings opens. Destruction disconnects the clipboard
subscription, browser observers, and sidebar monitors.

`ui/window/sidebar.rs` assembles the sidebar shell and connects its preferences,
browser events, and device monitors. Shared place-row bindings retain explicit direct
versus validated navigation; file drops still exclude virtual locations. Typed device
signals share a weak rebuild callback and retain their disconnect handles. Standard,
pinned, and device rows are separate rendering stages, with the chooser's local-only
filter preserved. Initial construction builds static places; device rows retain their
existing deferred rebuild timing. Trash and media-release policies remain in
`window.rs` rather than changing alongside assembly. Bookmark storage lives in
`adapters/bookmarks.rs`: every pin, unpin, reorder and deletion cleanup is one job on
the background settings writer (`storage::writer`) that reads the shared GTK file,
applies the change and saves it, checking just before its rename that no other program
changed the file since the read and starting over, at most twice, if one did. Every
open sidebar then adopts the pins the file holds. A symlinked bookmarks file is written
through to its target under the same ownership rules as `settings.toml`. Only the first
sidebar reads the file before the first window shows; later windows start from the
last adopted pins and re-read the file in the background. Outside edits are picked up
by the next update or new window, not by a live monitor.

`ui/window/pinned_drop.rs` owns folder pinning through dedicated gaps between pinned
rows, including an empty section. Existing pins reorder through the same gaps and
insertion indicator, using a provider-matched location snapshot rather than treating
their string reorder payload as a new folder. Reordering resolves source and target
locations against the latest bookmarks, preserves labels, and never resurrects a
concurrently unpinned source. Built-in places and pinned rows show before/after
insertion lines during compatible reorders within their own section. Pin gaps
supply the inter-row spacing without adding a second spacer around each target.
Row bodies retain their existing transfer and spring-navigation behavior; remote
pins have insertion gaps even when their rows cannot accept transfers. Internal folder
drags carry a provider-matched snapshot of entry locations, display names and directory
eligibility, without filesystem probes. External
folders take their label from the same metadata query that confirms their type, so drag
pins are labelled like menu pins. Trash locations and web links are rejected before any
probe; GVfs would otherwise fetch a hovered link. Pinning prefers the
Copy protocol action, but accepts Move-only offers from provider-matched internal
sources, whose drag-end handlers never delete data. Foreign sources must offer
Copy: completing an external Move could ask the other application to delete the
originals. Neither internal action dispatches a filesystem transfer. External file
lists and directory types are read asynchronously once per drag across that sidebar's
gaps. Validation issues one metadata query at a time and has a two-second deadline,
including payload loading. Pending pin drops wait for that result and never become
transfers; cancellation, failure, or timeout rejects the whole selection. Cancelling
a GIO query does not guarantee interruption of an already-blocked kernel call.
Only confirmed directory selections show an insertion indicator. Directory symlinks
are accepted without canonicalizing or collapsing different location spellings.
Each gap remembers the pins on both sides; insertion resolves the following pin against
the latest bookmarks, falls back to the preceding one, and rejects the drop when both
were unpinned elsewhere. Insertion preserves selection order, skips existing pins
without reordering them, and finishes the drop protocol only after the bookmark update
completes; a drop that changes nothing finishes as rejected.

### Window keyboard routing

`ui/window/keyboard.rs` owns the window's capture-phase keyboard dispatcher. Like every key
handler, it reads presses through `ui::keys` and matches catalog bindings (see
[keyboard shortcuts](keyboard-shortcuts.md)). Its ordered stages preserve shortcut precedence: modal and editing ownership, window/file commands,
focus traversal, transient dismissal, then item/directory navigation. The private
`commands.rs`, `focus.rs`, and `items.rs` modules implement those responsibilities without
introducing another browser controller. A stage returning `None` continues through Strata's
handlers; `Some(Propagation::Proceed)` ends dispatch and leaves the event to GTK. In
particular, editable controls and native single-pane selection must not fall through to
browser commands. Search, jump to folder, open terminal, refresh, and arrow scope
(`WINDOW_ACTIONS`) are dispatcher bindings, not GTK accelerators: when dispatch yields,
`Dispatcher::after_yield` activates the matching enabled `win.*` action and consumes the
key even when the action's guard then declines. Dispatch also yields while a modal or an
inline edit owns input, so `composition::add_guarded_window_action` installs those actions
with a gate: they do nothing while a modal layer other than the action's own (the search
palette) is on top and not already closing, or while an inline rename field or new entry
is open. The dispatcher yields on the same `BrowserView::inline_edit_is_open` predicate.
A key still unclaimed reaches `edit_focused_text`, which runs a focused text field's own
copy, cut, paste, select-all, undo, or redo action for a key that only the layout
fallback recognizes (see [keyboard shortcuts](keyboard-shortcuts.md#text-fields)). The
file chooser retains its separate, restricted keyboard policy when 10xer mode is off; it
has no window actions but applies `edit_focused_text` too.

When [10xer mode](10xer-mode.md) is on, that dispatcher runs its 10xer stages
(`tenxer_keys` and the `keyboard/` modules) ahead of the default pipeline. The
chooser builds the same dispatcher with a `ChooserPolicy` and, while the
preference is on, asks only its 10xer stages first (`ChooserKeys`); keys they
leave go to the chooser's own restricted map. Window-wide stages such as global
search, clipboard, and undo never see chooser keys, and `keyboard/chooser.rs`
refuses commands the request does not allow: **Enter** / **o** confirm a file,
**Esc** cancels after dismissing prompts or preview, and Open With, custom
actions, and clipboard verbs stay unavailable. Window-local browse / visual / chord / prompt state
lives in `ui/tenxer_mode.rs`, not on `Browser`. `after_yield` reads the
preference when a key arrives, so the jump to folder, open terminal, and arrow
scope window actions stay unbound in every window without application-wide state.
Chrome-visibility bindings hide pane Close/filter/refresh/sort in both
interactive browsers and the chooser. Tab and arrow focus stay inside the
Columns, List, and Icons panes; sidebar, header, and footer controls stay
pointer-operated. The preference is
`PreferenceManager::tenxer_mode` in `ui/preferences.rs`, not a theme setting.

Initial binding applies the saved mode without transition teardown. Real transitions
clear hidden queries and forced recursion, prompts, chords, and preview key ownership.
Footer preference/observer callbacks and prompt controllers use weak owners.
`gtk_window_destroy()` unrealizes a window but frees it only with its last
reference, so window and chooser cleanup, their key controllers, and every
preference binding anchored inside them are released on unrealize rather than on
the destroy signal. `ui/shortcut_reference.rs` supplies
shared Settings/F1 presentation; default F1 navigation remains view-specific.

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
logs, and history, with the presentation in `ui/jobs.rs` observing it.

Definitions are data, not code, until an action is invoked: opening a menu only
matches declarative rules. Invocations receive their inputs through files and the
environment rather than through a shell or an interpolated command line, and the
registry, job service, and runner keep filesystem, process, and widget
responsibilities apart. Actions are trusted local programs; this boundary is
organizational, not a security sandbox. See [Custom actions](custom-actions.md).

## Suggested source organization

```text
src/
├── app/          # orchestration, commands, state transitions
├── model/        # product models with no widget dependencies
├── services/     # capability contracts and shared request/result types
├── adapters/     # local files, search tools, themes, settings
├── ui/           # windows, components, factories, animation
└── main.rs       # startup and dependency composition
```

This is a direction, not a requirement to create empty modules. Move code only when the associated responsibility exists.

## Decision records

Significant decisions should be captured as short ADRs under `docs/adr/`, including context, decision, consequences, and status. Appropriate subjects include extension isolation, configuration format, preview sandboxing, and indexed search.
