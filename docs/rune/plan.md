# strata-components: plan

A component library that fills the gaps between Rune and what Strata's interface needs, so
Strata can move off GTK 4.

## 0. Baselines

| Input | Version |
| --- | --- |
| Strata | `lgse/strata` at `5738aad808ea4cc4671516bc569d85f862664809`, the feature map's watermark (151 nodes, release v1.1.1) |
| Rune | `anomalyco/rune` `dev` at `2a44156b21d63b0342a366b26d9222a6edd4622c` (2026-11-10), crates `rune`, `rune-kit`, `rune-motion`, `rune-fx` |
| Feature map | `wmfeht/strata-feature-map`. Node ids below (`browser/view-modes/columns`) are its ids. |

Rune is pre-release: pin a `dev` rev and move it on purpose. Rune takes no pull requests;
its maintainers ask for issues. Anything this plan needs from Rune core is either an issue
filed upstream or a shim in this repository that is deleted once Rune ships the feature.

The published crate keeps the name `strata-platform` (the owner's ruling on §10 question 1;
the options are in `docs/decisions/S5-upstream.md`, "Crate name"). The name is taken on
crates.io by an unrelated crate, so releases are git tags only (§8). No release has been
tagged yet.

The minimum libwayland is 1.23 (the owner's ruling on S3's decision 1, 2026-11-10): P0, P1,
P2 and P4 reach winit's display through `wl_proxy_get_display`, which 1.23 added. Ubuntu
24.04 (libwayland 1.22) is not supported. On an older libwayland the shims still degrade
rather than fail (`docs/decisions/S3-foreign-parent.md`, decision 1), but that path is
unsupported.

## 1. Findings in brief

Strata uses GTK 4 (`gtk4` 0.11, `sourceview5`, `gstreamer`, `gdk4-wayland`, `gdk4-x11`).
Its UI is mostly in `src/ui/`, which uses about 170 distinct `gtk::` types. The heaviest are
`Box`, `Label`, `Button`, `ScrolledWindow`, `Entry`, `Overlay`, `Popover`, `ListView`,
`GridView`, `ColumnView`, `Paned`, `Revealer`, `TextView`, `MediaStream`, `DrawingArea`,
`DragSource` and `DropTarget`.

Rune and rune-kit already cover a lot of this:

- **Core (`rune`)**: Taffy layout, springs and presence, virtual rows, sticky headers,
  horizontal scroll, text fields with IME, selectable text, AccessKit (AT-SPI on Linux),
  focus groups and modals, layout-aware chords (`Chord`, `tx.physical_key`, `tx.key_base`),
  `Pixels` for streamed RGBA, backdrop blur, a portal file dialog, multiple windows
  (`open_window`), and headless PNG and tree output with scripted input.
- **rune-kit**: Button, IconButton, Switch, Checkbox, Radio, Select, Combobox, PopupList,
  Menu and `context_menu` with submenus, Popover, HoverCard, Tooltip, Dialog, Toast and
  ToastRegion, Progress, Spinner and Loader, SegmentedControl, ShortcutRecorder, Keybind,
  ScrollBar, Table, Tree, Markdown, the syntax highlighter, and the `shell` crate (Sidebar,
  SidebarRow with reorder, SplitView, PaneState, TabStrip with reorder and tear-out,
  WindowTitlebar, Toolbar, Page).

The gaps fall into three tiers:

1. **Linux platform gaps (Rune core).** Without these the port cannot ship. Rune's own README
   gives the platform limits:
   - `drag_out` is macOS-only.
   - `on_drop_files` takes file paths only, and not on Wayland.
   - `popup_menu` is unsupported on Linux.
   - Custom titlebars and window controls exist only on macOS and Windows.
   - `Player` and `NativeSurface` are macOS-only.
   - The clipboard holds text, HTML, RTF and pasted images. It has no custom MIME types and
     cannot write images.
   - Nothing parents a window to a foreign window.
   - The double-click interval is a fixed 0.5 s on Linux.
2. **File-manager components.** Strata needs controls that no general kit carries: the shared
   entry collection (selection, marquee, pointer intent, rename) and its Columns, Icons and
   List layouts, plus breadcrumbs, the preview surfaces, and the progress dock.
3. **Kit extensions.** These are controls rune-kit lacks, or lacks a mode of: Slider, number
   field, colour picker, icon rail sidebar, column reorder and grouping in tables, and drop
   targets on tabs.

## 2. Coverage by Strata area

`Adopt`: use rune-kit as is. `Extend`: wrap or extend a rune-kit part. `Build`: new
component here. `Platform`: needs a tier-1 shim or a Rune issue.

