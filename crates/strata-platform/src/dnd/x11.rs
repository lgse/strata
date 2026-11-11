//! X11: XDND version 5 on P0's X11 actor, through its hook (one connection, one thread, and
//! P0's selection code serving and reading `XdndSelection` with INCR).
//!
//! - **Target:** each Rune window's `XdndProxy` points at the actor's window, which carries
//!   `XdndAware` and its own `XdndProxy` (as the spec asks of a proxy). Sources honouring the
//!   proxy send their messages there, naming the Rune window. winit then hears no XDND,
//!   which also turns off Rune's own X11 `on_drop_files`: an app using P1 takes its file
//!   drops through P1. The property is set again on every attach (a recreated window
//!   starts without it) and removed when the actor stops.
//! - **Source:** winit holds the implicit grab of the press, and a second connection can't
//!   take it (S2, Q6), so the source is grab-less: XI2 raw motion, key and release events on
//!   the root, `QueryPointer` for the position and modifiers, and a walk down to the
//!   `XdndAware` window under the pointer. The proposed action follows Ctrl and Shift
//!   throughout the drag. Only the grabbing client can change the cursor, so a drag shows no
//!   cursor feedback; the icon window follows the pointer instead.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rune::{DragOutcome, Mods};
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, ImageFormat, KeyButMask, PropMode, Rectangle, VisualClass, Window,
    WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use crate::transfer::{ReadError, SourceId};

type Done = Box<dyn FnOnce(Result<Vec<u8>, ReadError>) + Send>;
use crate::transfer::x11::X11;
use crate::transfer::x11::hook::{Hook, HookCx};
use crate::transfer::{ReadLimits, SourceSpec, Transfer};

use super::api::{Actions, DragAction, Error};
use super::hub::InPort;
use super::icon::{DragImage, bgra_premultiplied};
use super::negotiate;

const XDND_VERSION: u32 = 5;
/// How long a drop waits for the target's `XdndFinished`.
const FINISH_WAIT: Duration = Duration::from_secs(10);

/// What the actor thread tells the UI.
pub(crate) enum Note {
    Enter {
        key: String,
        port: XIn,
        mimes: Vec<String>,
        actions: Actions,
        /// In the window's pixels.
        x: f64,
        y: f64,
        mods: Mods,
    },
    Motion {
        key: String,
        x: f64,
        y: f64,
        mods: Mods,
    },
    Leave {
        key: String,
    },
    Drop {
        key: String,
    },
    Ended {
        id: u64,
        outcome: DragOutcome,
    },
}

