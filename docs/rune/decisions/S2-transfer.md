# S2 Data transfer beside winit

| | |
| --- | --- |
| Work package | [S2](../work/phase-0-groundwork/S2-transfer-beside-winit.md) |
| Rune | `78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5`, winit 0.30.13 (calloop-wayland-source 0.3.0), arboard 3.7.0 with wl-clipboard-rs 0.9.5 |
| Prototype crates | wayland-client 0.31.15, wayland-backend 0.3.17 (`client_system`), wayland-protocols 0.32.13, wayland-protocols-wlr 0.3.12, x11rb 0.13.2, Rust 1.99.1 |
| Ran here | sway 1.12 / wlroots 0.20.2 headless (gles2 renderer on the RTX 5090 render node), Xwayland 24.1.14 inside it, Xvfb 21.1.25, libwayland 1.26.0, NVIDIA 615.78.08, Nautilus 51.1 (GTK 4.24.1) inside the nested sway, kernel 7.2.15 |
| Hand test pending | Hyprland 0.56.2, GNOME (mutter), KDE (KWin), Dolphin, lavapipe |
| Prototype | `/home/wmfeht/dev/strata-spikes/s2` (never merged). Logs are under its `logs/<experiment>-<time>/`. A copy of the transfer code sat on the local branch `spike/s2-prototype`, never merged. The branch was deleted once P0 had merged; its tip was `ee886835`. |

## Verdicts

| # | Question | Answer | Where it ran |
| --- | --- | --- | --- |
| Q1 | Reach winit's `wl_display` and a seat without patching Rune? | **Yes, on libwayland ≥ 1.23.** `wl_proxy_get_display` on the surface from `WindowHandle::with_raw_handle`, then `Backend::from_foreign_display`. No on 1.22 (Ubuntu 24.04) without a private-ABI read. | sway |
| Q2 | A second event queue, and who dispatches it? | **Yes, and we must dispatch it.** A watcher thread that only polls the fd (driver A) races: without an every-turn hook, events wait for an unrelated wake (p99 123–279 ms at 200 changes × 30 ms, 37 ms at 500 × 5 ms). A reader thread that takes part in libwayland's read protocol, with dispatch on the UI thread (driver C), gave p99 0.92 ms at 200 changes × 30 ms and 1.13 ms at 500 × 5 ms, one run each. | sway |
| Q3 | Start a `wl_data_device` drag from a press Rune saw; does Rune's capture stay sane? | **The drag works; the capture doesn't.** winit gets a pointer leave and never the release. Rune stays `pressing()` through the whole drag. Afterwards it is cleaned up only by accident (sway bounces keyboard focus at the drop). Without that, `on_drag` keeps getting moves with no button held until the next press. | sway |
| Q4 | Drops with any MIME type and a position per motion, in Rune's coordinates? | **Yes.** Six types were offered and read intact, 8 MiB included. There was one motion event per pointer motion. Surface coordinates equal Rune window points at scales 1, 1.5 and 2: zero hit-test mismatches. Nautilus ↔ Rune works in both directions (by hand, no log). | sway, Nautilus in sway |
| Q5 | Data-control on Hyprland, sway, GNOME, KDE; fallback cost? | **Sway and Hyprland have both protocols, KWin has ext only, mutter has neither.** Without data-control, P2 hears the clipboard only while focused, and a write needs a fresh input serial. | sway; others from docs, hand test pending |
| Q6 | X11: share winit's connection, or a second one? | **A second `x11rb` connection.** Rune exposes no X connection, and winit owns XDND for its window. `XdndProxy` on Rune's window plus a grab-less source (XI2 raw events) work both ways. | Xvfb, Xwayland |
| Q7 | Drag icon from an element snapshot, headless? | **Yes**, through public API (`Runtime::new`, `render::Gpu::headless`, `Renderer`, `Offscreen`). It is pixel-identical to `--snap --crop`. Cold 150–255 ms, warm 7 ms. It renders a separate `Runtime`, not the live frame. | headless, sway |

**P0 has an approach with no Rune patch** (below), apart from libwayland 1.22 (Ubuntu
24.04): there it has no route to winit's display for drag and drop. Rune needs to expose five
things (R1–R5) for the shims to be deleted later. One of them (R5, ending a pointer capture)
is the only one P1 can't work around cleanly.

## Method

