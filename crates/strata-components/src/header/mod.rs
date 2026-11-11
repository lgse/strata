//! Strata's window header: the bar at the top of a browser window, with its actions and its
//! window buttons, and the window's 1 px frame (`app/window`).
//!
//! Work package: X1, `docs/work/phase-2-composites/X1-shell-demo.md`. The owner folded the
//! header component into X1 at P3's planning (`docs/work/phase-1-leaves/P3-client-side-decorations.md`).
//! Strata's header is read at lgse/strata `upstream/main` 35ee677ab
//! (`src/ui/window/composition/layout.rs`, `src/style.css`).
//!
//! # The shape
//!
//! - [`Header`] builds the bar: the sidebar toggle ([`Header::leading`]), the location bar
//!   ([`Header::center`]), the actions ([`Header::action`], built with [`action`]) and the
//!   window buttons ([`Header::window_buttons`]), left to right. The bar is a `.window_drag()`
//!   region: a press on it, or on any part of it without press handlers of its own, moves the
//!   window, and a double press maximises or restores it.
//! - [`frame`] draws the window's 1 px line, unless it is maximised or full screen.
//! - The window buttons come from data: [`WindowButtons`] (Strata's preferences) and
//!   [`WindowChrome`] (what the window manager says). A press reports a [`WindowAction`]; the
//!   host routes it.
//!
//! The component holds no state, does no I/O and never talks to the window system: it reads
//! the [`Look`](strata_kit::look::Look) and the data it is given. strata-components doesn't
//! depend on strata-platform, so the host maps the platform layer onto the data:
//!
//! ```ignore
//! // strata-platform `csd`, in the host.
//! let state = s.csd.state();
//! let caps = s.csd.capabilities();
//! let chrome = WindowChrome {
//!     maximized: state.maximized,
//!     fullscreen: state.fullscreen,
//!     can_minimize: caps.minimize,
//!     can_maximize: caps.maximize,
//!     active: s.active, // the root's on_window_focus
//! };
//! let bar = Header::new("header", look)
//!     .window_buttons(s.buttons, &chrome, &labels, |s: &mut State, action, tx| {
//!         match action {
//!             WindowAction::Minimize => { s.csd.minimize(tx); }
//!             WindowAction::ToggleMaximize => { s.csd.toggle_maximized(tx); }
//!             WindowAction::Close => { s.csd.close(tx); }
//!         }
//!     })
//!     .build();
//! // `csd::header` adds the window menu on a secondary click where the window manager has one.
//! let root = csd::decorate(header::frame("frame", col().child(csd::header(bar, &s.csd, csd)).child(body), look, &chrome), &s.csd, csd);
//! ```
//!
//! # Rules
//!
//! - **Which buttons show:** each one the user turned on, unless the window manager says it
//!   can't work (`Some(false)`); unknown shows. A fresh profile shows only Close. On Wayland,
//!   strata-platform says minimise can't work (Rune draft U7, not filed), so Minimize stays
//!   hidden there even when it is turned on: a deliberate change from GTK Strata.
//! - **Maximize or Restore:** a maximised window shows Restore ("Restore window"), a window
//!   known not to be shows Maximize ("Maximize window"). On Wayland Rune reports no maximised
//!   state (draft U6, not filed), so the button shows Maximize and reads "Maximize or restore
//!   window" (P3's guidance, the coordinator's ruling).
//! - **The double press** pairs within Rune's fixed 0.5 s, not the desktop's interval, and
//!   always maximises (drafts U13 and U8, not filed): `.window_drag()` pairs the presses itself,
//!   and a bar with a double-press handler of its own would no longer move the window.
//! - **Backdrop:** the bar draws the same whether its window has focus or not. Strata's
//!   stylesheet has no backdrop rule; [`WindowChrome::active`] is carried for when the owner
//!   decides (owner item).
//! - **Tokens:** the bar's height is `frame.bar` plus `frame.inset` above and below; actions
//!   are `frame.control` squares spaced by `frame.inset`; the fill is `bg_deep` (Strata's
//!   surface), the hairline and the frame `border_base` (its border at 60 %). Nothing else.
//!
//! # Coverage of `app/window`
//!
//! Scripts are `tests/scripts/x1/<name>.json`; "test" is in `crates/strata-gallery/tests/x1.rs`;
//! "unit" is in `header/tests.rs`; "golden" is a scene's accessibility or PNG golden.
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | With one tab open, the header holds, left to right, the sidebar toggle, the location bar, and the New tab, Search, Appearance, Settings, Minimize, Maximize, and Close buttons | golden `x1-header-states` (a11y: the order). The toggle and the location bar are the host's (C8, C6); the page draws stand-ins. |
//! | A fresh profile shows only the Close button… | golden `x1-header-states` (the first bar); unit `a_fresh_profile_shows_only_close` |
//! | Show minimize button, Show maximize button, and Show close button … immediately | script `button-preferences` (the header's half: a change shows at once). The Settings rows and "every open window" are Strata's (C20c), and the demo broadcasts them. |
//! | Saved window-button choices apply when a window is built… | **Strata policy** (its settings file) |
//! | Maximize maximizes the window; on a maximized window it shows a restore icon… | scripts `maximize`, `restore`; golden `x1-shell-maximized` |
//! | On a maximized window the Maximize button's tooltip and accessible name read "Restore window" | script `maximize` |
//! | Minimize minimizes the window and Close closes it | scripts `minimize`, `close` (the action the host receives); the window's response is a **hand test pending** |
//! | On such a compositor, dragging Strata's header moves the window and dragging a window edge resizes it | test `the_header_moves_the_window_and_its_buttons_keep_their_presses` (the move); edges are strata-platform `csd` (X11 only: Wayland has none until U6, a deliberate change) |
//! | With compositing, a client-decorated window has square corners and a 1 px border… | golden `x1-shell` (PNG) |
//! | A maximized, fullscreen, or tiled client-decorated window draws no border | **partial**: golden `x1-shell-maximized` (PNG) and unit `the_frame_follows_what_the_window_manager_says`, where the state is known (X11). On Wayland a maximised window keeps its 1 px line: Rune reports no maximised state until U6. A tiled window keeps it everywhere: neither Rune nor EWMH reports tiling (strata-platform `csd`, Limits) |
//! | On a client-decorating compositor … only Strata's header, with no GTK default title bar; a second browser window … | strata-platform `csd` (`Policy::Auto` is borderless); **hand test pending** (GNOME) |
//! | A client-decorated window without compositor alpha (`solid-csd`)… | **not applicable**: GTK's fallback, which a borderless Rune window doesn't have (proposed to the owner) |
//! | The portal file chooser window likewise…; Opening a new tab leaves the window without… | **not applicable here**: the chooser is P4 and Strata's; tabs are C7 |
//! | The sidebar toggle draws its panel icon at 17 px…; toggle tooltips… | **C8 and Strata**: the toggle is the host's element |
//!
//! # Rune drafts (not filed)
//!
//! - **U-X1-1, window activation in scripts.** Rune's headless scripts have no event that
//!   deactivates the window (`{"type": "window-focus", "focused": false}`), so what follows
//!   `on_window_focus` (the header's `active`) is testable only in process, through
//!   `Runtime::window_focus`.
//! - **U-X1-2, titlebar actions in scripts and trees.** What a press on a `.window_drag()`
//!   region did (a move, a double-click zoom) reaches neither a script's `expect` nor `--tree`;
//!   only `Runtime::take_titlebar_action` in process sees it.
//! - Reused: U6 (maximised state), U7 (minimise), U8 (window menu, the double-click action),
//!   U13 (the double-click interval), all in `docs/decisions/S5-upstream.md`.

mod bar;
mod chrome;
mod frame;

#[cfg(test)]
mod tests;

pub use bar::{Header, action};
pub use chrome::{HeaderLabels, Shown, WindowAction, WindowButtons, WindowChrome};
pub use frame::frame;
