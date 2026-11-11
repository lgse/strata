# V0 findings: assumption checks for the Rune vertical slice

No stop condition was hit. An offscreen Rune `Runtime` hosts strata-components' Columns view
over Strata's real `Browser`, with no display, session bus or GPU, on the host and in the
pinned test image. Seven findings below change what V5 to V7 have to build.

Run on 2026-11-10. This is the slice plan's V0 ("Strata on Rune: vertical slice with
QA-harness parity"), laid out as the workspace proposal has it: the component crates under
`crates/`, with `strata-app` and `strata-rune` beside them.

| Probe | Result | Stop condition |
| --- | --- | --- |
| Build | One workspace, one lockfile and Rust 1.99.1 build both sides | None defined |
| Host | Tree and key input work offscreen, in the pinned image too | Not hit |
| Core | No GTK, GDK, GtkSourceView or GStreamer under `Browser` or rename | Not hit |
| Loop | 10,000 entries: last `BrowserEvent` 10.6 ms after `navigate` (release) | Not hit offscreen; the windowed wake is untested |
| Name | Row and list names match; roles, the pane group and row descriptions differ | None defined |

## Revisions and setup

| Input | Revision |
| --- | --- |
| Strata | `strata-next` at `65eb2da03` |
| strata-components | `integration` at `e4f2df1a` (the plan named `84a48c9`) |
| Rune | `2a44156b` (the plan named `5d421fd`), the rev strata-components pins |
| Toolchain | Rust 1.99.1 on both sides |
| Test image | The pinned base for `65eb2da03`, as `scripts/image_base.py ensure` returns it |

The spike is a Git worktree of `strata-next` on a local branch, `spike/v0-rune-probes`, which
is not pushed and is never merged. It copies strata-components' four crates and its
`tests/golden`, `tests/scripts` and `tests/fixtures/code` to the same paths, and adds two
probe crates:

- `crates/strata-app`: mirrors `BrowserEvent`s into the Columns view's listings and state.
- `crates/strata-rune`: `strata-rune --probe <directory> <out>` owns the `Runtime`, iterates
  the default GLib context beside `Runtime::frame`, sends Down and writes the tree.

Probe runs cleared `DISPLAY`, `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` and
`DBUS_SESSION_BUS_ADDRESS`. Times are from one 24-core machine, on tmpfs.

## Build probe

Both sides build in one workspace on Rust 1.99.1, which `strata-next` already pins.

- **Manifest changes the import needs.** `[workspace.package]` gains `version` and
  `rust-version`, which the component manifests inherit. `members` gains the test peer at
  `crates/strata-platform/tests/support/strata-test-peer`, which `crates/*` does not match.
  `[workspace.dependencies]` gains 24 entries and `[workspace.lints.clippy]` two
  (`await_holding_lock`, `await_holding_refcell_ref`). No shared entry needed a change.
- **Lockfile.** 448 packages become 669. Two packages the GTK app uses move: `bitflags`
  2.13.1 to 2.13.2 and `smallvec` 1.15.2 to 1.16.5. The GTK app's tree gains no Rune crate.
- **New duplicates.** 22 packages gain a second or third version, among them `zbus` 5.19.0
  and 6.2.1, `x11rb` 0.13.2 and 0.14.0, and `resvg` 0.45.1 and 0.48.1 with `usvg` and
  `tiny-skia`.
- **Build times, debug, cold.** `strata-gallery` and `strata-rune` in 51 s, then the GTK app
  in 55 s. `strata-rune` in release takes 177 s.
- **Test data.** `tests/golden`, `tests/scripts` and `tests/fixtures/code` collide with no
  path Strata has.

`cargo deny check` passes on the unified workspace, but only because it checks what the root
package reaches, and the GTK app reaches no Rune-side crate. `cargo deny --workspace check`
fails all four sections:

| Section | Error | Count |
| --- | --- | --- |
| sources | Git source not allowed (the Rune crates) | 6 |
| licenses | BSL-1.0 not allowed (`clipboard-win`, `error-code`) | 2 |
| advisories | RUSTSEC-2025-0141, `bincode` unmaintained | 1 |
| bans | `strata-fs` used directly by `strata-app` | 1 |
| bans | `gstreamer` and `gstreamer-app` used by `strata-platform` | 2 |

The last row conflicts with a rule in `AGENTS.md`: no crate under `crates/` may depend on
`gstreamer`. `strata-platform` does, behind its `media-audio` feature.

## Host probe

The offscreen host works, and needs nothing the pinned image lacks.

