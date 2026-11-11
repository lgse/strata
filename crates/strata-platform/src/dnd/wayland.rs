//! Wayland: drags out through `wl_data_device.start_drag`, and drops in through P0's
//! [`DragOffer`], all on P0's queue on winit's display.
//!
//! P1 adds no queue and no thread (the owner's ruling on S2 Q2): its objects (the drag's
//! `wl_data_source`, the icon's `wl_surface`, `wl_shm_pool` and `wl_buffer`) are made on
//! P0's queue through [`Conn::with_globals`], with user-data types of P1's own, so their
//! handlers live here. P0's reader thread (S2's driver C) reads them, and its turn dispatches
//! them on the UI thread. The handlers only queue what they hear into P1's extension state
//! and wake the windows; everything that acts on it runs in a window's update.
#![expect(unsafe_code, reason = "memfd_create for the drag icon's shared memory")]

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::Arc;

use rune::{DragOutcome, WindowHandle};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::{self, WlShmPool};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

use crate::transfer::wayland::{Conn, State};
use crate::transfer::{self, DragOffer, ReadLimits, SourceSpec};

use super::api::{Actions, DragAction, Error};
use super::hub::{self, InPort};
use super::icon::{DragImage, buffer_for};

/// A drag source's user data: the drag's id.
pub(crate) struct DragSrc(u64);

/// The icon's pool and buffer: no events P1 needs.
pub(crate) struct IconPool;
pub(crate) struct IconBuf;

/// What a drag source heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SrcEvent {
    Target(bool),
    Action(Option<DragAction>),
    Dropped,
    Finished,
    Cancelled,
}

/// One drag out's objects.
struct Live {
    source: WlDataSource,
    spec: SourceSpec,
    icon: Option<WlSurface>,
    buffer: Option<(WlShmPool, WlBuffer)>,
}

/// P1's state on P0's queue (`State::extension`).
#[derive(Default)]
pub(crate) struct WlDnd {
    events: Vec<(u64, SrcEvent)>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
    live: HashMap<u64, Live>,
}

impl WlDnd {
    fn heard(&mut self, id: u64, ev: SrcEvent) {
        self.events.push((id, ev));
        if let Some(n) = &self.notify {
            // Queues an update; nothing runs here, inside P0's dispatch.
            n();
        }
    }
}

/// How a drag out is going, from its source's events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutState {
    action: Option<DragAction>,
    dropped: bool,
    ended: bool,
}

impl OutState {
    /// The drag runs: neither dropped nor ended.
    pub(crate) fn live(&self) -> bool {
        !self.dropped && !self.ended
    }

    /// The outcome, once the drag has one. A `cancelled` after `dnd_finished` (sway sends one,
    /// S2 Q3) is ignored.
    pub(crate) fn on(&mut self, ev: SrcEvent) -> Option<DragOutcome> {
        if self.ended {
            return None;
        }
        match ev {
            SrcEvent::Action(a) => self.action = a,
            SrcEvent::Dropped => self.dropped = true,
            SrcEvent::Finished => {
                self.ended = true;
                return Some(match self.action {
                    Some(DragAction::Move) => DragOutcome::Moved,
                    Some(DragAction::Link) => DragOutcome::Linked,
                    _ => DragOutcome::Copied,
                });
            }
            SrcEvent::Cancelled => {
                self.ended = true;
                return Some(DragOutcome::Cancelled);
            }
            SrcEvent::Target(_) => {}
        }
        None
    }
}

pub(crate) fn action_of(a: transfer::Action) -> DragAction {
    match a {
        transfer::Action::Copy => DragAction::Copy,
        transfer::Action::Move => DragAction::Move,
        transfer::Action::Ask => DragAction::Ask,
    }
}

fn to_p0(a: DragAction) -> Option<transfer::Action> {
    match a {
        DragAction::Copy => Some(transfer::Action::Copy),
        DragAction::Move => Some(transfer::Action::Move),
        DragAction::Ask => Some(transfer::Action::Ask),
        // Wayland has no link.
        DragAction::Link => None,
    }
}

fn dnd_bits(actions: Actions) -> DndAction {
    actions
        .iter()
        .filter_map(to_p0)
        .fold(DndAction::empty(), |m, a| {
            m | match a {
                transfer::Action::Copy => DndAction::Copy,
                transfer::Action::Move => DndAction::Move,
                transfer::Action::Ask => DndAction::Ask,
            }
        })
}

fn of_dnd(a: DndAction) -> Option<DragAction> {
    if a.contains(DndAction::Ask) {
        Some(DragAction::Ask)
    } else if a.contains(DndAction::Move) {
        Some(DragAction::Move)
    } else if a.contains(DndAction::Copy) {
        Some(DragAction::Copy)
    } else {
        None
    }
}

