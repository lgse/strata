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
application controller. Miller columns, the icon grid, the list table, and the tree must not own
independent navigation state. Mode-specific widget construction and interaction policy live behind
the UI presentation boundary (`ui/browser_modes.rs`); shared operations stay in the application
layer. A future mode should therefore add a renderer rather than add mode checks to filesystem,
navigation, or operation code.

The tree is the one presentation that holds entries of its own, because an expanded branch shows a
directory the navigation state does not track. It still asks the application layer to read and order
them (`Browser::list_directory`, `Browser::sorted_entries`), so listing budgets, sort order and
hidden-file rules stay in one place, and it drops every branch when its column reloads rather than
keeping a second view of the filesystem alive. Only the tree's root level maps back to positions in
the active column, so selection, renames and item menus apply there.

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