- **Tree and input.** The host, a `Runtime` at 1440x900 with the browser, builds in 13 ms. A five-entry folder
  is listed and shown 1.8 ms after `navigate`. Two Down presses move the keyboard cursor to
  rows 0 and 1, and `accessibility_json` returns the strip, the list and its rows with names,
  `position`, `setSize` and the `selected` state.
- **In the pinned image.** The same binary exits 0 on both fixtures with `--network none`
  and no display variables. `ldd` reports nothing missing.
- **What the image holds.** `vulkan-icd-loader` 1.4.357 and `mesa` 26.2.2, `libxkbcommon`
  and `wayland`. It has no Vulkan driver (`vulkan-swrast` is absent and
  `/usr/share/vulkan/icd.d` does not exist), no `sway` and no `xorg-xwayland`. The offscreen
  host uses none of them. Pixel capture and a session host in headless sway would.
- **What the image lacks for a build.** Its dependency stage builds `--bin strata` only, so
  it caches no Rune-side crate. The probe binary was built on the host and run in the image.
- **`Runtime::frame` returns "animating", not "has work".** After one key it stays true for
  389 ms with animations on and 179 ms with them off. What still animates with them off was
  not identified. With `Runtime::settle_all` after the key, the host is idle in 0.2 to 0.8 ms
  and the cursor is where it should be. The QA backend should settle that way.
- **A virtual list mounts 100 rows of 10,000.** The tree holds only those, as GTK's holds
  only realized rows. The backend needs a reveal step for the rest.

Strata's application layer has start-up duties that live in `src/main.rs` today. The first
probe run aborted with "file calls must not run on the UI thread": the first listing reads
the mount watcher, which `main.rs` starts with `adapters::mount_watch::start()` before it
calls `ui_thread::mark()`. `strata-app` needs that sequence from a shared crate.

## Core probe

The extraction already answered this. `cargo tree -p strata-fs -i gtk4` matches no package,
and `deny.toml` bans `gtk4`, `gdk4`, `sourceview5` and `gstreamer` outside the root package.
`Browser` and the rename operation are in `strata-fs`. `cargo tree -p strata-rune` likewise
holds no `gtk4`, `gdk4` or `sourceview5`.

What `Browser` does need from GLib is a UI thread that owns the default `MainContext`. It
schedules through `idle_add_local_once`, `idle_add_local_full`, `timeout_add_local_once`,
`timeout_add_seconds_local_once`, `timeout_add_local_full` and `MainContext::spawn_local`,
in eight places under `crates/strata-fs/src/app/browser/`.

## Loop probe

One thread owning both the `Runtime` and the default GLib context lists 10,000 entries
without a stall.

Each turn of the loop runs `MainContext::iteration(false)` until it returns false, then one
`Runtime::frame` stepped by the wall clock. `BrowserEvent` observers run inside GLib's
dispatch, where the `Runtime` owns the app state, so they queue changes that Rune's `poll`
hook applies at the next frame.

| Build, where | Last listing event | Listing shown | Copying rows |
| --- | --- | --- | --- |
| Release, host | 10.5 to 10.7 ms | 29.1 to 29.5 ms | 14.7 ms |
| Release, pinned image | 8.9 ms | 22.7 ms | 9.8 ms |
| Debug, host | 33.5 to 34.4 ms | 147 to 149 ms | 84 to 85 ms |

Times count from `Browser::navigate`; host rows are three runs each. The listing arrives as
one `EntriesReplaced` and seven `EntriesPublished` events in 11 GLib dispatches. "Copying
rows" is the probe's own cost: it reads the whole column again on each event, 80,000 row
copies in all. An adapter that copies only each published range would copy 10,000.

**The windowed wake was not tested.** In a window, Rune runs winit's loop, and nothing
iterates GLib while that loop sleeps. Two designs are open:

- **A tick.** `poll` iterates GLib and asks for `Tx::wake_after` while the browser has work
  in flight. It needs nothing new, and it polls.
- **GLib's own embedding.** The UI thread calls prepare and query, a helper thread polls the
  file descriptors and calls Rune's `Waker`, and the UI thread calls check and dispatch.
  glib 0.22 does not bind `query` or `check`, so this needs `glib::ffi` and an `unsafe`
  shim, which belongs in `strata-platform`.

