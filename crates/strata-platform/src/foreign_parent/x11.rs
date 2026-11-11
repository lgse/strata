//! X11 (an X11 session, or Xwayland): `WM_TRANSIENT_FOR`, `_NET_WM_WINDOW_TYPE_DIALOG` and
//! `_NET_WM_STATE_MODAL`, set on a connection of the shim's own.
//!
//! Rune exposes no X connection (draft U9, not filed), and properties can be set from any
//! client. Each call opens a connection, sets the properties, waits until the server has
//! them (`sync`), and closes it: the window manager then reads them when winit maps the
//! window.

use rune::WindowHandle;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, MapState, PropMode, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::outcome::{Fallback, Outcome, Reason, WindowSystem};
use super::parent::Parent;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WM_STATE,
        _NET_WM_STATE_MODAL,
    }
}

/// `_NET_WM_STATE` client message actions (EWMH).
const STATE_REMOVE: u32 = 0;
const STATE_ADD: u32 = 1;
/// The client message's source: a normal application.
const SOURCE_APPLICATION: u32 = 1;

/// Sets `window`'s (X11 window `child`) parent hints for `parent`. The window is a dialog
/// either way, as GTK's portal dialog always is; it is transient for, and modal to, a parent
/// that exists. A parent that doesn't, or isn't a window (a pixmap, the root), gets no
/// transient-for, and the window is centred on its screen (S3's fallback).
pub(super) fn apply(window: &WindowHandle, child: Window, parent: &Parent, modal: bool) -> Outcome {
    match set(child, parent, modal) {
        Ok(outcome) => {
            if matches!(
                outcome,
                Outcome::Parentless {
                    fallback: Fallback::Centred,
                    ..
                }
            ) {
                window.center();
            }
            outcome
        }
        Err(e) => Outcome::parentless(Reason::Failed(e), Fallback::Compositor),
    }
}

fn set(child: Window, parent: &Parent, modal: bool) -> Result<Outcome, String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::new(&conn).map_err(text)?.reply().map_err(text)?;
    let mapped = conn
        .get_window_attributes(child)
        .map_err(text)?
        .reply()
        .map_err(text)?
        .map_state
        != MapState::UNMAPPED;
    conn.change_property32(
        PropMode::REPLACE,
        child,
        atoms._NET_WM_WINDOW_TYPE,
        AtomEnum::ATOM,
        &[atoms._NET_WM_WINDOW_TYPE_DIALOG],
    )
    .map_err(text)?;
    let outcome = match parent {
        Parent::X11(xid) if xid.get() != child && exists(&conn, xid.get())? => Outcome::Parented {
            system: WindowSystem::X11,
            late: mapped,
        },
        Parent::X11(_) => Outcome::parentless(Reason::BadXid, Fallback::Centred),
        Parent::None => Outcome::parentless(Reason::NoParent, Fallback::Compositor),
        Parent::Wayland(_) => Outcome::parentless(
            Reason::Mismatch {
                parent: WindowSystem::Wayland,
                window: WindowSystem::X11,
            },
            Fallback::Compositor,
        ),
    };
    // A window reused for another request carries no stale parent or modal state.
    match (parent, outcome.is_parented()) {
        (Parent::X11(xid), true) => {
            conn.change_property32(
                PropMode::REPLACE,
                child,
                AtomEnum::WM_TRANSIENT_FOR,
                AtomEnum::WINDOW,
                &[xid.get()],
            )
            .map_err(text)?;
        }
        _ => {
            conn.delete_property(child, AtomEnum::WM_TRANSIENT_FOR.into())
                .map_err(text)?;
        }
    }
    let want_modal = modal && outcome.is_parented();
    if mapped {
        // EWMH: once mapped, a client asks the window manager to change its state.
        let event = ClientMessageEvent::new(
            32,
            child,
            atoms._NET_WM_STATE,
            [
                if want_modal { STATE_ADD } else { STATE_REMOVE },
                atoms._NET_WM_STATE_MODAL,
                0,
                SOURCE_APPLICATION,
                0,
            ],
        );
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(text)?;
    } else {
        // Before the map the client sets the property itself, keeping the states winit set.
        let current = conn
            .get_property(false, child, atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 1024)
            .map_err(text)?
            .reply()
            .map_err(text)?;
        let current: Vec<u32> = current.value32().into_iter().flatten().collect();
        if let Some(next) = with_state(&current, atoms._NET_WM_STATE_MODAL, want_modal) {
            conn.change_property32(
                PropMode::REPLACE,
                child,
                atoms._NET_WM_STATE,
                AtomEnum::ATOM,
                &next,
            )
            .map_err(text)?;
        }
    }
    conn.sync().map_err(text)?;
    Ok(outcome)
}

/// Whether `xid` names a window that can be a parent: one `GetWindowAttributes` answers for
/// (`GetGeometry` answers for a pixmap too), and no screen's root, for which
/// `WM_TRANSIENT_FOR` would mean ICCCM's group transient instead.
fn exists(conn: &RustConnection, xid: Window) -> Result<bool, String> {
    if conn.setup().roots.iter().any(|s| s.root == xid) {
        return Ok(false);
    }
    match conn.get_window_attributes(xid).map_err(text)?.reply() {
        Ok(_) => Ok(true),
        Err(ReplyError::X11Error(_)) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// `states` with `atom` present (`on`) or absent, or None when it already is.
pub(super) fn with_state(states: &[u32], atom: u32, on: bool) -> Option<Vec<u32>> {
    let has = states.contains(&atom);
    match (has, on) {
        (false, true) => Some(states.iter().copied().chain([atom]).collect()),
        (true, false) => Some(states.iter().copied().filter(|&a| a != atom).collect()),
        _ => None,
    }
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}