Each Wayland run started a private headless sway: `WLR_BACKENDS=headless`,
`WLR_LIBINPUT_NO_DEVICES=1`, a private `XDG_RUNTIME_DIR`, launched as `setpriv --no-new-privs
sway`. `WAYLAND_DISPLAY`, `DISPLAY`, `SWAYSOCK` and `HYPRLAND_INSTANCE_SIGNATURE` were unset
first. X11 runs used a private Xvfb, or the Xwayland of the nested sway. Nothing connected to the
owner's Hyprland session. Hyprland, GNOME and KDE answers come from protocol docs
([wayland.app](https://wayland.app/protocols/ext-data-control-v1) compositor tables) and from
the protocol headers Hyprland 0.56.2 installs under `/usr/include/hyprland`, never from a
running session.

The prototype has four crates:
- `probe`: display access, four queue drivers (A, A2, B, C), data device, data-control, X11 selections,
  XDND proxy target and grab-less source, element snapshot.
- `app`: a Rune window with a drag-out tile (`on_drag`), a button and three drop zones, the probe
  bound beside winit. It logs every Rune callback, `Runtime::pressing()`, layout and transfer
  event as JSON lines with `CLOCK_MONOTONIC` stamps.
- `peer`: "another app" on its own connection. It has an xdg toplevel, data device and
  data-control, and drives input through `zwlr_virtual_pointer_v1` and
  `zwp_virtual_keyboard_v1` (keymap from `xkbcli compile-keymap`). On X11 it uses x11rb with
  XTEST.
- `snap`: the Q7 timings and the pixel comparison.

`env/run.py` drives one experiment per question; `repros/` holds R1–R5.

## Q1 Display access

| Check | Result |
| --- | --- |
| Route | `App::on_window` → `WindowHandle::with_raw_handle` gives `RawWindowHandle::Wayland { surface }` only. `dlopen("libwayland-client.so.0", RTLD_NOLOAD)` + `dlsym("wl_proxy_get_display")` returns winit's display. Then `Backend::from_foreign_display`, `registry_queue_init` (own queue), `ObjectId::from_ptr` for winit's surface, as Rune's `platform/activation.rs` does. Attach took 0.4–0.7 ms. |
| Private layout | The `struct wl_proxy` layout read (offset 24) gave the same pointer on 1.26. It works but is a private ABI; not recommended. |
| Globals and seat | 57 globals on sway (`logs/globals-*`). A second `wl_seat` object with its own `wl_pointer` and `wl_keyboard` gets the same events and serials as winit's. |
| Rune still draws and takes input | Yes: frames keep drawing; a virtual click reached Rune's `on_press` and our `wl_pointer` saw the same button, before and after 100 hide/show cycles. |
| Hide/show 100× | On Wayland each show recreates the window: `on_window` ran 101 times. Rebinding only the surface worked every time; fds stayed at 59 (`logs/q1-150037`). |
| Attach/detach 1000× | No leak. Each new `wl_keyboard` makes sway resend the keymap to **every** keyboard of the client, winit's included, so 2 fds per bind sit in queues until the loop turns. The count reached 2,059 during the loop and returned to 59 once it turned. Without a keyboard there is no growth. Destroying without draining didn't leak on libwayland 1.26 (`logs/churn-*`). |
| libwayland < 1.23 | Simulated (`S2_FORCE_NO_GET_DISPLAY`): no display, and the app runs without transfer. |

The first prototype called `wl_seat.get_pointer` on headless sway's seat, which had no pointer
yet. That is a `missing_capability` protocol error. It **killed winit's connection with it**
(`Io error: Broken pipe`, then `run: Exit Failure: 1`): one connection, one fate. Bind
pointer and keyboard only from the `capabilities` event, as Rune's layer shell does.

## Q2 The second queue and its driver

The queue gets events only when someone reads the socket, and only our code dispatches it. winit
reads the socket, but dispatches only its own queue. Rune has no hook that runs on every loop
turn that a shim can use. Four drivers were measured with selection changes set by the peer
through data-control, each stamped in its MIME type. Latency is peer send to the UI thread
handling the event.

| Driver | How |
| --- | --- |
| A | The shape of Rune's layer-shell `Watch`, without Rune's every-turn dispatch: a thread polls the fd and wakes the UI, and the UI calls `dispatch_pending` |
| A2 | A, plus a non-blocking read on the UI thread when nothing was pending |
| B | Own thread: `prepare_read`, poll, `read`, `dispatch_pending`, then `AppHandle::update` |
| **C** | Own thread joins the read protocol (`prepare_read`, poll, `read`), then wakes the UI with `update_if`; the UI calls `dispatch_pending` |

Each row is one run (`logs/q2-<driver>-<time>`). "Ticker" means a 10 ms `AppHandle` update
thread was on to measure UI lag; it dominates idle CPU. The 500 × 5 ms runs idled 3 s, the
others 10 s.

| Run | Load | Ticker | `ext_data_control` latency p50 / p99 / max (ms) | Idle wakes | Idle CPU (ms) | Stop (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| a-144048 | 200 × 30 ms | on | 0.38 / **123.3** / 154.2 | 9 in 10 s | 90.2 | 0.19 |
| a-144200 | 200 × 30 ms | off | 0.51 / **278.6** / 309.4 | 12 in 10 s | 19.0 | 0.07 |
| a-144446 | 500 × 5 ms | on | 0.25 / **36.5** / 57.1 | 7 in 3 s | 31.6 | 0.11 |
| a2-144218 | 200 × 30 ms | off | 0.52 / **276.3** / 307.1 | 9 in 10 s | 25.9 | 0.11 |
| b-144106 | 200 × 30 ms | on | 0.42 / 0.77 / 2.62 | 24 in 10 s | 96.9 | 0.54 |
| b-144236 | 200 × 30 ms | off | 0.46 / 1.85 / 24.87 | 22 in 10 s | 25.1 | 0.25 |
| b-144453 | 500 × 5 ms | on | 0.31 / 1.51 / 2.26 | 15 in 3 s | 29.4 | 0.44 |
| **c-144420** | 200 × 30 ms | off | **0.53 / 0.92 / 1.29** | 6 in 10 s | 14.7 | 0.06 |
| **c-144500** | 500 × 5 ms | on | **0.28 / 1.13 / 1.72** | 0 in 3 s | 29.8 | 0.30 |

`q2-c-144329` is not in the table: it was the first C build, which saw no events at all (see
below). A2 ran once and C once per load, so these are single samples, not distributions.

- **A races.** When winit's loop reads the socket before the watcher polls it, the watcher never
  wakes. The events wait in our queue for the next unrelated socket activity: 35 of 200 were
  late by a multiple of the 30 ms step (`logs/q2-a-144048`). Reading on the UI thread (A2)
  doesn't help, because the data is already gone from the socket.
- **Rune's own overlays don't hit this.** `Windows::about_to_wait` (`window.rs:330`) draws each
  host every loop turn, and the layer-shell host's draw calls `layer.dispatch()` or
  `drain_hidden` (`overlay/draw.rs:67`). winit emits `AboutToWait` on every iteration
  (`wayland/event_loop/mod.rs:515`), so Rune dispatches its private queue whatever woke the
  loop. The race hits only code with no every-turn hook, which is P0's position: Rune exposes
  none.
- **A must use `update_if`.** With `update`, each wake commits and redraws. The redraw's frame
  callback makes the shared fd readable, which wakes the watcher again: about 100 wakes per
  second, none with events.
- **C can't miss a read.** While C holds a read intent, libwayland lets no reader read alone:
  winit's `read_events` waits for C's. calloop-wayland-source holds no read guard while sources
  dispatch, so neither side blocks the other. No deadlock and no UI stall showed in any run.
  No UI-thread tick ran more than 20 ms late, apart from window creation. The flip side:
  winit's UI thread can wait on C's read, so C must never block on the UI while it holds a read
  intent (P0 step 4).
- In wayland-backend's sys backend, all wayland-rs queues of one `Backend` share **one**
  libwayland queue, so a guard covers the backend, not the Rust queue. `ReadEventsGuard::read`
  also moves events into the Rust queues, so `prepare_read` succeeds again with events waiting.
  C must wake the UI after every read that returned events. The first C build missed every
  event because of this.
- Idle cost for the whole app over 10 s without the ticker: 15–26 ms of CPU and 6–7 Rune
  frames, with any driver. With the ticker on, 90–97 ms.

## Q3 Drag out from a Rune press

The tile has `on_drag` (Rune's `drag_out` is macOS-only; see R1). At 8 pt from the press, the
handler calls `wl_data_device.start_drag` with our `wl_seat`'s last button serial and winit's
surface as origin. Our `wl_pointer` had that serial before the threshold in every run. That
is luck of timing, not an ordering guarantee; see P1 in the constraints.
`start_drag` took 0.04 ms. The peer accepted `text/plain;charset=utf-8` and read it intact.
The source saw `target`, `action`, `dnd_drop_performed`, `send`, then `dnd_finished`. sway then
also sent `cancelled`, which P1 must ignore.

The timeline from `logs/q3-150047` and `logs/q3-novk-*` (milliseconds from `start_drag`):

| Time | sway with a keyboard (focus bounces) | No keyboard (no bounce) |
| --- | --- | --- |
| −97 | `on_drag` Down; `pressing()` true | same |
| +0.5 | `wl_pointer.leave` (winit's and ours); Rune `pointer_leave` keeps the capture (`left_in_capture`) | same |
| drag | `pressing()` stays true; Rune's pointer stays at the stale point | same |
| drop +0.3 | `wl_keyboard.leave` then `enter` to our client | nothing |
| drop +14 | winit `Focused(false)` → Rune `cancel_pointer` → `on_drag` Up with `cancelled: true`; `pressing()` false | — |
| pointer returns, no button | nothing | **4 `on_drag` Move events with no button held**; `pressing()` still true |
| next press elsewhere | normal press | Works: Rune's `pointer_down` overwrites the capture. The tile **never gets Up**. |

A cancelled drag (no target accepts) behaves the same. No release ever arrives, because the
compositor's drag grab takes it.

The swallow workaround (`S2_SWALLOW`) ignores the tile's moves after the drag starts. It fixes
the app's own logic, but Rune's `pressing()`, hover and cursor stay wrong until the next press.
That next press may land on another element, so the tile's handler never hears it. P1 has to
end the swallow from P0's own `wl_pointer` button event, which it receives anyway. Ending it
properly needs R5.

## Q4 Drops into Rune

The peer dragged six types across all three zones and dropped on one: `text/uri-list`,
`text/plain;charset=utf-8`, `text/x-moz-url` (UTF-16), `image/png`, `text/html` and
`application/x-s2-custom`.

- All six types were listed at `enter`. Every read matched the bytes the peer served (FNV hash):
  46, 17, 56, 4,096, 14 and 35 bytes. An 8 MiB `image/png` read in 69 ms on a worker thread,
  with no UI tick late by more than 20 ms (`logs/q4-s1-145048`).
- One `wl_data_device.motion` arrived per pointer motion: 34–45 per drag. Each was hit-tested
  on the UI thread with `tx.element_at(x, y, ..)` in ≤ 0.07 ms (one outlier 0.44 ms), followed
  by `offer.accept` and `set_actions` from that zone's type.
- The coordinates are surface-local logical points, the same as Rune's window points. There were
  0 mismatches between the zone Rune picked and the zone its layout contains, at sway scales 1,
  2 and 1.5 (fractional positions such as 259.199 included) (`logs/q4-s2-*`, `q4-s1.5-*`).
- `drop` is followed at once by `leave`. Keep the offer until the read's `finish`. The first
  build destroyed it on `leave` and lost the drop.
- **Modifiers:** Ctrl and Shift held on the virtual keyboard during a foreign drag reached
  neither the target's `wl_keyboard`, which has no focus while another client drags, nor
  `offer.action`. sway doesn't map modifiers to actions. Mutter is reported to change the
  action with Shift and Ctrl; that wasn't checked here. KWin is unknown. Hyprland's
  `DataDevice.hpp` declares no modifier listener for its drag state. All three are hand tests
  pending.
- **Nautilus 51.1** ran in the nested sway with a private D-Bus. This was observed by hand
  from ad-hoc scripts; no log was kept, so P1 must re-record it. Dragging a file into the Rune
  zone offered `application/x-gtk-local-dnd`, `text/plain;charset=utf-8`, `text/uri-list`,
  `application/vnd.portal.filetransfer` and `application/vnd.portal.files`, with actions
  copy|move|ask, and read `file:///tmp/s2files/alpha.txt`. Dragging the Rune tile into Nautilus
  copied both files of the `text/uri-list`. Nautilus's Ctrl+C was readable as
  `x-special/gnome-copied-files` and `text/uri-list`, and our `x-special/gnome-copied-files`
  pasted into Nautilus with Ctrl+V.
- Rune's own `on_drop_files`, `on_drop_hover` and `on_drop_move` heard nothing on Wayland, as
  its README says (R2).

## Q5 Data-control per compositor, and the fallback

| Compositor | `ext_data_control_manager_v1` | `zwlr_data_control_manager_v1` | Source |
| --- | --- | --- | --- |
| sway 1.12 | v1 | v2 | ran here (`logs/globals-*`) |
| Hyprland | yes (0.52.1) | yes (0.52.1) | wayland.app table; 0.56.2 installs `ext-data-control-v1.hpp` and `wlr-data-control-unstable-v1.hpp`. Hand test pending. |
| KWin 6.7 | yes | **no** | wayland.app table. Hand test pending. |
| mutter 51 | **no** | **no** | wayland.app table. Hand test pending. |

What data-control gave on sway (`logs/q5-145243`):
- The selection is heard while unfocused, 0.18–0.27 ms after the peer's change.
- The app wrote five types (`text/uri-list`, `x-special/gnome-copied-files`,
  `application/x-kde-cutselection`, `text/plain;charset=utf-8`, an 8 MiB `image/png`). The peer
  listed and read each one intact: the 8 MiB in 25–27 ms. Each type's writer ran only when asked.
- Ownership loss arrives as `cancelled` on our source.

The cost of falling back to `wl_data_device` (mutter, or no data-control):
- **The badge waits for focus.** `wl_data_device.selection` arrives only with keyboard focus. In
  the run, a change made while the peer had focus reached us 642 ms later, when our window was
  clicked. In general the wait is unbounded.
- **Writes need a fresh serial.** `set_selection` with an input serial older than the current
  selection's is **silently ignored** by wlroots: no error, no event. A write with the
  keyboard-enter serial after the peer's data-control change did nothing; the same write after
  a click (a fresh button serial) worked. Use the serial of the key or click that triggered the
  copy, from P0's own `wl_keyboard`/`wl_pointer`. Rune's events carry no serial. Detect a
  dropped write by the missing `selection` echo on our own device.
- Writers run once per request. Nautilus asked for `x-special/gnome-copied-files` six times for
  one paste, so cache each type's bytes for the life of the source.
- Primary selection: ext-data-control has it (`primary_selection` events seen); not exercised.

Coexistence with Rune's clipboard (arboard over wl-clipboard-rs, which speaks both
data-control protocols):
- Rune's `tx.copy` offered `text/plain`, `UTF8_STRING`, `TEXT`, `text/plain;charset=utf-8` and
  `STRING`. It cancelled our source, which correctly turns the badge off. The peer read it.
- Rune's `tx.read_clipboard` read our offer's text and HTML.
- On mutter, arboard finds no data-control and falls back to X11 through Xwayland (`arboard`
  `platform/linux/mod.rs`). Hand test pending.

## Q6 X11

Rune exposes no X connection: `with_raw_handle` gives `Xlib { window, visual_id }` only. Every
X11 piece runs on a second x11rb `RustConnection` with an unmapped override-redirect window.
Results on Xvfb (`logs/x11-xvfb-145518`) and Xwayland (`logs/x11-xwayland-145447`):

- **Selections:** owning `CLIPBOARD` and serving `TARGETS` with four types works, and so does
  INCR in 256 KiB chunks. 8 MiB was read by the peer in 13.5 ms on Xvfb and 10.7 ms on
  Xwayland; the app read the peer's 8 MiB in 27 ms and 6.5 ms. `SelectionClear` and XFixes
  `SetSelectionOwner` notifications arrive on ownership change, so the badge updates
  immediately.
- **XDND goes to winit by default.** Without a proxy, the peer's `XdndPosition` reached winit's
  window and winit answered (`XdndActionPrivate`, file URIs only); our connection saw nothing.
  Rune's `on_drop_files` fired, but only for files that exist.
- **`XdndProxy` on Rune's window works.** Pointing it at our window, with `XdndAware` 5 on ours,
  sends every XDND message there: enter with four types, positions translated to Rune points,
  and the drop read of `text/plain` (17 B). Both servers. It also turns off Rune's own X11
  `on_drop_files`; P1 replaces it.
- **The grab is winit's.** `XGrabPointer` from our connection during the press returns
  `AlreadyGrabbed` on both servers.
- **A grab-less source works.** XI2 `RawMotion`/`RawButtonRelease` on the root, `QueryPointer`,
  then a walk down to the `XdndAware` window (honouring its `XdndProxy`). The peer accepted and
  read the drop, and we got `XdndFinished` (accepted).
- **Rune's capture during an X11 drag:** winit keeps the implicit grab. Rune gets every Move (25
  in the run) and then a real Up, not cancelled. P1 must ignore the element's moves until that
  Up. Nothing is stuck afterwards. The cursor can't show drag feedback, because only the
  grabbing client can change the grab cursor.
- The grab-less source has no keyboard grab either, so Escape can't cancel the drag through
  X. Escape reaches Rune's window as a key event (it has focus), and P1 has to cancel from
  there. Unverified.
- Not built: modifier-driven actions (read `QueryPointer`'s mask), `MULTIPLE` targets, a drag
  icon window.

## Q7 Drag icon from an element snapshot

- **Route:** `Runtime::new(state, view, viewport, Env::new(dark, scale))`, `rt.frame(0.0)`,
  then `render::Gpu::headless`, `Renderer::new`, `upload_images`, `prepare` with
  `rt.text.atlas` and `rt.instances`, `draw` into an `Offscreen`, `read`. Crop to the element's
  `ElementInfo::rect`, which is `[x, y, w, h]` in points. This is `tooling/headless.rs` per
  frame, all public.
- **Parity:** pixel-identical to Rune's `--snap out.png --crop out` at scales 1, 1.5 and 2,
  once the snapshot uses an app's defaults: `rt.set_srgb_blend(true)`, a non-sRGB target, and
  `renderer.set_srgb_blend(true)`. Without that, 2.7 % of the pixels differ, all of them in the
  glyphs.
- **Cost** (`snap ours`): cold 154–255 ms, of which GPU init is 132–165 ms; warm p50 7.0–7.9 ms,
  max 8.6 ms, mostly `Renderer::new`. In the app, the first snapshot at drag start blocked the
  UI thread for 158 ms. P1 must warm the GPU and renderer at attach, or render at the press.
- **Second device:** `Gpu::headless` creates its own wgpu instance and device beside the
  window's. That costs VRAM, and on a hybrid-GPU machine it may pick a different adapter from
  the window's. Only one GPU was present here.
- **Wayland icon:** a `wl_shm` ARGB8888 (premultiplied BGRA) surface passed to `start_drag` was
  accepted and the drag ran. Scale-2 icons (`set_buffer_scale`) weren't tried.
- **Limit:** the snapshot is a second `Runtime` over a state the caller supplies, not the live
  window's frame: no hover, press or scroll state unless copied. Rune's `drag_out::snapshot` of
  the live frame is `pub(crate)`; R1 asks for it.
- **X11:** an override-redirect window moved on each raw motion is the expected route. Not
  built.

## P0 binding approach

No Rune patch. Everything goes behind P0's internal `transfer` module; P1, P2 and P4 build on it.

**Wayland**
1. **Display.** In `App::on_window`, take the `wl_surface` from `with_raw_handle` and recover
   winit's display with `wl_proxy_get_display`. Look it up at run time:
   `dlopen("libwayland-client.so.0", RTLD_NOLOAD)` + `dlsym`. Don't use wayland-sys's
   `libwayland_client_1_23` feature: feature unification would make winit's own `dlopen` of
   libwayland fail on 1.22 hosts, and with it all Wayland support. On < 1.23, report
   `Unsupported` (see owner question 1).
2. **Binding, once per display.** `Backend::from_foreign_display` and `registry_queue_init`.
   Bind `wl_seat` and `wl_data_device_manager` (then the device), `ext_data_control_manager_v1`
   (else `zwlr` v2), `wl_compositor` and `wl_shm` for icons, and `zxdg_importer_v2` for P4. Bind
   `wl_pointer` and `wl_keyboard` only from `capabilities`. The pointer and keyboard are for
   serials and modifiers, never for input: Rune keeps that.
3. **Surfaces: keep the `WindowHandle`, never the surface.** `with_raw_handle` says not to
   retain the pointer beyond the callback (`window/handle.rs:131`). A hide doesn't run
   `on_close` (Rune README, line 882), and a Wayland hide destroys the surface (Q1: 101
   `on_window` calls for 100 hides). A stored surface would dangle, and a reused address would
   misroute events.
   - Keep a clone of each window's `WindowHandle` from `on_window`. It is `Clone` and "refreshed
     across App close/reopen".
   - Resolve the surface at each use, on the UI thread, through `with_raw_handle`: the origin for
     `start_drag`, the surface for an icon or for P4. Build the `ObjectId::from_ptr` +
     `WlSurface::from_id` proxy inside that use only, as `platform/activation.rs` does for its
     own lifetime. `with_raw_handle` returns `Err` while the window is closed.
   - It must be the UI thread. `with_raw_handle` upgrades its weak reference to winit's
     `Window` for the call. Off the UI thread that temporary `Arc` can be the last one, and
     that thread would then drop winit's `Window`.
   - Route `wl_data_device.enter` and `wl_pointer.enter` during `dispatch_pending` (which runs on
     the UI thread under driver C). Compare the event's surface pointer with each kept handle's
     current surface, and keep no map.
   - Events are parsed on C's thread during the read (`sys/client_impl/mod.rs:952–1016`) and
     delivered later by `dispatch_pending` on the UI thread, so a foreign surface's pointer may
     be stale by then. Compare only `surface.id().as_ptr()`. Never send a request on the event's
     `WlSurface`, and never build a proxy from it. A misroute from a stale or reused address is
     harmless, because a `leave` follows.
   - Drop events whose surface matches no handle. On compositors without server-side
     decorations, winit's adwaita frame draws on subsurfaces of its own (sctk-adwaita), and a
     drag over the frame enters those. By source; not seen on sway, which decorates server-side.
   - Run here (`logs/teardown2/handle.jsonl`): the handle kept from the first `on_window` gave
     `Ok(surface 0x…f0330)`, then `Err("window is closed")` while hidden, then the new surface
     (`0x…62d330`) after `show`. That probe called `with_raw_handle` from a non-UI thread, so it
     shows the refresh, not the threading. The prototype itself still rebinds a stored surface
     per `on_window`; the no-map routing is designed, not run.
4. **Driver C.** One reader thread per display that takes part in libwayland's read protocol.
   After each read that returned events, it wakes the UI with a caller-supplied waker:
   `AppHandle::update_if`, returning whether anything changed. The UI side calls
   `dispatch_pending`, so protocol handlers and hit tests run on the UI thread. A stop pipe ends
   a reader that is in `poll` in under 1 ms. A reader waiting for a UI turn is another matter
   (the fourth invariant). Invariants, because winit's `read_events` waits for C's read while C
   holds a read intent:
   - C never waits on the UI thread while it holds a read intent. It reads or `cancel_read`s
     first, and only then wakes the UI and waits for the turn.
   - After the stop flag (or the stop pipe), C's only libwayland call is `cancel_read` on an
     intent it already holds. It checks the flag before each `prepare_read`, and an intent that
     races the flag is cancelled at the next `poll`, which the stop pipe wakes. The prototype
     does this (`wl.rs` 600–602 on `spike/s2-prototype`: the stop check drops the guard and
     breaks).
   - The stop must complete while winit's display is alive (step 6).
   - Stop also notifies the turn condvar, and the turn wait returns once the stop flag is set.
     The UI thread never joins on a wait that only a UI turn can end. The prototype breaks this:
     `Stop::stop` (`wl.rs` 492–502) only closes the pipe, and `wait_turn` (`wl.rs` 442–445)
     waits on the condvar with a 100 ms timeout. A join from the UI thread while C waits for a
     turn therefore blocks for up to 100 ms, and would deadlock without the timeout, since the
     joining thread is the one that would give the turn.
   - Added by P0's review: C never dispatches. It reads with `read_without_dispatch`, so events
     naming winit's `wl_surface` are parsed only by the UI thread's `dispatch_pending`.

   C's turn wait has a timeout (100 ms in the prototype). After `run` returns no turn comes. In
   the teardown runs below, the late stop took about 100 ms in 4 of 5 `late-stop-only` runs
   (0.05 ms in the fifth), and in 2 of 5 `late-both` runs (0.16–0.39 ms in the other three).
5. **Data off the UI thread.** Offers are read through a pipe on a worker thread, and the result
   comes back through the waker. Sources write each type lazily, on a worker, from bytes cached
   per source. fd hygiene:
   - every pipe `O_CLOEXEC`;
   - close our write end right after `receive` (and the flush), or the read never sees EOF;
   - a read timeout, so a source that never writes doesn't pin a worker;
   - a size cap on offer reads, so a source can't exhaust memory;
   - treat `EPIPE` on a source write as "the reader left". Rust already ignores `SIGPIPE`, and
     a library mustn't change the signal disposition.
6. **Teardown.** The reader must be stopped and joined, and every object bound on winit's
   display destroyed or leaked, before winit closes the display.
   - **When:** only when the event loop is ending, since step 2 binds once per display and the
     display outlives windows. There are two cases:
     - The quit path: P0 tears down itself, before `Tx::quit` or `AppHandle::quit`. After a
       quit, no further `on_close` runs: Rune skips the remaining hosts' hooks when the loop is
       already exiting (`window.rs:200–207`).
     - The last main window's `on_close` in an app without `keep_running`: closing it ends the
       app (`window.rs:200–207`).

     Not the last window's `on_close` in general: under `keep_running()` or `windowless()` the
     loop and the display live on.
   - `App::on_close` is `fn(&mut S, &mut Tx)`, so the transfer handle must be reachable from
     `S` or be process-wide. That is the P0 "transfer handle's home" decision below. Teardown
     from `on_close` is designed, not run.
   - **Order, on the UI thread:** set the stop flag, wake both the poll (pipe) and the turn wait
     (condvar), then join the reader, unconditionally and before `run` returns. Then destroy sources, offers, devices, pointer, keyboard and seat, and drop the
     `Backend`. wayland-backend's `ConnectionState::drop` (`sys/client_impl/mod.rs:1129–1152`)
     calls `wl_proxy_destroy` on every proxy it still knows, then `wl_event_queue_destroy`.
   - **Never** let the objects drop in a `thread_local` destructor or any value dropped after
     `run` returns. A plain `static` that is never dropped is a leak, which is fine for the
     proxies. Leaking (`mem::forget`) is the fallback for the objects, but not for the reader:
     a reader still running when the display closes is a use-after-free even if every object
     is leaked. It polls a closed (possibly reused) fd and calls `cancel_read` on a freed
     display.
   - **Exits with no hook:** a winit connection error, or `AppHandle::quit` called from code
     that skipped P0's teardown. There P0 has no safe path until R4's lifetime hook; P0 should
     document that it must be called first. Unverified.
   - **Measured** (`logs/teardown2/`, exit codes in `exit-codes.txt`; first pass in
     `logs/teardown/`), 5 runs each under sway, driver C:

     | Variant | Reader | Objects | Exit |
     | --- | --- | --- | --- |
     | early | stopped on the UI thread before quit | dropped on the UI thread before quit | 5 × 0 |
     | leak | stopped before quit | leaked | 5 × 0 |
     | late drop only | stopped before quit | dropped after `run` returned | **5 × 139 (SIGSEGV)** |
     | late stop only | left running; stopped after `run` returned | leaked | 5 × 0 |
     | late both | left running; stopped after `run` returned | dropped after `run` returned | **5 × 139** |

     The cores show `wl_proxy_destroy` ← `ConnectionState::drop` ← the `Backend` drop
     (`coredumpctl info` for `late-both` runs 3 and 4, PIDs 3826812 and 3826856, saved as
     `logs/teardown2/coredump-3826812.txt` and `coredump-3826856.txt`). So the crash comes from
     the late drop. The late stop didn't crash, but it isn't safe: in one
     `late-stop-only` run the stop took 0.05 ms, so `cancel_read` really ran on the freed
     display; in the other four the reader was stuck in its 100 ms turn wait and exited at the
     stop check. No crash is not
     evidence of safety for a use-after-free. Exit codes are from the shell (`$?`, 139 =
     SIGSEGV). In the first pass they were printed but not saved; the second pass saved them.
   - This matches S3's E1.7 (`docs/decisions/S3-foreign-parent.md`). The first version of this
     record called an un-stopped reader harmless. That was wrong: the prototype held the
     binding in a `static`, so its destructor never ran.
   - R4's lifetime hook would remove this rule.

**X11.** A second x11rb connection and window:
- selections with `TARGETS`, INCR and XFixes owner tracking;
- an XDND target behind `XdndProxy` set on Rune's window;
- a grab-less XDND source on XI2 raw events;
- our own window carries `XdndProxy` pointing at itself, as the XDND spec requires of a proxy
  window (`x11.rs:117`), plus `XdndAware` 5;
- an event thread (`X::run` in the prototype) that wakes the UI with `AppHandle::update_if`,
  returning whether any event arrived (the same reason as A's `update_if` rule);
- `XdndProxy` set again on every `on_window`, since a recreated window starts without it;
- a stop path. The prototype's thread blocks in `wait_for_event` and has none. Either set a stop
  flag and send a `ClientMessage` to our own window to wake it, or poll the connection's fd
  together with a stop pipe and drain with `poll_for_event`. Designed, not run.
- teardown in this order: stop and join the event thread, delete `XdndProxy` from Rune's
  window, then destroy our window and drop the connection. Otherwise sources keep sending to a
  window that no longer exists. Designed, not run.

**Starting point.** The prototype's `probe` (`wl.rs` drivers B and C, `x11.rs`, `display.rs`,
`snapshot.rs`), the peer and the harness scripts were on the local branch `spike/s2-prototype`
under `crates/strata-platform/src/transfer/` (deleted after P0 merged; tip `ee886835`). They were
reference code, not wired into the crate, and that branch was never merged. P0 should rewrite them to the repository's lints. Every
`unsafe` needs a `SAFETY` comment and a module-level `#[expect(unsafe_code, reason = ..)]`.

## What Rune must expose

| Ask | Why | Repro |
| --- | --- | --- |
| Linux `drag_out`, with several MIME types, lazy data, action negotiation, the live-frame snapshot as icon, and the outcome | P1's source half; with it, R5 stops mattering for drags | R1 |
| Drops on Wayland with arbitrary MIME types and a position per motion (and the same types on X11) | P1's target half | R2 |
| Clipboard writes with custom MIME types and images, reads by type, and an ownership-change notification | P2 | R3 |
| The display handle, `with_raw_display_handle` (Wayland display, X11 connection), and a guarantee or hook for when it is dropped | P0's route without `wl_proxy_get_display`, an X11 route on winit's connection, and a safe teardown point. One issue with S1's R7 and S3's display-handle ask; the union is under R4. | R4 |
| A public way to end a pointer capture (`Tx::release_pointer()` or similar) that delivers the cancelled Up | A shim-started drag otherwise leaves Rune pressing, with moves reaching `on_drag` with no button held | R5 |

## Constraints for later WPs

Recommended changes. None of these files was edited here.

- **W3 (`W3-compositor-harness.md`):**
  - Start sway as `setpriv --no-new-privs sway`. `/usr/bin/sway` carries `cap_sys_nice=ep` and
    goes realtime, and where `RLIMIT_RTTIME` is 0 (this agent shell) the kernel kills it at
    once. Unset `WAYLAND_DISPLAY`, `DISPLAY`, `SWAYSOCK` and `HYPRLAND_INSTANCE_SIGNATURE`
    first. Use a private `XDG_RUNTIME_DIR`.
  - Renderer: pixman gave Rune no surface on NVIDIA (`no GPU adapter … vulkan not compatible
    with provided surface`; Rune's Linux build has Vulkan only). `WLR_RENDERER=gles2` with
    `WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128` worked; that node is unprivileged and not the
    live session. `WLR_RENDERER=vulkan` worked too. pixman with lavapipe is the likely CI route,
    untested here (lavapipe isn't installed).
  - Headless sway's seat has no capabilities until a virtual device exists. The peer must
    create its `zwp_virtual_keyboard_v1` (keymap from `xkbcli compile-keymap --layout us`, NUL
    terminated) and `zwlr_virtual_pointer_v1` **before** the app starts. Otherwise the app
    never gets keyboard focus or a serial. `motion_absolute` takes the output layout's logical
    size as its extent.
  - `get_pointer` or `get_keyboard` on a seat without that capability is a protocol error that
    kills the client. That applies to the peer as much as to P0.
  - Xwayland starts on demand: get its display with `swaymsg exec 'echo $DISPLAY > file'`.
  - Rune's window has no `app_id`; match it by title. The peer floats clear of the app with
    `for_window [app_id="s2-peer"] floating enable, resize set 300 140, move position …`.
  - Nautilus nests fine under `dbus-run-session` with `GSK_RENDERER=cairo GTK_A11Y=none`. `grim`
    on the nested display gives screenshots for locating icons.
  - Rune draws on Xvfb with the NVIDIA driver here.
  - Tests must exercise P0's teardown (step 6): stop and destroy on the UI thread before `run`
    returns. A late drop crashed 5 of 5 runs.
- **P0 (`P0-transfer-substrate.md`):**
  - Display access is the `wl_proxy_get_display` route above, with an `Unsupported` result on
    libwayland < 1.23. Put "Ubuntu 24.04: no Wayland transfer through winit's display" in Done
    when, unless the owner decides otherwise (question 1).
  - Use driver C. A watcher that only polls the fd (driver A) races without the every-turn
    dispatch Rune's own overlays get from `about_to_wait` (Q2).
  - Protocol hygiene, since any error on our objects kills winit's connection. The fatal errors
    on P0's objects:
    - `wl_seat`: `missing_capability` (gate `get_pointer` and `get_keyboard` on capabilities);
    - `wl_data_offer`: `invalid_finish` (`finish` after `accept(NULL)` or before any `action`
      event), `invalid_offer` (`finish` or `set_actions` on a selection offer),
      `invalid_action_mask`, `invalid_action`;
    - `wl_data_source`: `invalid_action_mask`, `invalid_source` (`set_actions` on a selection
      source, or a drag source used for the selection);
    - `wl_data_device`: `role` (an icon surface that already has another role) and
      `used_source` (a source passed to `start_drag` or `set_selection` twice);
    - `wl_shm` / `wl_shm_pool`: `invalid_format`, `invalid_stride`, `invalid_fd` (drag icons);
    - `wl_surface`: `invalid_scale`, and `invalid_size`: a buffer whose size isn't divisible by
      the buffer scale. That is the scale-2 icon trap for P1.
    - data-control: `invalid_offer` (`offer` after `set_selection`) and `used_source` (a
      source reused).

    Also destroy the previous selection offer, and keep a dropped offer until `finish`.
  - Teardown is step 6 of the approach: stop, join and destroy on the UI thread, or leak.
  - Bind one `wl_keyboard` per display, not per window: each bind makes the compositor resend
    keymaps to winit's keyboards too.
  - Track the newest key and button serial on P0's own `wl_keyboard`/`wl_pointer`. Expose it to
    P1 (`start_drag`) and P2 (`set_selection`).
  - Dependencies: wayland-client, wayland-backend (`client_system`), wayland-protocols
    (`client`, `staging`), wayland-protocols-wlr (`client`), x11rb (`xfixes`, `xinput`) and
    libc. All are already in Rune's tree under MIT or MIT/Apache-2.0; check `deny.toml`.
  - Tests: the peer in the prototype already round-trips three types each way (Q4, Q5, Q6). It
    is a starting point for W3's peer.
- **P1 (`P1-drag-and-drop.md`):**
  - **Source on Wayland:**
    - Start from Rune's `on_drag` at the threshold with P0's press serial. At the threshold,
      call P0's `dispatch_pending` synchronously on the UI thread before reading the serial,
      so a button event that has been read is applied. If the serial is still missing, start on
      the next wake rather than block on a read on the UI thread.
    - From then on, swallow that element's moves and its Up, and end the swallow on the next
      button event P0 sees.
    - Treat sway's `Up { cancelled: true }` after the drop as no-op, and ignore `cancelled`
      after `dnd_finished`.
    - Rune shows the element pressed for the whole drag; that needs R5.
  - **Source on X11:** grab-less; ignore the element's moves until the real Up. No cursor
    feedback.
  - **Target:**
    - Hit-test every motion with `tx.element_at`; positions are already Rune points.
    - Call `accept` and `set_actions` per motion.
    - Read on a worker thread; keep the offer after `leave`; `finish`.
  - **Copy or move by modifiers:** a drop target on sway gets neither modifiers nor a changed
    action, so it can't be Strata's rule there (probably not on Hyprland either). Make it a
    hand-test row for GNOME and KDE, and an owner question.
  - **Drag icon:** warm the GPU and renderer at attach; render from a copy of the element's
    state with `srgb_blend` on.
  - **In-process fast path:**
    - Offer a private MIME type carrying a per-drag token, such as
      `application/x-inprocess-drag;token=<pid>-<counter>`, and match it at `enter` against
      the drag this process started. The type is generic or supplied by the caller, with no
      Strata name (strata-platform's rule). Seeing the drag on our own data device isn't enough: another
      process's drag arrives there too.
    - X11: compare `XdndEnter`'s source window with our own window as well.
    - On a match, carry the typed payload instead of serialized URIs. Unverified: not built.
  - `XdndProxy` turns off Rune's X11 `on_drop_files`, so P1 must cover file drops on X11 too.
  - Nautilus's offer list (Q4) is the reference set for GTK sources.
- **P2 (`P2-clipboard.md`):**
  - Use ext-data-control, else zwlr; there are no other cases on today's compositors.
  - Mutter has neither. There:
    - the badge updates only on focus;
    - writes need the triggering event's serial;
    - a stale serial is dropped silently: detect it by the missing echo.
  - Cache writer output per source.
  - For `set_selection` from Ctrl+C, call P0's `dispatch_pending` synchronously before reading
    the newest key serial, as P1 does for the press serial.
  - Data-control needs no surface or serial, so P2's data-control path could run on its own
    connection, which would also work on libwayland 1.22 (the peer does exactly that). Only the
    `wl_data_device` fallback needs winit's display.
  - Rune's `tx.copy` and P2 coexist: Rune's write cancels ours, and Rune's read sees ours. On
    mutter, Rune's clipboard goes through Xwayland (hand test pending).
  - X11: INCR above 256 KiB; XFixes for the badge.
- **P4 (`P4-foreign-parent.md`):** `zxdg_importer_v2` binds on the same P0 backend; sway lists
  v1. S3's note that `dispatch_pending` from a Rune update is enough holds only for
  request/response traffic. Events that arrive unprompted need driver C.
- **S5 (`S5-upstream-and-release.md`):**
  - File R1–R5. R4 merges S1's R7 and S3's display-handle ask: file it once, with the union
    given under R4 below, and link all three records.
  - Optionally add to R4 an ask for a public every-turn hook (Rune's `about_to_wait` already
    drives its own overlays). With it, a shim could dispatch like Rune does, without driver C.
- **plan.md (owner's own PR):** §4 tier 1 says shims bind "the same way Rune binds
  `zwlr_layer_shell_v1`". True for binding. Rune dispatches its queue from `about_to_wait`,
  which shims can't reach, so P0 uses driver C. §8's DnD risk is retired except for the capture
  problem (R5), teardown order (P0 step 6) and libwayland 1.22.

## What P0, P1 and P2 still have to decide

- **P0:**
  - where the transfer handle lives: reached through an app-state accessor, as components take
    state, or a process-wide value per display. Teardown depends on it: `App::on_close` gets only
    `&mut S` and `&mut Tx` (step 6);
  - how P1, P2 and P4 subscribe to its events, given one reader and one waker per display;
  - the CI GPU route (lavapipe with pixman, untested here; see W3).
- **P1:** scale-2 drag icons (`set_buffer_scale`), the X11 drag-icon window, touch-started
  drags.
- **P2:** `PRIMARY` (data-control and X11), X11 `MULTIPLE`, mutter's behaviour (hand test).

## Open questions for the owner

1. **libwayland < 1.23 (Ubuntu 24.04 LTS ships 1.22).** S3 asked the same for P4. Ubuntu
   24.04's default session is GNOME on Wayland, and mutter has no data-control at all (Q5). So
   on a default 24.04 desktop P0 has neither winit's display (for drag and drop and the
   `wl_data_device` clipboard) nor data-control on a connection of its own. The options:
   - a. Report Unsupported for Wayland drag and drop there. For the clipboard, use data-control
     on P2's own connection where the compositor has it (sway, Hyprland, KWin). Otherwise use
     the X11 route through Xwayland: X11 selections on a second connection, which mutter bridges
     to Wayland clients, as arboard falls back to (designed, not run). Report Unsupported where
     neither exists.
   - b. Hold P1 and P2 until R4.
   - c. Read the private `wl_proxy` layout (it matched on 1.26).

   Recommendation: **a**. It keeps copy and paste of files working on default 24.04 through
   Xwayland, and loses only drag and drop there, for one LTS that 26.04 replaces. c is a private
   ABI and should not ship. Whether mutter's Xwayland bridge carries `x-special/gnome-copied-files`
   and `text/uri-list` faithfully is a hand test.

   The owner chose a, and P0 and P2 built it. **Superseded on 2026-11-10** by the owner's ruling
   on S3's decision 1: Ubuntu 24.04 is dropped and libwayland 1.23 is the minimum supported
   version. The fallbacks above stay in the code, but libwayland < 1.23 is an unsupported
   environment, and the 24.04 hand test is dropped.
2. **Copy or move by modifier** can't follow Ctrl and Shift on sway as a drop target, and
   probably not on Hyprland. Accept the compositor's action there, or ask Hyprland? Hand tests
   on GNOME and KDE decide the rest.
3. **Capture until R5:** ship P1's swallow workaround with Rune showing the element pressed
   during a drag, or hold P1's source half for R5?

## Skipped checks

- Hyprland, GNOME, KDE: not run. The owner's rule forbids touching the live session, and S1
  found Hyprland 0.56 can't nest in sway 1.12. Their cells are from docs and installed headers.
- Dolphin (not installed). Nautilus ran only on Wayland, not X11.
- lavapipe and the pixman renderer for CI: not installed here.
- A Scale-2 drag icon, the X11 drag icon window, X11 modifier actions, X11 `MULTIPLE`, a
  primary-selection round trip, and touch-started drags: not built.
- libwayland 1.22 was simulated by skipping the symbol, not run on a 1.22 system.
- Designed but not run: the fast-path token (P1), the X11 event thread's stop path and
  deleting `XdndProxy` at teardown, Escape to cancel an X11 drag, multi-window routing by
  comparing each kept `WindowHandle`'s current surface (and dropping CSD subsurfaces), teardown
  from `on_close`, fd timeouts, the read size cap and `EPIPE` handling, and P2's Xwayland route
  on mutter.

## Appendix: Rune issue drafts for S5

Each was run against the pinned rev. The sources are in the prototype's `repros/`.

Common setup: each program is `src/main.rs` of a binary crate whose only dependency is
`rune = { git = "https://github.com/anomalyco/rune", rev = "78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5", default-features = false, features = ["accessibility", "dbus", "embedded-fonts", "svg"] }`.
Where a compositor is needed, a headless sway (`WLR_BACKENDS=headless`) with a
`zwlr_virtual_pointer_v1` client to press and move is enough; no real session is involved.
The runs here used the prototype's peer. `wlrctl pointer move <dx> <dy>` and `wlrctl pointer
click` speak the same protocol and should do (untested here).

### R1 Linux: `drag_out` is ignored

Compositor: none for the headless part; a Wayland or X11 session (headless sway plus a virtual
pointer is enough) for the windowed part.

Steps:
1. Run the program below in a Wayland or X11 session.
2. Press the tile, drag it out of the window, release over a file manager.

Expected: as on macOS, a system drag with the item's data starts at the threshold, the picture
is a snapshot of the element, and `on_drag_start` and `on_drag_end` run.

Actual:
- Headless (`--script drag.json --trace t.json`), with this `drag.json`:
  ```json
  {"events": [{"t": 0.1, "type": "move", "x": 70, "y": 45}, {"t": 0.15, "type": "down", "x": 70, "y": 45}, {"t": 0.2, "type": "move", "x": 90, "y": 45}, {"t": 0.3, "type": "move", "x": 200, "y": 80}, {"t": 0.4, "type": "up", "x": 200, "y": 80}]}
  ```
  The trace records `drag-out #0/tile`, then the program prints `on_drag_start` and
  `on_drag_end Cancelled`.
- Windowed under sway: only `on_drag Down`, 20 × `on_drag Move`, `on_drag Up`. No system drag,
  no `on_drag_start` or `on_drag_end`.

Ask:
- `drag_out` on Wayland (`wl_data_device`) and X11 (XDND);
- `DragItem` with arbitrary MIME types and lazy per-type data;
- copy, move, link and ask;
- a public live-frame element snapshot (today `drag_out::snapshot` is `pub(crate)`).

```rust
use rune::{AppSpec, Cx, DragItem, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("tile").size(140.0, 90.0).fill(rune::oklch(0.55, 0.15, 260.0))
        .drag_out(|_: &S| Some(DragItem::new().text("hello from rune")))
        .on_drag_start(|_: &mut S, _| println!("on_drag_start"))
        .on_drag_end(|_: &mut S, outcome, _| println!("on_drag_end {outcome:?}"))
        .on_drag(|_: &mut S, d, _| println!("on_drag {:?}", d.phase)))
}
fn main() { rune::run(AppSpec::new("r1", (320.0, 200.0), S::default, view)); }
```

### R2 Wayland: drops reach no handler, and no handler takes MIME types

Compositor: needed (headless sway plus a virtual pointer is enough).

Steps: run the program below under sway. From another client, drag `text/uri-list`,
`text/plain;charset=utf-8` and `image/png` onto the zone, and drop. Any drag source works, for
example `dragon-drop <file>` (GTK 3) or `ripdrag <file>` (GTK 4). The run here used the
prototype's peer, which offers the three types. Neither tool was run here.

Expected:
- the drop reaches a handler with the offered types and a position on every motion;
- at least the file drop reaches `on_drop_files`, as it does on X11.

Actual on Wayland: nothing prints: no hover, no move, no drop. The source sees `target None`,
then `cancelled`.

On X11, winit answers XDND itself (`XdndActionPrivate`). It takes file URIs only, of files that
exist, and positions come from sampling `QueryPointer`.

Ask: `on_drop(|s, DropData, tx|)` with the offered types, a typed read, the action, and a
position per motion, on Wayland and X11.

```rust
use rune::{AppSpec, Cx, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("zone").size(300.0, 200.0).fill(rune::oklch(0.6, 0.15, 25.0))
        .on_drop_files(|_: &mut S, files, _| println!("on_drop_files {files:?}"))
        .on_drop_hover(|_: &mut S, over, _| println!("on_drop_hover {over}"))
        .on_drop_move(|_: &mut S, p, _| println!("on_drop_move {} {}", p.x, p.y)))
}
fn main() { rune::run(AppSpec::new("r2", (340.0, 240.0), S::default, view)); }
```

### R3 Clipboard: no custom MIME types, no image writes

Compositor: needed, with data-control for `wl-paste` (headless sway plus a virtual pointer is
enough).

Steps: run the program below under sway, click the tile, then run `wl-paste --list-types`.

Expected: a way to offer `text/uri-list`, `x-special/gnome-copied-files`,
`application/x-kde-cutselection` and `image/png`. File managers need these for copy and paste of
files.

Actual: `STRING UTF8_STRING text/plain text/html TEXT text/plain;charset=utf-8`.
`ClipboardContent` has text, HTML and RTF only, with no image or typed setter. Reads give no
type list. Nothing tells the app when another client takes the clipboard.

Ask:
- `ClipboardContent::with_type(mime, bytes)` (ideally lazy) and `with_image(png)`;
- a typed read with the offered type list;
- an ownership-change callback.

```rust
use rune::{AppSpec, ClipboardContent, Cx, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("copy").size(140.0, 90.0).fill(rune::oklch(0.6, 0.15, 140.0))
        .on_press_tx(|_: &mut S, tx| {
            tx.copy_rich(ClipboardContent::new("file:///tmp/a.txt").with_html("<a href=\"file:///tmp/a.txt\">a.txt</a>"));
            // Not expressible: .with_type("text/uri-list", ..), .with_image(png)
        }))
}
fn main() { rune::run(AppSpec::new("r3", (320.0, 200.0), S::default, view)); }
```

### R4 `WindowHandle` has no display handle

Compositor: any Wayland session, or Xvfb for X11. No input needed.

This is one issue for three records; file it once. The union of the asks:
- **The display handle** (S1's R7, S3, S2): `WindowHandle::with_raw_display_handle(|d| ..)`,
  giving winit's `wl_display` on Wayland and its Xlib display or xcb connection on X11.
- **A lifetime guarantee or drop hook** (S2): how long that display lives, or a callback on the
  UI thread before winit closes it. Shims must destroy their objects first (P0 step 6), and today
  they must guess the moment from `on_close` and their own quit path.
- Optionally (S2), a public every-turn hook such as Rune's internal `about_to_wait`, so a shim
  can dispatch its queue the way Rune's own overlays do.

S3's hook that runs before an X11 window is mapped stays in S3's own issue (foreign parenting);
it isn't part of this one.

Steps: run the program below on Wayland and on X11.

Expected: a raw display handle next to the window handle, plus a guarantee of how long it
lives or a hook before it is dropped.

Actual:
- Wayland: `Ok("Wayland(WaylandWindowHandle { surface: 0x… })")`, nothing else. P0 must
  `dlsym` `wl_proxy_get_display`, which is absent on libwayland 1.22.
- X11: `Ok("Xlib(XlibWindowHandle { window: …, visual_id: … })")`, nothing else. Shims open a
  second connection, which can't release winit's pointer grab.

Ask: the three items above. The canonical text of the display-handle ask is S1's
[R7](S1-linux-window.md#r7-public-access-to-the-native-display-connection); this draft adds the
lifetime hook and the optional every-turn hook to it.

```rust
use rune::{App, AppSpec, Cx, El, col};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El { col().w_full().h_full() }
fn main() {
    App::new(AppSpec::new("r4", (320.0, 200.0), S::default, view))
        .on_window(|w| println!("{:?}", w.with_raw_handle(|h| format!("{:?}", h.as_raw()))))
        .run();
}
```

### R5 No public way to end a pointer capture

Compositor: none; it runs headless. Run the program below.

Steps: press an `on_drag` element and drag past the threshold. The pointer then leaves without a
release (a Wayland compositor's `wl_pointer.leave` when another drag starts; any system that
takes the pointer mid-press). It comes back with no button held.

Expected: the app can end the capture, so the element gets a cancelled Up and nothing stays
pressed. Rune does this itself for macOS `drag_out` (`start_drag_out`).

Actual:
```
pressing after the drag: true
on_drag phases: [(Down, false), (Move, false), (Move, false)]
after window_focus(false): pressing false, phases [(Down, false), (Move, false), (Move, false), (Up, true)]
```
`Runtime::cancel_pointer` is `pub(crate)`, and a windowed app has no `Runtime`. Only a focus loss
ends the capture: sway happens to send one at the drop, other compositors may not.

Ask: `Tx::release_pointer()` (or `AppHandle`/`WindowHandle::cancel_pointer`) that delivers the
cancelled Up and clears press and hover.

```rust
use rune::{Cx, Drag, El, Env, Phase, Runtime, col, el};
#[derive(Default)] struct S { phases: Vec<(Phase, bool)> }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("tile").size(100.0, 60.0)
        .on_drag(|s: &mut S, d: &mut Drag, _| s.phases.push((d.phase, d.cancelled()))))
}
fn main() {
    let mut rt = Runtime::new(S::default(), view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let (x, y) = rt.find("tile").unwrap();
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 12.0, y); // the app starts a system drag here
    rt.pointer_leave(); // the compositor takes the pointer; no release follows
    rt.pointer_move(x + 40.0, y + 20.0); // back, no button held
    rt.frame(0.0);
    println!("pressing after the drag: {}", rt.pressing());
    println!("on_drag phases: {:?}", rt.state.phases);
    rt.window_focus(false); // the only public path that ends it
    rt.frame(0.0);
    println!("after window_focus(false): pressing {}, phases {:?}", rt.pressing(), rt.state.phases);
}
```
