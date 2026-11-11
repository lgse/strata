# S1 Linux window baseline

| | |
| --- | --- |
| Work package | [S1](../work/phase-0-groundwork/S1-linux-window-baseline.md) |
| Rune | `78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5`, winit 0.30.13, wgpu 30.0.1 |
| Ran here | sway 1.12 / wlroots 0.20.1 headless, Xwayland 24.1.14 inside it, Xvfb 21.1.25, xdg-desktop-portal 1.22.1 with xdg-desktop-portal-gtk 1.15.3, NVIDIA RTX 5090 (driver 615.78.08), Mesa 26.2.5 without lavapipe |
| Hand test pending | Hyprland 0.56.2, GNOME (mutter, Wayland and X11), KDE, lavapipe on Strata's QEMU images |
| Prototype | `/home/wmfeht/dev/strata-spikes/s1` (never merged). Evidence paths below are relative to its `out/` directory. |

## Verdicts

- **P3 is blocked on Rune on Wayland, and "shim here" on X11 only if a hand test passes.**
  Rune's public API reaches winit's `drag_window` (through `.window_drag()`) and
  `set_maximized` (through `Tx::toggle_maximized`). It doesn't reach `drag_resize_window`,
  `show_window_menu` or, for a borderless window, `set_minimized`.
  `WindowHandle::with_raw_handle` gives only a raw *window* handle: an X11 id, or the
  `wl_surface` pointer. On X11 a second connection can send `_NET_WM_MOVERESIZE` with that id,
  and it resized the window under Xwayland. Real X window managers are expected to refuse it,
  because winit's connection holds the press's pointer grab (Q2), so the X11 shim depends on a
  hand test. On Wayland the raw handle isn't enough: resizing needs winit's own `xdg_toplevel`
  and the seat's button serial. winit itself resizes fine on both, so a Rune API would be a thin
  wrapper (repro R1).
- **P5 starts from `color-scheme` only.** Rune reads `org.freedesktop.appearance color-scheme`
  from the Settings portal and follows `SettingChanged` within a few milliseconds. It reads no
  other portal key. It reads reduced motion from GSettings or `kdeglobals` directly, only when a
  window is created or focused. On Hyprland, the Settings backend is xdg-desktop-portal-gtk,
  which offers every key P5 needs (simulated under sway with `XDG_CURRENT_DESKTOP=Hyprland`;
  real session pending). That follows from `/usr/share/xdg-desktop-portal/hyprland-portals.conf`
  (`default=hyprland;gtk`) and `/usr/share/xdg-desktop-portal/portals/hyprland.portal`, whose
  interfaces don't include Settings.
- **Menus scroll, submenus clip** (by source; measured headless and on sway). rune-kit menus and
  dropdowns shrink to the window and scroll. Submenus flip sides and shift vertically, but never
  shrink or scroll. A window narrower than about three menu widths can push a submenu off its
  side, and one shorter than the tallest submenu pushes it off the bottom. Tiling compositors
  also give a window less than its minimum size.
- **The double-click interval is a fixed 0.5 s on Linux**, on every compositor (by source;
  measured headless, on sway and through Xwayland). The desktop's setting (400 ms by default)
  is ignored, and an app can't change it.

## Method