/// The notes waiting for the UI, and what wakes it.
#[derive(Clone)]
pub(crate) struct Inbox {
    notes: Arc<Mutex<VecDeque<Note>>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Inbox {
    fn push(&self, note: Note) {
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(note);
        (self.notify)();
    }

    pub(crate) fn drain(&self) -> Vec<Note> {
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

/// What the UI asks of the actor thread.
enum Cmd {
    AddWindow {
        key: String,
        xid: Window,
    },
    Status {
        serial: u64,
        accept: bool,
        action: Option<DragAction>,
    },
    Read {
        serial: u64,
        mime: String,
        done: Done,
    },
    Finish {
        serial: u64,
        action: Option<DragAction>,
    },
    Start {
        id: u64,
        spec: SourceSpec,
        actions: Actions,
    },
    Icon {
        id: u64,
        image: Option<DragImage>,
    },
    Cancel {
        id: u64,
    },
    /// The UI saw the real release of drag `id`'s press (its guard did): a backstop for a
    /// release the XI2 events missed.
    Released {
        id: u64,
    },
}

/// Posts commands to the actor, from any thread.
#[derive(Clone)]
pub(crate) struct Handle(Transfer);

impl Handle {
    pub(crate) fn new(t: &Transfer) -> Handle {
        Handle(t.clone())
    }

    fn post(&self, cmd: Cmd) {
        if let Some(x) = self.0.x11() {
            x.post_hook(Box::new(cmd));
        }
    }

    pub(crate) fn icon(&self, id: u64, image: Option<DragImage>) {
        self.post(Cmd::Icon { id, image });
    }
}

/// Installs XDND on the actor. The hook keeps `t` to hand drop ports to the UI; the actor
/// drops it when it stops.
pub(crate) fn install(
    t: &Transfer,
    x: &X11,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> Result<Inbox, Error> {
    let inbox = Inbox {
        notes: Arc::default(),
        notify,
    };
    let hook = Xdnd {
        handle: Handle::new(t),
        inbox: inbox.clone(),
        atoms: None,
        windows: HashMap::new(),
        incoming: None,
        outgoing: None,
        serial: 0,
    };
    if !x.install_hook(Box::new(hook)) {
        return Err(Error::Platform("the X11 connection is gone".into()));
    }
    Ok(inbox)
}

pub(crate) fn add_window(x: &X11, key: &str, xid: Window) {
    x.post_hook(Box::new(Cmd::AddWindow {
        key: key.to_owned(),
        xid,
    }));
}

pub(crate) fn start(x: &X11, id: u64, spec: SourceSpec, actions: Actions) -> Result<(), Error> {
    if x.post_hook(Box::new(Cmd::Start { id, spec, actions })) {
        Ok(())
    } else {
        Err(Error::Platform("the X11 connection is gone".into()))
    }
}

pub(crate) fn released(x: &X11, id: u64) {
    x.post_hook(Box::new(Cmd::Released { id }));
}

pub(crate) fn cancel(x: &X11, id: u64) {
    x.post_hook(Box::new(Cmd::Cancel { id }));
}

/// A drag over one of the app's windows, as the UI answers it.
pub(crate) struct XIn {
    handle: Handle,
    serial: u64,
}

impl InPort for XIn {
    fn status(&self, accept: Option<&str>, action: Option<DragAction>) {
        self.handle.post(Cmd::Status {
            serial: self.serial,
            accept: accept.is_some() && action.is_some(),
            action,
        });
    }

    fn read(&self, mime: &str, done: Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>) {
        self.handle.post(Cmd::Read {
            serial: self.serial,
            mime: mime.to_owned(),
            done: Box::new(move |r| done(r.map_err(|e| e.to_string()))),
        });
    }

    fn dropped_action(&self, chosen: DragAction) -> DragAction {
        chosen
    }

    fn finish(&self, action: Option<DragAction>) {
        self.handle.post(Cmd::Finish {
            serial: self.serial,
            action,
        });
    }
}

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware, XdndProxy, XdndTypeList, XdndActionList, XdndActionDescription,
        XdndSelection, XdndEnter, XdndPosition, XdndStatus, XdndLeave, XdndDrop, XdndFinished,
        XdndActionCopy, XdndActionMove, XdndActionLink, XdndActionAsk, XdndActionPrivate,
        _NET_WM_WINDOW_TYPE, _NET_WM_WINDOW_TYPE_DND,
    }
}

/// A drag over one of our windows.
struct Incoming {
    serial: u64,
    source: Window,
    /// The Rune window it is over.
    target: Window,
    key: String,
    version: u32,
    mimes: Vec<String>,
    /// From `XdndActionList`, if the source set one.
    listed: Actions,
    /// The time of the newest position, for reads during the drag.
    time: u32,
    drop_time: Option<u32>,
    told: bool,
}

/// A target under our drag.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    window: Window,
    dest: Window,
    version: u32,
}

/// The drag this process started.
struct Outgoing {
    id: u64,
    mimes: Vec<Atom>,
    actions: Actions,
    target: Option<Target>,
    accepted: bool,
    action: Option<DragAction>,
    dropped: bool,
    deadline: Option<Instant>,
    icon: Option<IconWin>,
    /// The pointer and modifiers last sent.
    last: Option<(i16, i16, u16)>,
    /// The pointer's button mapping: raw events name physical buttons, and the drag's press
    /// is logical button 1.
    buttons: Vec<u8>,
}

struct IconWin {
    window: Window,
    hotspot: (i16, i16),
}

struct Xdnd {
    handle: Handle,
    inbox: Inbox,
    atoms: Option<Atoms>,
    windows: HashMap<Window, String>,
    incoming: Option<Incoming>,
    outgoing: Option<Outgoing>,
    serial: u64,
}

fn mods_of(mask: KeyButMask) -> Mods {
    Mods {
        shift: mask.contains(KeyButMask::SHIFT),
        ctrl: mask.contains(KeyButMask::CONTROL),
        cmd: mask.contains(KeyButMask::CONTROL),
        ..Mods::default()
    }
}

impl Xdnd {
    /// The atoms, interned the first time the hook runs; and its own window made XDND-aware.
    fn atoms(&mut self, cx: &mut HookCx<'_>) -> Option<Atoms> {
        if self.atoms.is_none() {
            let atoms = Atoms::new(cx.conn()).ok()?.reply().ok()?;
            let win = cx.win();
            let conn = cx.conn();
            let _ = conn.change_property32(
                PropMode::REPLACE,
                win,
                atoms.XdndAware,
                AtomEnum::ATOM,
                &[XDND_VERSION],
            );
            // A proxy's own XdndProxy names itself, so sources trust it.
            let _ = conn.change_property32(
                PropMode::REPLACE,
                win,
                atoms.XdndProxy,
                AtomEnum::WINDOW,
                &[win],
            );
            let _ = conn.xinput_xi_query_version(2, 2).map(|c| c.reply());
            self.atoms = Some(atoms);
        }
        self.atoms
    }

