# S5 Rune issues, crate name and release workflow

| | |
| --- | --- |
| Work package | [S5](../work/phase-0-groundwork/S5-upstream-and-release.md) |
| Rune | Pinned `78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5` (plan §0). Gaps re-checked at `dev` `17adeded5ae2e1bed2de03a015eaeaa9e00b93f5` (2026-11-07, 11 commits later) in a read-only clone. |
| Upstream snapshot | Rune's GitHub issues and crates.io, read on 2026-11-07. Both change daily: re-check before acting. |
| Repros | Headless repros compiled against `78d22ed` and run here, from a scratch crate (never merged). Compositor repros keep the S1–S3 measurements and weren't re-run. |
| Not done here | Nothing was filed, posted, reserved, tagged or pushed. Every issue below is a draft for the owner, and the release workflow hasn't run on GitHub. |

> **Filing packet (2026-11-10).** Every draft below, and those the later work packages wrote,
> is now in [`docs/upstream/rune/`](../upstream/rune/README.md): one ready-to-paste file per issue or
> comment, ranked by value, de-duplicated against upstream at `dev` `04b38ae`, with a mapping
> from each draft id to its entry. File from the packet; this record keeps the history.

## Verdicts

- **M0, Rune half.** None of P1–P5 is "Rune shipped" at `78d22ed` or at `dev` head. Each is
  "shim here", P3 only in part:

  | Module | Decision | Issue drafts it cites | Conditions, from the spikes |
  | --- | --- | --- | --- |
  | P1 `dnd` | Shim here, on P0 | [U1](#u1), [U2](#u2), [U9](#u9), [U10](#u10); upstream [#171](https://github.com/anomalyco/rune/issues/171) | libwayland ≥ 1.23. Wayland drag and drop is unsupported on Ubuntu 24.04 (S2 owner question 1). The capture workaround stays until U10. |
  | P2 `clipboard` | Shim here | [U5](#u5), [U9](#u9) | data-control on its own connection; `wl_data_device` fallback on mutter |
  | P3 `csd` | Shim here, in part | [U6](#u6), [U7](#u7), [U8](#u8), [U9](#u9); upstream [#145](https://github.com/anomalyco/rune/issues/145) | The platform-neutral part now. The native frame stays on Wayland until U6–U8. X11 borderless only if the mutter/KWin hand test passes. Rests on S1 owner question 1. **Superseded by the owner's P3 decision:** `Policy::Auto` is borderless on Wayland and X11, with no Wayland resize, minimise or window menu until U6–U8; the mutter/KWin hand test no longer gates X11. |
  | P4 `foreign-parent` | Shim here | [U11](#u11), [U9](#u9) | libwayland ≥ 1.23: the owner dropped Ubuntu 24.04 (S3 decision 1, ruled 2026-11-10). Below 1.23 it opens parentless, unsupported |
  | P5 `settings` | Shim here, over the Settings portal with zbus | [U12](#u12), [U13](#u13) | — |

  M0's "with an issue link" is **pending**: the links exist once the owner files the drafts.
- **Upstream already tracks some of these.** [#171](https://github.com/anomalyco/rune/issues/171)
  is Linux drag and drop for file paths. [#145](https://github.com/anomalyco/rune/issues/145)
  (patch offered in [#233](https://github.com/anomalyco/rune/issues/233)) is diagonal resize
  cursors. [#114](https://github.com/anomalyco/rune/issues/114) and
  [#226](https://github.com/anomalyco/rune/issues/226) ask for installed fonts by name, which
  `dev` `99c123d` (#245) now does. [#22](https://github.com/anomalyco/rune/issues/22) is Linux
  single instance, and [#30](https://github.com/anomalyco/rune/issues/30) Linux `Player`. The
  drafts drop those parts and link the issues.
- **U4 is withdrawn.** Held modifiers do reach a headless file drop at `78d22ed`
  ([U4](#u4)).
- **Corrections to the spike records:**
  - S2 R3 says `ClipboardContent` has "text, HTML and RTF only, with no image or typed setter".
    At `78d22ed` it also has `with_image` and `with_files`, and Linux writes one of them per copy:
    the files as `text/uri-list`, else the image as `image/png`, else HTML, else text
    (`clipboard/native.rs:136–150`; arboard 3.7.0). Rune's README still says "Copies don't write
    images yet" (README line 1283). [U5](#u5) asks for what is really missing.
  - S1 R1's ask for diagonal cursors is upstream #145, so [U6](#u6) leaves it out.
- **The crate keeps the name `strata-platform`** (the owner's ruling on question 1).
  `strata-platform` is taken on crates.io by an unrelated crate, so this crate can never be
  published there under it, and releases are git tags only. The options S5 weighed are under
  [Crate name](#crate-name).
- **`rune` is taken on crates.io too**, by the rune-rs scripting language (v0.14.2). Anomaly's
  Rune would need another name there, so plan §8's "publish once Rune is on crates.io" is
  further off than it reads.
- **Release workflow:** `.github/workflows/release.yml`, with tags
  `strata-platform-v<semver>+rune.<rev7>` and a draft prerelease. See
  [Release workflow](#release-workflow). The local checks pass; the GitHub run is pending.

## Upstream today

Open Rune issues that touch this repository, as of 2026-11-07:

| Issue | What it asks | Here |
| --- | --- | --- |
| [#171](https://github.com/anomalyco/rune/issues/171) | Linux file drops on Wayland, and `drag_out` on X11 and Wayland, for file paths | P1. U1 and U2 go beyond it (MIME types, actions, positions, snapshots) and link it. |
| [#145](https://github.com/anomalyco/rune/issues/145), [#233](https://github.com/anomalyco/rune/issues/233) | Diagonal resize cursors (#233 offers a patch) | P3 cites #145. No draft. |
| [#114](https://github.com/anomalyco/rune/issues/114), [#226](https://github.com/anomalyco/rune/issues/226) | Installed fonts by family name, fonts added while running | K2. Names resolve at `dev` `99c123d` (#245); both issues are still open. U16 asks for the list. |
| [#22](https://github.com/anomalyco/rune/issues/22) | Linux open-URL and reopen via `org.freedesktop.Application`, single instance | P8 cites it. Not in Rune at `78d22ed` or `dev` head. No draft. |
| [#30](https://github.com/anomalyco/rune/issues/30), [#100](https://github.com/anomalyco/rune/issues/100), [#112](https://github.com/anomalyco/rune/issues/112) | Linux `Player` | P7 cites #30 as related. P7 is an audio sink and clock, not `Player`. |
| [#23](https://github.com/anomalyco/rune/issues/23) | DMA-BUF for `NativeSurface`, a GL backend without Vulkan | S1's lavapipe-only note for plan §8 |
| [#139](https://github.com/anomalyco/rune/issues/139) | A covered Wayland window's `poll` and timers stop | A risk for P0's driver C and P9, which wake the UI through `AppHandle` |
| [#203](https://github.com/anomalyco/rune/issues/203) | `virtual_rows` costs O(rows) a frame when the row count changes | C1e and C3, at 100k entries |
| [#130](https://github.com/anomalyco/rune/issues/130) | Windows: frameless windows can't be moved or resized | The Windows twin of U6 |
| [#21](https://github.com/anomalyco/rune/issues/21) | App icons off macOS: freedesktop icon-theme lookup and SVG rasterizing, through `ImageCache` (PR #29 closed unmerged) | P6 cites it. Added by P6, after S5's survey missed it; U-P6-1 is a comment on it. |

## Draft index

Each draft is written for "a file manager on Rune" and doesn't name Strata. The owner may add
the name. U9 goes first, since five others link it.

| ID | Title | Source | Cited by | Compositor | Upstream | Filing | Filed as |
| --- | --- | --- | --- | --- | --- | --- | --- |
| [U9](#u9) | `WindowHandle` has no display handle, no lifetime hook and no every-turn hook | S1 [R7](S1-linux-window.md#r7-public-access-to-the-native-display-connection) (canonical), S2 [R4](S2-transfer.md#r4-windowhandle-has-no-display-handle), S3 [display ask](S3-foreign-parent.md#draft-rune-issue-for-s5) | P0, P1, P2, P3 (X11), P4 | Any Wayland session, or Xvfb; no input | New | 1 | — |
| [U1](#u1) | Linux `drag_out` with several MIME types, actions and an element snapshot | S2 [R1](S2-transfer.md#r1-linux-drag_out-is-ignored) | P1, P0, X2 | Headless trace, plus any session | New; extends #171 | 2 | — |
| [U2](#u2) | Wayland drops reach no handler; drops with any MIME type, the action and a position | S2 [R2](S2-transfer.md#r2-wayland-drops-reach-no-handler-and-no-handler-takes-mime-types) | P1, X2 | Yes | New; extends #171 | 3 | — |
| [U10](#u10) | No public way to end a pointer capture | S2 [R5](S2-transfer.md#r5-no-public-way-to-end-a-pointer-capture) | P1 | No | New | 4 | — |
| [U5](#u5) | Clipboard: custom MIME types, several forms per copy, typed reads, ownership changes | S2 [R3](S2-transfer.md#r3-clipboard-no-custom-mime-types-no-image-writes), corrected | P2, X3 | Yes, with data-control | New | 5 | — |
| [U6](#u6) | Linux: no interactive resize from a borderless window, and no maximised state | S1 [R1](S1-linux-window.md#r1-linux-no-way-to-start-an-interactive-resize-from-a-borderless-window) | P3 | Yes | New; links #145, #130 | 6 | — |
| [U7](#u7) | `Tx::minimize` is ignored for borderless windows | S1 [R2](S1-linux-window.md#r2-txminimize-is-ignored-for-borderless-windows) | P3 | Any | New | 7 | — |
| [U8](#u8) | No window menu on Linux, and no `wm_capabilities` | S1 [R3](S1-linux-window.md#r3-no-window-menu-on-linux) | P3 | GNOME or KDE for the expected result | New | 8 | — |
| [U11](#u11) | Make a window transient for another app's window, applied before it maps | S3 [draft](S3-foreign-parent.md#draft-rune-issue-for-s5) | P4 | Yes | New; links U9 | 9 | — |
| [U12](#u12) | Settings portal keys beyond `color-scheme` | S1 [R5](S1-linux-window.md#r5-settings-portal-keys-beyond-color-scheme) | P5, K2, C1b | Yes, plus a private bus | New | 10 | — |
| [U13](#u13) | The double-click interval is fixed at 0.5 s on Linux | S1 [R4](S1-linux-window.md#r4-the-double-click-interval-is-fixed-at-05-s-on-linux) | P5, C1b, P3 | No | New | 11 | — |
| [U14](#u14) | Popup surfaces for menus: submenus run off the window | S1 [R6](S1-linux-window.md#r6-popup-surfaces-for-menus) | C21, C22, the chooser's minimum size | No | New | 12 | — |
| [U15](#u15) | Virtual rows are vertical only: no grid or horizontal virtualization | S5 | C3 | No | New; related #203 | 13 | — |
| [U3](#u3) | File drops: acceptance can't see app state or the whole drag, and no drag identity or action | W1 ([`contract/dnd/binder.rs`](../../crates/strata-components/src/contract/dnd/binder.rs)) | W1's `Dnd::rune` binder (C1e, C6–C8, X2) | No | New | 14 | — |
| [U16](#u16) | List the installed font families | S5 | K2 | No | Comment on #114 | 15 | — |
| [U17](#u17) | rune-kit: export the Combobox fuzzy matcher, with match positions | W1 ([`contract/matched.rs`](../../crates/strata-components/src/contract/matched.rs)) | W1's `contract::fuzzy` (C17, C19, C20c, C20d) | No | New | 16 | — |
| [U4](#u4) | ~~Headless file drops can't carry held modifiers~~ | W1 ([`contract/dnd/tests.rs`](../../crates/strata-components/src/contract/dnd/tests.rs)) | — | — | Withdrawn | — | — |

## Filing

For the owner. Rune takes no pull requests and asks for agent-written issues (README,
"Contributing").

1. Before filing, re-check each gap at Rune's newest `dev`, and search the issues for a
   duplicate filed since 2026-11-07. The [re-check table](#re-check-at-dev-head) lists what to
   grep.
2. File U9 first. Put its link where U1, U2, U5, U6 and U11 say "(U9's link)".
3. File the rest in the index's order. U16 is a comment on #114, not a new issue. U1 and U2 are
   new issues that link #171. Commenting on #171 instead is an owner question.
4. Fill the index's "Filed as" column, and the README's
   [feature table](../../crates/strata-platform/README.md#features).
5. Each P-module's owner then replaces its `mod.rs` placeholder, `Rune issue: (filed by S5).` or
   (P9's) `Rune issue: S5 drafts.`, with the links from this index. P0 cites U9, P1 U1, U2, U9 and U10, P2 U5 and U9, P3 U6–U8, U9 and
   #145, P4 U11 and U9, P5 U12 and U13, P6 #21 (related), U-P6-1 and U-P6-2, P7 #30 (related), P8 #22, P9 "none:
   outside Rune's scope".

Each draft below gives a title, then the issue body to paste. Bracketed notes such as "(U9's
link)" are placeholders to fill.

## Drafts

Common environment for every draft: Rune `dev` at `78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5`
(winit 0.30.13, Rust 1.99.1), Arch Linux, kernel 7.2, NVIDIA RTX 5090. "Compositor" means any
Wayland or X11 session. A private headless sway is enough everywhere unless a draft says
otherwise, with a `zwlr_virtual_pointer_v1` client to press and move (`wlrctl pointer` speaks
it). Each gap is still present at `dev` `17adede`.

The headless sway recipe, for drafts that need one:

```sh
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
printf 'xwayland force\nexec echo $DISPLAY > %s/display\n' "$XDG_RUNTIME_DIR" > "$XDG_RUNTIME_DIR/sway.conf"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=gles2 \
  setpriv --no-new-privs sway -c "$XDG_RUNTIME_DIR/sway.conf" &
sleep 2
WAYLAND_DISPLAY=wayland-1 cargo run                                         # Wayland
env -u WAYLAND_DISPLAY DISPLAY=$(cat "$XDG_RUNTIME_DIR/display") cargo run  # X11 (Xwayland)
```

`setpriv` keeps a `cap_sys_nice` sway from going realtime, where a zero `RLIMIT_RTTIME` gets it
killed.

Unless a draft says otherwise, each program is `src/main.rs` of a binary crate whose only
dependency is:

```toml
rune = { git = "https://github.com/anomalyco/rune", rev = "78d22ed8eef60d76ac3f322ffc5883f3fcbc57f5", default-features = false, features = ["accessibility", "dbus", "embedded-fonts", "svg"] }
```

### U9

**Title:** Linux: `WindowHandle` gives no display handle, no warning before the display closes, and no every-turn hook

A file manager on Rune needs Wayland protocols Rune doesn't bind (`wl_data_device`,
`zxdg_importer_v2`, data-control) and X11 requests on winit's connection. Rune's own
`platform/activation.rs` binds a protocol on winit's display, but apps can't reach that display.

**Steps.** Run the program below on Wayland and on X11 (Xvfb is enough; no input is needed).

```rust
use rune::{App, AppSpec, Cx, El, col};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El { col().w_full().h_full() }
fn main() {
    App::new(AppSpec::new("u9", (320.0, 200.0), S::default, view))
        .on_window(|w| println!("{:?}", w.with_raw_handle(|h| format!("{:?}", h.as_raw()))))
        .run();
}
```

**Expected.** A raw display handle next to the window handle, and a stated lifetime for it.

**Actual.**
- Wayland: `Ok("Wayland(WaylandWindowHandle { surface: 0x… })")` and nothing else
  (`window/handle.rs:133`). The display can be recovered from the surface with
  `wl_proxy_get_display`, through unsafe FFI, and only on libwayland 1.23 or later. Ubuntu 24.04
  ships 1.22.
- X11: `Ok("Xlib(XlibWindowHandle { window: …, visual_id: … })")` and nothing else. A second X
  connection can't release winit's pointer grab, which `_NET_WM_MOVERESIZE` needs (EWMH).
- Nothing says when winit closes its display. Objects bound on it and dropped after `run`
  returns crashed the process in 5 of 5 runs (`wl_proxy_destroy` from the `Backend`'s drop).
- A second event queue on winit's display needs dispatching on every loop turn. Rune does that
  for its own overlays from `about_to_wait`, but an app has no such hook. A thread that only
  polls the fd races winit's reads: in the runs, events waited 123–279 ms (p99) for an unrelated
  wake.

**Ask.**
- `WindowHandle::with_raw_display_handle(|d| ..)`: winit's `wl_display` on Wayland, and its Xlib
  display or xcb connection on X11.
- A guarantee of how long that display lives, or a callback on the UI thread before winit closes
  it.
- Optionally, a public every-turn hook like the internal `about_to_wait`, so an app can dispatch
  its queue the way Rune's overlays do.

**Meanwhile** we recover the Wayland display with `dlsym("wl_proxy_get_display")`, run a reader
thread that joins libwayland's read protocol, and open a second X connection.

### U1

**Title:** Linux: `drag_out` does nothing; support several MIME types, actions and an element snapshot

Related: #171 (file drops on Wayland and `drag_out` on Linux, for file paths). This asks for the
same drag source with typed data. A file manager drags `text/uri-list` beside
`x-special/gnome-copied-files`, `application/x-kde-cutselection` and an in-process id, and needs
copy, move, link and ask.

**Compositor:** none for the headless part; any session for the windowed part.

**Steps.**
1. Headless: run the program below with `--script drag.json --trace t.json`.
2. Windowed: run it, press the tile, drag it out of the window and release over a file manager.

```json
{"events": [{"t": 0.1, "type": "move", "x": 70, "y": 45}, {"t": 0.15, "type": "down", "x": 70, "y": 45}, {"t": 0.2, "type": "move", "x": 90, "y": 45}, {"t": 0.3, "type": "move", "x": 200, "y": 80}, {"t": 0.4, "type": "up", "x": 200, "y": 80}]}
```

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
fn main() { rune::run(AppSpec::new("u1", (320.0, 200.0), S::default, view)); }
```

**Expected.** As on macOS: a system drag starts at the threshold with the item's data, its
picture is a snapshot of the element, and `on_drag_start` and `on_drag_end` run.

**Actual.**
- Headless: the trace records `drag-out #0/tile`, and the program prints `on_drag_start` and
  `on_drag_end Cancelled`.
- Windowed under sway: only `on_drag Down`, 20 × `on_drag Move` and `on_drag Up`. No system drag,
  no `on_drag_start` or `on_drag_end`. `drag_out::SUPPORTED` is `cfg!(target_os = "macos")`.

**Ask.**
- `drag_out` on Wayland (`wl_data_device`) and X11 (XDND).
- `DragItem` with arbitrary MIME types and lazy data per type.
- Copy, move, link and ask, and the outcome.
- A public snapshot of an element from the live frame for the drag icon (`drag_out::snapshot`
  is `pub(crate)`).

**Meanwhile** we start a `wl_data_device` drag from `on_drag`, on winit's display (U9's link),
and render the icon from a second headless `Runtime`.

### U2

**Title:** Wayland: drops reach no handler; take drops with any MIME type, the action and a position on every motion

Related: #171. A file manager takes `text/uri-list`, `text/plain`, `text/x-moz-url`, `image/png`
and HTML drops, and picks copy or move per target and per modifier, so it needs more than file
paths.

**Compositor:** needed.

**Steps.** Run the program below under sway. From another client, drag `text/uri-list`,
`text/plain;charset=utf-8` and `image/png` onto the zone and drop. This GTK 4 drag source
(PyGObject) offers exactly those three types:

```python
# A drag source offering text/uri-list, text/plain and image/png (PyGObject, GTK 4).
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk

PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d4948445200000001000000010806000000"
    "1f15c4890000000d49444154789c63f8cfc0f01f00050001ff89993d1d0000000049454e44ae426082")

def provider():
    def item(mime, data):
        return Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(data))
    return Gdk.ContentProvider.new_union([
        item("text/uri-list", b"file:///tmp/a.txt\r\n"),
        item("text/plain;charset=utf-8", b"hello from a drag"),
        item("image/png", PNG),
    ])

def activate(app):
    win = Gtk.ApplicationWindow(application=app, title="drag source")
    label = Gtk.Label(label="drag me")
    label.set_size_request(200, 120)
    source = Gtk.DragSource(actions=Gdk.DragAction.COPY | Gdk.DragAction.MOVE)
    source.connect("prepare", lambda *_: provider())
    source.connect("drag-cancel", lambda _s, _d, reason: print("drag-cancel", reason.value_nick) or False)
    source.connect("drag-end", lambda _s, _d, delete: print("drag-end, delete", delete))
    label.add_controller(source)
    win.set_child(label)
    win.present()

if __name__ == "__main__":
    app = Gtk.Application(application_id="org.example.DragSource")
    app.connect("activate", activate)
    app.run(None)
```

`dragon-drop` and `ripdrag` don't do for this: dragon offers only URI and text targets, and
ripdrag only a file list. Neither offers `image/png`.

```rust
use rune::{AppSpec, Cx, El, col, el};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("zone").size(300.0, 200.0).fill(rune::oklch(0.6, 0.15, 25.0))
        .on_drop_files(|_: &mut S, files, _| println!("on_drop_files {files:?}"))
        .on_drop_hover(|_: &mut S, over, _| println!("on_drop_hover {over}"))
        .on_drop_move(|_: &mut S, p, _| println!("on_drop_move {} {}", p.x, p.y)))
}
fn main() { rune::run(AppSpec::new("u2", (340.0, 240.0), S::default, view)); }
```

**Expected.** The drop reaches a handler with the offered types and a position on every motion.
At least the file drop reaches `on_drop_files`, as on X11.

**Actual.**
- Wayland: the Rune program prints nothing: no hover, no move, no drop. Nothing accepts the
  drag. In the measured run the source was a raw `wl_data_device` client offering the same
  three types (not the script above): it got `target` with no type while over the window, then
  `cancelled` at the drop. With the script above, `WAYLAND_DEBUG=1` should show the same pair,
  `wl_data_source.target(nil)` then `wl_data_source.cancelled()`, and the script should print
  `drag-cancel no-target`, then `drag-end, delete False`. The script hasn't been run against
  the Rune window.
- X11: winit answers XDND itself (`XdndActionPrivate`). It takes file URIs only, of files that
  exist, and positions come from sampling `QueryPointer`.

**Ask.** `on_drop(|s, DropData, tx|)` with the offered types, a typed read, the action, and a
position on every motion, on Wayland and X11. The file-drop hooks' own gaps, on every
platform, are a separate issue: (U3's link).

**Meanwhile** we bind `wl_data_device` on winit's display (U9's link) and put an `XdndProxy` on
the X11 window.

### U10

**Title:** No public way to end a pointer capture when a system drag takes the pointer

When an app starts a system drag from a press, the compositor takes the pointer: winit gets a
`wl_pointer.leave` and never the release. Rune stays pressing, and `on_drag` keeps getting
moves with no button held. Rune ends the capture itself for macOS `drag_out`
(`start_drag_out`), but apps can't.

**Compositor:** none; it runs headless.

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

**Expected.** The app can end the capture: the element gets a cancelled Up, and nothing stays
pressed.

**Actual** (run at `78d22ed`):

```
pressing after the drag: true
on_drag phases: [(Down, false), (Move, false), (Move, false)]
after window_focus(false): pressing false, phases [(Down, false), (Move, false), (Move, false), (Up, true)]
```

`Runtime::cancel_pointer` is `pub(crate)`, and a windowed app has no `Runtime`. Only a focus loss
ends the capture. sway happens to send one at the drop; other compositors may not.

**Ask.** `Tx::release_pointer()`, or `WindowHandle::cancel_pointer()`, that delivers the
cancelled Up and clears press and hover.

### U5

**Title:** Linux clipboard: custom MIME types, several forms in one copy, typed reads, and a notice when another app takes it

A file manager's copy and cut need `text/uri-list` beside `x-special/gnome-copied-files` and
`application/x-kde-cutselection` (which carry the copy-or-cut choice), plus text for editors, in
one selection. Its paste needs to know which types are offered, and its cut badge needs to know
when another app takes the clipboard.

**Compositor:** needed, with data-control for `wl-paste`.

**Steps.** Run the program below under sway, click each tile, and run `wl-paste --list-types`
after each.

```rust
use rune::{AppSpec, ClipboardContent, Cx, El, col, el};
use std::path::PathBuf;
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full()
        .child(el().key("html").size(140.0, 60.0).fill(rune::oklch(0.6, 0.15, 140.0))
            .on_press_tx(|_: &mut S, tx| tx.copy_rich(ClipboardContent::new("file:///tmp/a.txt")
                .with_html("<a href=\"file:///tmp/a.txt\">a.txt</a>"))))
        .child(el().key("files").size(140.0, 60.0).fill(rune::oklch(0.6, 0.15, 260.0))
            .on_press_tx(|_: &mut S, tx| tx.copy_rich(ClipboardContent::new("/tmp/a.txt")
                .with_files([PathBuf::from("/tmp/a.txt")]))))
    // Not expressible: .with_type("x-special/gnome-copied-files", b"cut\nfile:///tmp/a.txt"),
    // or files and text in one copy.
}
fn main() { rune::run(AppSpec::new("u5", (320.0, 200.0), S::default, view)); }
```

**Expected.** One copy can offer `text/uri-list`, `x-special/gnome-copied-files`,
`application/x-kde-cutselection` and text together.

**Actual.**
- The HTML copy lists `STRING UTF8_STRING text/plain text/html TEXT text/plain;charset=utf-8`
  (measured under sway).
- On Windows and Linux a copy holds one form: the files, else the image, else HTML beside the
  text, else the text (`clipboard/native.rs`, `write_one_form`). So the file copy offers
  `text/uri-list` alone, with no GNOME or KDE type and no text (by source, through arboard 3.7.0;
  not run here).
- `ClipboardContent` has no setter for another MIME type, and a read gives no list of the
  offered types.
- Nothing tells the app when another client takes the clipboard.
- The README still says "Copies don't write images yet", and its Linux row lists text and HTML
  only, though `with_image` and `with_files` are written.

**Ask.**
- `ClipboardContent::with_type(mime, bytes)`, ideally with lazy data, and every form of a
  copy written together.
- A typed read with the list of offered types.
- A callback when the app loses the clipboard.
- On X11, every form of the app's last copy handed to the clipboard manager at exit
  (`SAVE_TARGETS`), as arboard already does for Rune's text: today only what Rune wrote
  survives the app, and a shim's copy can be handed over only by the shim's own connection.
- A way to learn that a write was dropped. On Wayland's `wl_data_device` a write whose serial
  is older than the current selection's is ignored without an error (wlroots; S2 Q5), and
  `Tx::copy` says nothing either way: a write result, or the loss callback firing for it.

**Meanwhile** we write the selection over data-control on our own connection, or over
`wl_data_device` on winit's display (U9's link), and use X11 selections on a second connection.
P2 (`strata-platform::clipboard`) detects a dropped data-device write by the missing echo
before a `wl_display.sync`, and hands its own X11 copy to the clipboard manager at shutdown.
Upstream was searched again at `dev` `5d421fd` (2026-11-08): no commit touches
`crates/rune/src/clipboard*` since the pin, and no issue asks for any of this.

### U6

**Title:** Linux: no way to start an interactive resize from a borderless window, or to read whether it is maximised

A file manager draws its own header and window buttons in a borderless window. On Linux it can't
resize from its edges, and can't tell whether to show maximise or restore. Diagonal cursors for
the corners are #145. #130 is the same gap on Windows.

**Compositor:** needed.

**Steps.**
1. Run the program below in headless sway, floating.
2. With a virtual pointer, press 3 px inside the right edge, move 100 px right, and release.

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
    App::new(AppSpec::new("u6", (640.0, 400.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

**Expected.** The window grows by 100 px, as winit's `Window::drag_resize_window(East)` does
(checked with plain winit 0.30.13 under the same harness, on Wayland and Xwayland).

**Actual.** Nothing can start the resize. `WindowHandle::with_raw_handle` yields the
`wl_surface` only, and `xdg_toplevel.resize` needs winit's toplevel and the button serial. No
API reports the maximised state either: there's only `Tx::toggle_maximized`, and
`Cx::chrome_insets()` is 0 on Linux.

**Ask.**
- `.window_resize(ResizeDirection)`, like `.window_drag()`, or `Tx::drag_resize_window`.
- The window's maximised state for the view, so resize bands and the maximise/restore button
  can follow it.

**Meanwhile** we keep the native frame on Wayland. On X11 we send `_NET_WM_MOVERESIZE` from a
second connection, which can't release winit's grab (U9's link).

> Not part of the issue body. **Superseded by the owner's P3 decision:** P3 doesn't keep the
> native frame on Wayland. `Policy::Auto` is borderless on Wayland and X11, and a borderless
> Wayland window has no edge resize, minimise or window menu until U6–U8. Before filing, change
> the "Meanwhile" line above to say so.

### U7

**Title:** `Tx::minimize` is ignored for borderless windows

**Compositor:** any; the message comes before any request reaches it.

**Steps.** Run the program below as a debug build and click "Minimise".

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
    App::new(AppSpec::new("u7", (400.0, 240.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

**Expected.** The window minimises where the compositor supports it (GNOME, KDE, X11 window
managers). Borderless windows are exactly the ones that draw their own minimise button.

**Actual.** `rune: minimize ignored: a window without a minimize button can't minimize`, and no
request is sent. The check is `self.options.borderless || …` in `window/shell_requests.rs:20`.

**Ask.** Send the minimise request for borderless windows, or let the app say its window has a
minimise button of its own.

### U8

**Title:** Linux: no window menu, and no way to read the compositor's `wm_capabilities`

**Compositor:** one with a window menu (GNOME, KDE) to see the expected result. sway and Hyprland
have none. The missing call is visible in the code.

**Steps.** Run the program below and right-click the header.

```rust
use rune::*;
struct S;
fn view(_: &S, _: &mut Cx<S>) -> El {
    // A bare drag region: on Windows a secondary click here opens the window menu.
    col().w_full().h_full()
        .child(row().key("header").h(40.0).w_full().window_drag().child(text("right-click me")))
}
fn main() {
    App::new(AppSpec::new("u8", (400.0, 240.0), || S, view))
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .run();
}
```

**Expected.** `xdg_toplevel.show_window_menu` on Wayland, and `_GTK_SHOW_WINDOW_MENU` on X11
where the window manager supports it, as Rune already does on Windows.

**Actual.** Nothing. `chrome::window_menu` (`window/chrome.rs:95`) is a no-op off Windows, and
winit's X11 `show_window_menu` is a no-op too.

**Ask.**
- The window menu on Linux.
- `xdg_toplevel.wm_capabilities`, so apps can hide buttons the compositor can't honour. sway
  advertises only fullscreen; Hyprland fullscreen and maximize.
- Optionally, the desktop's titlebar double-click action
  (`org.gnome.desktop.wm.preferences action-double-click-titlebar`), instead of always
  maximising (`window/chrome.rs:72–77`).

The maximised state is asked in U6.

### U11

**Title:** Linux: make a window transient for another app's window, applied before it maps

An XDG portal backend (a file chooser in a file manager) gets its caller's window as
`wayland:<xdg-foreign handle>` or `x11:<xid>`, and its dialog must be transient for that window:
xdg-foreign v2 `set_parent_of` on Wayland, `WM_TRANSIENT_FOR` on X11. Rune has no API for
either. Reaching winit's display is a separate gap: (U9's link).

Even with the display, the parent has to be set before the window maps:
- On X11, `App::on_window` runs after winit has mapped the window, unless it was created hidden.
- `WindowOptions { visible: false }` holds only on a window's first creation.
  `window/create.rs:13` computes `visible = !launcher && (self.started || self.options.visible)`,
  so a recreated window maps at once.

**Compositor:** needed. Headless tooling never creates a native window, and
`WindowSpec::on_created` isn't called headless (`windows.rs:231`). Use the sway recipe above.

The program opens a hidden window, shows it, closes it (which destroys it, because of
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
    let app = rune::App::new(spec).args(Vec::new()).keep_running().release_gpu_when_closed().window_options(hidden)
        .on_window(|w| {
            let n = CREATED.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = w.with_raw_handle(|h| match h.as_raw() {
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
`WindowOptions::transient_for(ForeignParent::parse("wayland:…"))`, applied before the window maps
on every creation, with an outcome saying whether the compositor accepted it. Failing that, a
hook that runs before every native window maps, X11 included.

**Actual.**
- Wayland: `creation 1/2: wayland surface 0x…`. No transient-for, and no display accessor.
- X11: `creation 1: … map_state UNMAPPED`, then `creation 2: … map_state VIEWABLE`. The second
  creation ignored `visible: false` and mapped before `on_window`, so a `WM_TRANSIENT_FOR` set
  there reaches the window manager after the map request.

**Also needed** (P4, amending the draft):
- **Modal on Wayland.** A portal's request is modal by default, and Hyprland, Mutter and KWin
  offer `xdg_wm_dialog_v1`, whose `get_xdg_dialog` takes the window's `xdg_toplevel`.
  `with_raw_handle` gives only the `wl_surface`, and nothing recovers the toplevel from it, so
  a shim can parent a Wayland window but not mark it modal. Ask for `modal` with the
  transient-for (`set_modal` where the compositor has the protocol), or for the toplevel.
- **A Rune window transient for another of the same app's windows**, such as a folder chooser
  floating modal over the window it was opened from. GTK apps get it from `set_transient_for`.
  A shim could reach it only by exporting its own window through `zxdg_exporter_v2` and
  importing that, on a display Rune doesn't expose (U9's link).

**Meanwhile** P4 (`strata-platform::foreign_parent`) imports on winit's display through P0's
substrate and sets the X11 hints on a connection of its own, from `App::on_window` of a window
opened hidden, one window per request, and reports when it came too late. X11 windows are
modal through `_NET_WM_STATE_MODAL`. Upstream was searched again at `dev` `af7971ae`
(2026-11-09, 108 commits past the pin): no commit touches `window/handle.rs` or
`window/create.rs`, and no issue asks for a transient-for, a dialog or a display handle.

### U12

**Title:** Linux: read Settings portal keys beyond `color-scheme`, with change signals

A file manager follows the desktop's text scale, double-click time, cursor blink and animations
switch. They all live in `org.freedesktop.portal.Settings`, which Rune already reads for
`color-scheme`. The double-click interval Rune uses itself is a separate issue: (U13's link).

**Compositor:** needed, with a private session bus, because Rune reads the portal only for a real
window.

**Steps.**
1. Save the fake portal below as `fake_portal.py`. It serves the keys it is given, emits
   `SettingChanged` on a schedule, and logs every call it gets as a JSON line.
2. Make a binary crate named `u12` with the program below (any Rune window will do), and build
   it first, so the build doesn't eat into the timed run.
3. Start the headless sway from the recipe above, then run the window on a private bus with the
   fake portal.

```rust
use rune::{App, AppSpec, Cx, El, col, text};
fn view(_: &(), _: &mut Cx<()>) -> El { col().w_full().h_full().child(text("portal keys")) }
fn main() { App::new(AppSpec::new("u12", (320.0, 160.0), || (), view)).run(); }
```

```sh
cargo build
dbus-run-session -- sh -c '
  python3 -I fake_portal.py \
    --init "org.freedesktop.appearance color-scheme u 1; org.gnome.desktop.interface text-scaling-factor d 1.0; org.gnome.desktop.interface cursor-blink b true; org.gnome.desktop.interface cursor-blink-time i 1200; org.gnome.desktop.interface enable-animations b true; org.gnome.desktop.peripherals.mouse double-click i 400" \
    --schedule "3 org.gnome.desktop.interface text-scaling-factor d 1.5; 3.5 org.gnome.desktop.interface enable-animations b false; 4 org.gnome.desktop.peripherals.mouse double-click i 1200; 4.5 org.freedesktop.appearance color-scheme u 2" \
    > portal.log &
  sleep 1
  WAYLAND_DISPLAY=wayland-1 timeout 6 ./target/debug/u12
'
grep '"ev": "call"' portal.log
```

```python
"""A fake org.freedesktop.portal.Settings on the session bus.

Usage: python3 -I fake_portal.py [--v1] [--schedule "T NS KEY TYPE VALUE; ..."] [--init "NS KEY TYPE VALUE; ..."]
--v1 drops ReadOne (portal version 1, Read only). TYPE is a GVariant type string.
Prints JSON lines: ready, each method call, each emitted change.
"""
import json
import sys
import time

from gi.repository import Gio, GLib

XML = """
<node><interface name="org.freedesktop.portal.Settings">
  <method name="ReadAll"><arg type="as" direction="in"/><arg type="a{sa{sv}}" direction="out"/></method>
  <method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="ReadOne"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal>
  <property name="version" type="u" access="read"/>
</interface></node>
"""

START = time.monotonic()
args = sys.argv[1:]
v1 = "--v1" in args


def opt(name):
    return args[args.index(name) + 1] if name in args else ""


def parse(spec):
    out = []
    for item in filter(None, (s.strip() for s in spec.split(";"))):
        out.append(item.split(None, 4))
    return out


def log(ev, **f):
    print(json.dumps({"t": round(time.monotonic() - START, 3), "w": int(time.time() * 1000), "ev": ev, **f}), flush=True)


values = {}
for ns, key, ty, val in parse(opt("--init")):
    values[(ns, key)] = GLib.Variant.parse(GLib.VariantType(ty), val, None, None)


def call(conn, sender, path, iface, method, params, inv):
    p = params.unpack()
    log("call", method=method, args=p, sender=sender)
    if method == "ReadAll":
        res = {}
        for (ns, key), v in values.items():
            if not p[0] or any(ns == want or (want.endswith("*") and ns.startswith(want[:-1])) for want in p[0]):
                res.setdefault(ns, {})[key] = v
        inv.return_value(GLib.Variant("(a{sa{sv}})", (res,)))
        return
    v = values.get((p[0], p[1]))
    if v is None:
        inv.return_dbus_error("org.freedesktop.portal.Error.NotFound", "no such key")
    elif method == "ReadOne" and not v1:
        inv.return_value(GLib.Variant.new_tuple(GLib.Variant("v", v)))
    elif method == "ReadOne":
        inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "ReadOne")
    else:
        # Read wraps the value in a second variant, as real portals do.
        inv.return_value(GLib.Variant.new_tuple(GLib.Variant("v", GLib.Variant("v", v))))


def get_prop(conn, sender, path, iface, name):
    return GLib.Variant("u", 1 if v1 else 2)


def acquired(conn, name):
    info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
    conn.register_object("/org/freedesktop/portal/desktop", info, call, get_prop, None)
    for at, ns, key, ty, val in parse(opt("--schedule")):
        GLib.timeout_add(int(float(at) * 1000), change, conn, ns, key, ty, val)


def owned(conn, name):
    log("ready", name=name, version=1 if v1 else 2)


def change(conn, ns, key, ty, val):
    v = GLib.Variant.parse(GLib.VariantType(ty), val, None, None)
    values[(ns, key)] = v
    conn.emit_signal(None, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "SettingChanged",
                     GLib.Variant("(ssv)", (ns, key, v)))
    log("changed", ns=ns, key=key, value=val)
    return False


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.portal.Desktop", Gio.BusNameOwnerFlags.NONE, acquired, owned,
                 lambda *_: log("name-lost"))
GLib.MainLoop().run()
```

**Expected.** These reach the app, as `color-scheme` does.

**Actual.**
- The fake portal logs one call from Rune, `ReadOne(org.freedesktop.appearance, color-scheme)`,
  and nothing else (`platform/portal_appearance.rs`). When the schedule sets `color-scheme` to 2,
  the window turns light, so Rune does follow that key's changes. No API exposes the other keys.
- Rune reads reduced motion from GSettings or `kdeglobals`, only when a window is created or
  focused. That skips the portal (sandboxes) and its change signals. Under the GSettings keyfile
  backend the re-read stayed stale, because that backend needs a GLib main loop.

**Ask.** The portal's values for those keys, or a generic `Settings` portal reader, with change
signals.

### U13

**Title:** Linux: the double-click interval is fixed at 0.5 s

**Compositor:** none; it reproduces with Rune's headless tooling. A live check under headless sway
with GNOME's `double-click` set to 1200 ms gives the same result.

**Steps.** Run the program below with `--script dbl.json --tree out.json --at 3`, once with `gap`
0.45 and once with 0.55, and read the `doubles` text in `out.json`.

```rust
use rune::*;
#[derive(Default)] struct S { doubles: u32 }
fn view(s: &S, _: &mut Cx<S>) -> El {
    row().key("dbl").w(200.0).h(60.0).child(text(format!("doubles {}", s.doubles)))
        .on_press(|_: &mut S| {}).on_double_press(|s: &mut S| s.doubles += 1)
}
fn main() { App::new(AppSpec::new("U13", (400.0, 200.0), S::default, view)).run(); }
// dbl.json: {"events": [{"t": 0.5, "type": "double-click", "key": "dbl", "gap": 0.55}]}
```

**Expected.** The desktop's interval, or at least a way for the app to set it. GNOME and GTK
default to 400 ms (`org.gnome.desktop.peripherals.mouse double-click`, also in the Settings
portal), and KDE keeps `DoubleClickInterval` in `kdeglobals` (also 400 ms by default).

**Actual** (run at `78d22ed`). Gap 0.45 gives `doubles 1`, and 0.55 gives `doubles 0`. Under
sway, presses 583–1033 ms apart never pair with the desktop set to 1200 ms.
`Runtime::double_click_interval` is public, but windowed apps never get `&mut Runtime`. macOS
and Windows read the system value when a window is created (`window/create.rs:86–92`); Linux
doesn't. The interval also drives `.window_drag()`'s double-click maximise, word and line
selection, and `Drag::clicks`.

**Ask.** Read the desktop's interval on Linux (the Settings portal, else GSettings, else
`kdeglobals`), or add `App::double_click_interval(secs)` and a `Tx` setter.

Related: (U12's link) asks for the Settings portal's other keys, `double-click` among them.

### U14

**Title:** Linux: popup surfaces for menus, since submenus run off the window

A file manager's context menu has an "Open With" submenu, and its file chooser is a small window.
In-window menus scroll to fit, but submenus only flip and shift: they never shrink or scroll.
`tx.popup_menu` is unsupported on Linux.

This program also depends on `rune-kit` at the same rev.

**Compositor:** none; it runs headless.

**Steps.** Run the program below with `--tree t.json --size 420x400 --at 1`, then again with
`--long` added, and read the `ctx-item1-sub-menu` rect.

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
fn main() { App::new(AppSpec::new("U14", (420.0, 400.0), || S, view)).run(); }
```

**Expected.** The submenu stays visible, as an `xdg_popup` would.

**Actual** (run at `78d22ed`):
- With 8 rows it sits at `[288, 90, 160, 228]`: 28 pt past the right edge of a window at Rune's
  own default minimum size.
- With 20 rows it shifts up to `[288, 8, 160, 564]`, and still runs 172 pt past the bottom.

**Ask.** Menus, submenus and anchored popovers as `xdg_popup` surfaces on Wayland (and
override-redirect windows on X11), or failing that, submenus that shrink and scroll as menus do.

### U15

**Title:** Virtual rows are vertical only: a tile grid mounts every tile, and `.virtualize` in a horizontal scroll stacks diagonally

A file manager's icon view shows tens of thousands of thumbnails in a wrapped grid. Rune's
README says "Virtual rows are vertical only". Packing N tiles per virtual row works, but the app
must then reflow rows on every width change. A Miller-columns view scrolls horizontally.

**Compositor:** none; it runs headless.

```rust
use rune::{Cx, El, Env, GrowthSpec, Runtime, ScrollSpec, col, el, row};
const TILES: usize = 20_000;
const PER_ROW: usize = 8;
fn tile(i: usize) -> El { el().key(format!("t{i}")).size(96.0, 96.0) }
fn wrapped(_: &(), _: &mut Cx<()>) -> El {
    col().key("scroll").size(800.0, 600.0).scroll(ScrollSpec { stick_bottom: false })
        .child(row().key("grid").w(800.0).flex_wrap().children((0..TILES).map(tile)))
}
fn packed(_: &(), _: &mut Cx<()>) -> El {
    col().key("scroll").size(800.0, 600.0).scroll(ScrollSpec { stick_bottom: false })
        .child(col().key("rows").w_full().growth(GrowthSpec::default())
            .virtual_rows(TILES / PER_ROW, 96.0, |_: &(), r| format!("r{r}").into(),
                |_: &(), r| row().h(96.0).children((0..PER_ROW).map(|c| tile(r * PER_ROW + c)))))
}
fn horizontal(_: &(), _: &mut Cx<()>) -> El {
    row().key("scroll").size(800.0, 600.0).scroll_x()
        .child(row().key("cols").h_full().growth(GrowthSpec::default()).virtualize(96.0)
            .children((0..TILES).map(tile)))
}
fn count(name: &str, view: fn(&(), &mut Cx<()>) -> El) {
    let mut rt = Runtime::new((), view, (800.0, 600.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let tiles: Vec<_> = rt.elements().into_iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with('t'))).collect();
    println!("{name}: {} of {TILES} tiles have elements", tiles.len());
    for e in tiles.iter().take(3) { println!("  {} at {:?}", e.key.as_deref().unwrap_or(""), e.rect); }
}
fn main() {
    count("wrapped grid", wrapped);
    count("8 tiles per virtual row", packed);
    count("virtualize in scroll_x", horizontal);
}
```

**Expected.** A virtualized grid (or a wrap-aware `virtual_rows`) that mounts only what's near
the viewport, and either horizontal virtualization or a clear error for `.virtualize` inside a
horizontal viewport.

**Actual** (run at `78d22ed`):
- Wrapped grid: 20000 of 20000 tiles have elements, at `[0, 0]`, `[96, 0]`, `[192, 0]`.
- 8 tiles per virtual row: 120 of 20000, at `[0, 0]`, `[96, 0]`, `[192, 0]`.
- `.virtualize` on a row inside `scroll_x`: 19 of 20000, at `[0, 0]`, `[96, 96]`, `[192, 192]`.
  The window is chosen as if the row were vertical, so each tile also moves down one estimate.
  The debug warning reads `virtualized list "scroll/cols" … built 20000 rows to mount 19`.

**Ask.** Grid virtualization (a fixed tile size and a column count from the width would do), and
horizontal virtualization, or a panic for `.virtualize` in a horizontal viewport, as there is
for one without `.growth`.

### U3

**Title:** File drops: acceptance can't see app state or the whole drag, and nothing says a drag entered, left or which action it gets

A file manager decides while a drag hovers whether a folder row takes it: not into a read-only
folder, not a folder onto itself, copy or move by the source's volume. Rune has two hooks for
this, and both fall short:

- `El::accepts_files(Fn(&Path) -> bool)` (`el/drop.rs:70`) is asked one path at a time, with no
  `&S`. It can't read app state (is this folder writable? is it one of the dragged entries?), and
  it can't judge the drag as a whole (refuse when any one of the paths is the target itself).
- `El::on_drop_rejected_hover` (`el/drop.rs:123`) only says "over" or "not over", for a drag
  `accepts_files` already turned away.

Beyond those:

- No drag identity, and no enter or leave for the window. Two drags in a row can't be told apart,
  and an app can't set up or tear down a drag session.
- The action is never asked. The drop hands over paths with no copy or move, so an app can only
  apply its own rule at the drop, and can't show it while hovering.

The paths themselves are only in the drop: `on_drop_hover` and `on_drop_move` don't get them.

Related: (U2's link) asks for typed, multi-MIME drops on Wayland and X11. This issue is about the
file-drop hooks every platform already has.

**Compositor:** none; it runs headless.

```rust
use rune::{Cx, El, Env, Runtime, col, el};
use std::path::PathBuf;
#[derive(Default)] struct S { log: Vec<String>, read_only: bool }
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().w_full().h_full().child(el().key("zone").size(200.0, 150.0)
        // Asked per path, with no state: it can't see `read_only`, or the other paths.
        .accepts_files(|path| { println!("accepts_files({})", path.display()); true })
        .on_drop_hover(|s: &mut S, over, _| s.log.push(format!("on_drop_hover({over})")))
        .on_drop_move(|s: &mut S, p, _| s.log.push(format!("on_drop_move({}, {})", p.x, p.y)))
        .on_drop_files(|s: &mut S, files, _| s.log.push(format!("on_drop_files({files:?}), read_only {}", s.read_only))))
}
fn main() {
    let mut rt = Runtime::new(S { read_only: true, ..S::default() }, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let p = vec![PathBuf::from("/home/me/a.txt"), PathBuf::from("/home/me/b.png")];
    rt.drag_files(p.clone(), Some((10.0, 10.0)));
    rt.frame(0.02);
    rt.drag_files_to(Some((60.0, 70.0)));
    rt.frame(0.04);
    rt.drop_files(p, Some((60.0, 70.0)));
    rt.frame(0.06);
    for l in &rt.state.log { println!("{l}"); }
}
```

**Expected.** The app can decide acceptance from its state and the whole drag while it hovers,
tell one drag from the next, and say which action the drop would take.

**Actual** (run at `78d22ed`):

```
accepts_files(/home/me/a.txt)
accepts_files(/home/me/b.png)
… (the same pair five times in all: one per path, each time Rune aims the drag)
on_drop_hover(true)
on_drop_move(10, 10)
on_drop_move(60, 70)
on_drop_hover(false)
on_drop_files(["/home/me/a.txt", "/home/me/b.png"]), read_only true
```

The read-only folder took the drop: `accepts_files` had no way to see `read_only`. The paths
reach state only with the drop. The runtime holds them during the drag (`drag_files` stores
them), but `on_drop_hover` takes `(S, bool, Tx)`, and `Tx` has no `drag_paths()`.

**Ask**, for the gaps above only:
- An acceptance hook with `&S` and every dragged path at once, such as
  `accepts_drag(|s: &S, paths: &[PathBuf]| -> bool)`, or `Tx::drag_paths()` in `on_drop_hover`.
- A drag id, with window-level drag enter and leave callbacks.
- The drop's action: the source's allowed actions, and a way for the target to pick one while
  hovering (copy, move, link).

### U16

**A comment on [#114](https://github.com/anomalyco/rune/issues/114)**, which #226 links:

> Since `99c123d` (#245), a family named in a slot resolves through fontconfig, which covers the
> first ask here. A font picker also needs the list of installed families to offer, and none is
> public: `text::system_fonts::installed_family_files` is `pub(super)`, and the lookup collection
> it builds isn't exposed. Could Rune add `rune::text::installed_families() -> Vec<String>`
> (fontconfig on Linux, CoreText and DirectWrite elsewhere), ideally with whether each is
> monospaced, so a picker can offer the mono slot only monospaced families?
>
> At `78d22ed`, before #245, `set_font_families("C059", "monospace")` and
> `set_font_families("Noto Serif", …)` measure a 43-character line at 274 pt, the same as
> `"No Such Family"`, and Inter at 277 pt. Both fonts are installed (`fc-list`). That matches the
> issue; we haven't rebuilt at `dev` head to confirm #245.

The measurements were run here at `78d22ed` with a headless `Runtime`.

### U17

**Title:** rune-kit: export the Combobox fuzzy matcher, with match positions

A file manager's command palette, settings search and shortcut reference rank results as
`Combobox` does, and bold the matched characters. rune-kit's ranking is the private
`combobox::score` (`rune-kit/src/combobox.rs:44`). It returns a score but not which characters
matched, so apps copy it to rank the same way.

**Compositor:** none. It's a missing export, visible in the source. No program is needed.

**Ask.** A public `rune_kit::fuzzy_match(query, text) -> Option<FuzzyMatch { score, ranges }>`,
the one Combobox uses, with the matched character ranges (or char indices), so a list filtered
with it orders exactly as a Combobox does.

**Meanwhile** we keep a port of `score` that also records the matched ranges.

### U4

**Withdrawn.** W1 found that headless `drag_files` and `drop_files` couldn't carry held
modifiers. A headless check at `78d22ed` shows they do, in process:

```
no modifiers:              drop 1 file(s) mods Mods { shift: false, cmd: false, alt: false, ctrl: false, logo: false }
Ctrl held through PhysicalInput::Modifiers:
                           drop 1 file(s) mods Mods { shift: false, cmd: true, alt: false, ctrl: true, logo: false }
Shift held:                drop 1 file(s) mods Mods { shift: true, cmd: false, alt: false, ctrl: false, logo: false }
```

`Runtime::physical_input(PhysicalInput::Modifiers(m))` sets the runtime's held modifiers
(`runtime/input/physical.rs:41`), and every `Tx` reads them (`runtime/frame.rs:249`), the drop and
hover handlers included. Ctrl gets through, so the draft is withdrawn.

Scripts are the one gap at `78d22ed`: a script has no event that holds a modifier on its own
(`{"physical": …}` releases them straight after). `dev` `4805095` (#240) adds `{"hold": "ctrl"}`
to scripts, so it closes with the next Rune bump. Until then, W1's copy-or-move tests hold
modifiers in process with `physical_input`, not in scripts.

**2026-11-08, Rune `dev` `5d421fd` (the new pin): U-C1b-1 fixed by `4805095`.** A gallery script
with `{"hold": "ctrl"}` before a `tap` on `c1-ten-rows-light` adds the row to the selection ("2
items selected"); without the hold the tap replaces it.

## Considered, no draft

| Topic | Why no draft |
| --- | --- |
| Diagonal resize cursors | Upstream #145, with a patch offered in #233. P3 cites #145. |
| Single instance and activation (P8) | Upstream #22. Not in Rune at `78d22ed` or `dev` head. P8 cites it. |
| Linux `Player` (P7) | Upstream #30, #100 and #112. P7 is an audio sink and clock, not `Player`. |
| A GL fallback without Vulkan | Upstream #23. It belongs in a plan §8 note. |
| Icon theme lookup (P6) | S5 missed upstream [#21](https://github.com/anomalyco/rune/issues/21), which asks for exactly this. P6 cites it and adds [U-P6-1](#u-p6-1), a comment on #21, and [U-P6-2](#u-p6-2). |
| GLib bridge (P9) | Outside Rune's scope. It uses only `AppHandle` and the setup `Waker`. |
| S1's panic on disconnect | S1 dropped it: `App::try_run` returns the error. |
| S4 (syntect backend) | S4's record (`docs/decisions/S4-syntect-backend.md`, on S4's branch at `69be683`, not yet merged) makes no Rune ask. It picks syntect with `onig`, and mentions Rune only to note that rune-kit's highlighter has a similar long-line guard. |

## Re-check at `dev` head

Each gap at `dev` `17adede`, read in the clone:

| Draft | Evidence at `17adede` |
| --- | --- |
| U1 | `drag_out/mod.rs:28`: `SUPPORTED = cfg!(target_os = "macos")` |
| U2, U3 | `el/drop.rs:85`: `on_drop_hover(f: Fn(&mut S, bool, &mut Tx))`; winit 0.30.13 is unchanged |
| U5 | `clipboard/native.rs:139`: `write_one_form`; README line 1283 still says "Copies don't write images yet" |
| U6 | No `drag_resize_window` or `window_resize` anywhere in `rune/src` |
| U7 | `window/shell_requests.rs:20`: `self.options.borderless \|\| …` |
| U8 | `window/chrome.rs:95`: `window_menu`, Windows only |
| U9 | `window/handle.rs:133`: `with_raw_handle` is the only accessor |
| U10 | `runtime/input/pointer.rs:94`: `pub(crate) fn cancel_pointer` |
| U11 | `window/create.rs:13`: `visible = !launcher && (self.started \|\| self.options.visible)` |
| U12 | `platform/portal_appearance.rs:15`: `KEY = "color-scheme"`, the only key |
| U13 | `window/create.rs:86–92`: macOS and Windows only |
| U14 | `popup_menu/mod.rs:38`: `SUPPORTED = false` off macOS and Windows |
| U15 | README line 384: "Virtual rows are vertical only" |
| U16 | `text/system_fonts.rs:149`: `pub(super) fn installed_family_files`; names resolve since `99c123d` |
| U17 | `rune-kit/src/combobox.rs:44`: private `fn score` |

## Crate name

Plan §10 question 1. **Ruled by the owner: the crate keeps the name `strata-platform`.** No
rename, so the rename list below doesn't apply, and with the name taken on crates.io there is
nothing to hold there. The options as S5 put them, with availability on crates.io read on
2026-11-07:

| Option | crates.io | For | Against |
| --- | --- | --- | --- |
| `strata-platform` (now) | **Taken**, by an unrelated crate (v2027.0.0-alpha.1) | No rename | Can never be published to crates.io. Ties a generic crate to one app. |
| `strata-linux` | Free | A small rename | Still ties it to Strata |
| **`transom`** | Free | Short and neutral. A transom is the window above a door, the piece that spans a gap. Not read as an Anomaly crate. | Says nothing about what it does |
| `linux-glue`, `desktop-glue` | Free | Descriptive and searchable | Generic, with a weak identity |
| `wayshim` | Free | Descriptive | Suggests Wayland only, but the crate does X11 too |
| `runic-linux` | Free | Easy to find | Too close to the `rune-` prefix the plan rules out |

**Recommendation: `transom`.** Rename before the first tag (M1), because a rename after it
breaks every pin.

The rename is the owner's own PR. It touches:
- `crates/strata-platform/` (the folder and package name);
- the workspace dependency name in the root `Cargo.toml`;
- the feature docs in `lib.rs` and each `mod.rs`;
- `AGENTS.md`, plan §3, §8 and §10, and the work docs;
- `mise.toml`'s `compile-features` task, which names the crate;
- this repository's release files: `release.yml`'s `CRATE`, `TAG_PREFIX` and tag filter, and the
  README's and changelog's tag names.

The tag format becomes `transom-v<semver>+rune.<rev7>`.

**Holding the name** means publishing an empty `0.0.0` crate with no git dependencies from the
owner's crates.io account. That is outward-facing, and crates.io discourages squatting, so it's
the owner's call. With the name kept, M0's "crate name held" reads as "crate name settled"
(plan §7), and it is.

## Release workflow

`.github/workflows/release.yml`. The setup steps are copied from `ci.yml`, which is unchanged
and has no `workflow_call`.

**Tag:** `strata-platform-v<semver>+rune.<rev7>`, for example
`strata-platform-v0.1.0+rune.78d22ed`.
- `+…` is semver build metadata: information only, ignored for ordering. That fits, since a Rune
  bump is already a minor release.
- `git check-ref-format` accepts it, and cargo pins it with `tag = "…+rune.…"`, checked
  [locally](#verification). GitHub URLs write the `+` as `%2B`.

**Triggers.**
- A pushed tag matching `strata-platform-v*` runs every check and then the release.
- A manual run (`workflow_dispatch`, with a `tag` input) is a dry run: it checks the chosen
  commit under that tag name, and creates no tag and no release. That is the WP's "dry-run tag".

**`preflight`:**
1. The tag matches `^strata-platform-v<semver>\+rune\.[0-9a-f]{7}$`. A manual run's tag is free
   text, so it is printed only once it matches.
2. The semver equals the crate's version from `cargo metadata`.
3. A source or dependency is Rune's when its URL, without scheme, user, port, `.git` or a
   trailing `/`, is `github.com/anomalyco/rune`, so `rune-sys` is not, but an ssh or scp-style
   URL is. Every Rune source in `Cargo.lock` must read
   `https://github.com/anomalyco/rune?rev=<40 hex>#<the same 40>`, all with one rev. Every Rune
   dependency in the root and crate manifests must use that https URL and pin
   `rev = "<that rev>"`, with no `branch` or `tag`. No manifest may `[patch]` Rune's URL or have a
   `[replace]` section. The rev starts with the tag's seven digits.
4. `mise.toml`'s Rust equals `rust-toolchain.toml` at that Rune rev, read from cargo's git
   database after `cargo fetch --locked`.
5. The commit is on `origin/main`. A dry run only warns.
6. `CHANGELOG.md` has `## [<version>] - YYYY-MM-DD` naming the full Rune rev, and the README's
   [supported revs table](../../crates/strata-platform/README.md#supported-rune-revs) has a row
   with the tag. A dry run accepts `[Unreleased]` in place of the section, and only needs the
   README to name the rev.
7. That changelog section becomes the release notes, passed to the release job as an output.

**`build`:** `mise run fmt`, `mise run compile-features`, clippy and headless tests for the crate
with all features, `cargo doc` with `-D warnings`, `mise run deny`, and the **pin check**. The
pin check builds a throwaway app that pins Rune exactly as the README shows and depends on the
crate, and fails unless `cargo tree` finds one Rune.

**`release`:** on a pushed tag only, with `contents: write` and nothing else.
`gh release create --verify-tag --draft --prerelease`, titled `strata-platform <version> (Rune
<rev7>)`, with the notes. The owner publishes the draft. Cargo pins the tag, so the release page
is for people.

**Never** `cargo publish` or `cargo package --verify`. Cargo would drop the git source and keep
`rune = "0.0.1"`, a crates.io requirement. The `rune` on crates.io is the rune-rs language, which
has no 0.0.1 (its versions run 0.0.0, 0.1.0, … 0.14.2; read 2026-11-07), so the publish fails to
resolve. If it ever resolved, it would be the wrong crate.

**Follow-up, shared with `ci.yml`** (not changed here, since `ci.yml` is W0's and W3's): the
checkout steps could set `persist-credentials: false`, since no step pushes, and the release's
`rust-cache` step `save-if: false`, so tag runs don't write the cache that PRs read. Change both
workflows together, in one PR.

**Changelog rules** (`CHANGELOG.md`, Keep a Changelog 1.1.0):
- Each release heading is `## [<version>] - <date>`, followed by
  `Rune: [<rev7>](commit link) (<full rev>) · Rust <version>`.
- Sections: Added, Changed, Deprecated, Removed, Fixed. A Rune move goes under Changed as
  "Rune `old` → `new`", with the upstream changes that matter to the shims.

**README rules:**
- One row per tag in "Supported Rune revs": the tag, the full rev, Rust, the date.
- The pin example uses the full 40-digit rev. Cargo treats a short and a full rev as two sources
  and builds Rune twice. Checked locally: a 7-digit pin gave two `rune` packages, the full pin
  one.

**A Rune rev move** is its own PR and a minor release:
- every Rune rev in `Cargo.toml`, plus `Cargo.lock`;
- Rust in `mise.toml`, and plan §0 (the owner's);
- the crate's minor version;
- a changelog entry and a README row;
- a pass over the [draft index](#draft-index) to see whether Rune shipped anything. If it did,
  start the checklist below.

## Shim deprecation checklist

When Rune ships a feature that a module shims:

1. **Confirm it at a pinned rev.** Rune's README or CHANGELOG documents it on Wayland and on X11,
   and the draft's repro now gives the expected result, headless or under W3. Note the issue and
   the rev in the index's "Filed as" column.
2. **Release N, the Rune move:**
   - Mark each replaced public item `#[deprecated(since = "N", note = "Rune <rev7> ships this:
     use rune::…")]`.
   - The module docs say `Deprecated: Rune shipped <feature> at <rev7> (<issue>)`.
   - The feature still builds and behaves as before.
   - Callers inside this repository move to Rune's API, or say
     `#[expect(deprecated, reason = "…")]`. Clippy here denies `allow`.
   - The changelog's Deprecated section names each item's replacement, and the README's feature
     table says "deprecated in N, removed in N+1".
3. **During N:** apps, Strata included, move to Rune's API.
4. **Release N+1, the next minor:** delete the replaced items, or the module and its feature if
   Rune replaced all of it. Add a Removed changelog entry, and update the README table, this
   index and the P-module doc.
5. **Rune ships only part:** deprecate only the replaced items, and keep the module and its
   feature.

Removing a feature or a public item in a minor release is breaking under semver from 1.0 on.
The rule above holds while the crate is 0.x. What happens at 1.0 is an owner question.

## Verification

Run here on 2026-11-07, after review round 2 and the rebase onto `integration` (P9 and C15 merged), in
scratch clones under the worktree's `target/s5/`. Each failing case must fail at the step named,
not at an earlier one. Cargo ran offline
except for one `file://` fetch. A local runner ran `release.yml`'s own `run:` scripts, as
GitHub would (`bash -eo pipefail`, with `GITHUB_OUTPUT` and the step's `env`). The `uses:` steps
were skipped (the clone is the checkout, and mise was installed), and so was `apt-get` (the
libraries are installed). A fake `gh` first on `PATH` recorded the release call.

| Check | Expected | Result |
| --- | --- | --- |
| YAML loads. Triggers are a `strata-platform-v*` tag and a manual run. Default permissions are `contents: read`, and only `release` has `contents: write`. `release` runs on a push only. Every `uses:` is pinned to a 40-digit SHA used in `ci.yml`. `build`'s setup steps equal `integration`'s `ci.yml` (with `libglib2.0-dev`). No `${{ }}` inside a script. No `cargo publish` or `cargo package`. | pass | pass (15 assertions) |
| P1: a dry run of `strata-platform-v0.0.1+rune.78d22ed` on a branch | pass, with warnings | pass. It warned that the commit isn't on main, and that it used `[Unreleased]`. The toolchain step found Rune's rev by trying each `git/db/rune-*` in turn. |
| F1: tag version 0.1.0 against crate version 0.0.1 | fail at the version | fail: "the tag says 0.1.0, but strata-platform's Cargo.toml says '0.0.1'" |
| F2: tag rev `1234567` | fail at the Rune rev | fail: "the tag names Rune 1234567, but the workspace pins 78d22ed8…" |
| F3: malformed tags (`-rune.`, no rev, a leading zero, an uppercase rev, an 8-digit rev, the text `$(echo pwned)`) | fail at the tag, without printing it | fail, all six. The error is the fixed message; no log line prints the input. |
| F4: a push with no `[0.0.1]` changelog section | fail at the notes | fail: "CHANGELOG.md has no '## [0.0.1] - YYYY-MM-DD' section" |
| F5: a push of a commit not on main | fail at main | fail: "… isn't on main; tag a commit on main" |
| F6: a push whose heading has no date | fail at the notes | fail: "the [0.0.1] heading needs ' - YYYY-MM-DD' after it" |
| F7: `mise.toml` Rust 1.99.0 against Rune's 1.99.1 | fail at the toolchain | fail: "mise.toml pins Rust 1.99.0, Rune's rust-toolchain.toml says 1.99.1" |
| F8: `rune-kit`'s `Cargo.toml` rev differing from `Cargo.lock` | fail at the Rune rev | fail: "Cargo.toml: rune-kit pins Rune by {'rev': '…f4'}; it needs rev = "…f5"" |
| F10: `rune-kit` pinned `branch = "dev"`, with the lock as cargo writes it (`?branch=dev#<rev>`) | fail at the Rune rev | fail: "Cargo.lock pins Rune as '?branch=dev#…', not ?rev=<40 hex>#<the same>" |
| F11: `rune-kit` pinned `tag = "v0.0.1"` (`?tag=v0.0.1#<rev>`) | fail at the Rune rev | fail: "Cargo.lock pins Rune as '?tag=v0.0.1#…'" |
| F12: `rune-kit` pinned `rev = "78d22ed"` (`?rev=78d22ed#<rev>`) | fail at the Rune rev | fail: "Cargo.lock pins Rune as '?rev=78d22ed#…'" |
| F13, F14: `branch` or `tag` in `Cargo.toml` only, the lock untouched | fail at the Rune rev | fail: "Cargo.toml: rune-kit pins Rune by {'branch': 'dev'}" and "… {'tag': 'v0.0.1'}" |
| F15: `rune-kit`'s lock source `git+ssh://git@github.com/anomalyco/rune?rev=…#…` | fail at the Rune rev | fail: "Cargo.lock pins Rune as 'ssh://git@github.com/anomalyco/rune?rev=…', not https://github.com/anomalyco/rune?rev=<40 hex>#<the same>" |
| F16: `rune-kit = { git = "ssh://git@github.com/anomalyco/rune", rev = … }`, the lock untouched | fail at the Rune rev | fail: "Cargo.toml: rune-kit takes Rune from 'ssh://git@github.com/anomalyco/rune'; use https://github.com/anomalyco/rune" |
| F17: `[patch."https://github.com/anomalyco/rune"]` | fail at the Rune rev | fail: "Cargo.toml patches Rune ([patch.'https://github.com/anomalyco/rune'])" |
| F18: `[patch."ssh://git@github.com/anomalyco/rune.git"]` | fail at the Rune rev | fail: "Cargo.toml patches Rune ([patch.'ssh://git@github.com/anomalyco/rune.git'])". The first try keyed the patch `git@github.com:anomalyco/rune.git`, and cargo itself refused that manifest ("should be a URL or registry name"), so a working manifest can't spell it that way. |
| F19: a `[replace]` section | fail at the Rune rev | fail: "Cargo.toml has a [replace] section" |
| P3: a `rune-sys` package pinned `?branch=dev` in the lock and the manifest, and a `[patch.crates-io]`; only the Rune step runs, since cargo would refuse the made-up lock entry | the Rune step passes | passes. The round-1 check took `rune-sys` for Rune (a prefix match), so it would have refused this. |
| The URL normaliser on its own, against 11 spellings | Rune's: https with and without `.git` or `/`, a query, ssh, `git+ssh`, scp-style, an ssh port, mixed case; not Rune's: `rune-sys`, `runes` | as expected, once an ssh port was handled (the first version left `github.com:22/…`) |
| F9: a push with a changelog section but no README row | fail at the notes | fail: "the README's supported Rune revs table has no row for …" |
| P2: a push with a `[0.0.1] - 2026-11-07` section and a README row, on main | pass | pass. The section became the notes. |
| R1: `release` on a manual run | skipped | skipped |
| R2: `release` on a push | one `gh release create` | `gh release create strata-platform-v0.0.1+rune.78d22ed --repo … --verify-tag --draft --prerelease --title "strata-platform 0.0.1 (Rune 78d22ed)" --notes-file …`, with the notes written intact |
| `build`: fmt, compile-features, clippy, headless tests, docs with `-D warnings`, deny, with `glib-bridge` built and its 8 tests run under `--all-features` | pass | pass. The `apt-get` step isn't run here; its package list is checked against `ci.yml` above. |
| `build`: the pin check at the full rev | pass, one Rune | pass. The first run failed: `mise.toml`'s `CARGO_TERM_COLOR=always` coloured `cargo tree`'s ` (*)` marker. The workflow now passes `--color never`. |
| `build`: the pin check at a 7-digit rev | fail, two Runes | fail: `rev=78d22ed` and the full rev listed as two `rune` packages |
| A tag consumer: a scratch clone tagged `strata-platform-v0.0.1+rune.78d22ed`, pinned with `git = "file://…", tag = …` and Rune at the full rev | builds, one Rune | builds. `Cargo.lock` records `?tag=strata-platform-v0.0.1%2Brune.78d22ed`. One `rune` in `cargo tree`. |
| `cargo package --list -p strata-platform` (listing only) | includes `README.md` | includes it, with no `Cargo.toml` change. Cargo warns that the manifest has no `repository`. |
| Every draft's Rust program compiles at `78d22ed` | 14 programs build (U12's window added in round 2) | all build, with no warnings |
| U2's GTK 4 drag source: the content provider, the PNG and the signal handlers, without a display | three types, a valid PNG, handlers connect | `text/uri-list text/plain;charset=utf-8 image/png`. Every PNG chunk CRC checks, and gdk-pixbuf decodes it as 1×1 RGBA. `drag-cancel` and `drag-end` connect, and `NO_TARGET`'s nick is `no-target`. The window and the drag itself weren't run. |
| U12's fake portal and commands, as pasted, on a private bus (`dbus-run-session`) | answers and emits | `ReadOne`, `ReadAll` and `Read` answered (`gdbus`), and the draft's `--init` and `--schedule` strings parse and emit all four changes on time. The command block parses (`bash -n`), builds first, and runs `./target/debug/u12`. Not run against a Rune window here; S1 measured that. |
| `dragon-drop` and `ripdrag` as U2's source, read from their source on GitHub | — | dragon: URI and text targets only (`gtk_target_list_add_uri_targets`, `add_text_targets`). ripdrag: a `GdkFileList` only. Neither offers `image/png`, so U2 now carries its own source. |
| The headless drafts give their "Actual" | as written | U3 (rewritten in round 1), U10, U13, U14 and U15 print what the drafts quote, and U16's measurements and U4's check match |
| `actionlint`, `act`, `zizmor` | — | not installed, so not run |

**Pending for the owner:**
- the workflow on GitHub: a manual dry run on `main`, then a real tag push;
- actionlint;
- creating the draft release, and how its `contents: write` behaves;
- the issue links;
- the crate name and holding it (ruled since: the crate keeps `strata-platform`).

## Owner questions

S5 decides none of these. Each has a recommendation.

1. **Crate name.** `strata-platform` can't go on crates.io. Options are under
   [Crate name](#crate-name). Recommendation: `transom`, renamed before the first tag. Also
   whether to publish a `0.0.0` placeholder to hold it. **Ruled: keep `strata-platform`.**
2. **Tag format.** `strata-platform-v<semver>+rune.<rev7>`, which the workflow uses; the
   name-only `strata-platform-v<semver>`, with the rev only in the changelog; or
   `strata-platform/v<semver>+rune.<rev7>`. Recommendation: the first, since a tag that names
   its Rune rev tells users what to pin beside it.
3. **Release object.** The workflow creates a draft prerelease. Alternatives: publish at once,
   or create no release (the tag alone). Recommendation: the draft prerelease while Rune is
   pre-release.
4. **Versioning.** The crate takes the workspace version (0.0.1), shared with three unpublished
   crates. Recommendation: give `strata-platform` its own `version` in its `Cargo.toml` in the
   first release PR (M1), starting at 0.1.0.
5. **Upstream overlap.** U1 and U2 as new issues that link #171, or as comments on #171? U16 as a
   comment on #114? Add our use case to #145 and #22, or only cite them? Recommendation: new
   issues for U1 and U2 (each has its own repro and goes beyond file paths), U16 as a comment,
   and only cite #145 and #22.
6. **The 1.0 rule.** "Deprecate for one release, remove in the next minor" is only
   semver-compatible in 0.x. From 1.0, keep the removal for the next major, or keep a deprecated
   item as a thin wrapper over Rune's API until then? Recommendation: decide before 1.0. Until
   then, the plan's rule stands.
7. **Public naming.** The drafts say "a file manager on Rune". The owner may name Strata.
8. **P3's M0 cell** rests on S1's owner question 1 (the native frame on Wayland until Rune ships
   U6–U8).

## Constraints for later WPs

- **Every P-module:** fill its `Rune issue:` line from the [Filing](#filing) list once the owner
  files the drafts. Until then, link this record's anchor.
- **K2:** at `78d22ed`, an installed family named in a slot falls back as if it didn't exist
  (U16's measurement), so a picker offering installed fonts can't show them yet. `dev` `99c123d`
  resolves names, so they work after the next Rune move. No public API lists the families
  (U16). Recommendation for K2: offer the bundled families and `ui-monospace` until the move. Ask
  P5 for a fontconfig list only if the owner wants one before Rune has it, because listing fonts
  Rune can't draw at the pin would mislead.
- **W1, C1e, X2:** hold modifiers in process with `Runtime::physical_input(PhysicalInput::Modifiers(..))`
  until the Rune move that brings scripts' `{"hold": …}` ([U4](#u4)).
- **P2:** at `78d22ed`, Rune's own copy can already write files (`text/uri-list` only) or an
  image, one form per copy ([U5](#u5)).
- **P0, P9:** upstream #139 (a covered Wayland window's `poll` and timers stop) may stall wakes
  through `AppHandle`. Check it under W3 with a covered window.
- **C3:** [U15](#u15). `.virtualize` inside `scroll_x` mounts a window but lays tiles out on a
  diagonal, so don't use it horizontally.
- **`crates/strata-platform/Cargo.toml` (W0's file, its own PR):** add `repository`, which
  `cargo package` asks for. The crate's own `version` belongs to the first release PR (owner
  question 4).
- **plan.md (the owner's own PR):** §10 question 1 (the name), §8 (the `rune` name on crates.io,
  and #23 for the GL fallback).

## Not done

- No issue filed, nothing posted, no crate name held, no tag or release (the owner's rules).
- The compositor repros (U1, U2, U5 to U9, U11, U12) weren't re-run. They carry S1–S3's
  measurements, and U5's file-copy types are by source.
- U16 at `dev` head wasn't rebuilt. That #245 resolves names is by source.
- The workflow hasn't run on GitHub.
- S4's record was read on S4's branch (`69be683`) before it merged. If its merged version adds a
  Rune ask, fold it in here.

## Drafts added by later work packages

Drafts written after S5, by the work package named in each. The same rules hold: nothing here is
filed or posted, and each waits for the owner. The entries above are S5's and stay as they are.

### U-P1-1

From P1 (`dnd`): an element has one handler of each kind, and a shim can't read an element's key,
so a shim attaching drag and drop to a component's element takes over its `on_drag` or
`drawn_rect_into`. Draft, not filed; the text is in `crates/strata-platform/src/dnd/mod.rs`.

### U-P1-2

From P1 (`dnd`): `Tx::element_at` gives a path and key but no rect and no point in the element's
own coordinates, as Rune's own drops have in `drop_point`. Draft, not filed; the text is in
`crates/strata-platform/src/dnd/mod.rs`.

X2 adds to U-P1-1: W1's binders were given each element's key and scope (W1e) only because
Rune lets no shim read it, and C1e's rows needed a scope (P1's `drop_target_in`). To U10: still
no public way to end a capture at `dev` `f3160022`; upstream #408 (`dece2aff`) rewrote how
Rune holds a press, so P1's Escape is to be re-checked at the next bump (P1's module docs).

### U-X2-1

From X2 (`examples/dnd_wiring`). Draft, not filed; checked against open issues on 2026-11-09
(none matches; #265 is the press-side twin: pointer moves reach elements under a pinned
sibling).

**Title:** `element_at` finds layers that take no input, so a shim's hit test can't see through
an overlay

**Compositor:** none; it runs headless.

**Steps.** A list of rows, and over them a full-size pinned layer with no handlers that holds a
scroll bar along one edge (rune-kit's and C24's bar layers are built so):

```rust
fn view(_: &S, _: &mut Cx<S>) -> El {
    col().size(300.0, 200.0)
        .child(col().key("rows").child(el().key("row-1").size(300.0, 30.0).on_press(|_: &mut S| {})))
        .child(col().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).child(el().key("bar").w(6.0).h_full()))
}
// rt.element_at(150.0, 15.0)
```

**Expected.** A way to find the element a press or a drop would reach there (`row-1`), as
Rune's own presses and `on_drop_files` do: they skip elements with no handler of the kind.

**Actual** (by source at `5d421fd`, `runtime/elements.rs`): `element_at` returns the topmost
box, handlers or not (`#0/#1`, the layer), and `Pick` can only add transparent elements. P1
finds a drop target by the keys along that path (U-P1-1), so every target under such a layer
was out of reach: Columns' rows and backgrounds under C2's strip bar and each column's edge
bar, and C6's crumbs under their bar. Worked around in this repository by hanging C24's bars
from a zero-size anchor (`scroll::anchor`) instead of a layer the viewport's size; any other
passive layer over a target (an app's own) still hides it.

**Ask.** `Pick::reaching(kind)` (or `Pick::handlers_only()`): skip elements with no handler of
a kind (press, drop), as the press and drop routing do; or a drop-target routing hook for
non-file payloads (#357), which would make the pick unneeded.

### U-P8-1

From P8 (`single-instance`), cited by `crates/strata-platform/src/single_instance/mod.rs`.

**Title:** X11: `AppHandle::activate(token)` on an open window ignores `DESKTOP_STARTUP_ID`

**Compositor:** an X11 window manager that prevents focus stealing (mutter, KWin) shows the
effect. The request itself shows under Xvfb with no window manager, as below. sway (Xwayland)
focuses the window either way, so it can't show the refusal.

**Steps.** Under `Xvfb :99`, with `DISPLAY=:99`, run a Rune app whose window is open, and from
another thread call `handle.activate("demo_TIME12345")`, as a single-instance app does with the
startup id a second launch forwarded. Watch the root window for client messages:

```rust
// Beside the app: an x11rb client listening on the root window.
use x11rb::{connection::Connection, protocol::{Event, xproto::*}};
let (conn, screen) = x11rb::connect(None)?;
let root = conn.setup().roots[screen].root;
conn.change_window_attributes(root, &ChangeWindowAttributesAux::new()
    .event_mask(EventMask::SUBSTRUCTURE_NOTIFY))?;
conn.flush()?;
loop {
    if let Event::ClientMessage(m) = conn.wait_for_event()? {
        let name = conn.get_atom_name(m.type_)?.reply()?.name;
        println!("{} {:?}", String::from_utf8_lossy(&name), m.data.as_data32());
    }
}
```

**Expected.** As GTK does for a startup id: a `_NET_ACTIVE_WINDOW` message whose timestamp
(`data[1]`) is the id's `_TIME` value (12345), and a `_NET_STARTUP_INFO_BEGIN` "remove:
ID=demo_TIME12345" message, so the launcher's busy feedback ends.

**Actual** (by source at `78d22ed`, winit 0.30.13; the snippet above hasn't been run).
`window/lifecycle.rs` `open()` drops the token for an open X11 window (`let _ = token;
w.focus();`), and winit's `focus_window` sends `_NET_ACTIVE_WINDOW` with source 1 and
`CURRENT_TIME` (`platform_impl/linux/x11/window.rs:1783`). No startup-notification remove is
sent; winit sends one only while it creates a window (`x11/window.rs:560`). mutter and KWin may
refuse a timestamp-less request from an application, and only mark the window urgent.

**Ask.** On X11, `activate(token)` for an open window takes the `_TIME` timestamp from the
startup id for `_NET_ACTIVE_WINDOW`, and broadcasts the startup id's "remove" message.

Related: upstream [#22](https://github.com/anomalyco/rune/issues/22) (single instance), which
forwards these ids.

### U-P7-1

From P7 (`media-audio`), cited by `crates/strata-platform/src/media/mod.rs`.

**Title:** Linux: play app-decoded PCM with a playback clock, without `Player`

**Context.** A file manager decodes media in a sandboxed helper, which streams RGBA frames and
PCM back. The frames already work: `Pixels::set` from any thread, no view commit. The sound has
nowhere to go on Linux: `Player` takes a file or URL and has no Linux backend (#30, #100, #112),
and Rune has no API that takes PCM.

**Steps.** At `dev` `5d421fd`, look for a way to play a buffer of interleaved S16 stereo at
48 kHz and read back how much of it has been heard: there is none outside `Player`, whose Linux
backend reports "unsupported".

**Ask.** A small output: open it with a sample format, rate and channel count; push
interleaved PCM; play, pause and flush; read a position that follows the device (what has been
heard, its delay counted); and a callback or waker for underruns, the end and a lost device.
Video then paces on that position, as the strata-platform shim does on GStreamer's `appsrc`.
A `Player` with a Linux backend wouldn't cover it: the app has PCM, not a file.

Related: upstream [#30](https://github.com/anomalyco/rune/issues/30) and
[#112](https://github.com/anomalyco/rune/issues/112).

### U-P7-2

From P7 (`media-audio`).

**Title:** `Pixels::set` drops the image it replaces, so a stream allocates every frame

**Steps.** At `dev` `5d421fd`, stream 1280×720 RGBA at 30 fps into one `Pixels` with
`set(w, h, rgba)`: each call takes a new `Vec<u8>` and drops the previous one under the lock, so
a producer allocates and frees 3.7 MB a frame and can't reuse a buffer.

**Ask.** A way to get the replaced buffer back, `Pixels::replace(w, h, rgba) -> Vec<u8>` say, or
to fill a buffer in place (`Pixels::update(w, h, |rgba| …)`), so a pacer recycles its frames.

### U-P6-1

From P6 (`icon-theme`), cited by `crates/strata-platform/src/icon_theme/mod.rs`. A **comment on
[#21](https://github.com/anomalyco/rune/issues/21)**, not a new issue.

**Comment.** What `ImageCache::app_icon` lacks on Linux at `dev` `5d421fd`, for an app that
shows other apps' icons (an Open With list) and MIME icons:

- **The user's theme.** `freedesktop::icon_file` (`image.rs`) scans `icons/hicolor` only:
  no current theme, no `Inherits`, so an icon that ships only in Adwaita, Yaru or Breeze isn't
  found.
- **`index.theme` sizes.** It reads a size from the directory name (`48x48`, `@2`) and takes
  "the smallest at or above, else scalable, else the largest", not the theme's `Size`, `Type`,
  `Threshold`, `MinSize`, `MaxSize` and `Scale`.
- **Lookup by name.** It takes a `.desktop` path only, so a MIME icon name (`text-x-python`) or
  a list of fallback names (`gio::ThemedIcon::names`) can't be looked up.
- **Not found.** A missing icon and a broken file both resolve to `Image::error`, so an app
  can't tell "draw your fallback" from "the theme ships a bad file".
- It lists every size directory again on each call.

**Steps (headless).** With `XDG_DATA_HOME=$T/share`:

```sh
mkdir -p $T/share/icons/Fixture/48x48/apps $T/share/applications
printf '[Icon Theme]\nName=Fixture\nInherits=hicolor\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n' \
  > $T/share/icons/Fixture/index.theme
cp any-48px.png $T/share/icons/Fixture/48x48/apps/fixture-app.png
printf '[Desktop Entry]\nType=Application\nName=Fixture\nExec=true\nIcon=fixture-app\n' \
  > $T/share/applications/fixture.desktop
```

`ImageCache::new(waker).app_icon_scaled("$T/share/applications/fixture.desktop", 48.0, 1.0).wait()`
returns `Err("no icon for …")`, and nothing in the API names a theme to look in. (Read from
`image.rs` at `5d421fd`; not run.)

**Ask.** An image source that takes the theme by name, as #21 proposes:
`ImageCache::themed_icon(names, theme, size_pt, scale)` following the Icon Theme
Specification's lookup, with "not found" apart from a decode error. Or, smaller: a way to run a
path lookup on the cache's worker (`cache.file_with(|| -> Option<PathBuf>, size_pt, scale)`),
so a crate doing the lookup needs no thread of its own. A file manager does this today in a
shim that resolves on its own thread and hands the file to `ImageCache::file`.

### U-P6-2

From P6 (`icon-theme`), cited by `crates/strata-platform/src/icon_theme/mod.rs`.

**Title:** Draw a `Pixels` image in the element's colour, for symbolic icons loaded at run time

**Context.** Freedesktop symbolic icons (`*-symbolic.svg`) are drawn in one dark grey
(Adwaita's paint `#2e3436`) and are meant to be recoloured to the text colour, as GTK does. An
app that loads them from the icon theme at run time has `Pixels` (or SVG bytes), not a
`rune::Icon`.

**Steps.** At `dev` `5d421fd`, load
`/usr/share/icons/Adwaita/symbolic/places/folder-symbolic.svg` through `ImageCache::file` and
draw it with `El::pixels` on a dark background: it draws dark grey on dark grey.
`El::icon_one_color` would draw it as a silhouette in the element's colour, but only for a
`rune::Icon`, whose `body` is `&'static str`, so a body read at run time would have to leak.

**Ask.** `El::pixels_tint()` (or `pixels_mask`): draw an image's alpha in the element's colour,
as `icon_one_color` draws an icon's. Or a `rune::Icon` with an owned body (`Arc<str>`).