/// Wakes the windows when a drag source hears something.
pub(crate) fn set_notify(conn: &Conn, notify: Arc<dyn Fn() + Send + Sync>) {
    conn.with_globals(|_, _, st| st.extension::<WlDnd>().notify = Some(notify));
}

/// What the drag sources heard since the last call.
pub(crate) fn drain(conn: &Conn) -> Vec<(u64, SrcEvent)> {
    conn.with_globals(|_, _, st| std::mem::take(&mut st.extension::<WlDnd>().events))
        .unwrap_or_default()
}

/// Starts drag `id` from `window` with the newest press serial. `with_icon`: an icon
/// surface goes with it now, and its picture lands later ([`set_icon`]).
pub(crate) fn start(
    conn: &Arc<Conn>,
    window: &WindowHandle,
    id: u64,
    spec: &SourceSpec,
    actions: Actions,
    with_icon: bool,
) -> Result<(), Error> {
    if conn.is_failed() {
        return Err(Error::Platform("the Wayland connection failed".into()));
    }
    let mask = dnd_bits(actions);
    if mask.is_empty() {
        return Err(Error::Refused("Wayland has no link action"));
    }
    let started = conn.with_globals(|_, qh, st| -> Result<(), Error> {
        let (Some(ddm), Some(device)) = (st.data_device_manager(), st.data_device()) else {
            return Err(Error::Unsupported(
                "the compositor has no data device".into(),
            ));
        };
        let serial = st.press_serial().ok_or(Error::NoSerial)?;
        let source = ddm.create_data_source(qh, DragSrc(id));
        for m in spec.mimes() {
            source.offer(m.to_owned());
        }
        if source.version() >= 3 {
            source.set_actions(mask);
        }
        let icon = with_icon
            .then(|| st.compositor().map(|c| c.create_surface(qh, ())))
            .flatten();
        let device = device.clone();
        let origin = conn.with_window_surface(window, |origin| {
            device.start_drag(Some(&source), origin, icon.as_ref(), serial);
        });
        if let Err(e) = origin {
            source.destroy();
            if let Some(i) = icon {
                i.destroy();
            }
            return Err(e.into());
        }
        st.extension::<WlDnd>().live.insert(
            id,
            Live {
                source,
                spec: spec.clone(),
                icon,
                buffer: None,
            },
        );
        Ok(())
    });
    conn.flush();
    started.ok_or(Error::Refused(
        "winit's display: start a drag on the UI thread",
    ))?
}

/// Gives drag `id`'s icon surface its picture.
pub(crate) fn set_icon(conn: &Conn, id: u64, image: &DragImage) {
    let scale = image.scale.round().max(1.0) as u32;
    let (w, h, pixels, (hx, hy)) = buffer_for(image, scale);
    let fd = match memfd(&pixels) {
        Ok(fd) => fd,
        Err(e) => return hub::diagnostic(&format!("no memory for the drag icon: {e}")),
    };
    conn.with_globals(|_, qh, st| {
        let Some(shm) = st.shm().cloned() else {
            return;
        };
        let ext = st.extension::<WlDnd>();
        let Some(live) = ext.live.get_mut(&id) else {
            return;
        };
        let Some(surface) = live.icon.clone() else {
            return;
        };
        let (pool, buffer) = icon_buffer(&shm, qh, &fd, w, h, pixels.len());
        if surface.version() >= 3 {
            surface.set_buffer_scale(scale as i32);
        }
        // The hotspot: the surface's top-left sits at the pointer, so move it up and left.
        // Version 5 moved the offset from attach to its own request (attach with a non-zero
        // offset is then a protocol error).
        if surface.version() >= 5 {
            surface.attach(Some(&buffer), 0, 0);
            surface.offset(-hx, -hy);
        } else {
            surface.attach(Some(&buffer), -hx, -hy);
        }
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, w as i32, h as i32);
        } else {
            surface.damage(0, 0, i32::MAX, i32::MAX);
        }
        surface.commit();
        if let Some((p, b)) = live.buffer.replace((pool, buffer)) {
            b.destroy();
            p.destroy();
        }
    });
    conn.flush();
}

fn icon_buffer(
    shm: &WlShm,
    qh: &QueueHandle<State>,
    fd: &OwnedFd,
    w: u32,
    h: u32,
    len: usize,
) -> (WlShmPool, WlBuffer) {
    let pool = shm.create_pool(fd.as_fd(), len as i32, qh, IconPool);
    let buffer = pool.create_buffer(
        0,
        w as i32,
        h as i32,
        (w * 4) as i32,
        Format::Argb8888,
        qh,
        IconBuf,
    );
    (pool, buffer)
}

