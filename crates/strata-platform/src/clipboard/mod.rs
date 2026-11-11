//! Clipboard copies and pastes with any MIME types, for Rune apps on Wayland and X11.
//!
//! A [`Clipboard`] writes a [`Content`] of any types at once, each with its bytes or a lazy
//! writer, images too; lists what a selection offers; reads one type, off the UI thread,
//! with [`Limits`], as Rune's `Tx::read_clipboard` does; and tells the app when the selection
//! changes hands ([`Change`]), which a "files on the clipboard" badge needs. [`Files`] writes
//! and reads the formats file managers exchange for copied and cut files.
//!
//! ```ignore
//! let clip = Clipboard::<App>::new().on_change(|s, change, _| {
//!     if let Change::Lost { copy, .. } = change && s.cut == Some(*copy) { s.cut = None; }
//! });
//! // In App::on_window: clip.attach(w, &handle)?;
//! let copy = clip.copy(Selection::Clipboard, Content::new().files(&Files::from_paths(&paths, true)))?;
//! clip.read_files(Selection::Clipboard, Limits::default(), |s, files, _| s.paste(files))?;
//! ```
//!
//! Call [`shutdown`] on the UI thread before the event loop ends, as drag and drop's: it
//! hands an X11 clipboard to a clipboard manager, then tears down the transfer substrate
//! (shared with `dnd`) while winit's display is alive. Call it from `App::on_quit`, which
//! Rune runs in every open window before any closes for `Tx::quit` and `AppHandle::quit`,
//! **and** in the last window's `on_close`: when the last window closes, `on_quit` runs only
//! in the windows still open, so not in that one. Calling it before `Tx::quit` as well does no
//! harm; a second call finds nothing to do. Either module's shutdown does both modules' work.
//!
//! # What Rune lacks
//!
//! Rune issues: none filed yet; the drafts are in `docs/decisions/S5-upstream.md`.
//!
//! - **U5** (draft, not filed): Rune's `Tx::copy_rich` writes one form per copy on Linux (the
//!   files as `text/uri-list` only, else a PNG, else HTML beside text, else text), with no
//!   other MIME type, so a file manager's copy can't carry the GNOME and KDE formats beside
//!   text. A read gives no list of the offered types; nothing tells the app when another
//!   client takes the clipboard; nothing hands a copy's every form to a clipboard manager at
//!   exit, and nothing says when a write was dropped. Rune has no primary selection.
//! - **U9** (draft, not filed): no display handle and no hook before winit closes it. The
//!   module rides P0's substrate (`transfer`), which recovers the display and runs its own
//!   reader, and needs [`shutdown`] called first.
//!
//! # Routes
//!
//! | [`Route`] | Where | Hears another app's copy | Writes |
//! | --- | --- | --- | --- |
//! | `DataControl` | sway, Hyprland, KWin (ext only): Wayland data control, on winit's display or a connection of its own | at once, focused or not | at any time |
//! | `DataDevice` | mutter: `wl_data_device` on winit's display (libwayland ≥ 1.23) | at the app's next keyboard focus | with the serial of the key or click that asked |
//! | `X11` | X11; or a Wayland window with neither of the others (mutter with libwayland < 1.23, Ubuntu 24.04's GNOME: unsupported), through Xwayland | at once | at any time |
//!
//! [`Status`] says which one the app got. With none of them (a Wayland window with no route
//! and no Xwayland) [`Clipboard::attach`] returns [`Error::Unsupported`].
//!
//! # Rune's own clipboard
//!
//! Rune's `Tx::copy` and text fields write through arboard on a clipboard worker of their
//! own. The last writer wins: Rune's copy takes the selection from this module's
//! ([`Lost::Taken`]: it can't be told from another app's), and a copy here takes it from
//! Rune's. Rune's text fields paste from this module's copies like any app (offer text with
//! [`Content::text`]). Rune writes after its handler returns, on its worker, so a `Tx::copy`
//! and a [`Clipboard::copy`] in one handler land in no fixed order: do one or the other.
//!
//! # After the app exits
//!
//! - **X11:** at [`shutdown`] a copy that still holds `CLIPBOARD` is handed to the clipboard
//!   manager, if one runs, every type of it (`SAVE_TARGETS`, then `MULTIPLE`): lazy writers
//!   run then. The test peer asks as gsd-clipboard's and xfsettingsd's sources ask; other
//!   managers (Klipper among them) are hand tests below. The exit waits for its answer
//!   at most [`KEEP_AFTER_EXIT`] (2 s), or the copy's own wait
//!   ([`Content::keep_after_exit`]); [`Closed::hand_off`] says how it went. Without a manager
//!   nothing waits, and the copy goes with the app. Under wlroots' Xwayland (sway) the
//!   window manager owns `CLIPBOARD_MANAGER` itself and answers every hand-off at once,
//!   without taking anything (its source calls it a race): `hand_off` says
//!   [`HandOff::Saved`], and what survives is whatever wlroots' Wayland clipboard had already
//!   taken from the copy, which it does only while an Xwayland window has focus.
//! - **Wayland:** no client can hand a selection on. The copy goes with the app, unless the
//!   compositor or a clipboard manager keeps it (wl-clip-persist on sway and Hyprland, say;
//!   what KDE and mutter keep are hand tests below).
//!
//! # Known limits
//!
//! - **The data device hears another app's copy only at the next focus** (mutter). Losing
//!   the clipboard is heard at once on every route ([`Change::Lost`]); what the other app
//!   offers ([`Change::Offered`], [`Clipboard::offered`]) waits until the app's next keyboard
//!   focus there, for however long that takes.
//! - **A data-device write needs a fresh serial.** It takes the serial of the newest key or
//!   click the app had. A write whose serial is older than the current selection's is dropped
//!   by the compositor without a word; P0 notices the missing echo and the copy is
//!   [`Lost::Refused`]. On wlroots (sway) a write made while the app has no keyboard focus
//!   gets no echo either way, so it stays [`CopyState::Pending`]; what mutter does with one
//!   is a hand test below (it may refuse it). Copy from the key or click handler that asked.
//! - **The primary selection needs data control or X11.** On the data device route
//!   ([`Status::primary`] false) a primary copy is [`Error::Refused`]. This module binds no
//!   `zwp_primary_selection_v1`, which mutter has (a deliberate gap: nothing here needs it).
//! - **Through Xwayland, the compositor's bridge decides.** A Wayland window on the X11
//!   route reaches Wayland apps only through the compositor's bridge between X11 and
//!   Wayland selections. wlroots bridges the clipboard only while an Xwayland window has
//!   keyboard focus; wlroots compositors have data control, so they never take this route.
//!   mutter takes this route only with libwayland < 1.23, which is unsupported (below), so
//!   whether its bridge carries every type faithfully (`x-special/gnome-copied-files`,
//!   `text/uri-list`) is no longer a hand test. Wayland apps' UTF-8 text arrives there as
//!   `UTF8_STRING` ([`TEXT_TYPES`] lists every name text goes by).
//! - **libwayland < 1.23 with data control** (Ubuntu 24.04 on KDE, say) takes data control on
//!   a connection of its own: everything works but the data device, which isn't needed.
//!
//! libwayland below 1.23 is unsupported: 1.23 is the minimum (the owner's ruling on S3's
//! decision 1). The routes above still serve it, so the clipboard degrades rather than
//! fails there, but those paths get no hand test.
//!
//! # Hand tests pending
//!
//! Not run here: each needs a session the headless harness can't give.
//!
//! | Where | What to check |
//! | --- | --- |
//! | GNOME (mutter), libwayland ≥ 1.23 | the data-device route: copy and paste with Nautilus; another app's copy heard at focus; a copy made unfocused (Pending, or Refused?) |
//! | GNOME on X11 (gsd-clipboard), Xfce (xfsettingsd) | the copy outlives the app ([`HandOff::Saved`], then paste elsewhere) |
//! | KDE Plasma (KWin, Klipper) | data control (ext only); Dolphin copy, cut and paste both ways; on X11, whether Klipper takes a hand-off at all |
//! | Hyprland | data control; Nautilus and Dolphin both ways |
//! | Nemo, Caja, Thunar, the GNOME desktop icons | paste a copy and a cut made here |
//!
//! Work package: P2, `docs/work/phase-2-composites/P2-clipboard.md`.