Either way a `Waker` call must reach `poll`. strata-components records that at Rune
`5d421fd` a wake alone did not run `poll` while a Wayland window was fully covered, and a
`wake_after` timer did (Rune #139). That was not re-measured here. V5's test of a GLib idle
callback reaching the UI state within one frame is where this gets settled.

## Name probe

Names agree. Role names, the pane group and row descriptions do not.

Both trees are of the canonical fixture in Columns after two Down presses: GTK's from
`{"query":"tree"}` through `./scripts/qa-drive.sh`, Rune's from `accessibility_json`.

| Element | GTK harness | Rune | Same |
| --- | --- | --- | --- |
| Pane | `grouping`, named by the directory, described "Columns view" | No such node: the column's group has no name | No |
| List | `list`, named by the directory, described "Files" | `list-box`, named by the directory, no description | Name only |
| Row | `list item`, named by the file | `list-box-option`, named by the file | Name only |
| Row description | "Folder" or "File", through the `description` predicate | None, or the size badge's words | No |
| Selected row | `[selected] [focused]` on the row | `selected` on the row | Partly |
| Focus | On the row | On a `group` named "Columns", whose `activeDescendant` is the row | No |
| Open child pane | A second pane, for the focused folder | None | No |

- **Roles need the map the plan expects**, for example `list-box` to `list`,
  `list-box-option` to `list item`, `scroll-view` to `scroll pane` and `group` to
  `grouping`.
- **A host cannot set a row's description in Columns.** `ColumnsView` calls the host's
  `collection` hook first and then sets `Collection::describe` to the size badge's words, so
  a host's `describe` is replaced. V6 planned to use `Collection::describe`. This is a
  `component` gap, or a ruling that the badge's words are the description.
- **GTK's tree text prints no description on rows**, though the `description` predicate
  reads "Folder" and "File" for them. Compare through predicates, not the tree text.
- **Focus is modelled differently.** The backend has to report the strip's active descendant
  as the focused entry.

## What V5 to V7 have to build

Each of these was seen in a probe, and none is in the slice plan.

1. **Two owners of cursor and selection.** GTK launches with the first entry focused and
   selected, because `Browser` emits `FocusChanged` and `SelectionSetChanged` at load. The
   Columns view keeps its own cursor, which started empty, so the first Down landed on
   `archive` where GTK lands on `documents`. The adapter must apply the browser's focus and
   selection events to `ColumnsState`, and V6 must say which side moves the cursor.
2. **Hidden entries are filtered in the view.** A `Browser` column holds hidden entries; the
   GTK views filter them out of the model (`src/ui/browser/entry.rs`). The adapter must
   filter too, and map each shown row to its browser position.
3. **Mirroring is GTK view policy.** Moving to a folder opened its child pane in GTK.
   `Browser::select` emitted no `ColumnAdded`. `strata-app` has to port that policy.
4. **`EntrySource` cannot borrow from `Browser`.** `EntrySource::entry` returns names
   borrowed from the source, and `Browser` keeps its entries behind a `RefCell`. The adapter
   copies display fields, or `Browser` gains an accessor that can lend them.
5. **Start-up duties move out of `src/main.rs`**, as the host probe found.
6. **`cargo deny` needs `--workspace`**, in `mise.toml` and CI, once a crate exists that the
   GTK app does not depend on. Its rules then need the entries listed under the build probe.
7. **The QA backend settles with `Runtime::settle_all`**, and reads focus through the
   strip's active descendant.

## Owner decisions

| Decision | Status after V0 |
| --- | --- |
| Plan 2: raise the toolchain | Done on `strata-next` before V0. Both sides build on 1.99.1. |
| Plan 3: a software Vulkan driver in the image | Not needed for the offscreen host. |
| Plan 4: which strata-components revision | Open. The spike copied `integration` at `e4f2df1a` into `crates/`, as the proposal lays it out. |
| Plan 5: GTK's names and AT-SPI roles as the contract | Needs a ruling on row descriptions and on the pane group. |
| New: the windowed wake | A tick, or an `unsafe` GLib embedding shim in `strata-platform`. Needed by V5. |
| New: `gstreamer` in `strata-platform` | Change the `AGENTS.md` rule, or move `media-audio` out of `crates/`. Needed by the import. |

## Not verified

- The windowed binary, and whether GLib work wakes Rune's loop in a window.
- A build of the Rune side inside the pinned image.
- The component crates' own tests, goldens and Clippy under Strata's lint settings.
- A pixel capture from the offscreen host.
- The same listing on the GTK target. Launch and listing times on both targets are V10's.
- Rename through `Browser::rename`. The probe wired the operation provider and never
  renamed.
- A second column in Rune. The probe opened only the root.
- Any QA scenario run. `./scripts/qa.sh` was not run, and no file under `src/` changed.
- Which package depends on `bincode`.