Every run used a private headless sway inside a private D-Bus session, set up by the
prototype's `harness/lib.sh`. Its setup and traps are listed under W3 in
[Constraints for later WPs](#constraints-for-later-wps). Nothing connected to the owner's
Hyprland session. The window under test is `s1-window`, a plain Rune app that logs layout,
scale, focus, appearance, presses and window errors as JSON lines. `s1-winit` is winit 0.30.13
with Rune's features and no Rune. `s1-probe` reports Wayland globals, decoration modes and
`wm_capabilities`, and drives a virtual pointer.

Hyprland could not run here. It has no headless backend, and nested inside sway it aborts:
`invalid version for global xdg_wm_base (13): expected at most 5, got 6`. Aquamarine needs
`xdg_wm_base` v6 and sway 1.12 offers v5. Its cells give the expectation from Hyprland 0.56.2's
source (`src/protocols/XDGDecoration.cpp`, `XDGShell.cpp`). GNOME, KDE and mutter on X11 aren't
installed; their cells give the expectation from their documented protocol support, unverified.

## Q1 Open, resize, focus, fractional scale, GPU

| Compositor | Result | Evidence |
| --- | --- | --- |
| sway, Wayland, scale 1, 1.25, 1.5, 2 | Opens in about 0.55 s and reports the output scale. Follows `resize set` and tiling. Focus in and out follow sway. At 1.5 the live window is **pixel-identical** to Rune's headless `--snap --scale 1.5` (0 of 777,600 pixels differ; at 1, 0 of 345,600). The adapter is the RTX 5090, picked as "the compositor's GPU" from dmabuf feedback. | `q1/wayland-*/win.log`, `initial.png`, `live-crop.png`, `headless-snap.png` |
| sway with the pixman renderer (no dmabuf) | **No window**: `no GPU adapter: … vulkan not compatible with provided surface … gl support not compiled in`. | `q1/wayland-pixman/win.log` |
| sway, no Vulkan ICD (`VK_ICD_FILENAMES=/nonexistent.json`) | **No window**: `Failed to create surface for any enabled backend`. `AppHandle::window_error` reports it. | `q1/no-vulkan/win.log` |
| X11 through Xwayland, scale 1 | Opens, resizes and draws. Each refocus arrives as a focus-out and focus-in pair in the same millisecond. Adapter picked as "lowest power". | `q1/xwayland-1/` |
| X11 through Xwayland, sway scale 1.5 | Rune sees scale 1.0, and sway upscales the window (Xwayland isn't HiDPI-aware). With `WINIT_X11_SCALE_FACTOR=1.5` Rune sees 1.5, but sway's logical sizes no longer match: a 900×574 window lays out at 597×381 points. | `q1/xwayland-1.5*/` |
| X11 on Xvfb, no window manager | Opens and draws (NVIDIA Vulkan presents to Xvfb). Never focused. | `q1/xvfb/client.log` |
| Hyprland | Hand test pending. Expected to behave like sway: same protocols (fractional scale, viewporter, dmabuf feedback). Rune's adapter picker already handles Hyprland on NVIDIA (`gpu/pick.rs`). | — |
| GNOME, KDE | Hand test pending. Expected to work; both offer fractional scale. | — |
| lavapipe (QEMU images) | Hand test pending. Strata-side: no images here, and lavapipe isn't installed. | — |

Notes:
- **Rune's default build has no GL.** Rune asks wgpu for `Backends::all()` (`render.rs:560`), but
  on Linux it compiles only wgpu's `vulkan` feature, not `gles`. With that build, a machine
  without a Vulkan driver gets no window, and lavapipe (`mesa-vulkan-drivers`, `vulkan-swrast`)
  is the only software fallback. An app that enables `wgpu/gles` itself might add a GL fallback;
  that is untested.
- **When the compositor goes away,** `run` panics with `run: Exit Failure: 1` by design: it
  wraps `try_run` and panics on its error (`app.rs:500-507`). `App::try_run` returns
  `Err("run: Exit Failure: 1")` instead, with no panic (`r8/sway/r8.log`, prototype example
  `r8`). Strata should call `try_run`.

## Q2 Decorations off: move, resize, minimise, maximise, window menu

Each case is a borderless window driven by a virtual pointer and sway's rect read back
(`q2/*/steps.txt`).

| Action | Rune, sway Wayland | Rune, Xwayland | raw winit, sway Wayland | raw winit, Xwayland |
| --- | --- | --- | --- | --- |
| Move by dragging the header | Works (`.window_drag()`) | Works | Works (`drag_window`) | Works |
| Resize from an edge or corner | **No API.** The shim can't get `xdg_toplevel` from the raw handle. | Works through the shim: `_NET_WM_MOVERESIZE` from a second X connection | Works (`drag_resize_window`, E, W, SE) | Works |
| Maximise (`Tx::toggle_maximized`, header double-click) | Requested; sway ignores it (no maximize capability) | Requested; ignored | Requested; ignored | Requested; ignored |
| Minimise | **`Tx::minimize` is ignored for borderless windows** (`rune: minimize ignored`, `window/shell_requests.rs:20`) | Same. The shim's `WM_CHANGE_STATE` is requested; sway ignores it and the window stays visible. | Requested; sway can't minimise | Requested; ignored |
| Window menu | No Linux path in Rune (`window/chrome.rs:95`, Windows only) | The shim's `_GTK_SHOW_WINDOW_MENU` is requested; wlroots xwm doesn't support it | `show_window_menu` requested; sway has no window menu | winit's X11 `show_window_menu` is a no-op |

What each compositor offers, from `xdg_toplevel.wm_capabilities` and `_NET_SUPPORTED`:

| Compositor | Move and resize | Maximise | Minimise | Window menu | Status |
| --- | --- | --- | --- | --- | --- |
| sway | yes | no (`wm_capabilities` = fullscreen) | no | no | Ran here (`q3/sway/probe.json`) |
| wlroots xwm (Xwayland in sway) | `_NET_WM_MOVERESIZE` | `_NET_WM_STATE_MAXIMIZED_*` | `_NET_WM_STATE_HIDDEN` advertised, but a `WM_CHANGE_STATE` iconify request was ignored | no `_GTK_SHOW_WINDOW_MENU` | Ran here (`q3/sway/x11-root.txt`, `q2/rune-xwayland/node-after-x11min.json`) |
| Hyprland | yes, with a valid button serial | yes (`wm_capabilities` = fullscreen, maximize) | no (a minimise request only emits an event) | no handler | Hand test pending; from source |
| GNOME Wayland (mutter) | yes | yes | yes | yes | Hand test pending; from documented behaviour |
| GNOME X11 (mutter), KWin on X11, other X window managers | `_NET_WM_MOVERESIZE` | yes | yes | `_GTK_SHOW_WINDOW_MENU` (mutter) | Hand test pending. **The shim is expected to fail here.** The EWMH spec says the client must release all its grabs before sending `_NET_WM_MOVERESIZE`, because the window manager then grabs the pointer itself. winit does that on its own connection (`ungrab_pointer`, `platform_impl/linux/x11/window.rs:1699`). A second connection can't release winit's implicit grab from the press, so the window manager's grab is expected to fail. wlroots xwm takes input from the compositor and doesn't grab, which is why Xwayland can't show the problem. |
| KDE | yes | yes | yes | yes | Hand test pending; from documented behaviour |

Also found:
- Rune's `Cursor` has `ResizeColumn` and `ResizeRow` but no diagonal resize shapes. The prototype
  used `Cursor::Move` on its corners. `Cursor::Image` with the app's own PNG images is the interim
  fallback, at the cost of not following the cursor theme.
- **No window-state query.** Rune exposes no maximised state (only `Tx::toggle_maximized` and the
  `App::maximized` builder), and `Cx::chrome_insets()` is 0 on Linux. P3 can't tell whether to
  show maximise or restore, or whether to hide its resize bands on a maximised window. Part of
  R1 and R3.
- **The header double-click ignores the desktop's titlebar action.** Off macOS, Rune always
  toggles maximise (`window/chrome.rs:72-77`). GNOME's
  `org.gnome.desktop.wm.preferences action-double-click-titlebar` (minimise, shade, menu,
  none) isn't read.
- **Pointer state stays sane after the window manager takes a drag.** Under Xwayland, after two
  resizes run by the window manager, the next five clicks all reached Rune
  (`q2/rune-xwayland/win.log`). The same held after `.window_drag()` moves on Wayland.
- A `.window_drag()` region must have no press handlers of its own (`bare_drag_region`), and its
  double press always toggles maximise with Rune's fixed 0.5 s interval. P3 can't apply P5's
  interval to the header without giving up `.window_drag()`, and on Wayland that is the only way
  to start a move.
- `on_drag` `Phase::Down` runs while the button is still held, early enough for the X11 shim.

## Q3 Server-side decorations (`xdg-decoration`)

| Compositor | Offers server-side? | Rune decorated window | Rune borderless window | Status |
| --- | --- | --- | --- | --- |
| sway | Yes: `zxdg_decoration_manager_v1` and `org_kde_kwin_server_decoration_manager`. An unmapped toplevel gets `server_side` first, whatever it asks for. Once mapped, sway honours the request. | sway titlebar (`border normal`, 26 px) | No frame (`border csd`) | Ran here (`q3/sway/*.node.json`, `*.png`) |
| Xwayland in sway | `_MOTIF_WM_HINTS` honoured | sway titlebar | No frame | Ran here |
| Hyprland | Always `server_side`, for any request or unset (`xdgModeOnRequestCSD` returns the default) | No titlebar: Hyprland draws only its border, so winit draws no frame either | Same as decorated | Hand test pending; from source |
| GNOME (mutter) | No `xdg-decoration` | winit draws its sctk-adwaita frame with no title text (Rune's `wayland-csd-adwaita-notitle` feature) | No frame and no resize edges | Hand test pending; from documented behaviour |
| KDE | Yes; prefers server-side and honours a client-side request | Breeze titlebar | No frame | Hand test pending; from documented behaviour |

winit's sctk-adwaita frame also reads `org.gnome.desktop.wm.preferences button-layout` from the
portal (seen in `q4/fake-v2/portal.log`).

## Q4 The Settings portal today

| Case | Result | Evidence |
| --- | --- | --- |
| Fake portal, version 2 | Rune calls `ReadOne(org.freedesktop.appearance, color-scheme)` once at startup, then follows `SettingChanged`: light→dark and dark→light each reach the view **1 ms** after the signal, including while a fullscreen window covers it. `text-scaling-factor` changes are ignored. `color-scheme = 0` (no preference) leaves the last value in place (`window/events.rs:209`). | `q4/fake-v2/` |
| Fake portal, version 1 (`Read` only) | Rune falls back from `ReadOne` to `Read` and unwraps its double variant correctly. Same behaviour. | `q4/fake-v1-read-only/` |
| No portal | Dark | `q1/*/win.log` (no portal on the bus) |
| Real xdg-desktop-portal with xdg-desktop-portal-gtk, as on Hyprland (`XDG_CURRENT_DESKTOP=Hyprland`) | Version 2. `ReadAll` gives every P5 key: `org.gnome.desktop.interface` `text-scaling-factor`, `cursor-blink`, `cursor-blink-time`, `enable-animations`, `color-scheme`; `org.gnome.desktop.peripherals.mouse` `double-click` and `drag-threshold`; `org.freedesktop.appearance` `color-scheme` and `contrast`. Each `gsettings set` raised `SettingChanged`. Rune followed `prefer-light` in about 12 ms. | `q4/real-gtk/readall.txt`, `monitor.log`, `win.log` |
| Reduced motion | Read from GSettings (gio loaded with `dlopen`) or `kdeglobals`, not from the portal; `true` at startup with `enable-animations=false`. Rune re-reads it on each `Focused` event (`window/events.rs:366`). Here the re-read stayed stale, because the keyfile backend needs a GLib main loop that Rune doesn't run. Whether dconf gives a fresh value on refocus is hand test pending. | `q4/real-gtk/win.log` |
| GNOME (xdg-desktop-portal-gnome), Hyprland's real session | Hand test pending | — |

- **Side effect for W3 and P9:** loading gio inside the app (Rune's reduced-motion read) lets GIO
  spawn `gvfsd` on the session bus and mount a FUSE file system under `XDG_RUNTIME_DIR`. The
  harness sets `GIO_USE_VFS=local`.

## Q5 In-window menus against a small window

Headless `--tree` and `--snap` of a Strata-sized context menu: 16 rows, 160 pt wide, 463 pt of
rows, an 8-row "Open With" submenu, and a 24-row dropdown. Five window sizes and pointer anchors
at the corners and middle (`q5/summary.json`, `q5/*.png`).

| Window | Context menu | Submenu | Dropdown, 24 rows |
| --- | --- | --- | --- |
| 800×600 | Fits from the top; from the middle it scrolls (441 of 463 pt) | Fits | Scrolls (551 of 672) |
| 640×480 | Scrolls from anywhere below the top | Fits | Scrolls |
| 420×400 (Rune's default minimum) | Scrolls (331 of 463 from the top) | **Off the right edge by 28 pt** for a press at x = 130 | Scrolls (351) |
| 360×320 | Scrolls | Off by 88 pt (press at x = 130) | Scrolls |
| 320×240 | Scrolls (171 of 463) | Off by 18 to 150 pt from every press tested | Scrolls (191) |

- Menus above or below their anchor use `anchored_fit`: they flip, shift, keep 8 pt inside the
  window, and scroll their rows. Nothing clips vertically. Presses at the bottom-right flip
  upward.
- Submenus sit beside their row. Horizontally they only flip. Vertically they shift to stay
  8 pt inside (`runtime/anchor.rs:51` clamps the cross axis): in `q5/summary.json` the submenu's
  top moves from 84 or 90 to 8 in the 320×240 window. They never shrink or scroll, so a submenu
  taller than the window runs off the bottom. In the R6 program, a 20-row submenu (564 pt) in
  a 420×400 window starts at y = 8 and runs 172 pt past the bottom.
- A submenu has somewhere to go for every press only when the window is at least
  `3 × menu width + 16` pt wide: 496 pt for 160 pt menus. By that rule, at 420 pt, presses
  between x = 92 and 168 push it off-window; x = 130 was tested.
- Tiling ignores the minimum size. In sway with four tiled windows, the Rune window got 316×772
  pt at scale 1 and 209×505 pt at scale 1.5, below its 420×400 minimum. The open menu stayed
  inside (`q5/live-*/`). Hyprland tiles the same way (hand test pending).
- `tx.popup_menu` (native menus) is unsupported on Linux and answers `None`.
- Folder peek (C22) and other in-window popovers anchored beside an element follow the same
  rules as submenus: they flip and shift but don't shrink.

**Chooser minimum until Rune has popup surfaces:**
- **Width:** at least `3 × W + 16` pt, where W is the widest menu that has a submenu. Required.
- **Height, submenus:** at least the tallest submenu plus 16 pt. Required, because submenus
  can't scroll.
- **Height, menus:** the longest context menu plus about 16 pt, if it shouldn't scroll.
  Optional, because menus scroll.

For the 160 pt menus above, with 8-row submenus (228 pt), that is 496 × 244 required and
496 × 480 to avoid scrolling. Floating windows honour the minimum; tiled ones don't.

## Q6 Double-click interval

| Case | Result | Evidence |
| --- | --- | --- |
| Headless script, presses 0.30, 0.45, 0.55, 0.70 s apart | Pairs at 0.30 and 0.45; doesn't at 0.55 and 0.70 | `q6/headless-*.log` |
| sway Wayland, desktop set to 1200 ms in GSettings and the portal | Pairs at 346 and 484 ms; not at 583, 732 or 1033 ms | `q6/wayland/` |
| Xwayland, same settings | Pairs at 363 and 483 ms; not at 582, 732 or 1033 ms | `q6/x11/` |
| Hyprland, GNOME, KDE | Hand test pending. Expected the same: the value is a constant. | — |

- `Runtime::double_click_interval` defaults to 0.5 (`runtime.rs:1073`). macOS and Windows replace
  it from the system when a window is created (`window/create.rs:86-94`); Linux never does.
- The field is public, but a windowed app only ever gets `&Runtime` (`App::on_frame_drawn`), so
  it can't change it.
- It drives `on_double_press`, `.window_drag()` maximise, word and line selection, and
  `Drag::clicks`.
- GNOME's and GTK's default is 400 ms (`org.gnome.desktop.peripherals.mouse double-click`), so
  Rune on Linux pairs slower clicks than Strata on GTK did.

## Open questions for the owner and P3

1. **P3 fallback (not decided).** The options:
   - wait for Rune;
   - borderless on X11 only;
   - an X11-only P3.

   **Recommendation:** start P3 now with its platform-neutral part, and keep the native frame
   on Wayland until Rune ships R1–R3. The platform-neutral part is the edge-band hit test and
   direction state machine, the button handlers, the header with `.window_drag()`, and the menu
   trigger, all testable headless. Borderless on X11 depends on the GNOME-on-Xorg hand test
   (question 3): only if a real window manager accepts `_NET_WM_MOVERESIZE`, `WM_CHANGE_STATE`
   and `_GTK_SHOW_WINDOW_MENU` from P3's own x11rb connection does P3 ship that X11 backend now.
   Otherwise X11 keeps the native frame too.

   P3's unblockers in Rune are R1 (resize, cursors, window state), R2 (minimise), R3 (window
   menu) and R7 (the display handle). With R7, an X11 shim could send its requests on winit's
   own connection and release the grab first, as winit does. Once Rune exposes the calls, the
   Wayland backend is a small switch.

   Evidence:
   - On Hyprland the native frame costs nothing: Hyprland always answers `server_side` and
     draws no titlebar, so a decorated Rune window there looks borderless. Hyprland tiles it
     and resizes it through its own bindings.
   - On sway and KDE the native frame is a server titlebar above Strata's header.
   - On GNOME Wayland it is winit's adwaita frame with no title.
   - The cost of the alternatives: borderless on GNOME Wayland today gives a window that can't
     be resized, minimised or given its menu. Waiting blocks P3 on an upstream with no date.
2. **P3's Done-when** ("on Hyprland, GNOME and X11") can't be met on GNOME Wayland until Rune
   ships R1–R3. Should it be narrowed to Hyprland (decorated, which draws no titlebar there) and
   X11, with GNOME Wayland deferred?
3. **The X11 grab question.** The shim's second X connection worked under wlroots xwm, which
   doesn't grab. On mutter, KWin and similar window managers it is expected to fail: EWMH
   requires the client to release its grabs before `_NET_WM_MOVERESIZE`, and only winit's own
   connection can release the press's implicit grab (`x11/window.rs:1699`). A GNOME-on-Xorg hand
   test (`hand/session.sh gnome-x11 --x11`) settles it. If it fails as expected, the X11 backend
   also needs Rune (R1, or R7 for the connection).
4. **Hand tests.** Hyprland, GNOME, KDE and lavapipe results depend on the owner running
   `hand/session.sh` in the prototype (see its `README.md`). The spike never connected to the
   owner's session.

## Constraints for later WPs

Recommended changes. None of these files was edited here.

- **P3 (`P3-client-side-decorations.md`):**
  - The scope says resize edges go "through winit `drag_resize_window`". Rune doesn't expose
    that, so the scope needs R1, or the X11 shim plus the Wayland native frame.
  - Add to the scope:
    - minimise through `Tx::minimize` is ignored for borderless windows (R2);
    - Linux has no window-menu path (R3);
    - there are no diagonal resize cursors (R1; `Cursor::Image` meanwhile);
    - there is no maximised-state query, and `Cx::chrome_insets()` is 0 on Linux, so the
      maximise/restore button and the resize bands can't follow the window state (R1, R3);
    - the header double-click uses Rune's fixed 0.5 s, not P5's interval (R4), and always
      maximises, ignoring GNOME's `action-double-click-titlebar`.
  - The X11 shim on a second connection is expected to fail on real X window managers (Q2). Its
    unblockers are R1, R2, R3 and R7.
  - The W3 test "resizes from each edge under sway" can only target Xwayland (through the shim,
    which works there) or raw winit until Rune ships R1.
  - Under W3, input reaches clients only through a virtual pointer and a virtual keyboard (see
    W3 below).
- **P5 (`P5-desktop-settings.md`):**
  - On Hyprland the Settings backend is xdg-desktop-portal-gtk; xdph has no Settings interface.
    The Done-when should name it.
  - The double-click key lives in the `org.gnome.desktop.peripherals.mouse` namespace.
  - Rune's own reduced-motion read is separate from the portal: GSettings or `kdeglobals`,
    re-read on focus. P5's animations value should come from the portal. The integration point
    is `Cx::reduced_motion(on)` (`runtime/cx.rs:236`): the view calls it on every commit with
    the portal's `enable-animations`, which also works around Rune's stale focus-time read. The
    override lasts one commit, so it must be set every time the view runs.
  - Read `color-scheme = 0` as "no preference", not as light or dark.
  - `harness/fake_portal.py` in the prototype is a ready model for the fake portal under W3.
- **W3 (`W3-compositor-harness.md`):**
  - Run sway with `WLR_BACKENDS=headless`, `WLR_LIBINPUT_NO_DEVICES=1`, a private
    `XDG_RUNTIME_DIR` and `env -i`, inside `dbus-run-session`, and start clients through
    `swaymsg exec` so they see only the private display and bus.
  - Start sway as `setpriv --no-new-privs sway`. `/usr/bin/sway` has `cap_sys_nice` and goes
    realtime, and where `RLIMIT_RTTIME` is 0 the kernel kills it at once (exit 137).
  - Set `swaybg_command -`.
  - With `WLR_LIBINPUT_NO_DEVICES=1` the seat has no pointer or keyboard. `swaymsg seat … cursor`
    moves sway's cursor but sends clients nothing, and no client ever gets focus. Keep a
    virtual keyboard alive (`wtype -M shift -m shift -s MS`; keep Shift released, since a held
    Shift makes sway keep the aspect ratio on resize), and drive the pointer through
    `zwlr_virtual_pointer_v1`.
  - Run clients via `swaymsg exec` from a quoted launcher script: `;` splits sway commands.
  - Xwayland takes the next free display in the shared `/tmp/.X11-unix`, not a private one.
  - Set `GIO_USE_VFS=local`, and unmount FUSE leftovers under the private runtime dir on
    teardown.
  - Use the gles2 renderer: pixman offers no dmabuf, and the NVIDIA Vulkan driver then makes
    no surface.
  - Hyprland can't nest inside sway 1.12 (needs `xdg_wm_base` v6).
- **S5 (`S5-upstream-and-release.md`):** file R1–R7 below. Beyond the list S5 has, that adds
  borderless minimise (R2), the window menu (R3) and access to the display handle (R7; shared
  with S2 and S3, file once). Diagonal cursors, window state and `wm_capabilities` are folded
  into R1 and R3. The panic on disconnect was dropped: `try_run` returns the error.
- **plan.md (owner's own PR):**
  - §4 P3: replace the `drag_resize_window` premise with the Wayland/X11 split above.
  - §8: Rune's default build has no GL backend (it doesn't enable wgpu's `gles` feature), so
    lavapipe is the only fallback unless the app enables `wgpu/gles` itself, which is untested.
    Say so instead of "Vulkan or GL".
  - §4 tier 1 intro: shims can't bind on "winit's own Wayland display" through Rune's public
    API (next item).
- **S2, P0, P2, P4 (display handle):** Rune exposes no display handle: `with_raw_handle` gives
  the window handle only, and `rune::raw_window_handle` re-exports types, not access. On
  Wayland, the `wl_display` can be recovered from the surface pointer with
  `wl_proxy_get_display` (libwayland 1.23 or later; Ubuntu 24.04 ships 1.22). That needs unsafe
  FFI, so a lint exception in strata-platform. This spike didn't exercise it. On X11 a shim opens
  its own connection; that worked for client messages here. R7 asks Rune for the handle.

## Appendix: Rune issue drafts for S5

Common setup: a binary crate depending on `rune` at `78d22ed` with features `accessibility`,
`dbus`, `embedded-fonts` and `svg` (R6 also needs `rune-kit` at the same rev). The R1, R2, R3, R4
and R6 programs compile against that rev (prototype `crates/s1-window/examples/`). R4 and R6
reproduce with Rune's headless tooling alone. "Compositor" below means any Wayland or X11
session: a private headless sway with a virtual pointer is enough (prototype `harness/`); a
desktop isn't needed.

### R1 Linux: no way to start an interactive resize from a borderless window

Needs a compositor. Steps:
1. Run the program below in headless sway, floating.
2. With a virtual pointer, press 3 px inside the right edge, move 100 px right, and release.

Expected: the window grows by 100 px, as winit's `Window::drag_resize_window(East)` does (verified
with plain winit 0.30.13 under the same harness, Wayland and Xwayland).

Actual: nothing can start the resize. `WindowHandle::with_raw_handle` yields the `wl_surface`
only, and `xdg_toplevel.resize` needs winit's toplevel and the button serial.

Ask:
- `.window_resize(ResizeDirection)` (like `.window_drag()`) or `Tx::drag_resize_window`;
- `Cursor::Resize{N,S,E,W,NE,NW,SE,SW}`;
- the window's maximised state for the view (none today; `Cx::chrome_insets()` is 0 on Linux),
  so resize bands and the maximise/restore button can follow it.

```rust
use rune::*;
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(row().h(40.0).w_full().window_drag().child(text("drag me")))
        .child(el().pin(None, Some(0.0), Some(0.0), Some(0.0)).w(6.0).cursor(Cursor::ResizeColumn)
            .on_drag(|_: &mut S, d: &mut Drag, _: &mut Tx| {
                if d.phase == Phase::Down { /* nothing public reaches drag_resize_window */ }
            }))
}
fn main() {
    App::new(AppSpec::new("R1", (640.0, 400.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

### R2 `Tx::minimize` is ignored for borderless windows

Needs a compositor (any; the message comes before any request reaches it). Steps: run the program
below as a debug build and click "Minimise".

Expected: the window minimises where the compositor supports it (GNOME, KDE, X11 window
managers). Those are exactly the windows that draw their own minimise button.

Actual: `rune: minimize ignored: a window without a minimize button can't minimize`, and no
request. The check is `self.options.borderless || …` in `window/shell_requests.rs:20`. Seen
under headless sway and Xwayland (prototype `out/q2/rune-*/win.log`).

```rust
use rune::*;
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(row().h(40.0).w_full().window_drag().child(text("borderless")))
        .child(row().key("min").w(120.0).h(32.0).role("button").label("Minimise").child(text("Minimise"))
            .on_press_tx(|_: &mut S, tx: &mut Tx| tx.minimize()))
}
fn main() {
    App::new(AppSpec::new("R2", (400.0, 240.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

### R3 No window menu on Linux

Needs a compositor that has a window menu (GNOME, KDE; sway and Hyprland have none) to see the
expected result; the missing call is visible in the code. Steps: run the program below and
right-click the header.

Expected: `xdg_toplevel.show_window_menu` on Wayland, and `_GTK_SHOW_WINDOW_MENU` on X11 where
the window manager supports it, as Rune already does on Windows.

Actual: nothing. `take_window_menu` is only set when a Windows caption exists, and
`chrome::window_menu` is a no-op off Windows. winit's X11 `show_window_menu` is also a no-op.

Ask:
- the window menu on Linux;
- `xdg_toplevel.wm_capabilities` exposed (sway advertises only fullscreen; Hyprland
  fullscreen and maximize), so apps can hide buttons the compositor can't honour;
- the maximised state (as in R1);
- optionally, the desktop's titlebar double-click action
  (`org.gnome.desktop.wm.preferences action-double-click-titlebar`) instead of always
  maximising (`window/chrome.rs:72-77`).

```rust
use rune::*;
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    // A bare drag region: on Windows a secondary click here opens the window menu.
    col().w_full().h_full()
        .child(row().key("header").h(40.0).w_full().window_drag().child(text("right-click me")))
}
fn main() {
    App::new(AppSpec::new("R3", (400.0, 240.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

### R4 The double-click interval is fixed at 0.5 s on Linux

Headless; no compositor needed. Steps: run the program below with `--script dbl.json --tree
out.json --at 3`, once with `gap` 0.45 and once with 0.55, and read the `doubles` text. A live
check under headless sway with GNOME's `double-click` set to 1200 ms gives the same result.

Expected: the desktop's interval, or at least a way for the app to set it. GNOME and GTK default
to 400 ms (`org.gnome.desktop.peripherals.mouse double-click`, also in the Settings portal), and
KDE stores `DoubleClickInterval` in `kdeglobals` (also 400 ms by default).

Actual: 0.45 pairs (`doubles 1`) and 0.55 doesn't (`doubles 0`). Under sway, 583–1033 ms never
pair with the desktop at 1200 ms. `Runtime::double_click_interval` is public, but windowed apps
never get `&mut Runtime`.

```rust
use rune::*;
#[derive(Default)] struct S { doubles: u32 }
fn view(s: &S, _: &mut Cx<S>) -> El {
    row().key("dbl").w(200.0).h(60.0).child(text(format!("doubles {}", s.doubles)))
        .on_press(|_: &mut S| {}).on_double_press(|s: &mut S| s.doubles += 1)
}
fn main() { App::new(AppSpec::new("R4", (400.0, 200.0), S::default, view)).run(); }
// dbl.json: {"events": [{"t": 0.5, "type": "double-click", "key": "dbl", "gap": 0.55}]}
```

### R5 Settings portal keys beyond `color-scheme`

Needs a compositor and a private session bus, because Rune reads the portal only for a real
window. Steps: run any Rune app (R4's will do) on a private bus where a fake
`org.freedesktop.portal.Settings` emits changes to `text-scaling-factor`, `cursor-blink`,
`cursor-blink-time` and `enable-animations` (`org.gnome.desktop.interface`) and to
`double-click` (`org.gnome.desktop.peripherals.mouse`). The fake portal must be attached to the
issue: prototype `harness/fake_portal.py`, about 100 lines of Python with GLib's Gio, which logs
every call it receives.

Expected: these reach the app, as `color-scheme` does.

Actual: the fake portal's log shows one call from Rune, `ReadOne(org.freedesktop.appearance,
color-scheme)`, and nothing else; no API exposes the other keys. Rune reads reduced motion from
GSettings or `kdeglobals` only when a window is created or focused, which skips the portal
(sandboxes) and change signals.

### R6 Popup surfaces for menus

Already on S5's list; this adds the repro. Headless; no compositor needed. Steps: run the program
below with `--tree t.json --size 420x400 --at 1`, then again with `--long` added. Read the
`ctx-item1-sub-menu` rect.

Expected: the submenu stays visible, as an `xdg_popup` would.

Actual:
- With 8 rows it sits at `[288, 90, 160, 228]`: 28 pt past the right edge of a window at Rune's
  own default minimum size.
- With 20 rows it shifts up to `[288, 8, 160, 564]` and still runs 172 pt past the bottom.

Submenus flip and shift but never shrink or scroll, and `tx.popup_menu` is unsupported on
Linux.

```rust
use rune::*;
use rune_kit::{Kit, MenuEntry, MenuItem, context_menu};
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    let k = Kit::neutral(true);
    let rows = if rune::tooling::extra_args().iter().any(|a| a == "--long") { 20 } else { 8 };
    let sub = (0..rows).map(|i| MenuEntry::item(MenuItem::new(format!("App {i}")))).collect();
    let mut layer = Vec::new();
    let trigger = context_menu("ctx", el(), Some(PointerPress { x: 130.0, y: 60.0 }))
        .item(MenuItem::new("Open"))
        .item(MenuItem::new("Open With").submenu(sub, true))
        .item(MenuItem::new("Copy"))
        .build(&k, &mut layer);
    rune_kit::root(col().w_full().h_full().fill(k.c.bg_base).child(trigger), layer)
}
fn main() { App::new(AppSpec::new("R6", (420.0, 400.0), || S, view)).run(); }
```

### R7 Public access to the native display connection

Shared with S2 and S3 (display handle); file once. No repro program needed: it's a missing API,
visible in `window/handle.rs:133`. Steps: an app's `App::on_window` hook wants to bind a Wayland
protocol on winit's display (`xdg_foreign`, data-device, `xdg_activation`), as Rune's own
`platform/activation.rs` does, or to send X11 requests on winit's connection.

Expected: something like `WindowHandle::with_raw_display_handle`.

Actual: only the window handle is available.
- On Wayland the display can be recovered from the surface with `wl_proxy_get_display`, which
  needs unsafe FFI and libwayland 1.23 or later.
- On X11 a shim must open its own connection, and that connection can't release winit's pointer
  grab before `_NET_WM_MOVERESIZE` (see R1 and the Q2 notes).
