//! The X11 backend: requests to the window manager on this module's own connection, and a
//! thread that watches the window's state.
//!
//! winit's connection is out of reach (draft U9, not filed), so this one is separate. That is
//! enough for client messages and property reads, but not to release winit's pointer grab
//! before `_NET_WM_MOVERESIZE` (module docs, "The X11 grab").

use std::net::Shutdown;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::JoinHandle;

use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

use super::{Capabilities, Edge, Report, WindowState};

/// `WM_CHANGE_STATE`'s IconicState (ICCCM 4.1.4).
const ICONIC_STATE: u32 = 3;
/// `_NET_WM_MOVERESIZE`'s button and source: the left button, from a normal application.
const BUTTON_LEFT: u32 = 1;
const SOURCE_APPLICATION: u32 = 1;
/// The XInput 2 device id of the virtual core pointer, which `_GTK_SHOW_WINDOW_MENU` takes as
/// GTK sends it.
const CORE_POINTER: u32 = 2;

/// The atoms this backend uses, each interned by its X name.
macro_rules! atoms {
    ($($field:ident: $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        struct Atoms { $($field: Atom,)* }

        impl Atoms {
            fn intern(conn: &RustConnection) -> Result<Atoms, String> {
                $(let $field = conn.intern_atom(false, $name.as_bytes()).map_err(text)?;)*
                Ok(Atoms { $($field: $field.reply().map_err(text)?.atom,)* })
            }
        }
    };
}

atoms!(
    wm_change_state: "WM_CHANGE_STATE",
    net_supported: "_NET_SUPPORTED",
    net_wm_moveresize: "_NET_WM_MOVERESIZE",
    net_wm_state: "_NET_WM_STATE",
    net_wm_state_maximized_horz: "_NET_WM_STATE_MAXIMIZED_HORZ",
    net_wm_state_maximized_vert: "_NET_WM_STATE_MAXIMIZED_VERT",
    net_wm_state_fullscreen: "_NET_WM_STATE_FULLSCREEN",
    net_wm_allowed_actions: "_NET_WM_ALLOWED_ACTIONS",
    net_wm_action_resize: "_NET_WM_ACTION_RESIZE",
    net_wm_action_minimize: "_NET_WM_ACTION_MINIMIZE",
    net_wm_action_maximize_horz: "_NET_WM_ACTION_MAXIMIZE_HORZ",
    net_wm_action_maximize_vert: "_NET_WM_ACTION_MAXIMIZE_VERT",
    gtk_show_window_menu: "_GTK_SHOW_WINDOW_MENU",
);

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// One window's link to the X server: a connection for its requests, and a watcher thread on
/// a connection of its own. Dropping it stops the watcher and waits for its thread.
pub(crate) struct Link {
    conn: RustConnection,
    window: Window,
    root: Window,
    atoms: Atoms,
    /// Held for its `Drop`, which stops the thread.
    _watcher: Watcher,
}

impl Link {
    /// Connects to `$DISPLAY`, the display winit took, and starts watching `window`: `report`
    /// hears each change of its state or of the window manager's capabilities, tagged with
    /// `window`. [`Link::read`] gives the report as it is now.
    pub(crate) fn connect(
        window: Window,
        report: impl Fn(Report) + Send + 'static,
    ) -> Result<Link, String> {
        let (conn, screen) = x11rb::connect(None).map_err(text)?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|s| s.root)
            .ok_or("the X server has no such screen")?;
        let atoms = Atoms::intern(&conn)?;
        let watcher = Watcher::start(window, report)?;
        Ok(Link {
            conn,
            window,
            root,
            atoms,
            _watcher: watcher,
        })
    }

    /// The window's report now.
    pub(crate) fn read(&self) -> Result<Report, String> {
        read_report(&self.conn, &self.atoms, self.window, self.root).map_err(text)
    }

    pub(crate) fn begin_resize(&self, edge: Edge, at: Option<(i32, i32)>) -> Result<(), String> {
        let (x, y) = self.pointer(at)?;
        let data = [
            x,
            y,
            edge.net_wm_direction(),
            BUTTON_LEFT,
            SOURCE_APPLICATION,
        ];
        self.send(self.atoms.net_wm_moveresize, data)
    }

    pub(crate) fn minimize(&self) -> Result<(), String> {
        self.send(self.atoms.wm_change_state, [ICONIC_STATE, 0, 0, 0, 0])
    }

    pub(crate) fn window_menu(&self) -> Result<(), String> {
        let (x, y) = self.pointer(None)?;
        self.send(self.atoms.gtk_show_window_menu, [CORE_POINTER, x, y, 0, 0])
    }

    /// The pointer on the root window, as the client messages carry it.
    fn pointer(&self, at: Option<(i32, i32)>) -> Result<(u32, u32), String> {
        let (x, y) = match at {
            Some(at) => at,
            None => {
                let p = self
                    .conn
                    .query_pointer(self.root)
                    .map_err(text)?
                    .reply()
                    .map_err(text)?;
                (i32::from(p.root_x), i32::from(p.root_y))
            }
        };
        // Root coordinates go out as CARD32: two's complement for a pointer left of or above
        // the root's origin, as Xlib sends them.
        Ok((x.cast_unsigned(), y.cast_unsigned()))
    }

    /// A client message to the window manager (EWMH: sent to the root window).
    fn send(&self, kind: Atom, data: [u32; 5]) -> Result<(), String> {
        let event = ClientMessageEvent::new(32, self.window, kind, data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        self.conn
            .send_event(false, self.root, mask, event)
            .map_err(text)?;
        self.conn.flush().map_err(text)
    }
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

/// A thread that reports one window's changes, on a connection of its own, as Rune's own
/// `platform/x11/pointer.rs` (`MouseWatch`) watches the pointer.
struct Watcher {
    conn: Arc<RustConnection>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    fn start(window: Window, report: impl Fn(Report) + Send + 'static) -> Result<Watcher, String> {
        let (conn, screen) = x11rb::connect(None).map_err(text)?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|s| s.root)
            .ok_or("the X server has no such screen")?;
        let atoms = Atoms::intern(&conn)?;
        let mask = |m| ChangeWindowAttributesAux::new().event_mask(m);
        conn.change_window_attributes(
            window,
            &mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(text)?
        .check()
        .map_err(text)?;
        // A window manager that starts or restarts sets _NET_SUPPORTED again.
        conn.change_window_attributes(root, &mask(EventMask::PROPERTY_CHANGE))
            .map_err(text)?
            .check()
            .map_err(text)?;
        let conn = Arc::new(conn);
        let reader = conn.clone();
        let thread = std::thread::Builder::new()
            .name("strata-csd-x11".into())
            .spawn(move || watch(&reader, &atoms, window, root, report))
            .map_err(text)?;
        Ok(Watcher {
            conn,
            thread: Some(thread),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Shutting the socket down wakes the thread's wait_for_event, which then fails and
        // returns, without waiting for another event. Shutdown acts on the socket, so a
        // duplicate of the descriptor reaches it: no unsafe code.
        if let Ok(fd) = self.conn.stream().as_fd().try_clone_to_owned() {
            let _ = UnixStream::from(fd).shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The watcher's loop: reads the report on each relevant property change, and reports it when
/// it changed. Ends when the window is destroyed or the connection drops.
fn watch(
    conn: &RustConnection,
    atoms: &Atoms,
    window: Window,
    root: Window,
    report: impl Fn(Report),
) {
    let read = || read_report(conn, atoms, window, root);
    let mut last = read().ok();
    let ours = [atoms.net_wm_state, atoms.net_wm_allowed_actions];
    while let Ok(event) = conn.wait_for_event() {
        let relevant = match event {
            Event::DestroyNotify(e) if e.window == window => return,
            Event::PropertyNotify(e) => {
                (e.window == window && ours.contains(&e.atom))
                    || (e.window == root && e.atom == atoms.net_supported)
            }
            _ => false,
        };
        if !relevant {
            continue;
        }
        match read() {
            Ok(r) if last != Some(r) => {
                last = Some(r);
                report(r);
            }
            Ok(_) => {}
            // The window is gone.
            Err(_) => return,
        }
    }
}

fn atoms_of(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<Option<Vec<Atom>>, ReplyError> {
    let reply = conn
        // Up to 1024 atoms (the length is in 32-bit units). Every list read here is far shorter
        // (_NET_SUPPORTED runs to about a hundred), so `bytes_after` is never read.
        .get_property(false, window, property, AtomEnum::ATOM, 0, 1024)?
        .reply()?;
    // An unset property comes back with type None.
    if reply.type_ == u32::from(AtomEnum::NONE) {
        return Ok(None);
    }
    Ok(Some(
        reply.value32().map(Iterator::collect).unwrap_or_default(),
    ))
}

fn read_report(
    conn: &RustConnection,
    a: &Atoms,
    window: Window,
    root: Window,
) -> Result<Report, ReplyError> {
    let state = atoms_of(conn, window, a.net_wm_state)?.unwrap_or_default();
    let supported = atoms_of(conn, root, a.net_supported)?;
    let allowed = atoms_of(conn, window, a.net_wm_allowed_actions)?;
    let report = report_from(a, &state, supported.as_deref(), allowed.as_deref());
    Ok(Report {
        window: Some(window),
        ..report
    })
}

/// What the properties say. Without `_NET_SUPPORTED` (no EWMH window manager) nothing is
/// known. `_NET_WM_ALLOWED_ACTIONS`, where the window manager sets it, narrows what it lists.
fn report_from(
    a: &Atoms,
    state: &[Atom],
    supported: Option<&[Atom]>,
    allowed: Option<&[Atom]>,
) -> Report {
    let has = |list: &[Atom], atom| list.contains(&atom);
    let allows = |action| allowed.map(|l| has(l, action));
    let maximized =
        has(state, a.net_wm_state_maximized_horz) && has(state, a.net_wm_state_maximized_vert);
    let capabilities = match supported {
        None => Capabilities::default(),
        Some(s) => Capabilities {
            resize: Some(
                has(s, a.net_wm_moveresize) && allows(a.net_wm_action_resize) != Some(false),
            ),
            // WM_CHANGE_STATE is ICCCM: any window manager may honour it.
            minimize: allows(a.net_wm_action_minimize),
            maximize: allows(a.net_wm_action_maximize_horz)
                .zip(allows(a.net_wm_action_maximize_vert))
                .map(|(h, v)| h && v)
                .or(Some(has(s, a.net_wm_state_maximized_vert))),
            window_menu: Some(has(s, a.gtk_show_window_menu)),
        },
    };
    Report {
        window: None,
        state: WindowState {
            maximized: Some(maximized),
            fullscreen: Some(has(state, a.net_wm_state_fullscreen)),
        },
        capabilities,
    }
}

#[cfg(test)]
mod tests;
