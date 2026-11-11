//! Client-side decorations on Linux: a borderless window that the app's own header moves,
//! that resizes from its edges where the platform allows, and whose window buttons reach the
//! window manager.
//!
//! Rune lacks, at the pinned rev (`2a44156`):
//!
//! - **Interactive resize** of a borderless window. Rune doesn't reach winit's
//!   `drag_resize_window`, and on Wayland the raw window handle (the `wl_surface`) can't start
//!   one: `xdg_toplevel.resize` needs winit's toplevel and the press's serial. Rune also reports
//!   no maximised or full-screen state (`Cx::chrome_insets()` is 0 on Linux). Draft
//!   [U6](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u6),
//!   not filed. Diagonal resize cursors are upstream
//!   [#145](https://github.com/anomalyco/rune/issues/145); the Windows twin of U6 is
//!   [#130](https://github.com/anomalyco/rune/issues/130).
//! - **Minimise** of a borderless window: `Tx::minimize` ignores it. Draft
//!   [U7](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u7),
//!   not filed.
//! - **The window menu** on Linux, and the compositor's capabilities (`wm_capabilities`). Draft
//!   [U8](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u8),
//!   not filed.
//! - **The native display connection.** The X11 backend below opens a second connection, which
//!   can't release the pointer grab winit's connection holds during a press. Draft
//!   [U9](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u9),
//!   not filed.
//! - **The desktop's double-click interval.** `.window_drag()` maximises on a double press
//!   paired with Rune's fixed 0.5 s, and always maximises (GNOME's
//!   `action-double-click-titlebar` isn't read). Drafts
//!   [U13](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u13)
//!   and U8, not filed. The desktop's interval from a settings shim (this crate's `settings`
//!   feature, P5) can't apply to the header either: Rune pairs the presses inside
//!   `.window_drag()`, and a header with press handlers of its own is no longer a drag region,
//!   while `.window_drag()` is the only way to start a move on Wayland.
//!
//! Work package: P3, `docs/work/phase-1-leaves/P3-client-side-decorations.md`. The findings
//! behind it are `docs/decisions/S1-linux-window.md` (Q2, Q3).
//!
//! # The shape
//!
//! [`Policy::Auto`] gives a borderless window on Wayland and on X11; [`Policy::Native`] keeps
//! the native frame. The app keeps a [`Csd`] in its state and reaches it by accessor
//! (`fn(&mut S) -> &mut Csd`):
//!
//! ```ignore
//! struct State { csd: Csd }
//! fn csd(s: &mut State) -> &mut Csd { &mut s.csd }
//!
//! fn init() -> State {
//!     State { csd: Csd::new(Policy::Auto.frame(), Backend::from_env()) }
//! }
//!
//! fn view(s: &State, _: &mut Cx<State>) -> El {
//!     let close = row().key("close").role("button").label("Close")
//!         .on_press_tx(|s: &mut State, tx: &mut Tx| { s.csd.close(tx); });
//!     let bar = csd::header(row().h(40.0).w_full().child(text("Title")).child(close), &s.csd, csd);
//!     csd::decorate(col().w_full().h_full().child(bar), &s.csd, csd)
//! }
//!
//! fn main() {
//!     let app = App::new(AppSpec::new("App", (800.0, 600.0), init, view))
//!         .window_options(csd::window_options(Policy::Auto.frame(), WindowOptions::default()));
//!     let handle = app.handle();
//!     app.on_window(move |w| csd::attach(w, &handle, csd)).run();
//! }
//! ```
//!
//! - [`decorate`] puts the resize bands over the app's content where they can work: a
//!   borderless window on X11 that isn't maximised or full screen.
//! - [`header`] marks the app's header as a `.window_drag()` region (a press moves the window,
//!   a double press maximises it), and opens the window menu on a secondary click where the
//!   window manager has one.
//! - [`Csd::close`], [`Csd::minimize`] and [`Csd::toggle_maximized`] are the window buttons'
//!   handlers. The buttons' look is the app's. [`Csd::state`] says whether to show maximise or
//!   restore, and [`Csd::capabilities`] which buttons can work.
//! - [`attach`], from `App::on_window`, links the [`Csd`] to the native window, and on X11
//!   starts a watcher thread that reports the window's state and the window manager's
//!   capabilities through `AppHandle::update`.
//!
//! Every request answers an [`Outcome`]: sent, and how ([`Route`]), or unsupported, and why
//! ([`Gap`], which names its Rune draft). [`Csd::observe`] sees each one as it happens, and
//! hears a [`Notice::Error`] if the X11 connection fails: the module never prints.
//!
//! On X11 a request goes out once the window is linked, unless the window manager says it
//! can't ([`Gap::WindowManager`]). The window menu also waits until the window manager has said
//! it has one ([`Gap::NotKnown`]). Without its own X11 connection the module can do nothing on
//! X11: resize, minimise and the window menu say `Some(false)` and answer
//! [`Gap::NoConnection`], while Close and Maximise still go through Rune.
//!
//! # For the app's header
//!
//! - **Buttons:** hide one only when its capability is `Some(false)`. `None` is unknown, and
//!   the request goes out.
//! - **Maximise or Restore:** follow [`Csd::state`]. `maximized: None` (Wayland, U6) means the
//!   app can't know, so show one label that fits both.
//! - **Gallery and golden scenes:** build `Csd::new(frame, Backend::Headless)` explicitly, never
//!   [`Backend::from_env`], so a scene renders the same on any machine and under any session.
//!   A headless `Csd` mounts the bands of a borderless frame and records requests without
//!   sending them; [`Csd::apply`] with a [`Report`] sets the state and capabilities a scene
//!   shows.
//!
//! # What each backend does
//!
//! | | Wayland, borderless | X11, borderless | Native frame |
//! | --- | --- | --- | --- |
//! | Move | the header (`.window_drag()`) | the header | the frame and the header |
//! | Resize | **none** (U6): no bands; only compositor shortcuts | bands send `_NET_WM_MOVERESIZE` | the frame |
//! | Maximise | `Tx::toggle_maximized` | `Tx::toggle_maximized` | `Tx::toggle_maximized` |
//! | Maximised state | unknown (U6) | `_NET_WM_STATE`, watched | unknown on Wayland (U6), watched on X11 |
//! | Minimise | **unsupported** (U7) | `WM_CHANGE_STATE` | `Tx::minimize` |
//! | Window menu | **none** (U8) | `_GTK_SHOW_WINDOW_MENU` where `_NET_SUPPORTED` lists it | the frame's own |
//! | Without its X11 connection | — | no bands, minimise or menu ([`Gap::NoConnection`]) | — |
//! | Close | `Tx::close_window` | `Tx::close_window` | `Tx::close_window` |
//!
//! On Wayland the bands aren't mounted at all, rather than mounted to answer
//! [`Gap::RuneResize`]: a band that can't resize would still show a resize cursor, take presses
//! from the content under the window's edges, and promise something nothing delivers. A
//! direct [`Csd::begin_resize`] call there answers the gap.
//!
//! # Limits
//!
//! - **The X11 grab.** The EWMH spec says the client must release its grabs before
//!   `_NET_WM_MOVERESIZE`, because the window manager grabs the pointer itself. winit's
//!   connection holds the press's implicit grab, and this backend's own connection can't release
//!   it. wlroots' X11 window manager (Xwayland under sway) doesn't grab, so resizing works there.
//!   mutter and KWin on X11 are expected to refuse it; that is a hand test, still pending.
//!   Until U9, nothing here can fix it.
//! - **The window menu** is sent when the secondary button is released (Rune fires
//!   `on_secondary_press` on release). GTK sends it on press. Whether mutter minds is a hand
//!   test, still pending.
//! - **A secondary click over the header's children.** The header's handler takes the
//!   secondary click of every descendant without one of its own, as GTK's header bar does for
//!   its buttons, except a text field: Rune routes a field's secondary click to its own field
//!   menu, which Linux can't show (Rune's popup menus are macOS and Windows only, draft
//!   [U14](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u14)).
//!   So a right-click on a header text field without its own `on_secondary_press` focuses it and
//!   moves its caret, and shows nothing. Give the field its own `on_secondary_press`.
//! - **Corners** show `Cursor::Move` until Rune has diagonal resize cursors (#145).
//! - **Tiled state** (GTK's `tiled-*`, which drops the window's border) isn't reported:
//!   neither Rune nor EWMH has it.
//!
//! # Where it stands
//!
//! The owner chose borderless on both backends, accepting the gaps above. They are deliberate,
//! and they change the behaviour of a GTK 4 app with its own header, which is always
//! client-decorated with GTK's invisible resize edges:
//!
//! - **Wayland:** borderless, with no edge resize, minimise or window menu until Rune ships
//!   U6–U8; only the compositor's own shortcuts resize it. The window moves and maximises from
//!   its header and closes from its button. On Hyprland, which draws no title bar either way,
//!   nothing is lost.
//! - **X11:** borderless through the shim. It works on wlroots' X11 window manager; mutter and
//!   KWin are a hand test, still pending.
//!
//! The work package's done-when, narrowed to match: the example drags, maximises and closes on
//! every backend; on X11 it also resizes from each edge and minimises, and opens the window menu
//! where the window manager has one; on Wayland the gaps answer their drafts. Automated: headless
//! tests here, and session tests under headless sway (`tests/session/p3.rs`). Hand tests still
//! pending: Hyprland, GNOME (Wayland, and mutter on X11), KDE (Wayland, and KWin on X11).
//!
//! S1's constraints, as this module meets them:
//!
//! - Rune doesn't reach `drag_resize_window`: X11 sends `_NET_WM_MOVERESIZE` itself, and
//!   Wayland mounts no bands (U6).
//! - `Tx::minimize` ignores a borderless window: X11 sends `WM_CHANGE_STATE`, and Wayland answers
//!   [`Gap::RuneMinimize`] (U7).
//! - Linux has no window-menu path: X11 sends `_GTK_SHOW_WINDOW_MENU` where the window manager
//!   lists it, and Wayland has none (U8).
//! - There are no diagonal resize cursors: corners show `Cursor::Move` (#145).
//! - Rune reports no maximised state: X11 reads `_NET_WM_STATE`, and Wayland reports `None` (U6).
//! - The header's double-click uses Rune's fixed 0.5 s and always maximises (U13, U8).
//! - The second X11 connection can't release winit's grab, so mutter and KWin may refuse the
//!   resize (U9).
//! - Under W3, edge resizing is tested on Xwayland only, since Wayland has none.

mod edge;
mod frame;
mod request;
mod state;
mod view;
mod x11;

pub use edge::{Bands, Edge};
pub use frame::{Backend, Frame, Policy, window_options};
pub use request::{Gap, Outcome, Request, Route};
pub use state::{Capabilities, Csd, Notice, Report, WindowState};
pub use view::{attach, decorate, header};

#[cfg(test)]
mod tests;
