//! The peer on X11: one x11rb connection. A thread blocks on X events and the stdin thread
//! reads commands; both feed one channel that the main thread works through, so all state
//! lives on one thread.
//!
//! Selections are `CLIPBOARD` and `PRIMARY`, with `TARGETS`, `TIMESTAMP` and INCR both ways.
//! Ownership and conversions carry real server timestamps (ICCCM), from a zero-length property
//! append. Drag and drop is XDND version 5: the source runs on the implicit grab of the press on
//! its own window, honours a target's `XdndProxy`, and speaks the lower of the two versions.
//!
//! `map` replies once the window is in the root's `_NET_CLIENT_LIST`, which wlroots updates
//! when the compositor maps the window's `wl_surface`. Up to wlroots 0.20.2, a window whose
//! first buffer reaches the compositor before its pairing with the X window (they travel on
//! different sockets) is never mapped in sway; wlroots fixed it after 0.20.2 ("no
//! synchronization between the two" in `xwayland_surface_associate`). A window that isn't
//! listed within [`MAP_RETRY`] (times `STRATA_TEST_TIMEOUT_SCALE`) is unmapped and mapped
//! again, which pairs a new surface, until the test's own timeout gives up.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use strata_test_peer::protocol::{
    Action, Backend, Command, Event, Exported, Mapped, MultipleRead, Offer, Payload, Request,
    Selection, Targets, TypedWindow, Via,
};
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux,
    ConnectionExt as _, CreateWindowAux, EventMask, NotifyMode, PropMode, Property,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::{out, read_stdin};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD, PRIMARY, TARGETS, TIMESTAMP, MULTIPLE, INCR, UTF8_STRING,
        CLIPBOARD_MANAGER, SAVE_TARGETS, ATOM_PAIR,
        _NET_WM_NAME, _NET_WM_PID, _NET_CLIENT_LIST, STRATA_PEER_TIME,
        XdndAware, XdndProxy, XdndTypeList, XdndSelection, XdndEnter, XdndPosition,
        XdndStatus, XdndLeave, XdndDrop, XdndFinished, XdndActionCopy, XdndActionMove,
        XdndActionAsk, XdndActionList, XdndActionLink,
    }
}

/// Data larger than this goes by INCR, in chunks this size. It stays well under the core
/// protocol's request limit (256 KiB) without BIG-REQUESTS.
const INCR_CHUNK: usize = 64 * 1024;

const XDND_VERSION: u32 = 5;

/// How long a mapped window may stay out of `_NET_CLIENT_LIST` before the peer maps it again,
/// before `STRATA_TEST_TIMEOUT_SCALE`.
const MAP_RETRY: Duration = Duration::from_millis(500);

/// How long an INCR transfer may stall before it is dropped, before the scale: the harness's
/// own wait.
const INCR_STALL: Duration = Duration::from_secs(10);