    fn action_atom(a: &Atoms, action: Option<DragAction>) -> Atom {
        match action {
            Some(DragAction::Copy) => a.XdndActionCopy,
            Some(DragAction::Move) => a.XdndActionMove,
            Some(DragAction::Link) => a.XdndActionLink,
            Some(DragAction::Ask) => a.XdndActionAsk,
            None => NONE,
        }
    }

    fn action_of(a: &Atoms, atom: Atom) -> Option<DragAction> {
        [
            (a.XdndActionCopy, DragAction::Copy),
            (a.XdndActionMove, DragAction::Move),
            (a.XdndActionLink, DragAction::Link),
            (a.XdndActionAsk, DragAction::Ask),
            // Private: the source's own meaning; a copy is the safe reading.
            (a.XdndActionPrivate, DragAction::Copy),
        ]
        .into_iter()
        .find(|(x, _)| *x == atom)
        .map(|(_, d)| d)
    }

    fn message(cx: &HookCx<'_>, dest: Window, window: Window, ty: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, ty, data);
        let _ = cx
            .conn()
            .send_event(false, dest, EventMask::NO_EVENT, event);
    }

    fn property32(cx: &HookCx<'_>, w: Window, prop: Atom, ty: impl Into<Atom>) -> Option<Vec<u32>> {
        let r = cx
            .conn()
            .get_property(false, w, prop, ty, 0, 1 << 12)
            .ok()?
            .reply()
            .ok()?;
        let v: Vec<u32> = r.value32()?.collect();
        (!v.is_empty()).then_some(v)
    }

    fn pointer(cx: &HookCx<'_>) -> Option<(i16, i16, KeyButMask)> {
        let p = cx.conn().query_pointer(cx.root()).ok()?.reply().ok()?;
        Some((p.root_x, p.root_y, p.mask))
    }

    // ---- target: drags over the app's windows ----

    fn enter(&mut self, cx: &mut HookCx<'_>, a: &Atoms, m: &ClientMessageEvent) {
        let l = m.data.as_data32();
        let Some(key) = self.window_key(m.window) else {
            return;
        };
        let source = l[0];
        let version = l[1] >> 24;
        let atoms: Vec<Atom> = if l[1] & 1 != 0 {
            Self::property32(cx, source, a.XdndTypeList, AtomEnum::ATOM).unwrap_or_default()
        } else {
            l[2..5].iter().copied().filter(|t| *t != NONE).collect()
        };
        let mimes = atoms.into_iter().filter_map(|t| cx.name(t)).collect();
        let listed = Self::property32(cx, source, a.XdndActionList, AtomEnum::ATOM)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| Self::action_of(a, t))
            .collect();
        self.serial += 1;
        self.incoming = Some(Incoming {
            serial: self.serial,
            source,
            target: m.window,
            key,
            version,
            mimes,
            listed,
            time: CURRENT_TIME,
            drop_time: None,
            told: false,
        });
    }

    fn window_key(&self, w: Window) -> Option<String> {
        self.windows.get(&w).cloned().or_else(|| {
            // A source that names the proxy instead of the target: one window can only be it.
            (self.windows.len() == 1)
                .then(|| self.windows.values().next().cloned())
                .flatten()
        })
    }