/// A sealed-size anonymous file holding `bytes`.
fn memfd(bytes: &[u8]) -> std::io::Result<OwnedFd> {
    // SAFETY: a NUL-terminated name and a flag; no pointers kept.
    let fd = unsafe { libc::memfd_create(c"dnd-icon".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: memfd_create succeeded, so `fd` is a new descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = File::from(fd);
    file.write_all(bytes)?;
    Ok(OwnedFd::from(file))
}

/// Ends drag `id`: destroys its source (which cancels a drag still running) and its icon.
pub(crate) fn end(conn: &Conn, id: u64) {
    conn.with_globals(|_, _, st| {
        if let Some(live) = st.extension::<WlDnd>().live.remove(&id) {
            live.source.destroy();
            if let Some(i) = live.icon {
                i.destroy();
            }
            if let Some((p, b)) = live.buffer {
                b.destroy();
                p.destroy();
            }
        }
    });
    conn.flush();
}

impl Dispatch<WlDataSource, DragSrc> for State {
    fn event(
        st: &mut Self,
        _: &WlDataSource,
        e: wl_data_source::Event,
        data: &DragSrc,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = data.0;
        let ext = st.extension::<WlDnd>();
        let ev = match e {
            wl_data_source::Event::Send { mime_type, fd } => {
                if let Some(w) = ext.live.get(&id).and_then(|l| l.spec.writer(&mime_type)) {
                    transfer::pipe::spawn_write(fd, w.clone());
                }
                return;
            }
            wl_data_source::Event::Target { mime_type } => SrcEvent::Target(mime_type.is_some()),
            wl_data_source::Event::Action { dnd_action } => SrcEvent::Action(match dnd_action {
                WEnum::Value(a) => of_dnd(a),
                WEnum::Unknown(_) => None,
            }),
            wl_data_source::Event::DndDropPerformed => SrcEvent::Dropped,
            wl_data_source::Event::DndFinished => SrcEvent::Finished,
            wl_data_source::Event::Cancelled => SrcEvent::Cancelled,
            _ => return,
        };
        ext.heard(id, ev);
    }
}

impl Dispatch<WlShmPool, IconPool> for State {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wl_shm_pool::Event,
        _: &IconPool,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, IconBuf> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: wl_buffer::Event,
        _: &IconBuf,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// A drag over one of the app's windows, through P0's offer. On the UI thread only.
pub(crate) struct WlIn {
    offer: DragOffer,
}

impl WlIn {
    pub(crate) fn new(offer: DragOffer) -> WlIn {
        WlIn { offer }
    }
}

impl InPort for WlIn {
    fn status(&self, accept: Option<&str>, action: Option<DragAction>) {
        let r = self.offer.accept(accept).and_then(|()| {
            // One action, preferred: the compositor then takes that one or none, so what the
            // app was told it would get is what it gets.
            match action.and_then(to_p0) {
                Some(a) => self.offer.set_actions(&[a], Some(a)),
                None => self.offer.set_actions(&[], None),
            }
        });
        if let Err(e) = r {
            hub::diagnostic(&format!("answering a drag: {e}"));
        }
    }

    fn read(&self, mime: &str, done: Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>) {
        self.offer.read(mime, ReadLimits::default(), move |r| {
            done(r.map_err(|e| e.to_string()))
        });
    }

    fn dropped_action(&self, chosen: DragAction) -> DragAction {
        // The compositor's settled action, else ask (still unsettled), else what we asked.
        match self.offer.settled_action() {
            Some(a) => action_of(a),
            None if chosen == DragAction::Ask => DragAction::Ask,
            None => chosen,
        }
    }

    fn finish(&self, action: Option<DragAction>) {
        let Some(a) = action.and_then(to_p0) else {
            // Refused: the offer goes unfinished, and the source hears `cancelled`.
            self.offer.abandon();
            return;
        };
        // An ask drop is settled by answering it with one action.
        if self.offer.settled_action().is_none()
            && let Err(e) = self.offer.set_actions(&[a], Some(a))
        {
            hub::diagnostic(&format!("answering an ask drop: {e}"));
        }
        // P0's review, S1: confirm the settled action before finishing.
        let r = self
            .offer
            .confirm_action()
            .and_then(|_| self.offer.finish());
        if let Err(e) = r {
            hub::diagnostic(&format!("finishing a drop: {e}"));
            self.offer.abandon();
        }
    }
}