| Area / node | Rune / rune-kit today | Verdict | Component (§4) |
| --- | --- | --- | --- |
| `app/window` (header, window buttons, composition) | `WindowTitlebar`, `Toolbar`; Rune keeps the native frame on Linux, and P3 makes the window borderless | Platform + Extend | P3, X1's header |
| `app/window/key-dispatch` (staged key routing, Ctrl+wheel) | Key routing table, `on_key`, `on_wheel` | Adopt; port stages as app code | — |
| `app/keyboard-shortcuts`, `.../remapping` | `Chord`, `Keybind`, `ShortcutRecorder` | Adopt; Strata keeps its own layout rules on `physical_key` / `key_base` | — |
| `app/shortcut-reference` (F1 panel, footer) | Dialog, TextInput, Keybind | Extend | C19 |
| `app/dialogs` (blurred backdrop, focus, Enter/Escape) | `Dialog`, `.modal(..)`, `backdrop_blur` | Extend | C15 |
| `app/accessibility` | AccessKit with roles, names, live regions, `position_in_set` | Adopt; port names and roles per component | — |
| `browser/view-modes/columns` (Miller columns) | Horizontal scroll + virtual rows | Build | C2 |
| `browser/view-modes/icons` (thumbnail grid) | Virtual rows only (vertical) | Build | C3 |
| `browser/view-modes/list`, `.../column-layout` | `Table`: sort, resize, select; no reorder, groups, row DnD, marquee | Build on C1, reuse Table's column sizing | C4 |
| `browser/selection/*`, `browser/scrolling`, `.../position-restore` | Focus groups, `scroll_anchor`, `scroll_into_view` | Build | C1, C24 |
| `operations/rename`, `.../typed-paths` | TextInput | Build | C5 |
| `browser/navigation/location-bar`, `.../completion` | TextInput, PopupList, `scroll_x` | Build | C6 |
| `browser/tabs` | `TabStrip` (reorder, tear-out, middle-click close) | Extend: drop targets, New-tab drops | C7 |
| `browser/sidebar`, `.../pins`, `.../drag-to-pin`, `.../rail` | `Sidebar`, `SidebarSection`, `SidebarRow::reorder` | Extend: icon rail, insertion gaps, external drops, device rows | C8 |
| `browser/context-menu` | `Menu`, `context_menu` (in-window) | Adopt; Strata adds keyboard open and grouping | C21 |
| `browser/search`, `.../filter`, `.../enter-target`, `browser/navigation/folder-jump` | Combobox, PopupList, Dialog | Extend | C17, C18 |
| `browser/thumbnails/*` | `Pixels` | Build (binding layer) | C25 |
| `browser/properties/*` | Dialog, Field | Adopt + compose | C15 |
| `browser/folder-customization` | — (GTK `ColorDialogButton`, `EmojiChooser`) | Build | C20 |
| `operations/drag-and-drop/*` | X11 file-path drops only; no Linux drag source | Platform | P1, C1 |
| `operations/clipboard/*` | Text, HTML, RTF, pasted images | Platform | P2 |
| `operations/progress`, `.../dock`, `.../jobs` | Toast, ToastRegion, Progress | Extend | C16 |
| `operations/trash/animation`, `operations/delete/dissolve` | Shared-element motion, `rune-fx` | Build (small) | C23 |
| `operations/clipboard/conflicts`, `devices/volumes/*` dialogs | Dialog, RadioGroup, Field | Adopt + compose; mount prompts need C26 | C15, C26 |
| `preview/preview-panel`, `.../layout` | `SplitView` inspector pane | Extend | C9 |
| Source previews, `preview/preview-panel/find` (sourceview5) | `rune_kit::syntax` (about 13 languages and a 32 KiB cut when planned; 80 lexers and a 128 KiB cut at `2a44156`) | Build | C10 |
| `preview/preview-panel/documents/*` | `Markdown` (CommonMark + GFM only) | Build | C11 |
| `preview/preview-panel/pdf` | `Pixels` | Build | C12 |
| `preview/preview-panel/media/*` | `Player` fails on Linux | Platform + Build | P7, C13 |
| `preview/preview-panel/model-views`, `.../routes`, `.../gcode` | `Pixels`, shapes, `on_wheel` | Build (thin) | C14 |
| `preview/quick-preview/folder-peek` | `HoverCard`, `hover_delays` | Adopt | C22 |
| `integration/portal-file-chooser/*` | Portal *client* only; no foreign parent | Platform | P4 |
| `integration/open-with` | — (GTK `IconTheme` for app icons) | Platform + Build | P6, C15 |
| `integration/10xer-mode/*` (footer prompts, chord panel) | TextInput, Keybind | Build | C18 |
| `settings/preferences` (Settings window, search, pages) | `Page`, Sidebar, Switch, Select, SegmentedControl | Extend | C20 |
| `settings/themes/*`, `browser/view-modes/density`, `.../text-size` | `Theme::from_json`, `Kit`, `set_font_families` | Build (theme mapping) | K1–K6 |
| `settings/themes/portal-text-scaling` | Portal `color-scheme` only | Platform | P5 |

Nodes with no UI (archives, filesystem compatibility, sandbox, undo engine, providers,
packaging, updates engine) carry over unchanged. Their dialogs use C15.

## 3. Repository layout

```
strata-components/
  crates/
    strata-platform/   tier 1: Linux shims Rune lacks (Wayland + X11). Published. Each module names its Rune issue.
      tests/support/strata-test-peer/  the compositor tests' peer process (test tooling, unpublished)
    strata-kit/        theme mapping onto rune_kit::Kit, icon catalogs, density, text size, the bundled font
    strata-components/ tier 2 and 3 widgets, on rune + rune-kit + strata-kit
    strata-gallery/    one headless scene per component state, like rune-kit's kit-gallery; the X2/X3 demos
  tests/
    golden/            PNG and accessibility-tree goldens, and the tree each script ends on (RUNE_BLESS=1 rewrites)
    scripts/           Rune headless input scripts, one per behavior group
    fixtures/          shared test inputs (S4's code fixtures)
  docs/
    decisions/         the spike records (S1–S5) and the owner's rulings on them
    work/              one work package per component part, and their status
```

Rules:

- Components take state by accessor (`fn(&S) -> &X`, `fn(&mut S) -> &mut X`), as rune-kit's
  Table and Tree do. They own no I/O. Listing, thumbnails and search results arrive as data.
- Components read only `Kit` tokens and `strata-kit` extensions, never a theme by name.
- Each component ships with a gallery scene, an a11y golden, and a headless script for every
  feature-map Behavior bullet it implements. `featuremap.py qa --node <id>` lists the bullets.