/// `STRATA_TEST_TIMEOUT_SCALE`, which the session passes on: slow machines wait longer.
fn scaled(d: Duration) -> Duration {
    let scale = std::env::var("STRATA_TEST_TIMEOUT_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    d.mul_f64(scale)
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("x11: {e}")
}

enum Input {
    Command(Request),
    X(XEvent),
    /// An event on the drop proxy's connection.
    Proxy(XEvent),
    Lost(String),
}

/// What a selection conversion the peer asked for is for.
enum Conversion {
    Targets(u64),
    Read(u64),
    Drop {
        source: Window,
        mime: String,
        action: Option<Action>,
    },
    /// As clipboard manager: the `MULTIPLE` conversion of a hand-over.
    Multiple,
    /// As clipboard manager: pair `n` of that conversion, which came by INCR.
    Part(usize),
    /// A test's `MULTIPLE` read: the request id and each pair's property.
    ReadMultiple(u64, Vec<Atom>),
}

/// A hand-over in progress: the `SAVE_TARGETS` request to answer, and each type's data as it
/// arrives.
struct Saving {
    requestor: Window,
    property: Atom,
    time: u32,
    parts: Vec<SavedPart>,
}

struct SavedPart {
    mime: String,
    target: Atom,
    property: Atom,
    data: Option<Result<Vec<u8>, String>>,
}

/// A selection the peer owns: its offers and the server time it took it.
struct Owned {
    offers: Vec<Offer>,
    time: u32,
}

struct IncrIn {
    why: Conversion,
    data: Vec<u8>,
    last: Instant,
}

struct IncrOut {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Vec<u8>,
    offset: usize,
    last: Instant,
}

struct DropPolicy {
    mimes: Vec<String>,
    actions: Vec<Action>,
    preferred: Action,
    read_all: bool,
    ask_answer: Option<Action>,
}

struct DropIn {
    source: Window,
    mime: Option<String>,
    action: Option<Action>,
    /// Every type to read at the drop (the accepted one first).
    reads: Vec<String>,
}

/// A drop target under the drag: the window, where its messages go (it or its `XdndProxy`),
/// and the XDND version both sides speak.
#[derive(Clone, Copy, PartialEq)]
struct Target {
    window: Window,
    dest: Window,
    version: u32,
}

struct DragOut {
    mimes: Vec<String>,
    action: Action,
    target: Option<Target>,
    accepted: bool,
}

/// The drop proxy: a window on a second connection, so messages sent to it arrive apart from
/// messages sent to the peer's own window.
struct Proxy {
    _conn: Arc<RustConnection>,
}

struct X {
    conn: Arc<RustConnection>,
    atoms: Atoms,
    root: Window,
    win: Window,
    rx: Receiver<Input>,
    tx: Sender<Input>,
    /// Inputs read while waiting for a server timestamp, handled next.
    backlog: VecDeque<Input>,
    /// The selections the peer owns (`CLIPBOARD`, `PRIMARY`, `XdndSelection`).
    owned: HashMap<Atom, Owned>,
    /// Conversions in flight, by the property they land in, with the selection and target.
    pending: HashMap<Atom, (Atom, Atom, Conversion)>,
    incr_in: HashMap<Atom, IncrIn>,
    incr_out: Vec<IncrOut>,
    next_property: u32,
    map_reply: Option<u64>,
    /// While `map` waits: when to map again.
    map_retry: Option<Instant>,
    /// The time of the press that is still held, if any.
    press_time: Option<u32>,
    drop_policy: Option<DropPolicy>,
    /// A drop's reads still out, by its source, and whether every one so far arrived: the
    /// drop finishes when the last one does.
    drop_left: HashMap<Window, (usize, bool)>,
    drop: Option<DropIn>,
    drag: Option<DragOut>,
    proxy: Option<Proxy>,
    /// As clipboard manager: whether it answers hand-overs.
    manager: Option<bool>,
    saving: Option<Saving>,
}

pub fn run() -> Result<(), String> {
    let (conn, screen) = x11rb::connect(None).map_err(err)?;
    let conn = Arc::new(conn);
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::new(conn.as_ref())
        .map_err(err)?
        .reply()
        .map_err(err)?;
    let win = conn.generate_id().map_err(err)?;
    let mask = EventMask::PROPERTY_CHANGE
        | EventMask::STRUCTURE_NOTIFY
        | EventMask::FOCUS_CHANGE
        | EventMask::BUTTON_PRESS
        | EventMask::BUTTON_RELEASE
        | EventMask::POINTER_MOTION
        | EventMask::ENTER_WINDOW
        | EventMask::LEAVE_WINDOW;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        300,
        200,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .background_pixel(0x0080_8080)
            .event_mask(mask),
    )
    .map_err(err)?;
    conn.change_property32(
        PropMode::REPLACE,
        win,
        atoms.XdndAware,
        AtomEnum::ATOM,
        &[XDND_VERSION],
    )
    .map_err(err)?;
    conn.xfixes_query_version(5, 0)
        .map_err(err)?
        .reply()
        .map_err(err)?;
    let owner_changes = xfixes::SelectionEventMask::SET_SELECTION_OWNER
        | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
        | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE;
    for selection in [atoms.CLIPBOARD, atoms.PRIMARY] {
        conn.xfixes_select_selection_input(win, selection, owner_changes)
            .map_err(err)?;
    }
    // The window manager lists mapped windows in the root's _NET_CLIENT_LIST.
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(err)?;
    conn.flush().map_err(err)?;

    let (tx, rx) = channel();
    let commands = tx.clone();
    read_stdin(
        move |req| {
            let _ = commands.send(Input::Command(req));
        },
        || {},
    );
    forward_events(conn.clone(), tx.clone(), Input::X);

    out::event(Event::Ready {
        backend: Backend::X11,
        pid: std::process::id(),
        globals: Vec::new(),
        data_control: None,
    });
    let mut x = X {
        conn,
        atoms,
        root,
        win,
        rx,
        tx,
        backlog: VecDeque::new(),
        owned: HashMap::new(),
        pending: HashMap::new(),
        incr_in: HashMap::new(),
        incr_out: Vec::new(),
        next_property: 0,
        map_reply: None,
        map_retry: None,
        press_time: None,
        drop_policy: None,
        drop_left: HashMap::new(),
        drop: None,
        drag: None,
        proxy: None,
        manager: None,
        saving: None,
    };
    loop {
        let input = match x.next_input() {
            Ok(input) => input,
            Err(RecvTimeoutError::Timeout) => {
                x.tick();
                let _ = x.conn.flush();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match input {
            Input::Command(req) => {
                if !x.command(req) {
                    break;
                }
            }
            Input::X(e) => x.event(e, false),
            Input::Proxy(e) => x.event(e, true),
            Input::Lost(e) => return Err(format!("x11 connection lost: {e}")),
        }
        x.tick();
        let _ = x.conn.flush();
    }
    Ok(())
}

/// Sends every event of `conn` to the main loop, wrapped by `wrap`, from a thread of its own.
fn forward_events(conn: Arc<RustConnection>, tx: Sender<Input>, wrap: fn(XEvent) -> Input) {
    std::thread::spawn(move || {
        loop {
            match conn.wait_for_event() {
                Ok(e) => {
                    if tx.send(wrap(e)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Input::Lost(e.to_string()));
                    break;
                }
            }
        }
    });
}

impl X {
    /// The next input: the backlog first, then the channel, waking for the next deadline.
    fn next_input(&mut self) -> Result<Input, RecvTimeoutError> {
        if let Some(input) = self.backlog.pop_front() {
            return Ok(input);
        }
        let mut wait = Duration::from_secs(1);
        if let Some(at) = self.map_retry {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
        self.rx.recv_timeout(wait)
    }

    /// Deadlines: map the window again, drop stalled INCR transfers.
    fn tick(&mut self) {
        if self.map_retry.is_some_and(|at| Instant::now() >= at) {
            self.map_again();
        }
        let stall = scaled(INCR_STALL);
        self.incr_out.retain(|o| o.last.elapsed() < stall);
        let stalled: Vec<Atom> = self
            .incr_in
            .iter()
            .filter(|(_, i)| i.last.elapsed() >= stall)
            .map(|(a, _)| *a)
            .collect();
        for atom in stalled {
            if let Some(i) = self.incr_in.remove(&atom) {
                self.finish(i.why, Err("the INCR transfer stalled".into()));
            }
        }
    }

    /// A server timestamp: append nothing to a property of the peer's window and read the time
    /// of the change. Inputs that arrive meanwhile wait in the backlog.
    fn server_time(&mut self) -> Result<u32, String> {
        self.conn
            .change_property8(
                PropMode::APPEND,
                self.win,
                self.atoms.STRATA_PEER_TIME,
                AtomEnum::STRING,
                &[],
            )
            .map_err(err)?;
        self.conn.flush().map_err(err)?;
        let deadline = Instant::now() + scaled(INCR_STALL);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(Input::X(XEvent::PropertyNotify(p)))
                    if p.window == self.win && p.atom == self.atoms.STRATA_PEER_TIME =>
                {
                    return Ok(p.time);
                }
                Ok(other) => self.backlog.push_back(other),
                Err(_) => return Err("x11: no server timestamp".into()),
            }
        }
    }

    fn command(&mut self, req: Request) -> bool {
        let id = req.id;
        let result = match req.command {
            Command::Quit => {
                out::ok(id);
                return false;
            }
            Command::Sync => self
                .conn
                .get_input_focus()
                .map_err(err)
                .and_then(|c| c.reply().map_err(err))
                .map(|_| out::ok(id)),
            Command::Map {
                title,
                width,
                height,
            } => self.map(id, &title, width, height),
            Command::Devices { .. }
            | Command::PointerMove { .. }
            | Command::PointerButton { .. }
            | Command::PointerPath { .. }
            | Command::Key { .. }
            | Command::Type { .. } => Err(
                "input comes from a Wayland peer's virtual devices; Xwayland passes it on".into(),
            ),
            Command::SelectionSet {
                selection,
                via,
                offers,
            } => self.selection_set(id, selection, via, offers),
            Command::SelectionTargets { selection, via } => Self::x11_only(via).and_then(|()| {
                let target = self.atoms.TARGETS;
                self.convert(
                    self.selection_atom(selection),
                    target,
                    Conversion::Targets(id),
                )
            }),
            Command::SelectionRead {
                selection,
                via,
                mime,
            } => Self::x11_only(via).and_then(|()| {
                let target = self.atom(&mime)?;
                self.convert(self.selection_atom(selection), target, Conversion::Read(id))
            }),
            Command::SelectionReadMultiple { selection, mimes } => {
                self.read_multiple(id, selection, &mimes)
            }
            Command::DragStart { offers, actions } => self.drag_start(id, offers, actions),
            Command::DropAccept {
                mimes,
                actions,
                preferred,
                read_all,
                ask_answer,
            } => {
                self.drop_policy = (!mimes.is_empty()).then_some(DropPolicy {
                    mimes,
                    actions,
                    preferred,
                    read_all,
                    ask_answer,
                });
                out::ok(id);
                Ok(())
            }
            Command::DropProxy => self.drop_proxy().map(|()| out::ok(id)),
            Command::ActivationToken => {
                Err("xdg_activation_v1 is Wayland's; an X11 peer has no token".into())
            }
            Command::Export => {
                out::value(
                    id,
                    Exported {
                        handle: format!("x11:{:x}", self.win),
                    },
                );
                Ok(())
            }
            Command::WindowsOfType { ty } => self.windows_of_type(&ty).map(|w| out::value(id, w)),
            Command::ClipboardManager { answer } => self.server_time().and_then(|time| {
                let manager = self.atoms.CLIPBOARD_MANAGER;
                self.own(manager, Vec::new(), time)?;
                self.manager = Some(answer);
                out::ok(id);
                Ok(())
            }),
        };
        if let Err(e) = result {
            out::reply(id, Err(e));
        }
        true
    }

    fn x11_only(via: Via) -> Result<(), String> {
        match via {
            Via::X11 => Ok(()),
            other => Err(format!("an X11 peer has no {other:?}; use via: x11")),
        }
    }

    fn atom(&self, name: &str) -> Result<Atom, String> {
        Ok(self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(err)?
            .reply()
            .map_err(err)?
            .atom)
    }

    fn atom_name(&self, atom: Atom) -> String {
        self.conn
            .get_atom_name(atom)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .unwrap_or_default()
    }

    /// The root's children whose `_NET_WM_WINDOW_TYPE` lists `ty`.
    fn windows_of_type(&self, ty: &str) -> Result<Vec<TypedWindow>, String> {
        let (prop, want) = (self.atom("_NET_WM_WINDOW_TYPE")?, self.atom(ty)?);
        let tree = self
            .conn
            .query_tree(self.root)
            .map_err(err)?
            .reply()
            .map_err(err)?;
        let mut out = Vec::new();
        for w in tree.children {
            let listed = self
                .conn
                .get_property(false, w, prop, AtomEnum::ATOM, 0, 16)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().map(|mut v| v.any(|a| a == want)))
                .unwrap_or(false);
            if !listed {
                continue;
            }
            let geo = self
                .conn
                .get_geometry(w)
                .map_err(err)?
                .reply()
                .map_err(err)?;
            let attrs = self
                .conn
                .get_window_attributes(w)
                .map_err(err)?
                .reply()
                .map_err(err)?;
            out.push(TypedWindow {
                xid: w,
                width: geo.width,
                height: geo.height,
                mapped: attrs.map_state == x11rb::protocol::xproto::MapState::VIEWABLE,
            });
        }
        Ok(out)
    }

    /// The first 32-bit value of a window property, if it has one.
    fn property32(&self, window: Window, property: Atom, ty: AtomEnum) -> Option<u32> {
        self.conn
            .get_property(false, window, property, ty, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    }

    fn selection_atom(&self, selection: Selection) -> Atom {
        match selection {
            Selection::Clipboard => self.atoms.CLIPBOARD,
            Selection::Primary => self.atoms.PRIMARY,
        }
    }

    fn selection_of(&self, atom: Atom) -> Option<Selection> {
        if atom == self.atoms.CLIPBOARD {
            Some(Selection::Clipboard)
        } else if atom == self.atoms.PRIMARY {
            Some(Selection::Primary)
        } else {
            None
        }
    }

    fn action_atom(&self, action: Action) -> Atom {
        match action {
            Action::Copy => self.atoms.XdndActionCopy,
            Action::Move => self.atoms.XdndActionMove,
            Action::Ask => self.atoms.XdndActionAsk,
            Action::Link => self.atoms.XdndActionLink,
        }
    }

    fn action_of(&self, atom: Atom) -> Option<Action> {
        [Action::Copy, Action::Move, Action::Ask, Action::Link]
            .into_iter()
            .find(|a| self.action_atom(*a) == atom)
    }

    fn map(&mut self, id: u64, title: &str, width: u32, height: u32) -> Result<(), String> {
        let c = &self.conn;
        let class = b"strata-test-peer\0strata-test-peer\0";
        c.change_property8(
            PropMode::REPLACE,
            self.win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )
        .map_err(err)?;
        c.change_property8(
            PropMode::REPLACE,
            self.win,
            self.atoms._NET_WM_NAME,
            self.atoms.UTF8_STRING,
            title.as_bytes(),
        )
        .map_err(err)?;
        c.change_property8(
            PropMode::REPLACE,
            self.win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            class,
        )
        .map_err(err)?;
        c.change_property32(
            PropMode::REPLACE,
            self.win,
            self.atoms._NET_WM_PID,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )
        .map_err(err)?;
        c.configure_window(
            self.win,
            &ConfigureWindowAux::new().width(width).height(height),
        )
        .map_err(err)?;
        c.map_window(self.win).map_err(err)?;
        self.map_reply = Some(id);
        self.map_retry = Some(Instant::now() + scaled(MAP_RETRY));
        Ok(())
    }

    /// Whether the window manager lists the peer's window as mapped.
    fn listed(&self) -> bool {
        self.conn
            .get_property(
                false,
                self.root,
                self.atoms._NET_CLIENT_LIST,
                AtomEnum::WINDOW,
                0,
                4096,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|mut v| v.any(|w| w == self.win)))
            .unwrap_or(false)
    }

    /// Replies to `map` once the window is listed.
    fn check_mapped(&mut self) {
        if self.map_reply.is_some() && self.listed() {
            self.map_retry = None;
            if let Some(id) = self.map_reply.take() {
                out::value(
                    id,
                    Mapped {
                        xid: Some(self.win),
                    },
                );
            }
        }
    }

    /// The window is still not listed: unmap it and map it again (see the module docs).
    fn map_again(&mut self) {
        self.check_mapped();
        if self.map_retry.is_none() {
            return;
        }
        eprintln!("strata-test-peer: window not mapped by the compositor; mapping it again");
        let _ = self.conn.unmap_window(self.win);
        let _ = self.conn.map_window(self.win);
        self.map_retry = Some(Instant::now() + scaled(MAP_RETRY));
    }

    /// Takes `selection` with `offers` at `time`, and checks the server agrees.
    fn own(&mut self, selection: Atom, offers: Vec<Offer>, time: u32) -> Result<(), String> {
        self.owned.insert(selection, Owned { offers, time });
        self.conn
            .set_selection_owner(self.win, selection, time)
            .map_err(err)?;
        let owner = self
            .conn
            .get_selection_owner(selection)
            .map_err(err)?
            .reply()
            .map_err(err)?
            .owner;
        if owner != self.win {
            self.owned.remove(&selection);
            return Err("the X server did not give the peer the selection".into());
        }
        Ok(())
    }

    fn selection_set(
        &mut self,
        id: u64,
        selection: Selection,
        via: Via,
        offers: Vec<Offer>,
    ) -> Result<(), String> {
        Self::x11_only(via)?;
        let time = self.server_time()?;
        self.own(self.selection_atom(selection), offers, time)?;
        out::ok(id);
        Ok(())
    }

    /// Asks the selection's owner for `target` now, into a property of the peer's own.
    fn convert(&mut self, selection: Atom, target: Atom, why: Conversion) -> Result<(), String> {
        let time = self.server_time()?;
        self.convert_at(selection, target, why, time)
    }

    fn convert_at(
        &mut self,
        selection: Atom,
        target: Atom,
        why: Conversion,
        time: u32,
    ) -> Result<(), String> {
        self.next_property += 1;
        let property = self.atom(&format!("STRATA_PEER_{}", self.next_property))?;
        self.pending.insert(property, (selection, target, why));
        self.conn
            .convert_selection(self.win, selection, target, property, time)
            .map_err(err)?;
        Ok(())
    }

    /// Points the window's `XdndProxy` at a window on a second connection.
    fn drop_proxy(&mut self) -> Result<(), String> {
        let (conn, _) = x11rb::connect(None).map_err(err)?;
        let conn = Arc::new(conn);
        let proxy = conn.generate_id().map_err(err)?;
        conn.create_window(
            0,
            proxy,
            self.root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1),
        )
        .map_err(err)?;
        // The spec: the proxy's own XdndProxy names itself, so a source can trust it.
        conn.change_property32(
            PropMode::REPLACE,
            proxy,
            self.atoms.XdndProxy,
            AtomEnum::WINDOW,
            &[proxy],
        )
        .map_err(err)?;
        conn.flush().map_err(err)?;
        self.conn
            .change_property32(
                PropMode::REPLACE,
                self.win,
                self.atoms.XdndProxy,
                AtomEnum::WINDOW,
                &[proxy],
            )
            .map_err(err)?;
        forward_events(conn.clone(), self.tx.clone(), Input::Proxy);
        self.proxy = Some(Proxy { _conn: conn });
        Ok(())
    }

    fn drag_start(
        &mut self,
        id: u64,
        offers: Vec<Offer>,
        actions: Vec<Action>,
    ) -> Result<(), String> {
        let Some(time) = self.press_time else {
            return Err("a drag needs a press on the peer's window that is still held".into());
        };
        let mimes: Vec<String> = offers.iter().map(|o| o.mime.clone()).collect();
        let types = mimes
            .iter()
            .map(|m| self.atom(m))
            .collect::<Result<Vec<_>, _>>()?;
        self.conn
            .change_property32(
                PropMode::REPLACE,
                self.win,
                self.atoms.XdndTypeList,
                AtomEnum::ATOM,
                &types,
            )
            .map_err(err)?;
        let listed: Vec<Atom> = actions.iter().map(|a| self.action_atom(*a)).collect();
        self.conn
            .change_property32(
                PropMode::REPLACE,
                self.win,
                self.atoms.XdndActionList,
                AtomEnum::ATOM,
                &listed,
            )
            .map_err(err)?;
        // The drag's selection carries the time of the press that started it.
        self.own(self.atoms.XdndSelection, offers, time)?;
        self.drag = Some(DragOut {
            mimes,
            action: actions.first().copied().unwrap_or(Action::Copy),
            target: None,
            accepted: false,
        });
        out::ok(id);
        Ok(())
    }

    /// A client message to `dest` about `window` (they differ when `dest` is a proxy).
    fn message_to(&self, dest: Window, window: Window, ty: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, ty, data);
        let _ = self
            .conn
            .send_event(false, dest, EventMask::NO_EVENT, event);
    }

    fn message(&self, to: Window, ty: Atom, data: [u32; 5]) {
        self.message_to(to, to, ty, data);
    }

    // ---- events ----

    fn event(&mut self, e: XEvent, proxied: bool) {
        match e {
            XEvent::ClientMessage(m) => self.client_message(&m, proxied),
            _ if proxied => {}
            XEvent::MapNotify(m) if m.window == self.win => self.check_mapped(),
            XEvent::FocusIn(f) if f.event == self.win && f.mode == NotifyMode::NORMAL => {
                out::event(Event::Focus { focused: true });
            }
            XEvent::FocusOut(f) if f.event == self.win && f.mode == NotifyMode::NORMAL => {
                out::event(Event::Focus { focused: false });
            }
            XEvent::EnterNotify(e) if e.event == self.win => {
                out::event(Event::PointerEnter {
                    x: f64::from(e.event_x),
                    y: f64::from(e.event_y),
                });
            }
            XEvent::LeaveNotify(e) if e.event == self.win => out::event(Event::PointerLeave),
            XEvent::ButtonPress(b) if b.event == self.win => {
                self.press_time = Some(b.time);
                out::event(Event::PointerButton { pressed: true });
            }
            XEvent::ButtonRelease(b) if b.event == self.win => {
                self.press_time = None;
                if self.drag.is_some() {
                    self.drag_release(b.time);
                }
                out::event(Event::PointerButton { pressed: false });
            }
            XEvent::MotionNotify(m) if m.event == self.win => {
                out::event(Event::PointerMotion {
                    x: f64::from(m.event_x),
                    y: f64::from(m.event_y),
                });
                if self.drag.is_some() {
                    self.drag_motion(m.root_x, m.root_y, m.time);
                }
            }
            XEvent::SelectionRequest(r) => self.serve(r),
            XEvent::SelectionClear(c) => {
                self.owned.remove(&c.selection);
                if let Some(selection) = self.selection_of(c.selection) {
                    out::event(Event::SelectionCancelled {
                        selection,
                        via: Via::X11,
                    });
                }
            }
            XEvent::XfixesSelectionNotify(n) => {
                if let Some(selection) = self.selection_of(n.selection) {
                    out::event(Event::SelectionChanged {
                        selection,
                        via: Via::X11,
                        mimes: None,
                    });
                }
            }
            XEvent::SelectionNotify(n) => self.converted(n),
            XEvent::PropertyNotify(p) if p.window == self.root => {
                if p.atom == self.atoms._NET_CLIENT_LIST {
                    self.check_mapped();
                }
            }
            XEvent::PropertyNotify(p) => self.property(p.window, p.atom, p.state),
            _ => {}
        }
    }

    /// A reader asks for one of the peer's selections.
    fn serve(&mut self, r: SelectionRequestEvent) {
        if r.selection == self.atoms.CLIPBOARD_MANAGER
            && r.target == self.atoms.SAVE_TARGETS
            && self.manager.is_some()
        {
            if self.manager == Some(true)
                && let Err(e) = self.start_save(&r)
            {
                out::error(format!("SAVE_TARGETS: {e}"));
                self.notify(&r, x11rb::NONE);
            }
            return;
        }
        let property = if r.property == x11rb::NONE {
            r.target
        } else {
            r.property
        };
        let ok = self.answer(&r, property).unwrap_or_else(|e| {
            out::error(e);
            false
        });
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: r.time,
            requestor: r.requestor,
            selection: r.selection,
            target: r.target,
            property: if ok { property } else { x11rb::NONE },
        };
        let _ = self
            .conn
            .send_event(false, r.requestor, EventMask::NO_EVENT, notify);
    }

    fn answer(&mut self, r: &SelectionRequestEvent, property: Atom) -> Result<bool, String> {
        let Some(owned) = self.owned.get(&r.selection) else {
            return Ok(false);
        };
        // ICCCM: refuse a request from before the peer took the selection.
        if r.time != x11rb::CURRENT_TIME && r.time < owned.time {
            return Ok(false);
        }
        let (offers, owned_at) = (owned.offers.clone(), owned.time);
        let a = &self.atoms;
        if r.target == a.TARGETS {
            let mut list = vec![a.TARGETS, a.TIMESTAMP];
            for o in &offers {
                list.push(self.atom(&o.mime)?);
            }
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    r.requestor,
                    property,
                    AtomEnum::ATOM,
                    &list,
                )
                .map_err(err)?;
            return Ok(true);
        }
        if r.target == a.TIMESTAMP {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    r.requestor,
                    property,
                    AtomEnum::INTEGER,
                    &[owned_at],
                )
                .map_err(err)?;
            return Ok(true);
        }
        if r.target == a.MULTIPLE {
            return Ok(false);
        }
        let mime = self.atom_name(r.target);
        let Some(offer) = offers.iter().find(|o| o.mime == mime) else {
            return Ok(false);
        };
        let data = offer
            .data
            .bytes()
            .map_err(|e| format!("serving {mime}: {e}"))?;
        out::event(Event::SourceSend { mime });
        if data.len() <= INCR_CHUNK {
            self.conn
                .change_property8(PropMode::REPLACE, r.requestor, property, r.target, &data)
                .map_err(err)?;
            return Ok(true);
        }
        // INCR: announce the size, then send a chunk each time the reader deletes the last.
        // The reader's window must report property changes to us; ours already does.
        if r.requestor != self.win {
            self.conn
                .change_window_attributes(
                    r.requestor,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .map_err(err)?;
        }
        self.conn
            .change_property32(
                PropMode::REPLACE,
                r.requestor,
                property,
                self.atoms.INCR,
                &[data.len() as u32],
            )
            .map_err(err)?;
        self.incr_out.push(IncrOut {
            requestor: r.requestor,
            property,
            target: r.target,
            data,
            offset: 0,
            last: Instant::now(),
        });
        Ok(true)
    }

    fn property(&mut self, window: Window, atom: Atom, state: Property) {
        // INCR out: the reader deleted the last chunk; send the next, then an empty one.
        if state == Property::DELETE
            && let Some(i) = self
                .incr_out
                .iter()
                .position(|o| o.requestor == window && o.property == atom)
        {
            let o = &mut self.incr_out[i];
            let end = (o.offset + INCR_CHUNK).min(o.data.len());
            let chunk = o.data[o.offset..end].to_vec();
            o.offset = end;
            o.last = Instant::now();
            let _ = self.conn.change_property8(
                PropMode::REPLACE,
                o.requestor,
                o.property,
                o.target,
                &chunk,
            );
            if chunk.is_empty() {
                self.incr_out.remove(i);
            }
            return;
        }
        // INCR in: a chunk landed on the peer's window; an empty one ends the transfer.
        if window == self.win && state == Property::NEW_VALUE && self.incr_in.contains_key(&atom) {
            let Some(reply) = self
                .conn
                .get_property(true, self.win, atom, AtomEnum::ANY, 0, u32::MAX / 4)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                return;
            };
            if reply.value.is_empty() {
                if let Some(i) = self.incr_in.remove(&atom) {
                    self.finish(i.why, Ok(i.data));
                }
            } else if let Some(i) = self.incr_in.get_mut(&atom) {
                i.data.extend_from_slice(&reply.value);
                i.last = Instant::now();
            }
        }
    }

    /// A conversion the peer asked for has landed, or was refused.
    fn converted(&mut self, n: SelectionNotifyEvent) {
        if n.property == x11rb::NONE {
            // Refused, or nobody owns the selection: the property is gone, so match the
            // selection and target asked for.
            let key = self
                .pending
                .iter()
                .find(|(_, (s, t, _))| *s == n.selection && *t == n.target)
                .map(|(k, _)| *k);
            if let Some((_, _, why)) = key.and_then(|k| self.pending.remove(&k)) {
                self.finish(
                    why,
                    Err("the owner refused the conversion, or there is none".into()),
                );
            }
            return;
        }
        let Some((_, _, why)) = self.pending.remove(&n.property) else {
            return;
        };
        let reply = match self
            .conn
            .get_property(true, self.win, n.property, AtomEnum::ANY, 0, u32::MAX / 4)
            .map_err(err)
            .and_then(|c| c.reply().map_err(err))
        {
            Ok(r) => r,
            Err(e) => return self.finish(why, Err(e)),
        };
        if let Conversion::ReadMultiple(id, props) = why {
            let pairs: Vec<u32> = reply.value32().map(Iterator::collect).unwrap_or_default();
            return self.multiple_read_landed(id, &props, &pairs);
        }
        if matches!(why, Conversion::Multiple) {
            let pairs: Vec<u32> = reply.value32().map(Iterator::collect).unwrap_or_default();
            return self.multiple_landed(&pairs);
        }
        if reply.type_ == self.atoms.INCR {
            self.incr_in.insert(
                n.property,
                IncrIn {
                    why,
                    data: Vec::new(),
                    last: Instant::now(),
                },
            );
            return;
        }
        if matches!(why, Conversion::Targets(_)) {
            let atoms: Vec<Atom> = reply.value32().map(Iterator::collect).unwrap_or_default();
            let mut bytes = Vec::with_capacity(atoms.len() * 4);
            atoms
                .iter()
                .for_each(|a| bytes.extend_from_slice(&a.to_ne_bytes()));
            return self.finish(why, Ok(bytes));
        }
        self.finish(why, Ok(reply.value));
    }

    fn finish(&mut self, why: Conversion, data: Result<Vec<u8>, String>) {
        match why {
            Conversion::ReadMultiple(id, _) => {
                out::reply(id, data.map(|_| serde_json::Value::Null))
            }
            Conversion::Multiple => {
                // Refused as a whole: every part failed.
                if let Some(saving) = self.saving.as_mut() {
                    for part in &mut saving.parts {
                        part.data
                            .get_or_insert_with(|| data.clone().map(|_| Vec::new()));
                    }
                }
                self.save_if_done();
            }
            Conversion::Part(n) => {
                if let Some(part) = self.saving.as_mut().and_then(|s| s.parts.get_mut(n)) {
                    part.data = Some(data);
                }
                self.save_if_done();
            }
            Conversion::Read(id) => out::reply(
                id,
                data.and_then(|d| serde_json::to_value(out::received(&d)).map_err(err)),
            ),
            Conversion::Targets(id) => {
                let mimes = data.ok().map(|bytes| {
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| u32::from_ne_bytes(*c))
                        .filter(|a| {
                            ![
                                self.atoms.TARGETS,
                                self.atoms.TIMESTAMP,
                                self.atoms.MULTIPLE,
                            ]
                            .contains(a)
                        })
                        .map(|a| self.atom_name(a))
                        .filter(|n| !n.is_empty())
                        .collect()
                });
                out::value(id, Targets { mimes });
            }
            Conversion::Drop {
                source,
                mime,
                action,
            } => {
                let left = self.drop_left.entry(source).or_insert((1, true));
                left.0 = left.0.saturating_sub(1);
                left.1 &= data.is_ok();
                let (last, accepted) = (left.0 == 0, left.1);
                if last {
                    self.drop_left.remove(&source);
                }
                // An ask drop is settled with the policy's answer.
                let action = match action {
                    Some(Action::Ask) => self
                        .drop_policy
                        .as_ref()
                        .and_then(|p| p.ask_answer)
                        .or(action),
                    other => other,
                };
                if last {
                    let action_atom = action
                        .filter(|_| accepted)
                        .map_or(x11rb::NONE, |a| self.action_atom(a));
                    self.message(
                        source,
                        self.atoms.XdndFinished,
                        [self.win, u32::from(accepted), action_atom, 0, 0],
                    );
                }
                match data {
                    Ok(d) => out::event(Event::Dropped {
                        mime,
                        action,
                        data: out::received(&d),
                    }),
                    Err(e) => out::error(format!("drop of {mime}: {e}")),
                }
            }
        }
    }

    // ---- a test's MULTIPLE read ----

    fn read_multiple(
        &mut self,
        id: u64,
        selection: Selection,
        mimes: &[String],
    ) -> Result<(), String> {
        let mut pairs = Vec::with_capacity(mimes.len() * 2);
        let mut props = Vec::with_capacity(mimes.len());
        for mime in mimes {
            self.next_property += 1;
            let property = self.atom(&format!("STRATA_PEER_{}", self.next_property))?;
            pairs.extend([self.atom(mime)?, property]);
            props.push(property);
        }
        self.next_property += 1;
        let multi = self.atom(&format!("STRATA_PEER_{}", self.next_property))?;
        self.conn
            .change_property32(
                PropMode::REPLACE,
                self.win,
                multi,
                self.atoms.ATOM_PAIR,
                &pairs,
            )
            .map_err(err)?;
        let sel = self.selection_atom(selection);
        let time = self.server_time()?;
        self.pending.insert(
            multi,
            (
                sel,
                self.atoms.MULTIPLE,
                Conversion::ReadMultiple(id, props),
            ),
        );
        self.conn
            .convert_selection(self.win, sel, self.atoms.MULTIPLE, multi, time)
            .map_err(err)?;
        Ok(())
    }

    fn multiple_read_landed(&mut self, id: u64, props: &[Atom], pairs: &[u32]) {
        let answered: Vec<Atom> = pairs.as_chunks::<2>().0.iter().map(|[_, p]| *p).collect();
        let mut results = Vec::with_capacity(props.len());
        for (i, &property) in props.iter().enumerate() {
            let refused = answered.get(i).is_none_or(|p| *p == x11rb::NONE);
            let data = (!refused)
                .then(|| {
                    self.conn
                        .get_property(true, self.win, property, AtomEnum::ANY, 0, u32::MAX / 4)
                        .ok()
                        .and_then(|c| c.reply().ok())
                })
                .flatten()
                .filter(|r| r.type_ != self.atoms.INCR)
                .map(|r| out::received(&r.value));
            results.push(data);
        }
        out::value(id, MultipleRead { results });
    }

    // ---- clipboard manager ----

    fn notify(&self, r: &SelectionRequestEvent, property: Atom) {
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: r.time,
            requestor: r.requestor,
            selection: r.selection,
            target: r.target,
            property,
        };
        let _ = self
            .conn
            .send_event(false, r.requestor, EventMask::NO_EVENT, notify);
    }

    /// An owner hands `CLIPBOARD` over: take every type it names in one `MULTIPLE`, asked as
    /// gsd-clipboard and xfsettingsd ask (`clipboard-manager.c`, `save_targets`): into the
    /// property `MULTIPLE` itself, each pair `(target, target)`, so each type lands in a
    /// property named after it.
    fn start_save(&mut self, r: &SelectionRequestEvent) -> Result<(), String> {
        if r.property == x11rb::NONE {
            return Err("no property names the types to save".into());
        }
        let targets: Vec<Atom> = self
            .conn
            .get_property(false, r.requestor, r.property, AtomEnum::ATOM, 0, 4096)
            .map_err(err)?
            .reply()
            .map_err(err)?
            .value32()
            .map(Iterator::collect)
            .unwrap_or_default();
        let mut parts = Vec::with_capacity(targets.len());
        let mut pairs = Vec::with_capacity(targets.len() * 2);
        for target in targets {
            let property = target;
            pairs.extend([target, property]);
            parts.push(SavedPart {
                mime: self.atom_name(target),
                target,
                property,
                data: None,
            });
        }
        let multi = self.atoms.MULTIPLE;
        self.conn
            .change_property32(
                PropMode::REPLACE,
                self.win,
                multi,
                self.atoms.ATOM_PAIR,
                &pairs,
            )
            .map_err(err)?;
        let clipboard = self.atoms.CLIPBOARD;
        let multiple = self.atoms.MULTIPLE;
        self.pending
            .insert(multi, (clipboard, multiple, Conversion::Multiple));
        self.conn
            .convert_selection(self.win, clipboard, multiple, multi, r.time)
            .map_err(err)?;
        self.saving = Some(Saving {
            requestor: r.requestor,
            property: r.property,
            time: r.time,
            parts,
        });
        Ok(())
    }

    /// The `MULTIPLE` answer landed: each pair's property holds its data, or INCR's size, or
    /// the pair names `None` (refused).
    fn multiple_landed(&mut self, pairs: &[u32]) {
        let answered: Vec<(Atom, Atom)> = pairs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[t, p]| (*t, *p))
            .collect();
        let Some(n) = self.saving.as_ref().map(|s| s.parts.len()) else {
            return;
        };
        for i in 0..n {
            let (target, property) = {
                let part = &self.saving.as_ref().expect("saving").parts[i];
                (part.target, part.property)
            };
            let answer = answered.iter().find(|(t, _)| *t == target).map(|(_, p)| *p);
            let data = match answer {
                None | Some(x11rb::NONE) => Some(Err("refused".to_owned())),
                Some(p) if p != property => Some(Err("answered into another property".to_owned())),
                Some(_) => match self
                    .conn
                    .get_property(true, self.win, property, AtomEnum::ANY, 0, u32::MAX / 4)
                    .map_err(err)
                    .and_then(|c| c.reply().map_err(err))
                {
                    Ok(reply) if reply.type_ == self.atoms.INCR => {
                        self.incr_in.insert(
                            property,
                            IncrIn {
                                why: Conversion::Part(i),
                                data: Vec::new(),
                                last: Instant::now(),
                            },
                        );
                        None
                    }
                    Ok(reply) => Some(Ok(reply.value)),
                    Err(e) => Some(Err(e)),
                },
            };
            if let Some(data) = data
                && let Some(part) = self.saving.as_mut().map(|s| &mut s.parts[i])
            {
                part.data = Some(data);
            }
        }
        // As gsd: take CLIPBOARD now, while INCR parts may still be coming, and answer the
        // owner once they are all in.
        let clipboard = self.atoms.CLIPBOARD;
        match self.server_time() {
            Ok(time) => {
                if let Err(e) = self.own(clipboard, Vec::new(), time) {
                    out::error(format!("taking the saved clipboard: {e}"));
                }
            }
            Err(e) => out::error(format!("taking the saved clipboard: {e}")),
        }
        self.save_if_done();
    }

    /// Every part is in: serve it from `CLIPBOARD` (taken when the `MULTIPLE` landed), answer
    /// the owner, and say what was saved.
    fn save_if_done(&mut self) {
        if self
            .saving
            .as_ref()
            .is_none_or(|s| s.parts.iter().any(|p| p.data.is_none()))
        {
            return;
        }
        let Some(saving) = self.saving.take() else {
            return;
        };
        let mut offers = Vec::new();
        let (mut saved, mut refused) = (Vec::new(), Vec::new());
        for part in saving.parts {
            match part.data {
                Some(Ok(bytes)) => {
                    let got = out::received(&bytes);
                    let data = match (got.text, got.file) {
                        (Some(text), _) => Payload::Text(text),
                        (None, Some(file)) => Payload::File(file),
                        (None, None) => Payload::Text(String::new()),
                    };
                    offers.push(Offer {
                        mime: part.mime.clone(),
                        data,
                    });
                    saved.push(part.mime);
                }
                _ => refused.push(part.mime),
            }
        }
        if let Some(owned) = self.owned.get_mut(&self.atoms.CLIPBOARD) {
            owned.offers = offers;
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: saving.time,
            requestor: saving.requestor,
            selection: self.atoms.CLIPBOARD_MANAGER,
            target: self.atoms.SAVE_TARGETS,
            property: saving.property,
        };
        let _ = self
            .conn
            .send_event(false, saving.requestor, EventMask::NO_EVENT, notify);
        let _ = self.conn.flush();
        out::event(Event::ManagerSaved { saved, refused });
    }

    // ---- XDND ----

    fn client_message(&mut self, m: &ClientMessageEvent, proxied: bool) {
        let a = &self.atoms;
        let l = m.data.as_data32();
        let ty = m.type_;
        if ty == a.XdndEnter {
            self.drop_enter(l, proxied);
        } else if ty == a.XdndPosition {
            self.drop_position(l);
        } else if ty == a.XdndLeave {
            self.drop = None;
            out::event(Event::DropLeave);
        } else if ty == a.XdndDrop {
            self.drop_drop(l);
        } else if ty == a.XdndStatus {
            let action = self.action_of(l[4]);
            // Only the target under the drag now: a late status from the last one is stale.
            if let Some(d) = self.drag.as_mut()
                && d.target.is_some_and(|t| t.window == l[0])
            {
                d.accepted = l[1] & 1 != 0;
                let action = action.filter(|_| d.accepted);
                out::event(Event::DragAction { action });
            }
        } else if ty == a.XdndFinished {
            let action = self.action_of(l[2]);
            self.drag = None;
            self.owned.remove(&self.atoms.XdndSelection);
            out::event(Event::DragFinished { action });
        }
    }

    fn drop_enter(&mut self, l: [u32; 5], proxied: bool) {
        let source = l[0];
        let mut types: Vec<Atom> = l[2..5].iter().copied().filter(|t| *t != 0).collect();
        if l[1] & 1 != 0
            && let Some(list) = self
                .conn
                .get_property(
                    false,
                    source,
                    self.atoms.XdndTypeList,
                    AtomEnum::ATOM,
                    0,
                    1024,
                )
                .ok()
                .and_then(|c| c.reply().ok())
        {
            types = list.value32().map(Iterator::collect).unwrap_or_default();
        }
        let mimes: Vec<String> = types.into_iter().map(|t| self.atom_name(t)).collect();
        let mime = self
            .drop_policy
            .as_ref()
            .and_then(|p| p.mimes.iter().find(|m| mimes.contains(m)).cloned());
        let reads = match (&mime, &self.drop_policy) {
            (Some(_), Some(p)) if p.read_all => p
                .mimes
                .iter()
                .filter(|m| mimes.contains(m))
                .cloned()
                .collect(),
            (Some(m), _) => vec![m.clone()],
            _ => Vec::new(),
        };
        self.drop = Some(DropIn {
            source,
            mime,
            action: None,
            reads,
        });
        out::event(Event::DropEnter { mimes, proxied });
    }

    fn drop_position(&mut self, l: [u32; 5]) {
        // The source proposes an action (XDND 2 and later); take it if the policy allows it,
        // else the policy's preferred one.
        let proposed = self.action_of(l[4]);
        let chosen = self.drop_policy.as_ref().map(|p| match proposed {
            Some(a) if a == p.preferred || p.actions.contains(&a) => a,
            _ => p.preferred,
        });
        let Some(d) = self.drop.as_mut() else { return };
        d.action = chosen.filter(|_| d.mime.is_some());
        let (source, accept, action) = (d.source, d.mime.is_some(), d.action);
        let (rx, ry) = ((l[2] >> 16) as i16, (l[2] & 0xffff) as i16);
        let local = self
            .conn
            .translate_coordinates(self.root, self.win, rx, ry)
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or((0, 0), |r| (r.dst_x, r.dst_y));
        let action = action.map_or(x11rb::NONE, |a| self.action_atom(a));
        // XdndStatus: bit 0 accepts, bit 1 asks for a position on every motion.
        self.message(
            source,
            self.atoms.XdndStatus,
            [self.win, u32::from(accept) | 2, 0, 0, action],
        );
        out::event(Event::DropMotion {
            x: f64::from(local.0),
            y: f64::from(local.1),
        });
    }

    fn drop_drop(&mut self, l: [u32; 5]) {
        let Some(d) = self.drop.take() else { return };
        if d.mime.is_none() {
            self.message(d.source, self.atoms.XdndFinished, [self.win, 0, 0, 0, 0]);
            return;
        }
        self.drop_left.insert(d.source, (d.reads.len(), true));
        for mime in d.reads {
            let target = match self.atom(&mime) {
                Ok(t) => t,
                Err(e) => return out::error(e),
            };
            let why = Conversion::Drop {
                source: d.source,
                mime,
                action: d.action,
            };
            if let Err(e) = self.convert_at(self.atoms.XdndSelection, target, why, l[2]) {
                out::error(e);
            }
        }
    }

    /// The XdndAware top-level window under a root point, other than the peer's own, with where
    /// its messages go: its `XdndProxy` when the proxy names itself, else the window.
    fn target_at(&self, x: i16, y: i16) -> Option<Target> {
        let mut w = self.root;
        for _ in 0..8 {
            let r = self
                .conn
                .translate_coordinates(self.root, w, x, y)
                .ok()?
                .reply()
                .ok()?;
            if r.child == x11rb::NONE || r.child == self.win {
                return None;
            }
            w = r.child;
            if let Some(version) = self.property32(w, self.atoms.XdndAware, AtomEnum::ATOM) {
                let proxy = self
                    .property32(w, self.atoms.XdndProxy, AtomEnum::WINDOW)
                    .filter(|p| {
                        self.property32(*p, self.atoms.XdndProxy, AtomEnum::WINDOW) == Some(*p)
                    });
                return Some(Target {
                    window: w,
                    dest: proxy.unwrap_or(w),
                    version: version.min(XDND_VERSION),
                });
            }
        }
        None
    }

    fn drag_motion(&mut self, x: i16, y: i16, time: u32) {
        let target = self.target_at(x, y);
        let Some(d) = self.drag.as_ref() else { return };
        let (old, mimes, action) = (d.target, d.mimes.clone(), d.action);
        if old.map(|t| t.window) != target.map(|t| t.window) {
            if let Some(old) = old {
                self.message_to(
                    old.dest,
                    old.window,
                    self.atoms.XdndLeave,
                    [self.win, 0, 0, 0, 0],
                );
            }
            if let Some(t) = target {
                let more = u32::from(mimes.len() > 3);
                let mut l = [self.win, (t.version << 24) | more, 0, 0, 0];
                for (i, m) in mimes.iter().take(3).enumerate() {
                    l[2 + i] = self.atom(m).unwrap_or(x11rb::NONE);
                }
                self.message_to(t.dest, t.window, self.atoms.XdndEnter, l);
            }
            if let Some(d) = self.drag.as_mut() {
                d.target = target;
                d.accepted = false;
            }
        }
        if let Some(t) = target {
            let position = ((x as u16 as u32) << 16) | (y as u16 as u32);
            self.message_to(
                t.dest,
                t.window,
                self.atoms.XdndPosition,
                [self.win, 0, position, time, self.action_atom(action)],
            );
        }
    }

    fn drag_release(&mut self, time: u32) {
        let Some(d) = self.drag.as_ref() else { return };
        match d.target {
            Some(t) if d.accepted => {
                self.message_to(
                    t.dest,
                    t.window,
                    self.atoms.XdndDrop,
                    [self.win, 0, time, 0, 0],
                );
                out::event(Event::DragDropped);
            }
            target => {
                if let Some(t) = target {
                    self.message_to(
                        t.dest,
                        t.window,
                        self.atoms.XdndLeave,
                        [self.win, 0, 0, 0, 0],
                    );
                }
                self.drag = None;
                self.owned.remove(&self.atoms.XdndSelection);
                out::event(Event::DragCancelled);
            }
        }
    }
}