    fn position(&mut self, cx: &mut HookCx<'_>, a: &Atoms, m: &ClientMessageEvent) {
        let l = m.data.as_data32();
        let Some(inc) = self.incoming.as_mut().filter(|i| i.source == l[0]) else {
            return;
        };
        let (rx, ry) = ((l[2] >> 16) as i16, (l[2] & 0xffff) as i16);
        inc.time = l[3];
        let proposed = if inc.version >= 2 {
            Self::action_of(a, l[4])
        } else {
            Some(DragAction::Copy)
        };
        let local = cx
            .conn()
            .translate_coordinates(cx.root(), inc.target, rx, ry)
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or((0.0, 0.0), |r| (f64::from(r.dst_x), f64::from(r.dst_y)));
        let mods = Self::pointer(cx).map_or(Mods::default(), |(_, _, m)| mods_of(m));
        let note = if inc.told {
            Note::Motion {
                key: inc.key.clone(),
                x: local.0,
                y: local.1,
                mods,
            }
        } else {
            inc.told = true;
            let actions = proposed.map_or(inc.listed, |p| inc.listed | Actions::of(p));
            Note::Enter {
                key: inc.key.clone(),
                port: XIn {
                    handle: self.handle.clone(),
                    serial: inc.serial,
                },
                mimes: inc.mimes.clone(),
                actions,
                x: local.0,
                y: local.1,
                mods,
            }
        };
        self.inbox.push(note);
    }

    fn status(
        &mut self,
        cx: &mut HookCx<'_>,
        a: &Atoms,
        serial: u64,
        accept: bool,
        action: Option<DragAction>,
    ) {
        let Some(inc) = self
            .incoming
            .as_ref()
            .filter(|i| i.serial == serial && i.drop_time.is_none())
        else {
            return;
        };
        let action = if accept {
            Self::action_atom(a, action)
        } else {
            NONE
        };
        // Bit 0 accepts; bit 1 asks for a position on every motion.
        Self::message(
            cx,
            inc.source,
            inc.source,
            a.XdndStatus,
            [inc.target, u32::from(accept) | 2, 0, 0, action],
        );
    }

    fn finish(&mut self, cx: &mut HookCx<'_>, a: &Atoms, serial: u64, action: Option<DragAction>) {
        let Some(inc) = self.incoming.take_if(|i| i.serial == serial) else {
            return;
        };
        let accepted = action.is_some();
        let data = if inc.version >= 5 {
            [
                inc.target,
                u32::from(accepted),
                Self::action_atom(a, action),
                0,
                0,
            ]
        } else {
            [inc.target, 0, 0, 0, 0]
        };
        Self::message(cx, inc.source, inc.source, a.XdndFinished, data);
    }

    // ---- source: the drag this process started ----