- `strata-platform` is a published crate for any Rune app on Linux, not just Strata:
  - It keeps the name `strata-platform` (the owner's ruling; §0).
  - It depends on `rune` only, never on rune-kit, strata-kit or Strata, and has no Strata names
    in its API.
  - MIT, like Rune. It follows semver.
  - Each Rune rev bump is a minor release, since Rune's own types appear in its API.
  - Each module sits behind a cargo feature (`dnd`, `clipboard`, `csd`, `foreign-parent`,
    `settings`, `icon-theme`, `media-audio`, `single-instance`, `glib-bridge`), so apps take
    only what they use.
  - When Rune ships a feature, the module is deprecated for one release, then removed in the
    next minor.
  - Each module's docs link its Rune issue and say what Rune lacks.
  - Its tests run without Strata. Examples are small Rune apps, as in Rune's `examples/`.

## 4. Components

### Tier 1: `strata-platform`

Each item gets a Rune issue with a headless repro. Shims bind protocols on winit's own Wayland
display, the same way Rune binds `zwlr_layer_shell_v1` and `xdg_activation_v1`
(`platform/layer_shell.rs`, `platform/activation.rs`).

**P1 Drag and drop (Wayland and X11).** This is the largest blocker.
- Drag source: `wl_data_device` / XDND, with several MIME types per drag (`text/uri-list`,
  `x-special/gnome-copied-files`, `application/x-kde-cutselection`, an in-process id), action
  negotiation (copy, move, link, ask), a drag icon from an element snapshot, and the outcome.
- Drop target: arbitrary MIME types, not just paths. That covers `text/plain`, `text/uri-list`,
  `text/x-moz-url`, `image/png` and HTML for `operations/drag-and-drop/content-drops`. It
  reports position on every motion event, and the chosen action follows Ctrl and Shift
  (`.../copy-or-move`).
- In-process fast path: a drag between Strata's own windows or panes carries typed entries,
  not serialized URIs.
- The API mirrors Rune's: `drag_source(|s| DragItem)`, `on_drop(|s, DropData, tx|)`, plus
  hover, move and reject hooks.

**P2 Clipboard with custom MIME types.** Copy, cut and paste of files need offers in
`text/uri-list` plus the GNOME and KDE cut formats (`operations/clipboard/file-manager-formats`).
They also need image writes, which Rune lacks ("Copies don't write images yet"). The shim uses
`wl_data_control` or `wl_data_device` and X11 selections. It keeps Rune's async read shape.

**P3 Client-side decorations on Linux.** Rune keeps the native frame on Linux and has no
resize-edge handling. Strata draws its own header with window buttons (`app/window`). As
shipped, by the owner's P3 ruling, `Policy::Auto` is borderless on Wayland and X11, and
`Policy::Native` keeps the native frame:
- **Wayland:** the header drags and double-click maximises through Rune. There is no edge
  resize, minimise or window menu until Rune ships them (drafts U6–U8): Rune doesn't reach
  winit's `drag_resize_window`, and the shim can't start a resize from the raw handle.
- **X11:** a second connection sends `_NET_WM_MOVERESIZE` for edge resize, `WM_CHANGE_STATE`
  to minimise, and `_GTK_SHOW_WINDOW_MENU` where the window manager lists it. mutter and KWin
  are a hand test, still pending.
- No in-app resize fallback. The header itself is built in X1 (`strata_components::header`).
These Wayland gaps are deliberate parity regressions against GTK Strata.

**P4 Foreign parenting for the portal chooser.** The FileChooser backend must be transient
for the requesting app's window. The handle is `wayland:<xdg_foreign handle>` or `x11:<xid>`,
so the shim needs `zxdg_importer_v2` and X11 `WM_TRANSIENT_FOR`. Hyprland placement
(`integration/portal-file-chooser/window-placement`) stays Strata-side. S3's three decisions,
ruled by the owner on 2026-11-10 (`docs/decisions/S3-foreign-parent.md`):
1. libwayland 1.23 is the minimum (§0). On an older one the window opens parentless
   (`Reason::NoDisplay`), an unsupported environment rather than a supported fallback.
2. An `x11:` parent on a Wayland chooser stays parentless, as GTK Strata does. An Xwayland
   helper process is the upgrade path if users miss it.
3. A parentless chooser is placed and focused by the compositor's policy, and Strata also calls
   Rune's tokenless `focus()` on it; the shim makes no `focus()` call. GNOME is verified by hand
   before M6.

**P5 Desktop settings.** Rune reads only `color-scheme`. Strata also needs the text scaling
factor (`settings/themes/portal-text-scaling`), the double-click time (click modes, slow
second-click rename), cursor blink, and the animations switch. All come from
`org.freedesktop.portal.Settings` (`org.gnome.desktop.interface`, `org.gnome.desktop.peripherals`),
with change signals.

**P6 Freedesktop icon theme lookup.** The Open With chooser and desktop entries show
application icons from the icon theme. GTK's `IconTheme` did this. The shim follows the
spec's directory and size matching, rasterizes SVG with `resvg` (already a Strata dependency),
and delivers the result as `Pixels`.

**P7 Media output.** Strata already decodes with FFmpeg in a sandbox and streams RGBA frames
and PCM (`preview/preview-panel/media`). Video frames go to `rune::Pixels`, which updates in
place without re-running the view. Audio needs a sink and a clock. P7 keeps GStreamer
`appsrc` (it does not need GTK) over PipeWire: it is Strata's path, and it tests in CI on
`fakesink` with a manual clock. Rune's `Player` is not needed.

**P8 Single instance and activation.** `gtk::Application` provided uniqueness, forwarding of
the command line, and D-Bus activation. FileManager1, `--portal`, `xdg-open` and the
`Exec=strata %U` launcher all rely on it (`integration/file-manager-interface`,
`browser/navigation/startup-arguments`). On Linux, Rune hands a second launch to a new
process. The shim owns a zbus well-known name. It forwards argv and activation tokens to the
primary instance, which opens windows with `rune::open_window` and focuses with
`activate(token)`.

**P9 GLib bridge.** GIO stays: volumes, mount operations, monitors, AppInfo and GVfs. GIO
async needs a running `glib::MainContext`. Run one on a dedicated thread and marshal results
into Rune through `AppHandle::update` and the setup `Waker`. This is plumbing, not UI, but
every device, network and monitor feature depends on it.

Not shimmed: native popup menus. Use rune-kit `Menu`, which draws in the window. The risk is
that menus clip to the window, where GTK used `xdg_popup` surfaces. That matters for a small
chooser window and for folder peek. File a Rune issue for popup surfaces, and size the
chooser's minimum to fit its menus meanwhile.

### Tier 2: Strata components

Some components shipped as several work packages (`docs/work/README.md`): C1 as C1a–C1f, C2
and C2b, C10 as C10a and C10b, C11 and C11b, C14 as C14a–C14c, C20 as C20a–C20d, and C24 and
C24b. Where a component below differs from this plan's first draft, the text says what shipped
and why; each such deviation is signed off by the owner, and its work package and module docs
give the detail.

**C1 EntryCollection (core).** One model and interaction layer under all three views. It
serves `browser/selection` and its children `marquee`, `pointer-intent`, `click-modes` and
`keyboard-navigation`, plus `browser/scrolling` and `browser/view-modes/position-restore`
(206 Behavior bullets in all).
- Selection: anchor, cursor kept apart from selection, Shift and Ctrl ranges, Escape, and
  empty-space clicks. In 10xer mode, a "filled" selection versus the cursor item
  (`integration/10xer-mode/file-verbs`). Selection is held by entry id and survives
  re-sorting and live updates.
- Pointer intent: content and gutter hit regions (`src/ui/pointer.rs`), and deciding between
  click, drag and marquee at the threshold. A press that crossed the threshold never
  activates. A modified press claims only on release.
- Click modes: single or double click per view and per kind, and slow-second-click rename,
  timed from the P5 double-click interval.
- Marquee: a band drawn over virtual rows, hit-tested against unmounted rows by index
  geometry. It auto-scrolls on a Rune glide, not stepped instant scrolls, which Rune warns about.
- Keyboard: arrows and `h`/`j`/`k`/`l` move between regions (the arrow keys never change
  directory), plus Page Up and Page Down, Home and End, and Ctrl+Up and Ctrl+Down. It reports
  the active row to a11y with `active_descendant`.
- Rows can host a drag source (P1), a drop-target highlight on folder rows, a rename editor
  (C5), and a thumbnail slot (C25).
- Position restore: cursor, selection and scroll per folder, held across refreshes with
  `scroll_anchor` and `keep_on_screen`.
- Performance target: 100,000 entries, scrolling at 60 Hz with no view commit per frame,
  using Rune's own Table numbers as the bar.

**C2 ColumnsView.** Miller columns (`browser/view-modes/columns`, 108 bullets). Columns are
EntryCollections in a horizontal `scroll_x` strip. Each column is its own virtual list. The
view selects the folder you came from, reserves the preview slot (C9), and auto-scrolls the
strip during drags (`operations/drag-and-drop/column-autoscroll`). Only one column holds the
keyboard cursor. C2b adds the size badge from the host and the column-width polish (spread on
release, autofit on a second press).

**C3 IconsGrid.** Thumbnail tiles (`browser/view-modes/icons`, `.../thumbnail-size`,
`.../folder-views`). Rune virtualizes only vertical rows, so the grid packs N tiles per
virtual row, where N comes from the viewport width and the tile size. It reflows when the
width or size changes and keeps the cursor tile in view. It steps sizes along the ladder
(Ctrl+Shift+wheel) and shows a two-line caption with its own hit region.

**C4 ListTable.** The List view (`browser/view-modes/list`, `.../column-layout`). It is built
on C1 and borrows rune-kit `TableColumn` sizing. It adds:
- sortable headings, with sorting done by the app (`browser/view-modes/sorting`);
- reorder by dragging headings;
- a visibility popover;
- sticky file-type group headers;
- one column layout shared by every window and the chooser.

Fork Table only if upstream accepts none of these as issues.

**C5 InlineRenameEditor.** `operations/rename` and `.../typed-paths`. A text field placed over
the row's label: Rune's core `text_input`, not rune-kit's TextInput, which takes no
right-click handler for C21's field menu (draft U-C5-1). It selects the stem, then the whole
name. It validates live, with a message, and commits or cancels on Enter, Escape and focus
loss. Tab moves to the next entry, and the row stays in the viewport.

**C6 LocationBar.** `browser/navigation/location-bar` and `.../completion`. Breadcrumbs that
toggle to an entry. Overflowing crumbs scroll and keep the last crumb visible. Each crumb has
a hierarchy menu, opens in a new tab on middle-click, and accepts drops (P1). Completion is its
own list on virtual rows, fed by the host, with Tab to complete and stale results dropped by
generation. It isn't rune-kit's PopupList, whose items can't draw matched ranges (draft
U-C6-4).

