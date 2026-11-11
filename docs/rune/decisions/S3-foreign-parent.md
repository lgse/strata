# S3 Foreign parent survey

Spike S3 ([work package](../work/phase-0-groundwork/S3-foreign-parent-survey.md)) asks whether
a Rune window can be made transient for another app's window on Hyprland, GNOME, KDE and sway,
for the portal FileChooser (plan §4 P4, §8). This record is what P4 builds against.

## Verdict

**No blocker found.** All four compositors implement `zxdg_importer_v2`. Rune gives a shim the
window's `wl_surface` or X11 window id before the first frame, which is enough to parent the
window from outside Rune. On sway, a Rune window parented this way floats, stacks above its
parent and shows over a fullscreen parent. On Xwayland, `WM_TRANSIENT_FOR` set from a second X
connection does the same. Real `wayland:` handles from GTK 3, GTK 4 and Chromium imported
cleanly.

Three conditions needed an [owner decision](#owner-decisions) before P4 ships. None stopped P4
from starting, and the owner ruled all three on 2026-11-10:

1. Ubuntu 24.04's libwayland 1.22 gives the shim no route to winit's `wl_display`. **Ruled:**
   Ubuntu 24.04 is dropped, and libwayland 1.23 is the minimum supported version.
2. An `x11:` parent can't be honoured by a chooser running on Wayland. **Ruled:** parentless
   (a), with the Xwayland helper (b) as the upgrade path.
3. The FileChooser portal carries no activation token, so a parentless chooser is placed and
   focused only by the compositor's own policy. That matters most on GNOME. **Ruled:**
   compositor policy plus a tokenless `focus()` (a + b), which Strata calls, not the shim.

Only sway and Xwayland ran here. Every Hyprland, GNOME and KDE cell is a hand test pending with
a source-based expectation. QEMU runs (Tier B) were not approved for this spike;
[How to run Tier B](#how-to-run-tier-b) gives the commands. What P4 must still settle is under
[Open for P4](#open-for-p4).

## Environment

| Item | Version or setting |
| --- | --- |
| Rune | `78d22ed` (plan §0); winit 0.30.13, wayland-client 0.31.15, wayland-backend 0.3.17, x11rb 0.13.2 |
| Measured compositor | sway 1.12, headless (`WLR_BACKENDS=headless`, gles2 renderer), Xwayland 24.1.14 |
| No-WM X server | Xvfb 21.1.25 |
| Portal | xdg-desktop-portal 1.22.1 on a private bus, with a stub `impl.portal.FileChooser` |
| Requesters | GTK 3.24.52 and GTK 4.24.1 (PyGObject), zenity 4.2.2, Chromium 155.0.8059.84 |
| Read from source | Hyprland 0.56.2, Mutter 46–50, KWin 6.7, GTK 3.24.41–3.24.52, xdg-desktop-portal-gtk, Chromium `xdg_foreign_wrapper.cc` |

Every run used a private `XDG_RUNTIME_DIR`, `dbus-run-session` and a headless sway. Nothing
touched the desktop session. The prototype and raw results live outside this repository in
`~/dev/strata-spikes/s3/`: `proto/` (the `s3-child` Rune app and the `s3-peer` raw client),
`env/` (session and experiment scripts), `requesters/` and `results/`. Result paths below are
relative to `results/`.

## Results

Evidence tiers:

- **Measured** here, on headless sway or Xwayland or Xvfb.
- **Source**: read from the compositor's or toolkit's code at the version named.
- **Pending**: a hand test is pending, with the expectation from source.

"X11" means an X11 requester and an X11 chooser window, which on a Wayland session go through
Xwayland. A Wayland chooser with an `x11:` parent is covered under
[Q4](#q4-fallbacks-when-parenting-fails). Details are in the notes below the table.

| Compositor | Session | Parent takes effect | Requester handles | Fallback |
| --- | --- | --- | --- | --- |
| sway 1.12 | Wayland | Measured: yes [1] | Measured: usable [2] | Measured: parentless [3] |
| sway 1.12 | X11 (Xwayland) | Measured: yes, if set before map [4] | Measured: `x11:` [5] | Measured: centre on the screen [6] |
| Hyprland 0.56.2 | Wayland | Source, pending [7] | Source, pending [8] | Pending: parentless [9] |
| Hyprland 0.56.2 | X11 (Xwayland) | Pending [10] | Source: `x11:` [5] | Pending: as on sway [6] |
| GNOME (Mutter 46–50) | Wayland | Source, pending [11] | Source, pending [12] | Pending: parentless [13] |
| GNOME | X11 | Pending [14] | Source: `x11:` [5] | Pending: centre on the screen [6] |
| KDE (KWin 6.7.5) | Wayland | Source, pending [15] | Source, pending [16] | Pending: parentless [17] |
| KDE (kwin_x11 6.7.5) | X11 session | Pending [18] | Source: `x11:` [5] | Pending: centre on the screen [6] |

1. sway offers `zxdg_importer_v1` and `_v2`, and no `xdg_wm_dialog_v1`. The parented child
   floats, centred, and is shown over a fullscreen parent. A bad handle gets `destroyed` within
   one roundtrip. A v1-exported handle imports through v2.
2. GTK 3 and GTK 4 send `wayland:` (v2 export). Chromium sends `wayland:` (v1 export, see
   [Q3](#q3-do-requests-carry-a-usable-handle)). zenity sends `""`.
3. Parentless: tiled and focused. Over a fullscreen window: hidden and unfocused.
4. `WM_TRANSIENT_FOR` with the dialog type: floated, `transient_for` in the IPC tree, shown over
   a fullscreen parent. Only if set before sway maps the window ([Q2](#q2-wm_transient_for-on-a-winit-x11-window)).
5. GTK sends `x11:400003`; Chromium sends `x11:0x400003`.
6. A bad xid: the shim sets no transient-for and centres the window on the screen. sway still
   floats a dialog-type window.
7. v2 importer and exporter only, no v1, and `xdg_wm_dialog_v1` (Hyprland 0.56.2 binary symbols,
   `src/protocols/XDGForeignV2.cpp`). A bad handle gets `destroyed` at import. `set_parent_of`
   calls `setNewParent`. How Hyprland floats or places a child is unverified.
8. GTK 4 and GTK ≥ 3.24.44 export v2. Chromium exports v2, the only exporter offered. GTK below
   3.24.44 sends `""`.
9. Strata's existing window rule centres the chooser (`src/portal/window_geometry.rs`).
10. Hyprland's XWM reads transients: the binary has `set_transient` and a loop guard.
11. v2 importer only, in Mutter 46, 47, 48 and main. Mutter creates the `MetaWindow` when the
    client calls `get_toplevel` (`xdg_surface_constructor_get_toplevel` →
    `meta_window_wayland_new`), so a parent set before the first commit passes Mutter's
    `is_valid_child`. `set_parent_of` calls `meta_window_set_transient_for`. A bad handle gets
    `destroyed`. An invalid child surface is a **fatal protocol error**. Ubuntu 24.04 (Mutter 46)
    gets no parenting from Rune because of libwayland 1.22, and is unsupported
    ([decision 1](#1-ubuntu-2404-and-libwayland-122)).
12. GTK 4 and GTK ≥ 3.24.44 export v2. Ubuntu 24.04's GTK 3.24.41 has only the v1 exporter,
    finds no exporter on Mutter, warns "Server is missing xdg_foreign support" and sends `""`.
13. Mutter places it. Focus without an activation token is uncertain
    ([decision 3](#3-no-activation-token-and-focus-of-a-parentless-chooser)).
14. GNOME 50 ships no Xorg session; Ubuntu 24.04's GNOME 46 still has one. Under Xwayland,
    Mutter's XWM honours `WM_TRANSIENT_FOR`.
15. v2 only (`src/wayland/xdgforeign_v2.cpp`). A bad handle gets an inert object and
    `destroyed` straight away. `set_parent_of` links the transient.
16. As GNOME, with Fedora 44's GTK 3.24.52 (v2).
17. KWin's placement and focus-stealing prevention apply.
18. ICCCM transients, standard in KWin.

Firefox was not installed, so every Firefox cell is pending. From source, a distribution
Firefox behaves like GTK 3: its file picker is GTK 3's `GtkFileChooserNative` on the system GTK.
The snap and Flatpak builds bundle their own GTK, so their GTK 3 version, and with it the
exporter, is the bundle's.

## Findings by question

### Q1. `zxdg_importer_v2` and `set_parent_of` on a winit toplevel

- **Globals** (`e1/e1.1-globals.json`): sway offers `zxdg_importer_v1` and `_v2` and the
  matching exporters, plus `xdg_activation_v1` and `zwlr_virtual_pointer_manager_v1`. It has no
  `xdg_wm_dialog_v1`. The Hyprland binary has only v2 (`CXDGForeignImporterProtocolV2`) and
  has `xdg_wm_dialog_v1`. Mutter and KWin have v2 only. wayland.app's support table is out of
  date for Hyprland (it says 0.52.1 lacks the protocol).
- **Takes effect** (`e1/e1.2-*`): imported in `App::on_window`, the Rune window maps as sway
  treats a transient. It floats at 424×302, centred, where the parentless control is tiled. It
  is shown over a fullscreen parent, where the parentless control is hidden. sway's IPC shows no
  xdg parent, so floating and fullscreen visibility are the observables.
- **Order matters** (`e1/e1.5-*`): set after the first frame (`App::on_ready`), the import
  succeeds but sway has already tiled the window. Sway decides floating at map. `on_window`
  runs before the first frame on Linux (`window/create.rs:232-233`), so it is the right hook.
- **Hidden-first works on Wayland too** (`r2/t1-wayland-hidden-first.*`). With
  `WindowOptions { visible: false }`, `on_window` runs, the import lands, then `show()` maps the
  window and it floats. So P4 can open hidden on both backends, for a window's first creation.
- **Cross-version** (`e1/e1.3-*`): a v1-exported handle imports through v2 on sway, because
  wlroots shares one registry between v1 and v2.
- **Invalid or stale handle** (`e1/e1.4-*`): `destroyed` arrives within the roundtrip after
  `import_toplevel`, and the window stays parentless. Hyprland, Mutter and KWin send `destroyed`
  at import too (source).
- **Parent closes** (`e1/e1.6a-*`): `destroyed` reaches the shim's private queue about one
  second in, when the parent exits. It needed only `dispatch_pending` from an
  `AppHandle::update`, because winit's loop reads the shared socket. Sway leaves the child
  floating.
- **Reopen** (`e1/e1.6b-*`): Rune destroys a Wayland window on close. On reopen `on_window`
  runs again, the new surface imports again, and it floats again.
- **Connection safety** (E1.7, `e1-exit/`):
  - Import dropped on the UI thread before quitting: 5 of 5 runs exit 0
    (`summary-drop-vs-keep.txt`), as do all 11 E1 runs, and none logs a protocol error.
  - Import left to a thread-local destructor after `run()` returns: 5 of 5 runs died with
    SIGSEGV. `coredumpctl info` shows frames `wl_proxy_destroy` ←
    `wayland_backend … destroy_object_inner` ← `ZxdgImportedV2::destroy` ← the `WlParent` drop ←
    `__call_tls_dtors` ← `exit` (`coredumpctl-last.txt`). winit had already closed the display.
  - Import still alive after `run()` returns, but leaked with `mem::forget`: 5 of 5 runs exit 0
    (`summary.txt`).
  - Mutter (`is_valid_child`) and Hyprland post a fatal protocol error if `set_parent_of` gets a
    surface that isn't an xdg_toplevel. On winit's shared connection that kills the whole app.
    The surface from `with_raw_handle` is always winit's xdg_toplevel, so P4 must never pass any
    other surface.
- **Raw client** (`e1-raw/`): `s3-peer --import` parents a plain wayland-client toplevel the
  same way. It's the Tier B tool, since it needs no GPU and only glibc 2.34.

### Q2. `WM_TRANSIENT_FOR` on a winit X11 window

- winit 0.30 has no transient-for on X11. `with_parent_window` embeds a child window instead.
  The shim sets `WM_TRANSIENT_FOR`, `_NET_WM_WINDOW_TYPE_DIALOG` and, for modal requests,
  `_NET_WM_STATE_MODAL`, through its own x11rb connection. Properties can be set from any
  client.
- **winit maps the window inside `create_window`**, so a visible window is already mapped when
  `on_window` runs (`mapped_when_set: true`). Sway still floated it, because its XWM decides at
  the Xwayland surface's first commit, after Rune's first frame. That's a race, not a guarantee.
  Set after the first frame, sway records `transient_for` but keeps the window tiled
  (`e2/e2.1-on-ready`).
- **Hidden-first works for a window's first creation only.** Open with
  `WindowOptions { visible: false }`, set the properties in `on_window` (`mapped_when_set:
  false`), then `WindowHandle::show()`. The window floats as a dialog with `transient_for` set
  and shows over a fullscreen parent (`e2/e2.1-hidden-first*`). On Xvfb with no WM, the same
  three properties are present before map (`e2.2/`).
  - A later creation ignores `visible: false`: `window/create.rs:13` computes
    `visible = !launcher && (self.started || self.options.visible)`, and `started` is set after
    the first open (`window/events.rs:19`).
  - Measured (`repro/x11.txt`): with `visible: false` and `release_gpu_when_closed`, creation 1
    is `UNMAPPED` at `on_window` and creation 2 is already `VIEWABLE`.
  - A fresh `rune::open_window` host starts with `started: false` (`window.rs:157-172`, 554), so
    its first creation honours `visible: false`.
- **Close and reopen on X11** (`r2/t2-*`, `r2/t3-*`):
  - Close hides the window rather than destroying it (`window/lifecycle.rs` `hide`). Reopen maps
    the same window id with no second `on_window`, so the old `WM_TRANSIENT_FOR` stays in place.
  - Rewriting the properties through a stored `WindowHandle::with_raw_handle` while the window
    is hidden, then calling `open()`, moved the transient to the new parent
    (`transient_for` 0x400000 → 0x600000, `mapped_when_set: false`).
- **A bad xid** (`e2/e2.1-invalid-xid`): `GetGeometry` on the parent fails, and the WM floats a
  dialog-type window with no parent. P4 should check the parent first, then skip the
  transient-for and centre the window on the screen.

### Q3. Do requests carry a usable handle?

Requests went from each requester through xdg-desktop-portal to the stub (`e3/requests.jsonl`,
`e3-chromium/requests.jsonl`, exporters in `e3-exporter/`):

| Requester | Wayland | X11 | Exporter |
| --- | --- | --- | --- |
| GTK 3.24.52 `GtkFileChooserNative` (`GTK_USE_PORTAL=1`) | `wayland:<32 hex>` | `x11:400003` | `zxdg_exporter_v2` |
| GTK 4.24.1 `GtkFileDialog` (`GDK_DEBUG=portals`) | `wayland:<32 hex>` | `x11:600004` | `zxdg_exporter_v2` |
| zenity 4.2.2 `--file-selection` | `""` (no parent window) | not run | none |
| Chromium 155 (`<input type=file>`) | `wayland:<32 hex>` | `x11:0x400003` | `zxdg_exporter_v1` on sway |
| Firefox | pending | pending | expected as GTK 3 |

- Every real `wayland:` handle imported into the Rune child while the requester waited: no
  `destroyed`, and the child floated over the requester (`e3/children/req1–2`,
  `e3-chromium/children/req1`). GTK unexports the handle after the response, so the import must
  happen while the request is open.
- **`wayland:` handles are opaque.** wlroots happens to use 32 hex digits. Mutter uses random
  ids and KWin UUIDs, so P4 must not validate their shape.
- **`x11:` hex format differs.** GTK sends bare hex and Chromium prefixes `0x`, so P4's parser
  must accept both.
- Every request carried the `modal` option (`e3/requests.jsonl`). The portal documents it as
  defaulting to true.
- **Chromium binds whichever exporter the registry announces first** (`xdg_foreign_wrapper.cc`
  `Instantiate`). That's v1 on sway, and v2 on Hyprland, Mutter and KWin, which offer nothing
  else. On a compositor with both, P4 imports a v1 handle through v2. That works on wlroots,
  which shares the registry. A compositor with separate v1 and v2 tables would need a v1
  importer fallback; of the four targets, only sway has v1 at all.
- **GTK 3 below 3.24.44** has only the v1 exporter. The gtk-3-24 NEWS lists "Support the
  xdg_foreign_v2 protocol" under 3.24.44 (24-01-2025); 3.24.43's source lacks it. On a v2-only
  compositor such a GTK sends `""`. That covers Ubuntu 24.04's 3.24.41 on Mutter 46.
- The portal passes `parent_window` through unchanged. Neither `OpenFile`'s options nor the 1.22
  interface XML carry an activation token. Tokens appear only in OpenURI, Email, AppChooser,
  GlobalShortcuts and DynamicLauncher.

### Q4. Fallbacks when parenting fails

The work package offered "centring, or an `xdg_activation` token only". **The token option
doesn't exist.** The FileChooser request carries no token, and the chooser can't mint one: a
Wayland compositor issues tokens only against a recent input serial on a focused surface of the
client. Centring is a no-op on Wayland (winit can't position windows there). So the fallback is
**parentless, plus per-compositor placement**:

| Case | Fallback | Evidence |
| --- | --- | --- |
| Empty handle (`""`) | Parentless | Measured (zenity) |
| `wayland:` handle → `destroyed` | Parentless. Drop the import. | Measured (E1.4) |
| `x11:` handle, chooser on Wayland | Parentless (as Strata does today, `src/ui/chooser.rs:2315`), or an X11 helper process ([decision 2](#2-x11-parents-on-a-wayland-session)) | Measured: both paths work on sway (`e3/children/req4–6`, `e3-chromium/children/req2–3`) |
| `x11:` handle with a bad xid | No transient-for. Centre on the screen. | Measured (E2.1) |
| Parent closes while the chooser is open | Keep the chooser open and parentless. | Measured (E1.6a) |
| Ubuntu 24.04 (libwayland 1.22) | Unsupported. The shim still opens it parentless ([decision 1](#1-ubuntu-2404-and-libwayland-122)) | Source |

How a parentless chooser is placed and focused, per compositor (E4.1, `e4/`):

| Compositor | Placement | Focus |
| --- | --- | --- |
| sway | Tiled beside the focused window | Focused, unless a fullscreen window is on the workspace (measured) |
| Hyprland | Strata's window rule centres it on the monitor | Expected to take focus, as new windows do by default (pending) |
| GNOME | Mutter's placement | Uncertain without a token. Strata also calls the tokenless `focus()`; the hand test before M6 is pending ([decision 3](#3-no-activation-token-and-focus-of-a-parentless-chooser)) |
| KDE | KWin's placement policy | Focus-stealing prevention, Low by default (pending) |

Sway never focuses a view that maps while its workspace has a fullscreen view, **even a
transient one** (`should_focus` in `sway/tree/view.c`). Measured: over a fullscreen parent, the
parented chooser is visible but the parent keeps the seat's focus. On Wayland, client-side focus
(`on_window_focus`) never fired, because headless sway has no keyboard device; sway's seat focus
in the IPC tree is the measure. On Xwayland it did fire (`r2/`).

## API recipe for P4

Steps 1–4 ran in `proto/src/bin/s3-child.rs`. The lifetime rules in step 5 are what the
prototype got wrong or only partly tested; each is marked.

1. **Hook.** Register `App::on_window` on the chooser window's own `App`. For a window added
   beside others, use `WindowSpec::from_app(App::new(spec).on_window(..))`. `on_window` runs on
   the UI thread after every native creation, before the first frame. That includes every
   recreation (Rune destroys Wayland windows on close). Don't use `WindowSpec::on_created`: it's
   `FnOnce` and runs once (`windows.rs:232`, 294; `window/create.rs:232`). Never parent from
   `on_ready`.
2. **Parse the handle.**
   - Trim surrounding whitespace.
   - `wayland:<handle>`: any non-empty rest, kept opaque.
   - `x11:<hex>`: one optional `0x` or `0X`, then 1–8 hex digits that fit a `u32` and aren't 0.
     Reject empty, `0x0x…`, overflow and 0. Don't use `trim_start_matches`, which accepts
     `0x0x`.
   - Anything else, including `""`, means no parent.
   - A kind that doesn't match the window's backend is an outcome to report, not an error.
3. **Wayland.**
   - `WindowHandle::with_raw_handle` gives `RawWindowHandle::Wayland` with the `wl_surface`
     pointer, and nothing else. `WindowHandle` has no display accessor (`window/handle.rs:133`;
     S1 R7).
   - Get the display with `wl_proxy_get_display(surface)`, looked up with `dlopen`
     (`RTLD_NOLOAD`) and `dlsym` at run time. Don't enable wayland-sys's
     `libwayland_client_1_23` feature: features unify, and on libwayland 1.22 winit's own load
     of the library would then fail.
   - `Backend::from_foreign_display`, then a private `EventQueue` via `registry_queue_init`, as
     in Rune's `platform/activation.rs`.
   - Bind `zxdg_importer_v2`, `import_toplevel`, then `set_parent_of` on the surface wrapped with
     `ObjectId::from_ptr`. Destroy the importer. Do one `roundtrip` and read `destroyed`. It
     costs one round trip on the UI thread.
4. **X11.**
   - Open the chooser hidden (`WindowOptions { visible: false }`), on a window whose first
     creation this is (see step 6).
   - In `on_window`, take the xid from `RawWindowHandle::Xlib` or `Xcb`. Open an x11rb
     connection to `$DISPLAY` and check the parent with `GetGeometry`.
   - Set `WM_TRANSIENT_FOR` and `_NET_WM_WINDOW_TYPE` = `_NET_WM_WINDOW_TYPE_DIALOG`. Set
     `_NET_WM_STATE_MODAL` only when the request's `modal` option is true (the default). `sync`,
     then `WindowHandle::show()`.
   - Before reopening a hidden window for another request, rewrite or delete the properties
     through `with_raw_handle` while it is still hidden, then `open()`. Measured in `r2/t3-*`.
     Better still, use a window per request (step 6).
5. **Lifetimes and threads.**
   - The `wl_surface` pointer is valid only inside the `with_raw_handle` callback. Never keep it.
     Wrap it, bind, import and parent inside the callback.
   - The private connection and queue are UI-thread only. Keep them in a UI-thread
     thread-local, not in `Send` state.
   - Use **one private connection and queue per process**, made on first use. The prototype
     makes a new registry on every creation, which leaks a `wl_registry` per reopen.
   - Keep **one import per window, keyed by the window** (its `WindowHandle` key). A new
     creation or request replaces that window's import.
   - `on_close` is a plain `fn(&mut S, &mut Tx)` (`app.rs:372`) and can't capture the shim. So
     the imports live in the shim's thread-local registry, and the app's `on_close` calls a free
     function such as `release(window_key)` to drop that window's import on the UI thread.
   - Anything still bound on winit's display after `run()` returns must be leaked, never
     destroyed. Use a guard the app holds across `run()`: once the loop has ended, its `Drop`
     `mem::forget`s whatever remains in the registry. Measured: destroying segfaults 5 of 5
     times, and leaking exits cleanly 5 of 5 times (`e1-exit/`). The guard type itself is
     unverified.
   - `dispatch_pending` from a Rune update or timer only reports a later `destroyed`. It changes
     nothing: the window simply stays open, parentless.
6. **A window per request.** Open each chooser with `rune::open_window` under a unique key. Its
   first creation honours `visible: false`, its hooks run once for that request, and no stale
   X11 property can carry over. Reusing a window works only with the rewrite in step 4 (X11),
   and on Wayland only because recreation re-runs `on_window`.
7. **Report** the outcome to the caller (see [Open for P4](#open-for-p4) (c)), so Strata can
   log it and apply Hyprland placement.
8. **Optional:** when `modal` is true, `xdg_wm_dialog_v1` `set_modal` on Hyprland, Mutter and
   KWin. sway lacks it. Not prototyped.

## Open for P4

- **(a) Knowing the backend before creation.** winit 0.30 picks Wayland when `WAYLAND_DISPLAY`
  or `WAYLAND_SOCKET` is non-empty, else X11 when `DISPLAY` is non-empty
  (`platform_impl/linux/mod.rs`, the backend match). P4 can apply the same rule, or skip it:
  hidden-first is measured on both backends (Q1, Q2), so P4 can open hidden everywhere.
- **(b) A window per request** (recipe step 6): confirm `rune::open_window` per request suits
  Strata's chooser, and what the key is (the portal request handle is unique).
- **(c) Outcome values.** At least:
  - no handle;
  - kind mismatch (`x11:` on Wayland, `wayland:` on X11);
  - no importer global;
  - no display (libwayland < 1.23);
  - destroyed at import;
  - destroyed later;
  - bad xid;
  - parented.
- **(d) Unsafe code.** The workspace sets `unsafe_code = "deny"` (`Cargo.toml`). `dlsym`,
  `Backend::from_foreign_display` and `ObjectId::from_ptr` are unsafe, so strata-platform needs
  a scoped exception for the foreign-parent module (or for P0's display module), with
  `// SAFETY:` comments.
- **(e) The P0/P4 boundary.** P0 owns display access: the `dlsym` lookup, the one private
  connection and queue per process, the X11 connection, and the after-`run()` leak guard. P4
  owns parsing, the import and `set_parent_of`, the X11 properties, and the outcome. S1 R7 would
  replace P0's lookup.

## Owner decisions

S3 decided none of these; it listed options and a recommendation for each. **The owner ruled
all three on 2026-11-10**, each under its heading below. P4's `policy.rs` holds the code for
decisions 1 and 2.

### 1. Ubuntu 24.04 and libwayland 1.22

The shim's only route to winit's `wl_display` is `wl_proxy_get_display`, which exists from
libwayland 1.23 (2024). Ubuntu 24.04 LTS ships 1.22.0; Fedora 44 and Arch ship 1.26, and NixOS
26.05 ships 1.25. On 24.04 the chooser opens parentless on GNOME. GTK 3 apps there send `""`
anyway, because GTK 3.24.41 can't export to Mutter.

- **a.** Accept parentless on libwayland < 1.23 and report it. 24.04 users stay parentless until
  Rune exposes the display (S1 R7).
- **b.** Hold P4 until Rune adds a display handle or a parent API.
- **c.** Read the display pointer out of libwayland's private `struct wl_proxy` layout.

**Recommendation: a.** It costs one LTS, which Ubuntu 26.04 replaces, and GTK 3 apps on 24.04
are parentless regardless. c depends on a private ABI and should not ship.

**Ruled (2026-11-10): drop support for Ubuntu 24.04.** libwayland 1.23 is the minimum supported
version, for P4 and for every shim that reaches winit's display (P0, P1, P2). The graceful
behaviour stays: on an older libwayland P4 reports `Parentless { NoDisplay }`, and P0, P1 and
P2 keep their existing fallbacks. That is an unsupported environment, not a supported
fallback: it gets no hand test, and whether to remove its code is a separate question.

### 2. `x11:` parents on a Wayland session

Electron, Java, Wine and other Xwayland apps send `x11:`. A Wayland chooser can't be transient
for an X11 window: no protocol exists for it. xdg-desktop-portal-gtk handles this by opening its
dialog on a second, X11 display. winit allows one event loop and one backend per process, so a
Rune chooser can't.

- **a.** Parentless. This is what Strata does today (`src/ui/chooser.rs:2315` ignores `x11:`).
- **b.** For an `x11:` parent, run that request's chooser in a helper process with
  `WAYLAND_DISPLAY` unset, so it opens on Xwayland. Measured on sway with GTK 3 and Chromium:
  it floats as a transient (`e3/children/req6`, `e3-chromium/children/req3`). The costs are a
  second process, X11 input and IME quality, and an Xwayland start on first use.
- **c.** Ask Rune to open one window on Xwayland from a Wayland app. Rune already does this for
  its GNOME overlay (`platform/x11/xwayland.rs`, override-redirect, no keyboard), but a
  focusable dialog is far more work.

**Recommendation: a for the first Rune release** (parity with today), with b recorded as the
upgrade path if users miss it.

**Ruled (2026-11-10): a.** An `x11:` parent on a Wayland chooser stays parentless
(`Reason::Mismatch`). The Xwayland helper process (b) is recorded as the upgrade path.

### 3. No activation token, and focus of a parentless chooser

FileChooser carries no token, so "activation token only" isn't available, and a parentless
chooser is placed and focused by the compositor's policy alone. On sway a new window takes focus
(measured); on Hyprland it is expected to (pending). GNOME may deny focus to a tokenless Wayland
window and show "is ready" instead. That's the case for `""` and `x11:` parents and for Ubuntu
24.04, and it's pending a hand test.

- **a.** Accept compositor policy, and verify GNOME by hand before M6.
- **b.** Also call Rune's tokenless `focus()`. Rune turns it into winit's
  `request_user_attention` (`window/lifecycle.rs:77`, 377), which GNOME is expected to show as
  urgent (pending).
- **c.** Ask xdg-desktop-portal upstream for an activation token in FileChooser requests. That
  is outward-facing and the owner's call.

**Recommendation: a plus b.** Revisit c only if the GNOME hand test shows the chooser opening
behind the app.

**Ruled (2026-11-10): a plus b, with the `focus()` call left to Strata.** The shim calls no
`focus()`; Strata calls Rune's tokenless `focus()` on a parentless chooser. GNOME is verified by
hand before M6.

## Constraints for later WPs

- **P4**
  - Follow the [API recipe](#api-recipe-for-p4) and settle [Open for P4](#open-for-p4).
  - Never call `set_parent_of` on anything but the window's own toplevel surface.
  - Its sway test can't read an xdg parent from IPC. Assert floating and fullscreen visibility
    instead, plus `transient_for` for Xwayland.
  - P4's "Depends on P0 (display access)" holds. The split is (e) above.
- **W3**
  - Arch's `/usr/bin/sway` has `cap_sys_nice=ep`. Under a hard `RLIMIT_RTTIME` of 0 it goes
    realtime and the kernel SIGKILLs it, so start it with `setpriv --no-new-privs`.
  - Use `WLR_RENDERER=gles2` so wgpu gets dmabuf. pixman also runs, for clients that need no
    GPU.
  - Headless sway has no input devices. Clients never bind `wl_pointer` or `wl_keyboard`, so
    clicks need `zwlr_virtual_pointer_v1` (`s3-peer --click X,Y`), and Wayland focus events need
    a virtual keyboard.
  - Create the private `XDG_RUNTIME_DIR` **before** `dbus-run-session`. Otherwise an activated
    service (gvfsd here) inherits the desktop's `/run/user/<uid>`. Set `GIO_USE_VFS=local`.
    Unmount the document portal's `$XDG_RUNTIME_DIR/doc` (`fusermount3 -u -z`) before removing
    the directory.
  - `XDG_DESKTOP_PORTAL_DIR` also makes xdg-desktop-portal read `portals.conf` from that
    directory.
  - Hyprland 0.56 can't nest in sway 1.12: aquamarine needs `xdg_wm_base` v6 and sway offers v5.
    Hyprland has no headless-only mode.
- **S2 / P0**
  - The display handle is S1 R7 ([S1 record](S1-linux-window.md#r7-public-access-to-the-native-display-connection)),
    shared with S2 and S3: file once.
  - Meanwhile `wl_proxy_get_display` via `dlsym` works on libwayland ≥ 1.23. Enabling
    wayland-sys's 1.23 feature would break 1.22 hosts.
  - A private queue on the foreign display needs only `dispatch_pending` from a Rune update to
    see its events. Keep one connection and queue per process.
  - Anything bound on winit's connection must be destroyed on the UI thread before the event
    loop ends, or leaked after it.
- **S5**
  - File S1 R7 once, for S1, S2 and S3.
  - File the [draft below](#draft-rune-issue-for-s5) separately: it asks only for a
    transient-for and pre-map hook, and links R7.

## Draft Rune issue for S5

**Title:** Linux: make a window transient for another app's window, applied before it maps

**Why.** An XDG portal backend (a FileChooser) gets its caller's window as `wayland:<xdg-foreign
handle>` or `x11:<xid>`, and its dialog must be transient for that window: xdg-foreign v2
`set_parent_of` on Wayland, `WM_TRANSIENT_FOR` on X11. Rune has no API for either.

From outside Rune an app can only half do it:
- Reaching winit's Wayland display is a separate gap, filed as R7 (link the R7 issue here).
- Even with the display, the parent has to land before the window maps.
  - On X11, `App::on_window` runs after winit has mapped the window, unless the window was
    created hidden.
  - `WindowOptions { visible: false }` is honoured only on a window's first creation.
    `window/create.rs:13` computes `visible = !launcher && (self.started ||
    self.options.visible)`, so a recreated window maps at once.

**Needs a real compositor.** Headless tooling never creates a native window, and
`WindowSpec::on_created` documents that it isn't called headless (`windows.rs:231`). A headless
sway is enough:

```sh
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
printf 'xwayland force\nexec echo $DISPLAY > %s/display\n' "$XDG_RUNTIME_DIR" > "$XDG_RUNTIME_DIR/sway.conf"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=gles2 \
  setpriv --no-new-privs sway -c "$XDG_RUNTIME_DIR/sway.conf" &
sleep 2
WAYLAND_DISPLAY=wayland-1 cargo run                                         # Wayland
env -u WAYLAND_DISPLAY DISPLAY=$(cat "$XDG_RUNTIME_DIR/display") cargo run  # X11 (Xwayland)
```

`setpriv` stops a `cap_sys_nice` sway from going realtime, where a zero `RLIMIT_RTTIME` would get
it killed. The program opens a hidden window, shows it, closes it (destroying it, because of
`release_gpu_when_closed`), reopens it and quits:

```rust
// Cargo.toml: rune (rev 78d22ed, default-features = false, features = ["embedded-fonts"]), x11rb 0.13
use std::sync::atomic::{AtomicU32, Ordering};
use rune::raw_window_handle::RawWindowHandle;
use rune::{AppSpec, Cx, El, el};

fn view(_: &(), _: &mut Cx<()>) -> El { el() }
static CREATED: AtomicU32 = AtomicU32::new(0);

fn main() {
    let spec = AppSpec::new("transient", (400.0, 300.0), || (), view);
    let hidden = rune::WindowOptions { visible: false, ..Default::default() };
    // release_gpu_when_closed makes close destroy the native window, so open() creates it again.
    let app = rune::App::new(spec).args(Vec::new()).keep_running().release_gpu_when_closed().window_options(hidden)
        .on_window(|w| {
            let n = CREATED.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = w.with_raw_handle(|h| match h.as_raw() {
                // A transient-for needs this surface's xdg_toplevel; there is no Rune API for it.
                RawWindowHandle::Wayland(wl) => println!("creation {n}: wayland surface {:p}", wl.surface),
                RawWindowHandle::Xlib(x) => {
                    use x11rb::protocol::xproto::ConnectionExt;
                    let (c, _) = x11rb::connect(None).unwrap();
                    let a = c.get_window_attributes(x.window as u32).unwrap().reply().unwrap();
                    println!("creation {n}: x11 window {:x}, map_state {:?}", x.window, a.map_state);
                }
                _ => {}
            });
            if n == 1 { w.show(); }
        });
    let handle = app.handle();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        handle.close();
        std::thread::sleep(std::time::Duration::from_millis(500));
        handle.open();
        std::thread::sleep(std::time::Duration::from_secs(1));
        handle.quit();
    });
    app.run();
}
```

**Expected.** A way to open a window transient for a foreign parent, for example
`WindowOptions::transient_for(ForeignParent::parse("wayland:…"))`, applied before the window
maps on every creation, with an outcome saying whether the compositor accepted it. Failing that,
a hook that runs before every native window maps, X11 included.

**Actual** (measured at `78d22ed`):
- Wayland: `creation 1/2: wayland surface 0x…`. Rune offers no transient-for, and
  `WindowHandle` has no display accessor (`window/handle.rs:133`; R7).
- X11: `creation 1: … map_state UNMAPPED`, then `creation 2: … map_state VIEWABLE`. The second
  creation ignored `visible: false` and mapped before `on_window`, so a `WM_TRANSIENT_FOR` set
  there reaches the WM after the map request.

## How to run Tier B

The local strata-qemu-testing goldens can turn the pending cells into measurements. They boot
headless from throwaway overlays, need about 8 GiB of RAM each, and download nothing. Run one
guest at a time.

| Guest | Covers | Notes |
| --- | --- | --- |
| `arch`, `omarchy-4` | Hyprland 0.56.2, Wayland and Xwayland; Chromium on omarchy-4 | `s3-child` runs here too (glibc 2.44) |
| `fedora-workstation` | KWin 6.7.5 Wayland, Firefox 155, GTK 3.24.52 | `s3-peer` only |
| `fedora-workstation-x11` | kwin_x11 6.7.5 | `s3-peer --x11` only |
| `ubuntu-2404` | GNOME 46, libwayland 1.22, GTK 3.24.41, snap Firefox | **Dropped:** Ubuntu 24.04 is unsupported (decision 1). Was decisions 1 and 3; `s3-peer` only |
| `nixos` | GNOME 50.4 | Prebuilt binaries need nix-ld or `patchelf` for the loader |

```sh
cd ~/dev/strata-qemu-testing
mise run vm-run -- fedora-workstation          # headless; prints the SSH port
K=~/.cache/strata-qemu-testing/keys/id_ed25519 P=<port> S=~/dev/strata-spikes/s3
ssh -i $K -p $P tester@127.0.0.1 mkdir -p s3
scp -i $K -P $P -r $S/proto/target/debug/s3-peer $S/requesters $S/qemu/guest-probe.sh tester@127.0.0.1:s3/
ssh -i $K -p $P tester@127.0.0.1 'bash ~/s3/guest-probe.sh kde' | tee $S/results/tierb-fedora-workstation.txt
```

`guest-probe.sh` takes `hyprland`, `gnome` or `kde`. It dumps the foreign globals, parents a raw
child to a peer, tries an invalid handle and a parent that exits, and captures each requester's
`parent_window` with `dbus-monitor`. It prints `hyprctl clients -j` on Hyprland and a KWin
script's `transient`, `transientFor` and `active` on KDE. GNOME has no transient-for IPC, so
judge from the run's QMP screendump.

For Firefox, open it in the guest and send Ctrl+O through the run's QMP socket
(`~/.cache/strata-qemu-testing/runs/<run>/qmp.sock`):

```sh
python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.recv(4096)
for m in [b"{\"execute\":\"qmp_capabilities\"}", b"{\"execute\":\"send-key\",\"arguments\":{\"keys\":[{\"type\":\"qcode\",\"data\":\"ctrl\"},{\"type\":\"qcode\",\"data\":\"o\"}]}}"]:
    s.send(m); print(s.recv(4096))' ~/.cache/strata-qemu-testing/runs/<run>/qmp.sock
```

`guest-probe.sh` has not been run, since Tier B wasn't approved. Expect to adjust it on first
use.

## Not done

- No Hyprland, GNOME or KDE run, and no Firefox: Tier B not approved, and Firefox isn't
  installed here.
- Client-side keyboard focus on Wayland wasn't observable: headless sway has no keyboard device.
- `xdg_wm_dialog_v1` `set_modal` wasn't prototyped: sway lacks it.
- GTK 4 with its X11 backend wasn't run through the X11 helper path. GTK 3 and Chromium were.
- Unverified: the after-`run()` leak guard as a type (only a plain `mem::forget` was run); one
  connection per process; per-window import keys; the `modal: false` branch; the strict `x11:`
  parser. The prototype doesn't implement these.
