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
platform choosers, such as GTK's color dialog, are also kept native.

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
applying it, with the saved position already taken out of `ListNavigation`. Footer
and selection observers therefore see restored state without dropped events.

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
bindings refresh once per GTK frame, outside layout, for visible/overscan items.
These presentation refreshes never resubmit file work or reset a name label or active
rename editor. Identical active thumbnail/metadata requests are reused. Missing
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
keyboard actions use the selected pane rather than stale hover state. A completed plain
click on background clears selections; returning to an inactive column then
selects its first visible entry. Presses and marquee releases do not clear selections. Click
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
| `ui/search_session.rs` | Root, hidden-file and recursive-scope inputs; worker/receiver, query intent, generation, bounded event draining, cancellation and status/coverage delivery. It knows neither widgets nor acceptance/navigation policy. |
| `ui/inline_search.rs` | Single-pane search composition and transactional consumer selection publication. `inline_search/collection.rs` owns stable result objects and displayed-order lookup; `presentation.rs` builds and binds typed row/card parts. |
| `ui/browser/columns/search.rs` | Native Columns result reconciliation. Its result vector is authoritative; the string model is only a label projection, updated in lockstep under selection suppression. Columns keeps its multi-depth navigation and native collection. |
| `ui/collection_edit.rs` | A rename lease on an entry identity and typed editor/display handles: validation, submission signals, cancellation, focus, reveal tick and cleanup. It has no `ViewState` or `browser_modes` dependency. |
| Browser/chooser adapters | Activation, single-click preview/navigation, selection cardinality, filename/preview updates, context commands and filesystem-operation coordination. `browser/inline_edit.rs` retains pending-operation identities, optimistic labels, refresh/error handling and operation safety. |

Normal Columns resolves displayed positions through `ViewMap`; normal List/Icons use
`SourceIndexMap` and the current view model (including List grouping). Filtered Columns
resolves **all** result actions through its authoritative result vector. Single-pane results
resolve activation, selection, drag, context and edit targets through the sorted GTK model;
the path/rank map only supplies ordering to its sorter. Basenames are never identity.

Single-pane publication captures selected/focused paths before reconciliation and restores
selection by identity. Restoring focus never reselects an explicitly deselected entry.
If the last selected result disappears, it retains the nearest previous
selection slot; Columns intentionally leaves the selection empty when its selected result
vanishes. Columns uses per-column hidden-file preferences and does not display a separate
coverage/status row; single-pane search uses browser preferences and shows partial coverage or
empty/indexing status. Nonlocal locations keep their native, nonrecursive filter fallback.
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
cross-mode launch smoke test. See [GUI validation](e2e-testing.md) and
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

Filesystem work for Trash lives in `adapters/trash.rs`. Measurement shares one entry/time budget
across root and descendant batches; depth truncation and unreadable descendants remain branch-local.
Deleting Trash streams its own batches, independently of any incomplete measurement. Native path
and GIO URI conversion lives in `adapters/gio_location.rs`, shared by files, operations, preview and
browser presentation. It preserves native bytes and sanitizes credentials on inbound GIO locations.

Local archive operations live under `adapters/local_operations/archive/`:

| Responsibility | Owner |
| --- | --- |
| Operation entry points, worker lifecycle and progress events | `archive.rs` in the parent directory |
| Staged publication, source traversal and compression writers | `compression.rs` |
| Per-operation extraction state, copying, cleanup, size preflight and outcomes | `extraction.rs` |
| Confined destination writes, path validation and conflict naming | `destination.rs` |
| ZIP, TAR/gzip and 7z member enumeration, passwords and decoder errors | `decoders.rs` |

Every decoder feeds one `ExtractionSession` per operation. The session has no codec or widget
API dependencies; decoders lend it member streams and provide already-known pending names on
cancellation. Member identity tracking stays inside each decoder rather than assuming unique names
or matching header/callback order. The session validates pending names and applies established
root renames without filesystem probes or name reservations; final leaf conflicts remain unknown
until a member is attempted. Sequential formats do not scan unread content to complete that list.
Before writing, the session checks claimed uncompressed size against destination free space from
`fstatvfs` on the pinned root, and it refuses a member whose extracted size does not match the
size declared by the archive header. ZIP and 7z advertise a total up front, so an oversized
archive is refused before any member is written; TAR streams check each member as it arrives, so
extraction stops at the free-space boundary and members already written stay in place. The
guarantee is that extraction never exceeds the free space observed when the session opened;
it does not model per-file overhead such as block rounding or inodes. Filesystems that report no
capacity (`f_blocks == 0`, as FUSE mounts without `statfs` do) skip the free-space checks and
keep only the declared-size match. The `zip` crate does not bound inflated output by the header
size itself, so that match is the control that stops a ZIP member lying about its size.