    fn start(
        &mut self,
        cx: &mut HookCx<'_>,
        a: &Atoms,
        id: u64,
        spec: SourceSpec,
        actions: Actions,
    ) {
        self.end(cx, a, None);
        let mimes: Vec<Atom> = spec.mimes().filter_map(|m| cx.atom(m)).collect();
        let win = cx.win();
        let _ = cx.conn().change_property32(
            PropMode::REPLACE,
            win,
            a.XdndTypeList,
            AtomEnum::ATOM,
            &mimes,
        );
        let listed: Vec<Atom> = actions
            .iter()
            .map(|d| Self::action_atom(a, Some(d)))
            .collect();
        let _ = cx.conn().change_property32(
            PropMode::REPLACE,
            win,
            a.XdndActionList,
            AtomEnum::ATOM,
            &listed,
        );
        // With ask listed, a target that asks shows each action's description, in the
        // list's order, NUL-separated.
        if actions.has(DragAction::Ask) {
            let text = descriptions(actions);
            let _ = cx.conn().change_property8(
                PropMode::REPLACE,
                win,
                a.XdndActionDescription,
                AtomEnum::STRING,
                &text,
            );
        } else {
            let _ = cx.conn().delete_property(win, a.XdndActionDescription);
        }
        cx.own(a.XdndSelection, SourceId::next(), spec);
        let buttons = cx
            .conn()
            .get_pointer_mapping()
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.map)
            .unwrap_or_default();
        let mask = xinput::XIEventMask::RAW_MOTION
            | xinput::XIEventMask::RAW_BUTTON_RELEASE
            | xinput::XIEventMask::RAW_KEY_PRESS
            | xinput::XIEventMask::RAW_KEY_RELEASE;
        let _ = cx.conn().xinput_xi_select_events(
            cx.root(),
            &[xinput::EventMask {
                deviceid: 1, // XIAllMasterDevices
                mask: vec![mask],
            }],
        );
        self.outgoing = Some(Outgoing {
            id,
            mimes,
            actions,
            target: None,
            accepted: false,
            action: None,
            dropped: false,
            deadline: None,
            icon: None,
            last: None,
            buttons,
        });
        self.motion(cx, a, CURRENT_TIME, true);
        // The XI2 events are selected only now, and a fast flick's release may already
        // have happened (review round 1, B1): no raw release would ever come, and the drag
        // would drop on the next click anywhere. The pointer's own state says.
        let held = Self::pointer(cx).is_some_and(|(_, _, m)| m.contains(KeyButMask::BUTTON1));
        if !held {
            self.release(cx, a, CURRENT_TIME);
        }
    }

    /// Whether a raw event's button (physical) is the logical primary button.
    fn primary(&self, detail: u32) -> bool {
        let map = self.outgoing.as_ref().map_or(&[][..], |o| &o.buttons[..]);
        is_primary(map, detail)
    }

    /// The XDND-aware window under a root point, with where its messages go.
    fn target_at(
        cx: &HookCx<'_>,
        a: &Atoms,
        x: i16,
        y: i16,
        skip: Option<Window>,
    ) -> Option<Target> {
        let mut w = cx.root();
        for _ in 0..8 {
            let r = cx
                .conn()
                .translate_coordinates(cx.root(), w, x, y)
                .ok()?
                .reply()
                .ok()?;
            if r.child == NONE || Some(r.child) == skip {
                return None;
            }
            w = r.child;
            if let Some(v) = Self::property32(cx, w, a.XdndAware, AtomEnum::ATOM) {
                let proxy = Self::property32(cx, w, a.XdndProxy, AtomEnum::WINDOW)
                    .map(|p| p[0])
                    .filter(|p| {
                        Self::property32(cx, *p, a.XdndProxy, AtomEnum::WINDOW).map(|q| q[0])
                            == Some(*p)
                    });
                return Some(Target {
                    window: w,
                    dest: proxy.unwrap_or(w),
                    version: v[0].min(XDND_VERSION),
                });
            }
        }
        None
    }

    fn motion(&mut self, cx: &mut HookCx<'_>, a: &Atoms, time: u32, force: bool) {
        let Some((x, y, mask)) = Self::pointer(cx) else {
            return;
        };
        let win = cx.win();
        let Some(out) = self.outgoing.as_mut().filter(|o| !o.dropped) else {
            return;
        };
        if !force && out.last == Some((x, y, mask.bits())) {
            return;
        }
        out.last = Some((x, y, mask.bits()));
        if let Some(icon) = &out.icon {
            let _ = cx.conn().configure_window(
                icon.window,
                &ConfigureWindowAux::new()
                    .x(i32::from(x) - i32::from(icon.hotspot.0))
                    .y(i32::from(y) - i32::from(icon.hotspot.1)),
            );
        }
        let skip = out.icon.as_ref().map(|i| i.window);
        let target = Self::target_at(cx, a, x, y, skip);
        if out.target.map(|t| t.window) != target.map(|t| t.window) {
            if let Some(old) = out.target {
                Self::message(cx, old.dest, old.window, a.XdndLeave, [win, 0, 0, 0, 0]);
            }
            if let Some(t) = target {
                let more = u32::from(out.mimes.len() > 3);
                let mut l = [win, (t.version << 24) | more, 0, 0, 0];
                for (i, m) in out.mimes.iter().take(3).enumerate() {
                    l[2 + i] = *m;
                }
                Self::message(cx, t.dest, t.window, a.XdndEnter, l);
            }
            out.target = target;
            out.accepted = false;
            out.action = None;
        }
        if let Some(t) = target {
            let proposed = negotiate::proposed_action(out.actions, mods_of(mask));
            let position = ((x as u16 as u32) << 16) | (y as u16 as u32);
            Self::message(
                cx,
                t.dest,
                t.window,
                a.XdndPosition,
                [win, 0, position, time, Self::action_atom(a, proposed)],
            );
        }
        cx.flush();
    }

    fn release(&mut self, cx: &mut HookCx<'_>, a: &Atoms, time: u32) {
        let win = cx.win();
        let Some(out) = self.outgoing.as_mut().filter(|o| !o.dropped) else {
            return;
        };
        match out.target {
            Some(t) if out.accepted => {
                Self::message(cx, t.dest, t.window, a.XdndDrop, [win, 0, time, 0, 0]);
                out.dropped = true;
                out.deadline = Some(Instant::now() + FINISH_WAIT);
                if let Some(icon) = out.icon.take() {
                    let _ = cx.conn().destroy_window(icon.window);
                }
                cx.flush();
            }
            _ => self.end(cx, a, Some(DragOutcome::Cancelled)),
        }
    }

    /// Ends the drag this process started, telling the UI `outcome` (None: say nothing).
    fn end(&mut self, cx: &mut HookCx<'_>, a: &Atoms, outcome: Option<DragOutcome>) {
        let Some(out) = self.outgoing.take() else {
            return;
        };
        let win = cx.win();
        if !out.dropped
            && let Some(t) = out.target
        {
            Self::message(cx, t.dest, t.window, a.XdndLeave, [win, 0, 0, 0, 0]);
        }
        let _ = cx.conn().xinput_xi_select_events(
            cx.root(),
            &[xinput::EventMask {
                deviceid: 1,
                mask: vec![xinput::XIEventMask::from(0u32)],
            }],
        );
        cx.disown(a.XdndSelection);
        if let Some(icon) = out.icon {
            let _ = cx.conn().destroy_window(icon.window);
        }
        cx.flush();
        if let Some(outcome) = outcome {
            self.inbox.push(Note::Ended {
                id: out.id,
                outcome,
            });
        }
    }

    fn icon(&mut self, cx: &mut HookCx<'_>, id: u64, image: &DragImage) {
        let Some(a) = self.atoms else {
            return;
        };
        let Some(out) = self.outgoing.as_mut().filter(|o| o.id == id && !o.dropped) else {
            return;
        };
        match icon_window(cx, &a, image) {
            Ok(icon) => out.icon = Some(icon),
            Err(e) => hub_note(&format!("no X11 drag icon: {e}")),
        }
        self.motion(cx, &a, CURRENT_TIME, true);
    }
}