mod api;
mod book;
mod content;
pub mod files;
mod handle;
mod hub;

pub use api::{
    Change, Closed, CopyId, CopyState, Data, Error, HandOff, Limits, Lost, Offered, ReadError,
    Route, Selection, Status,
};
pub use content::{Content, KEEP_AFTER_EXIT, TEXT_TYPES};
pub use files::Files;
pub use handle::Clipboard;
#[doc(hidden)]
pub use handle::TestRoute;

/// Hands an X11 clipboard to a clipboard manager, then tears down the clipboard with the
/// transfer substrate (drag and drop's too), on the UI thread before the event loop ends.
/// See the module docs for when.
pub fn shutdown() -> Result<Closed, Error> {
    hub::shutdown()
}

/// A test switch for strata-platform's own tests: makes the platform end the clipboard's
/// connection (the X11 one, else a Wayland connection of our own), to check how a failure is
/// reported. False without one.
#[doc(hidden)]
pub fn provoke_connection_failure() -> bool {
    crate::transfer::Transfer::current().is_some_and(|t| {
        t.provoke_x11_connection_failure() || t.provoke_protocol_error_on_own_connection()
    })
}

/// Where problems that no call could return go (a failed connection, a hand-off the
/// clipboard manager didn't take): stderr by default. On the UI thread.
pub fn on_diagnostic(f: impl Fn(&str) + 'static) {
    hub::set_diagnostics(std::rc::Rc::new(f));
}