**C7 BrowserTabs.** Extends rune-kit `TabStrip`. It adds drop targets on tabs and on the New
tab button: files transfer into the tab's folder, a folder drag opens a tab, content drops are
saved. It adds a window title that follows the active tab, and reopen-closed (`browser/tabs`,
`.../session-restore`).

**C8 PlacesSidebar.** Its own list of rune-kit `SidebarRow`s with their reorder, in place of
rune-kit's `Sidebar` and `SidebarSection`: their list activates each row the arrow keys reach,
their headings take no focus, and a collapsed section drops the open pin. It adds:
- the full, icon rail and hidden steps (`browser/sidebar/rail`; the kit has no rail);
- insertion gaps in PINNED that take internal and external folder drops
  (`browser/sidebar/pins/drag-to-pin`);
- collapsible PINNED and DEVICES sections;
- device rows with eject and unlock actions, and drive dividers (`devices/volumes/drive-order`);
- the Trash target, which is also the fly destination for C23.

**C9 PreviewDrawer.** `preview/preview-panel` and `.../layout`. A resizable drawer: its own
split with a `DrawerState` per window, not SplitView's inspector (a whole window shell, with
fixed names and focus keys) or a PaneState (whose collapse and width rules contradict
Strata's). The header holds the Rendered/Source switch,
Find, Open, Wrap and Close. The drawer follows the narrow-window rules and the reserved Columns
slot. It hosts C10–C14 and serves both the browser and the chooser.

**C10 CodeView.** Replaces `sourceview5` for source and plain-text previews and
`preview/preview-panel/find`.
- Virtual lines, so large files open instantly, with line numbers and wrap.
- Highlighting with **syntect**, on a dedicated highlighter thread (C10a). When this was planned,
  rune-kit's scanners covered about 13 languages and coloured only the first 32 KiB of a block.
  At `2a44156` rune-kit's highlighter is Pierre's Highlights: 80 lexers, kind-based colours and a
  128 KiB cut. Whether C10a moves to it is open for the owner (it reopens S4).
  - **Grammars:** syntect's default set plus `two-face` (bat's extra syntaxes), to come close
    to sourceview's language coverage. The backend is `onig`, the owner's decision after S4
    (`docs/decisions/S4-syntect-backend.md`): `onig` meets the 8 ms viewport reading on every
    fixture, where `fancy-regex` needs a state cached for every line and never meets it on
    Markdown. The build gains Oniguruma, a C library.
  - **The thread:** `onig`'s `ParseState` is neither `Send` nor `Sync`, so one thread owns
    every parse state, with no `unsafe`, and the UI gets only colour data back over channels.
  - **Detection:** file name and extension first (`find_syntax_for_file`), then the first line
    for shebangs and modelines (`find_syntax_by_first_line`). Fall back to plain text.
  - **Virtual lines:** keep a parse and highlight state checkpoint every 64 lines (S4: 64
    meets the 8 ms reading at p95 on all six fixtures). A scroll highlights from the nearest
    checkpoint up to the viewport plus overscan, so a jump to the end of a large file never
    parses the whole file on the UI thread. Results come back from the highlighter thread
    and turn into colour-only `Span`s. Lines draw plain until their colours arrive, so
    nothing moves when they land.
  - **Colours:** K1 builds one syntect `Theme` per Strata syntax palette, from scope
    selectors. A theme switch re-colours lines without re-parsing them.
  - **Shared:** C11 code blocks and the action editor use the same highlighter through
    rune-kit's `Markdown::highlight` hook and TextArea spans, so one file colours the same
    everywhere.