/// Diagnostics from the actor thread go to stderr: the UI's diagnostic hook is the UI
/// thread's.
fn hub_note(message: &str) {
    eprintln!("strata-platform dnd: {message}");
}

/// The largest icon side, in pixels: a bigger picture is cropped to it.
const ICON_MAX: u32 = 1024;

/// Whether physical button `detail` (what XI2 raw events name) is logical button 1 under the
/// pointer's button mapping `map` (`GetPointerMapping`: logical number by physical index).
fn is_primary(map: &[u8], detail: u32) -> bool {
    let logical = usize::try_from(detail)
        .ok()
        .and_then(|d| d.checked_sub(1))
        .and_then(|i| map.get(i).copied())
        .map_or(detail, u32::from);
    logical == 1
}

/// `XdndActionDescription`: each listed action's name, in the list's order, NUL-separated.
fn descriptions(actions: Actions) -> Vec<u8> {
    actions
        .iter()
        .flat_map(|d| {
            let name = match d {
                DragAction::Copy => "Copy",
                DragAction::Move => "Move",
                DragAction::Link => "Link",
                DragAction::Ask => "Ask",
            };
            name.bytes().chain([0])
        })
        .collect()
}

/// The icon window's size and its premultiplied BGRA pixels: cropped to [`ICON_MAX`] a side,
/// since X window sizes are 16-bit and a wider picture would wrap.
fn icon_pixels(image: &DragImage) -> (u16, u16, Vec<u8>) {
    let (w, h) = (image.width.min(ICON_MAX), image.height.min(ICON_MAX));
    let full = bgra_premultiplied(&image.rgba);
    let src_row = image.width as usize * 4;
    let pixels = full
        .chunks(src_row)
        .take(h as usize)
        .flat_map(|r| r[..w as usize * 4].iter().copied())
        .collect();
    (w as u16, h as u16, pixels)
}

