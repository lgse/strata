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
before alternate-mode dispatch.

Pane helpers share string-model splicing, but authoritative entry borrows end before GTK
notifications. Reload detaches selection/filter models without detaching the collection
views; completion or failure reconnects them. Teardown retains its stronger detachment.
Busy insertions/publication, replacement, and splices keep their distinct count/spinner
rules. Selection restoration captures existing pane focus before applying the selection
and preserves explicit focus requests and empty-selection behavior. Renderer construction,
rename, pointer policy, and preference ownership remain separate responsibilities.

Pointer intent is shared through `ui/pointer.rs` and `ui/marquee.rs`. In all three modes,
thumbnail slots, rendered row text/metadata, and Icons' caption region are item drag targets;
unused label allocation and the gutters beside thumbnails are marquee origins. Both paths use
GTK's configured drag threshold. Within collection viewports, marquees claim the sequence only
after that threshold, leaving simple clicks and modifier-clicks intact. A completed plain click
on background clears selections; returning to an inactive column then selects its first visible entry. Presses and marquee releases do not clear selections. Click activation and automatic preview wait for release
and reject cancelled gestures, drag motion, and recycled items. Marquees anchor and cache mapped
item geometry in scroll-content coordinates, independent of native GtkScrollable or GtkViewport
layout. Edge and wheel scrolling refresh selection after layout/paint, even without pointer motion;
unmapped rows cannot overwrite cached geometry with stale allocations. The visible band stays
clipped to the viewport, while earlier off-screen hits remain selected. Release completes pending
layout-dependent selection before disconnecting the frame handler.

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
libraries and compression behavior remain unchanged. Archive unit tests sit in each module's
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
remain in `window.rs` rather than changing alongside assembly.

### Window keyboard routing

`ui/window/keyboard.rs` owns the window's capture-phase keyboard dispatcher. Its ordered
stages preserve shortcut precedence: modal and editing ownership, window/file commands,
focus traversal, transient dismissal, then item/directory navigation. The private
`commands.rs`, `focus.rs`, and `items.rs` modules implement those responsibilities without
introducing another browser controller. A stage returning `None` continues through Strata's
handlers; `Some(Propagation::Proceed)` ends dispatch and leaves the event to GTK. In
particular, editable controls and native single-pane selection must not fall through to
browser commands. The file chooser retains its separate, restricted keyboard policy.

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