The private member boundary currently retains legacy lossy TAR-name conversion and regular-file
output for non-directory entries, including links. It is not a complete archive-entry model;
native names and entry-type semantics belong in the decoder compatibility evaluation. Format
libraries remain behind the adapter boundary. See [archive creation](archives.md) for
container-specific encoding, classification and cancellation behavior. Archive unit tests sit in each module's
adjacent `tests.rs`; provider-level tests remain in `archive/tests.rs`, with shared test-only builders
in `fixtures.rs`.

Feature unit tests sit beside their implementations. Cross-feature browser tests remain in
`ui/browser/tests/`; GTK tests that need independent initialization can use
`test_support::gtk_test`, which launches a subprocess with disposable XDG directories. Set
`STRATA_REQUIRE_GTK_TESTS=1` when exercising those tests on a display to make unavailable GTK a
failure rather than a skip.

This separation is not a redesign of operation policy or a claim that all UI filesystem calls have
been eliminated. Collision probes, destination creation and permission editing still deserve
application/adapter boundaries in focused follow-ups. Likewise, alternate renderers, staged
publication/metadata orchestration, native transfer security and the settings workspace should be
refactored independently of browser composition. Investigation and scope decisions are recorded in
[issue #397](https://github.com/lgse/strata/issues/397).

### Browser directory-event routing

`app/browser/loading.rs` dispatches provider events through an owned open-load target:
native batches stage for sorting/publication, while remote batches keep the first-batch
and coalesced-tail paths. Completion carries truncation and both filesystem capabilities
together. Requests outside the open-load gate retain their existing peek/failure handling;
stale work is still checked against the owning directory or peek request.

`loading/metadata.rs` separates full-sort fills, applied by location, from viewport fills,
validated against directory identity and row-position/location tokens. Metadata chunks
never complete a sort; `MetadataFinished` retains that responsibility. Both modules release
state and routing borrows before synchronous observer dispatch, allowing observers to
navigate safely. Sorting, metadata scheduling, and cancellation remain in the browser
controller; event routing does not change those policies.

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
existing deferred rebuild timing. Bookmark storage, Trash, and media-release policies
remain in `window.rs` rather than changing alongside assembly. Bookmark mutations
read the shared GTK file before applying changes and adopt them only after a
successful save. This preserves sequential external edits, not simultaneous writes;
other windows are refreshed on their next bookmark action, not by a live monitor.

### Window keyboard routing

`ui/window/keyboard.rs` owns the window's capture-phase keyboard dispatcher. Its ordered
stages preserve shortcut precedence: modal and editing ownership, window/file commands,
focus traversal, transient dismissal, then item/directory navigation. The private
`commands.rs`, `focus.rs`, and `items.rs` modules implement those responsibilities without
introducing another browser controller. A stage returning `None` continues through Strata's
handlers; `Some(Propagation::Proceed)` ends dispatch and leaves the event to GTK. In
particular, editable controls and native single-pane selection must not fall through to
browser commands. The file chooser retains its separate, restricted keyboard policy
when 10xer mode is off.

When [10xer mode](10xer-mode.md) is on, that dispatcher skips the default
`h`/`j`/`k`/`l` arrow remap and command pipeline and runs `keyboard/tenxer.rs`
instead. The chooser installs the same dispatcher alongside its default map and
delegates to it while the preference is on: **Enter** / **o** still confirm a
file, **Esc** cancels after dismissing prompts or preview, and global search /
Open With stay unavailable. Window-local browse / visual / chord / prompt state
lives in `ui/tenxer_mode.rs`, not on `Browser`. Per-window preference bindings
update the shared `gtk::Application` accelerators idempotently; window destruction
does not restore them while other windows still use 10xer mode.
Chrome-visibility bindings hide pane Close/filter/refresh/sort in both
interactive browsers and the chooser. The preference is
`PreferenceManager::tenxer_mode` in `ui/preferences.rs`, not a theme setting.

Initial binding applies the saved mode without transition teardown. Real transitions
clear hidden queries and forced recursion, prompts, chords, and preview key ownership.
Footer preference/observer callbacks and prompt controllers use weak owners so a
closed window can release its view and bindings. `ui/shortcut_reference.rs` supplies
shared Settings/F1 presentation; default F1 navigation remains view-specific.

## Capability boundaries

### File source

Enumerates locations, retrieves metadata, watches changes, and reports supported actions. Begin with local files. Avoid designing a universal remote filesystem API before a second backend exists.

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