/// An override-redirect window with a 32-bit visual showing `image`, taking no input, so the
/// walk to the target under the pointer passes through it.
fn icon_window(cx: &mut HookCx<'_>, a: &Atoms, image: &DragImage) -> Result<IconWin, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let conn = cx.conn();
    let screen = &conn.setup().roots[0];
    let visual = screen
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
        .ok_or("no 32-bit visual")?;
    let (w, h, pixels) = icon_pixels(image);
    let cmap = conn.generate_id().map_err(|e| err(&e))?;
    conn.create_colormap(
        x11rb::protocol::xproto::ColormapAlloc::NONE,
        cmap,
        cx.root(),
        visual,
    )
    .map_err(|e| err(&e))?;
    let win = conn.generate_id().map_err(|e| err(&e))?;
    conn.create_window(
        32,
        win,
        cx.root(),
        -1000,
        -1000,
        w,
        h,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .border_pixel(0)
            .background_pixel(0)
            .colormap(cmap),
    )
    .map_err(|e| err(&e))?;
    // An empty input region: the pointer is never "in" the icon.
    let _ = conn.shape_rectangles(
        shape::SO::SET,
        shape::SK::INPUT,
        x11rb::protocol::xproto::ClipOrdering::UNSORTED,
        win,
        0,
        0,
        &[] as &[Rectangle],
    );
    let _ = conn.change_property32(
        PropMode::REPLACE,
        win,
        a._NET_WM_WINDOW_TYPE,
        AtomEnum::ATOM,
        &[a._NET_WM_WINDOW_TYPE_DND],
    );
    let gc = conn.generate_id().map_err(|e| err(&e))?;
    conn.create_gc(gc, win, &CreateGCAux::new())
        .map_err(|e| err(&e))?;
    conn.map_window(win).map_err(|e| err(&e))?;
    let row = usize::from(w) * 4;
    // Rows per request, under the server's request limit.
    let per = (conn.maximum_request_bytes().saturating_sub(64) / row.max(1)).max(1);
    for (i, chunk) in pixels.chunks(row * per).enumerate() {
        let rows = (chunk.len() / row) as u16;
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            w,
            rows,
            0,
            (i * per) as i16,
            0,
            32,
            chunk,
        );
    }
    let _ = conn.free_gc(gc);
    let _ = conn.free_colormap(cmap);
    let hot = |pt: f32| (pt * image.scale).round() as i16;
    Ok(IconWin {
        window: win,
        hotspot: (hot(image.hotspot.0), hot(image.hotspot.1)),
    })
}

impl Hook for Xdnd {
    fn event(&mut self, cx: &mut HookCx<'_>, event: &XEvent) {
        let Some(a) = self.atoms(cx) else {
            return;
        };
        match event {
            XEvent::ClientMessage(m) => {
                let ty = m.type_;
                if ty == a.XdndEnter {
                    self.enter(cx, &a, m);
                } else if ty == a.XdndPosition {
                    self.position(cx, &a, m);
                } else if ty == a.XdndLeave {
                    let l = m.data.as_data32();
                    if let Some(inc) = self
                        .incoming
                        .take_if(|i| i.source == l[0] && i.drop_time.is_none())
                        && inc.told
                    {
                        self.inbox.push(Note::Leave { key: inc.key });
                    }
                } else if ty == a.XdndDrop {
                    let l = m.data.as_data32();
                    let win = cx.win();
                    match self.incoming.as_mut().filter(|i| i.source == l[0]) {
                        Some(inc) if inc.told => {
                            inc.drop_time = Some(l[2]);
                            self.inbox.push(Note::Drop {
                                key: inc.key.clone(),
                            });
                        }
                        _ => {
                            // Nothing heard yet to drop on: refuse.
                            Self::message(cx, l[0], l[0], a.XdndFinished, [win, 0, 0, 0, 0]);
                            self.incoming = None;
                        }
                    }
                } else if ty == a.XdndStatus {
                    let l = m.data.as_data32();
                    if let Some(out) = self
                        .outgoing
                        .as_mut()
                        .filter(|o| o.target.is_some_and(|t| t.window == l[0]))
                    {
                        out.accepted = l[1] & 1 != 0;
                        out.action = Self::action_of(&a, l[4]);
                    }
                } else if ty == a.XdndFinished {
                    let l = m.data.as_data32();
                    let ours = self
                        .outgoing
                        .as_ref()
                        .is_some_and(|o| o.dropped && o.target.is_some_and(|t| t.window == l[0]));
                    if ours {
                        let version = self
                            .outgoing
                            .as_ref()
                            .and_then(|o| o.target)
                            .map_or(0, |t| t.version);
                        let accepted = version < 5 || l[1] & 1 != 0;
                        let action = if version >= 5 {
                            Self::action_of(&a, l[2])
                        } else {
                            self.outgoing.as_ref().and_then(|o| o.action)
                        };
                        let outcome = match (accepted, action) {
                            (false, _) => DragOutcome::Cancelled,
                            (true, Some(DragAction::Move)) => DragOutcome::Moved,
                            (true, Some(DragAction::Link)) => DragOutcome::Linked,
                            (true, _) => DragOutcome::Copied,
                        };
                        self.end(cx, &a, Some(outcome));
                    }
                }
            }
            XEvent::XinputRawMotion(e) => self.motion(cx, &a, e.time, false),
            XEvent::XinputRawKeyPress(e) | XEvent::XinputRawKeyRelease(e) => {
                // A modifier pressed or let go: propose the action again.
                self.motion(cx, &a, e.time, false);
            }
            XEvent::XinputRawButtonRelease(e) if self.primary(e.detail) => {
                self.release(cx, &a, e.time);
            }
            _ => {}
        }
    }