- Find highlights and selection with copy.
- The document-position scrollbar (`preview/preview-panel/documents/scrollbar`): a thumb that
  follows document position, not estimated heights.
- An editable variant for the custom-action script editor (`integration/custom-actions`):
  Rune's core `text_input` with highlight spans, not rune-kit's TextArea, which takes no
  right-click handler for C21's field menu (U-C21-2). It keeps a parse state per line. An edit
  re-highlights in the foreground from its line to the bottom of the viewport plus overscan,
  then a background pass runs on until the parse state matches the cached one again (S4's
  workaround 1). With `onig`, an edit inside a heredoc or a fenced block runs to that block's
  end, so lines past the pass are provisional until it finishes.

**C11 DocumentView.** `preview/preview-panel/documents` and `.../odt`. Strata already parses
Markdown, an HTML subset, RTF, DOCX and ODT into its own model. C11 renders that model natively:
headings, paragraphs, lists, tables, images, links, selectable text across blocks, and find
highlights. Spreadsheets (CSV, TSV, XLS, XLSX, ODS) use C11b's SheetGrid: a virtual grid with
sheet tabs (rune-kit TabStrip), row numbers and column letters. It is its own grid styled as
rune-kit Table, as C4 is: Table selects whole rows only, sorts by one key, can't scroll
sideways and its cells take no presses. Tables inside documents stay static until the grid is
wired into them. rune-kit `Markdown` is the reference for table sizing and multi-block
selection, not the renderer, since it parses its own Markdown.

**C12 PdfView.** `preview/preview-panel/pdf`. A virtual column of page images. The sandboxed
renderer already returns PNG pages, which become `Pixels`. Pages re-render at the zoom level.
Glyph boxes from the renderer form a text layer, which draws selection and builds the copied
text. There is no password prompt: Strata's renderer can't take a password, so a locked PDF
shows Strata's message with an Open button.

**C13 MediaPlayer.** `preview/preview-panel/media`, `.../audio`, `.../video`.
- A video surface (`Pixels`, streamed from P7).
- The audio view: artwork, tags, and a live spectrum drawn as `Live`-bound elements with
  Strata's gradient, not `canvas` shapes, which can't change without a view commit.
- A waveform scrubber with chapter ticks and storyboard frames on hover.
- Transport controls, plus the timeline key map shared with quick preview.
- Hand-off to the default player at the current position.