    fn command(&mut self, cx: &mut HookCx<'_>, cmd: Box<dyn std::any::Any + Send>) {
        let Ok(cmd) = cmd.downcast::<Cmd>() else {
            return;
        };
        let Some(a) = self.atoms(cx) else {
            return;
        };
        match *cmd {
            Cmd::AddWindow { key, xid } => {
                let _ = cx.conn().change_property32(
                    PropMode::REPLACE,
                    xid,
                    a.XdndProxy,
                    AtomEnum::WINDOW,
                    &[cx.win()],
                );
                self.windows.retain(|_, k| *k != key);
                self.windows.insert(xid, key);
                cx.flush();
            }
            Cmd::Status {
                serial,
                accept,
                action,
            } => self.status(cx, &a, serial, accept, action),
            Cmd::Read { serial, mime, done } => {
                match self.incoming.as_ref().filter(|i| i.serial == serial) {
                    Some(inc) => {
                        let time = inc.drop_time.unwrap_or(inc.time);
                        cx.read(a.XdndSelection, &mime, time, ReadLimits::default(), done);
                    }
                    None => done(Err(ReadError::Gone)),
                }
            }
            Cmd::Finish { serial, action } => self.finish(cx, &a, serial, action),
            Cmd::Start { id, spec, actions } => self.start(cx, &a, id, spec, actions),
            Cmd::Icon { id, image } => {
                if let Some(image) = image {
                    self.icon(cx, id, &image);
                }
            }
            Cmd::Cancel { id } => {
                if self.outgoing.as_ref().is_some_and(|o| o.id == id) {
                    // The UI has told the window already.
                    self.end(cx, &a, None);
                }
            }
            Cmd::Released { id } => {
                // Already dropped or ended when the raw release came first: nothing to do.
                // Otherwise it drops on the last target the drag reached (or cancels), with
                // CurrentTime: the release's own time is the X server's, which Rune's Up
                // doesn't carry, and XDND accepts CurrentTime for XdndDrop.
                if self
                    .outgoing
                    .as_ref()
                    .is_some_and(|o| o.id == id && !o.dropped)
                {
                    self.release(cx, &a, CURRENT_TIME);
                }
            }
        }
        cx.flush();
    }

    fn deadline(&self) -> Option<Instant> {
        self.outgoing.as_ref().and_then(|o| o.deadline)
    }

    fn tick(&mut self, cx: &mut HookCx<'_>, now: Instant) {
        let Some(a) = self.atoms else {
            return;
        };
        if self
            .outgoing
            .as_ref()
            .and_then(|o| o.deadline)
            .is_some_and(|d| d <= now)
        {
            hub_note("the drop target never sent XdndFinished: the drag ends cancelled");
            self.end(cx, &a, Some(DragOutcome::Cancelled));
        }
    }

    fn stop(&mut self, cx: &mut HookCx<'_>) {
        let Some(a) = self.atoms else {
            return;
        };
        self.end(cx, &a, None);
        // The app's windows outlive the actor: sources must stop sending to our window.
        for xid in self.windows.keys() {
            let _ = cx.conn().delete_property(*xid, a.XdndProxy);
        }
        cx.flush();
    }
}

#[cfg(test)]
mod tests;