**C14 Viewers.** Thin parts on `Pixels`, `on_wheel` and `on_magnify`, over C14a's pixels
viewport (C14b the viewers, C14c their follow-ups):
- an image view with zoom and pan. This is new against Strata, whose image preview has no
  zoom: over it Ctrl+wheel zooms instead of changing the text size, and a drag above fit pans
  instead of dragging the file out;
- the 3D model view: a software-rendered frame, four camera presets, an orbit indicator, and
  badges. No orbit by drag: as in Strata, only the camera buttons change the view, and a drag
  on the picture drags the file;
- the route view: an image plus a marker animated along the route and the elevation profile;
- G-code thumbnails;
- archive preview: C1 in read-only mode over the member tree.

**C15 StrataDialog.** The modal shell for `app/dialogs` (45 bullets), built on Rune's
primitives (`El::modal`, `backdrop_blur`, presence transitions, `default_button`) rather than
wrapping rune-kit `Dialog`. At the pin `Dialog` has a fixed height or a fit that doesn't
scroll, hardcodes its look, has no footer slot, and builds its scrim so it can't blur. It has:
- a blurred backdrop (`backdrop_blur`) that covers the header;
- open and close motion, dismissal by clicking the backdrop, and initial focus;
- Enter submits and Escape cancels;
- sizing that stays inside the window.

Every action dialog builds on it: Properties, conflicts, Connect to server, passphrase,
Format, Open With, leftovers review and the update dialog.

**C16 ProgressDock.** `operations/progress`, `.../dock` and `.../jobs`. Cards in the
bottom-right with progress and speed readouts; byte counts show in the progress dialog only.
Each card cancels its own operation: as in Strata, the card closes and the host opens its
dialog. The dock also shows brief notices. A blocking progress dialog is built on C15, and
the Jobs dashboard sits in the footer. Built on Rune primitives, not rune-kit's ToastRegion,
Toast and Card: at the pin ToastRegion fixes its key, insets and width and has no height cap
or scroll (draft U-C16-1).

**C17 SearchPalette.** Ctrl+K global search, `browser/navigation/folder-jump` and the 10xer
fuzzy folder picker (`integration/10xer-mode/places`). A C15 modal with a core `text_input`
field over a virtual result list, with matched characters highlighted and results that stream
in while the query runs.
It keeps empty and loading states distinct.

**C18 FilterBar and FooterPrompt.** The Ctrl+F filter with the Enter-target outline
(`browser/search/filter/enter-target`), and the 10xer footer prompts (`/`, `f`, `s`,
`go ›`, `rename ›`) with the chord key panel (`integration/10xer-mode`).

**C19 Footer and ShortcutReference.** The footer: F1 button, item and selection count, and
the Files on clipboard badge. The F1 panel shows the active key map, with search and
categories, using rune-kit `Keybind` chips (`app/shortcut-reference`).

**C20 Settings controls.** rune-kit `Page`, Switch, Select, SegmentedControl and
ShortcutRecorder cover most rows. To build, in four work packages (C20a the Slider and
NumberField, C20b the ColorPicker, C20c the tabbed form and settings search, C20d the
IconPicker):
- **Slider** with snap points: the density sliders, text size and thumbnail size. Rune core
  has the slider role with `on_step` and `on_step_limit`, but the kit has no control.
- **NumberField** (spin button): thumbnail workers, the corner roundness percent.
- **ColorPicker**: hue, value and alpha, hex entry, and swatches. It serves folder colours
  (`browser/folder-customization`) and the theme editor (`settings/themes`).
- **IconPicker**: a searchable grid of bundled icons and emoji, on C1e with C3's tile
  geometry rather than C3's `IconsGrid`, whose tiles are file tiles (captions, thumbnails,
  rename, marquee). It replaces GTK's `EmojiChooser`.
- **Tabbed form**: the action editor's notebook (`integration/custom-actions`): its own
  tablist and pages. At the pin SegmentedControl is a radio group with text labels only, so
  it can be neither a tablist nor carry an error mark (draft U-C20c-1).
- **Settings search**: results that jump to and highlight the row (`settings/preferences`).

**C21 Context menus.** rune-kit `context_menu` with Strata's rules: open from the keyboard
(Menu key, Shift+F10) at the cursor item, a background menu, grouped sections, the custom
actions section, Send to submenus, and a row-hover look from `Kit::interactive`
(`browser/context-menu`, 100 bullets). Behavior lives in Strata. The component is the anchor
and placement policy.

**C22 Folder peek.** Adopt `HoverCard` with `El::hover_delays`. Up to 8 rows, off by default
(`preview/quick-preview/folder-peek`).

**C23 Motion effects.**
- The trash flight: icons fly between rows and the sidebar Trash button, using Rune's
  shared-element motion (`operations/trash/animation`).
- The dissolve after a permanent delete (`operations/delete/dissolve`): fragments cut from a
  ghost of the row the host supplies, or flat `canvas` shards without one. Not a `rune-fx`
  compositor slot: the owner ruled it out on integration cost (`rune::run_with` in every
  host, wgpu in strata-components, every golden rendered through it).

Both play only when the operation finishes before its progress card would show.

**C24 Scroll extras.** Middle-click autoscroll, which needs `pointer_buttons([Middle])` and a
`scroll_to_with` glide. Overlay scrollbars are C24's own, not rune-kit `ScrollBar`, whose keys
collide across collections and which can't sit outside C4's sideways scroll. Wheel routing
follows Rune's latching rules (`browser/scrolling`). C24b fits the bars onto C3, C4 and the
archive list, and adds the outside-wheel rule for panels.

**C25 ThumbnailStore.** Maps an entry to a `Pixels` handle under an LRU byte budget, and loads
visible rows first using virtual-row layout reports. It updates the image in place without a
view commit. The sandbox workers and disk cache (`browser/thumbnails/*`) stay Strata-side.

**C26 MountPrompts.** These replace `gtk::MountOperation` (11 files). A `gio::MountOperation`
whose `ask-password`, `ask-question` and `show-processes` signals open C15 dialogs through P9.
They cover sign-in, the SSH host key and LUKS passphrases (`remote/file-providers/network-locations`,
`devices/volumes/encrypted`). C26 holds the dialogs, driven by plain requests and answered with
plain responses. The GIO binding lives in Strata (the owner's ruling, option B): it is tangled
with Strata's policy, and reading `show-processes`' pids needs `unsafe`. C26's module docs give
the mapping it must follow. Every question and process list uses C26's dialogs, where GTK
Strata left non-SFTP questions and GVfs's busy dialog to GTK.

### Tier 3: `strata-kit` (theme and look)

**K1 Theme mapping.** Turns Strata's 95 bundled themes, custom theme files and the Omarchy
Quattro mapping (`settings/themes`, `.../omarchy`) into `rune_kit::Kit`. Use
`Kit::from_palette` for the base and override the semantic tokens per theme. Syntax palettes
map onto `SyntaxTheme`, and element glow onto `rune-fx` glow. Theme switches animate on
`Tx::color_motion`.

**K2 Shape, density and type.**
- Corner roundness from 0 to 200 % scales `Kit.radius` (`settings/themes/corner-roundness`).
- Frame and Content density become spacing tokens that components read
  (`browser/view-modes/density`).
- Text size scales the type ramp (`browser/view-modes/text-size`).
- The interface font switches through `Tx::set_font_families` (`settings/themes/interface-font`).
  The picker's list of installed families arrives as data from the host: Rune resolves an
  installed family by name but has no public way to list them (draft U16, upstream #114).

**K3 Icon catalogs.** Lucide (single colour, accent-tinted) and Catppuccin (multi-colour)
become `rune::Icon` catalogs, generated at build time as `themes/opencode/tools/icons.py`
does. Rune's icon rasterizer draws bodies that name their own colours in those colours, so
Catppuccin needs no special path. The file-name → icon mapping stays in Strata
(`settings/themes/icons`, `.../catppuccin`).

**K4 User overrides.** These replace the GTK user stylesheet, which is kept as a feature in a
new form. The file is `$XDG_CONFIG_HOME/strata/overrides.toml`.
- It sets any `Kit` token or strata-kit extension by name: colours, radii, spacing, type,
  glow, and syntax scope colours. It can scope a value to light or dark, or to a component
  (`[list.row]`, `[sidebar.row]`) where the component reads a token through a named slot.
- Overrides apply after the theme, Omarchy following, roundness and density. They reload
  live when the file changes, through the same path as a theme switch.
- Errors list each unknown token or bad value with its line. Rows that parse still apply, as
  `settings.toml` recovery does (`settings/preferences/storage`).
- Settings → Appearance keeps the row. It opens the file, shows the error count, and has a
  Reset action.
- Migration: on first launch, Strata reads an existing user stylesheet and converts
  the rules it can map, meaning plain colour, radius and padding declarations on Strata's
  documented style classes. It writes `overrides.toml` and leaves the CSS in place. It lists
  every rule it did not convert in a one-time notice. It never deletes the CSS.
- Component slots are API: renaming one is a breaking change, recorded in the changelog.
- No icon overrides (a file type → icon): the owner deferred §10 question 2, so K4 shipped
  without them.

**K5 Not ported.** The "interface renderer" choice (a GSK setting) and the uncomposited popup
fallback have no Rune equivalent. Drop both, with a migration note. A saved value is ignored
and removed on the next save.

**K6 Bundled font.** Added by the owner after K4. Strata ships JetBrains Mono and uses it for
every preview's monospace text and as its default interface font, so `strata_kit::fonts`
bundles JetBrains Mono 2.304 (byte-identical to Strata's, with its OFL 1.1 licence) and makes
it the default for both font slots. A chosen interface font goes in front of it; the mono
slot stays the bundled font. There is no Nerd Font or generic monospace fallback.

## 5. GTK services that are not widgets

Strata-side work that no component owns, listed so nothing falls through:

| GTK use | Replacement |
| --- | --- |
| `gtk::RecentManager` (`browser/navigation/recent`) | Read and write `recently-used.xbel` directly |
| `gtk::Settings` (16 files) | P5 |
| `gtk::IconTheme` | P6 |
| `gtk::FileLauncher`, AppInfo launch | `gio::AppInfo` launch or the OpenURI portal, with activation tokens |
| `gdk::Texture` (40 files) | `rune::Pixels` |
| `gtk::CssProvider`, `style.css` | strata-kit tokens (K1–K3); the user stylesheet becomes K4 |
| GDK `no-portals` handling (`integration/file-manager-interface/service-activation`) | Likely unnecessary without GTK in the process. Verify, then retire the node. |
| In-process QA harness, X/Wayland input injection (`src/qa`) | Rune headless scripts, `--tree` and AccessKit actions |

## 6. Testing

- **Per component:** a gallery scene for each state (light and dark), an a11y tree golden, and
  headless scripts on Rune's virtual clock. Scripts are the port's QA scenarios. Each names
  the feature-map Behavior bullets it checks.
- **Performance:** perf-profile tests like rune-kit's, at 100k entries for C1–C4 and 50k lines
  for C10, plus `RUNE_COMMIT_BUDGET_MS=8` runs over scripted scrolls.
- **Platform shims:** P1–P4 need real compositors. Run them under headless sway and Xwayland,
  as Rune checks its layer shell, plus Hyprland in Strata's QEMU images. DnD tests drag between
  Strata and a second process, as Strata's `tests/qa/foreign_drag_source.py` does today.
- **Parity:** run the feature map's QA sweeps (`qa/sweeps/<area>.md`) against the Rune build.
  Every Behavior bullet that the port intentionally changes needs a decision recorded in an
  issue.

## 7. Phases

| Phase | Scope | Exit check |
| --- | --- | --- |
| 0. Spike | Rune window on Hyprland, GNOME and X11. File Rune issues for P1–P5 and popup surfaces. P9 bridge prototype. Settle the crate name (ruled: it keeps `strata-platform`, §0), and set up `strata-platform`'s CI and release workflow. syntect `fancy-regex` vs `onig` benchmark (S4: `onig`, `docs/decisions/S4-syntect-backend.md`). | Each of P1–P5: Rune shipped, or "shim here" decided. Crate name settled. |
| 1. Foundations | strata-kit K1–K4, P3, P5, P8, P9, C15, the gallery. First `strata-platform` release. | A themed empty window with the header, opened by `strata %U`, a second launch forwarded |
| 2. Collection | C1, C3, C4, C2, C5, C24, C25 | 100k-entry folder at 60 Hz. Selection and rename scripts pass. |
| 3. Chrome | C6, C7, C8, C19, C21, C22, C17, C18 | Navigation, tabs, sidebar, search and 10xer sweeps pass headless |
| 4. Operations | P1, P2, C16, C23, C26 and the conflict dialogs | DnD and clipboard round-trips with Nautilus and Dolphin on Wayland and X11 |
| 5. Preview | C9, C10, C11, C12, P7 + C13, C14 | Preview sweeps pass. The sandbox protocol is unchanged. |
| 6. Settings and chooser | C20a–C20d, P4, P6, the portal chooser window on Rune | A portal Open/Save from Firefox and a GTK app on Hyprland. `cargo tree` shows no `gtk4`, `gdk4*`, `sourceview5` or `gdk-pixbuf`. |

Phases 2 and 3 can run in parallel after phase 1. Phase 4's P1 is the long pole: start the
shim during phase 1 if the Rune issue gets no answer.

This is a full port. GTK and Rune never ship together in one release: no binary embeds both
toolkits, and no process runs the chooser on GTK. Until phase 6 passes, releases come from the
GTK branch, and the Rune build ships only as a nightly.

## 8. Risks

- **Rune is pre-release.** APIs move, and we cannot send fixes upstream. Pin the rev, take
  updates deliberately, and keep shims thin.
- **A published crate on a moving base.** Rune isn't on crates.io; apps pin it by git rev.
  So `strata-platform` cannot be published to crates.io while Rune isn't: crates.io rejects git
  dependencies. The name `strata-platform` is also taken there by an unrelated crate, and the
  owner ruled to keep it (§0), so the crate is released as git tags, with each tag naming the
  Rune rev it supports. Users pin the tag beside the same Rune rev. If two Rune revs end up in one
  build, cargo builds both, and their types don't match.
- **Full port, no fallback.** Without a GTK chooser to fall back on, a P4 failure on some
  compositor blocks the release. Test P4 on Hyprland, GNOME, KDE and sway in phase 0, not
  phase 6.
- **GPU requirement.** Rune draws through wgpu. Machines without Vulkan or GL fall back to
  lavapipe, which is slower than GTK's Cairo path. Measure it on the QEMU images.
- **Menus clip to the window** until Rune has popup surfaces (§4, end of tier 1).
- **Wayland DnD inside winit.** Binding `wl_data_device` beside winit's own seat handling may
  conflict with its pointer state. Prototype P1 in phase 0.
- **Accessibility parity.** Strata labels AT-SPI objects in detail (`app/accessibility`).
  AccessKit exposes mounted virtual rows only. Check Orca on long listings.
- **Scope.** The feature map lists 5,709 Behavior bullets. The port's parity bar is the
  sweeps, not the GTK code, and each deliberate change goes through an issue.

## 9. Decisions

| # | Question | Decision | Where it applies |
| --- | --- | --- | --- |
| 1 | Full port, or keep the portal chooser on GTK for a while? | Full port. No GTK in any shipped binary. | §7 (phase 6 exit, release rule), §8 |
| 2 | Shims private or published? | Published `strata-platform` crate, generic to any Rune app on Linux | §3 rules, §7 phases 0–1, §8 |
| 3 | Keep user stylesheets? | Yes, as a token override file with CSS migration | K4, §5 |
| 4 | Highlighting engine for C10? | syntect, on the `onig` backend with a dedicated highlighter thread and a checkpoint every 64 lines (S4) | C10, C11, K1 |
| 5 | Crate name (was §10 question 1)? | Keep `strata-platform`. It is taken on crates.io, so releases are git tags only. | §0, §3, §8 |
| 6 | Oldest libwayland supported? | 1.23. Ubuntu 24.04 (1.22) is dropped; older hosts still degrade, unsupported (S3 decision 1, 2026-11-10). | §0, P0–P2, P4 |
| 7 | P4 with an `x11:` parent on a Wayland chooser, and focus without a token? | Parentless, with an Xwayland helper as the upgrade path; compositor placement plus a tokenless `focus()` that Strata calls (S3 decisions 2 and 3, 2026-11-10) | P4 |
| 8 | Where does C26's GIO binding live? | In Strata | C26 |

## 10. Open questions

1. Settled: the crate keeps the name `strata-platform` (§9, decision 5).
2. Override file scope: should K4 also cover icon overrides (a file type → icon), which
   `browser/folder-customization` does per path today? Deferred by the owner: K4 shipped
   without them.
