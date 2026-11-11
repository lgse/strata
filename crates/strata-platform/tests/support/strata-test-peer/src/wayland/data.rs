//! Selections through `wl_data_device` and data control, and drag and drop.
//!
//! Data control binds `ext_data_control_v1` where the compositor has it (wlroots 0.19 and
//! later) and `zwlr_data_control_v1` otherwise (sway 1.9 on Ubuntu 24.04).

use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Mutex;

use strata_test_peer::protocol::{Action, Event, Offer, Selection, Via};
use wayland_client::backend::ObjectId;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{
    wl_data_device::{self, WlDataDevice},
    wl_data_device_manager::DndAction,
    wl_data_offer::{self, WlDataOffer},
    wl_data_source::{self, WlDataSource},
    wl_seat::WlSeat,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

use super::{PendingSet, St};
use crate::out;

/// What one of the peer's sources is for, and what it offers.
pub struct Source {
    kind: SourceKind,
    offers: Vec<Offer>,
    /// A drag source's last action from the compositor.
    action: Mutex<Option<Action>>,
}

enum SourceKind {
    Selection(Selection, Via),
    Drag,
}

impl Source {
    fn new(kind: SourceKind, offers: Vec<Offer>) -> Self {
        Source {
            kind,
            offers,
            action: Mutex::new(None),
        }
    }

    fn send(&self, mime: String, fd: OwnedFd) {
        let data = match self.offers.iter().find(|o| o.mime == mime) {
            Some(o) => o.data.bytes().map_err(|e| e.to_string()),
            None => Err(format!("{mime} is not offered")),
        };
        out::serve(mime, data, fd);
    }

    fn cancelled(&self) {
        match self.kind {
            SourceKind::Selection(selection, via) => {
                out::event(Event::SelectionCancelled { selection, via })
            }
            SourceKind::Drag => out::event(Event::DragCancelled),
        }
    }
}

/// An offer of the current selection, from whichever protocol made it.
pub enum OfferRef {
    DataDevice(WlDataOffer),
    Ext(ExtDataControlOfferV1),
    Wlr(ZwlrDataControlOfferV1),
}

impl OfferRef {
    fn id(&self) -> ObjectId {
        match self {
            OfferRef::DataDevice(o) => o.id(),
            OfferRef::Ext(o) => o.id(),
            OfferRef::Wlr(o) => o.id(),
        }
    }

    fn receive(&self, mime: String, fd: BorrowedFd<'_>) {
        match self {
            OfferRef::DataDevice(o) => o.receive(mime, fd),
            OfferRef::Ext(o) => o.receive(mime, fd),
            OfferRef::Wlr(o) => o.receive(mime, fd),
        }
    }

    fn destroy(&self) {
        match self {
            OfferRef::DataDevice(o) => o.destroy(),
            OfferRef::Ext(o) => o.destroy(),
            OfferRef::Wlr(o) => o.destroy(),
        }
    }
}

pub enum DataControl {
    Ext(ExtDataControlManagerV1, ExtDataControlDeviceV1),
    Wlr(ZwlrDataControlManagerV1, ZwlrDataControlDeviceV1),
}

impl DataControl {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<St>, seat: &WlSeat) -> Option<Self> {
        if let Ok(m) = globals.bind::<ExtDataControlManagerV1, _, _>(qh, 1..=1, ()) {
            let d = m.get_data_device(seat, qh, ());
            return Some(DataControl::Ext(m, d));
        }
        let m = globals
            .bind::<ZwlrDataControlManagerV1, _, _>(qh, 1..=2, ())
            .ok()?;
        let d = m.get_data_device(seat, qh, ());
        Some(DataControl::Wlr(m, d))
    }

    pub fn family(&self) -> &'static str {
        match self {
            DataControl::Ext(..) => "ext",
            DataControl::Wlr(..) => "wlr",
        }
    }

    fn set(
        &self,
        qh: &QueueHandle<St>,
        selection: Selection,
        offers: Vec<Offer>,
    ) -> Result<(), String> {
        let mimes: Vec<String> = offers.iter().map(|o| o.mime.clone()).collect();
        let source = Source::new(SourceKind::Selection(selection, Via::DataControl), offers);
        match self {
            DataControl::Ext(m, d) => {
                let s = m.create_data_source(qh, source);
                mimes.into_iter().for_each(|mime| s.offer(mime));
                match selection {
                    Selection::Clipboard => d.set_selection(Some(&s)),
                    Selection::Primary => d.set_primary_selection(Some(&s)),
                }
            }
            DataControl::Wlr(m, d) => {
                if selection == Selection::Primary && d.version() < 2 {
                    return Err("zwlr_data_control_v1 v1 has no primary selection".into());
                }
                let s = m.create_data_source(qh, source);
                mimes.into_iter().for_each(|mime| s.offer(mime));
                match selection {
                    Selection::Clipboard => d.set_selection(Some(&s)),
                    Selection::Primary => d.set_primary_selection(Some(&s)),
                }
            }
        }
        Ok(())
    }
}

/// How the peer's window answers drops (`drop-accept`).
#[derive(Clone)]
pub struct DropPolicy {
    pub mimes: Vec<String>,
    pub actions: Vec<Action>,
    pub preferred: Action,
    pub read_all: bool,
    pub ask_answer: Option<Action>,
}

/// The drag over the peer's window, as a drop target.
pub struct Drop {
    offer: WlDataOffer,
    mime: Option<String>,
    action: Option<Action>,
    /// Every type to read at the drop (the accepted one first).
    reads: Vec<String>,
    ask_answer: Option<Action>,
}

fn flags(action: Action) -> DndAction {
    match action {
        Action::Copy => DndAction::Copy,
        Action::Move => DndAction::Move,
        Action::Ask => DndAction::Ask,
        Action::Link => DndAction::empty(),
    }
}

fn action_of(flags: DndAction) -> Option<Action> {
    [Action::Copy, Action::Move, Action::Ask]
        .into_iter()
        .find(|a| flags.contains(self::flags(*a)))
}

impl St {
    pub(super) fn current_mimes(&self, via: Via, selection: Selection) -> Option<Vec<String>> {
        let offer = self.current.get(&(via, selection))?;
        self.offer_mimes.get(&offer.id()).cloned()
    }

    /// A new selection offer (or none) for one protocol and selection.
    fn set_current(&mut self, via: Via, selection: Selection, offer: Option<OfferRef>) {
        let mimes = offer
            .as_ref()
            .and_then(|o| self.offer_mimes.get(&o.id()).cloned());
        let old = match offer {
            Some(o) => self.current.insert((via, selection), o),
            None => self.current.remove(&(via, selection)),
        };
        if let Some(old) = old {
            self.offer_mimes.remove(&old.id());
            old.destroy();
        }
        *self.generation.entry((via, selection)).or_default() += 1;
        out::event(Event::SelectionChanged {
            selection,
            via,
            mimes,
        });
    }

    pub(super) fn selection_set(
        &mut self,
        id: u64,
        selection: Selection,
        via: Via,
        offers: Vec<Offer>,
    ) -> Result<(), String> {
        let mimes: Vec<String> = offers.iter().map(|o| o.mime.clone()).collect();
        match via {
            Via::DataControl => {
                let dc = self
                    .dc
                    .as_ref()
                    .ok_or("the compositor has no data control")?;
                dc.set(&self.qh, selection, offers)?;
            }
            Via::DataDevice => {
                if selection == Selection::Primary {
                    return Err("wl_data_device has no primary selection; use data control".into());
                }
                let (Some(ddm), Some(device)) = (&self.ddm, &self.data_device) else {
                    return Err("the compositor has no wl_data_device_manager".into());
                };
                let serial = self
                    .focus_serial
                    .ok_or("the data device needs keyboard focus on the peer's window")?;
                let source = ddm.create_data_source(
                    &self.qh,
                    Source::new(SourceKind::Selection(selection, via), offers),
                );
                mimes.iter().for_each(|m| source.offer(m.clone()));
                device.set_selection(Some(&source), serial);
            }
            Via::X11 => return Err("a Wayland peer has no X11 selections".into()),
        }
        let key = (via, selection);
        self.pending_sets.push(PendingSet {
            id,
            key,
            generation: self.generation.get(&key).copied().unwrap_or(0),
            mimes,
        });
        Ok(())
    }

    pub(super) fn selection_read(
        &mut self,
        id: u64,
        selection: Selection,
        via: Via,
        mime: String,
    ) -> Result<(), String> {
        let offer = self
            .current
            .get(&(via, selection))
            .ok_or("nobody owns the selection")?;
        let offered = self
            .offer_mimes
            .get(&offer.id())
            .cloned()
            .unwrap_or_default();
        if !offered.contains(&mime) {
            return Err(format!("{mime} is not offered (offered: {offered:?})"));
        }
        let (mut reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        offer.receive(mime, writer.as_fd());
        let _ = self.conn.flush();
        drop(writer);
        std::thread::spawn(move || {
            let mut data = Vec::new();
            match reader.read_to_end(&mut data) {
                Ok(_) => out::value(id, out::received(&data)),
                Err(e) => out::reply(id, Err(format!("reading the selection: {e}"))),
            }
        });
        Ok(())
    }

    pub(super) fn drag_start(
        &mut self,
        id: u64,
        offers: Vec<Offer>,
        actions: Vec<Action>,
    ) -> Result<(), String> {
        let (Some(ddm), Some(device)) = (&self.ddm, &self.data_device) else {
            return Err("the compositor has no wl_data_device_manager".into());
        };
        let surface = self.surface().ok_or("map the peer's window first")?;
        let serial = self
            .press_serial
            .ok_or("a drag needs a press on the peer's window that is still held")?;
        let mimes: Vec<String> = offers.iter().map(|o| o.mime.clone()).collect();
        let source = ddm.create_data_source(&self.qh, Source::new(SourceKind::Drag, offers));
        mimes.into_iter().for_each(|m| source.offer(m));
        if source.version() >= 3 {
            let all = actions
                .iter()
                .fold(DndAction::empty(), |acc, a| acc | flags(*a));
            source.set_actions(all);
        }
        device.start_drag(Some(&source), surface, None, serial);
        self.reply_after_sync(id);
        Ok(())
    }
}

impl Dispatch<WlDataDevice, ()> for St {
    event_created_child!(St, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);

    fn event(
        st: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                st.offer_mimes.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Selection { id } => {
                st.set_current(
                    Via::DataDevice,
                    Selection::Clipboard,
                    id.map(OfferRef::DataDevice),
                );
            }
            wl_data_device::Event::Enter { serial, id, .. } => {
                let Some(offer) = id else { return };
                let mimes = st.offer_mimes.get(&offer.id()).cloned().unwrap_or_default();
                let policy = st.drop_policy.clone();
                let mime = policy
                    .as_ref()
                    .and_then(|p| p.mimes.iter().find(|m| mimes.contains(m)).cloned());
                offer.accept(serial, mime.clone());
                if offer.version() >= 3 {
                    let (actions, preferred) = match (&mime, &policy) {
                        (Some(_), Some(p)) => (
                            p.actions
                                .iter()
                                .fold(flags(p.preferred), |all, a| all | flags(*a)),
                            flags(p.preferred),
                        ),
                        _ => (DndAction::empty(), DndAction::empty()),
                    };
                    offer.set_actions(actions, preferred);
                }
                let reads = match (&mime, &policy) {
                    (Some(_), Some(p)) if p.read_all => p
                        .mimes
                        .iter()
                        .filter(|m| mimes.contains(m))
                        .cloned()
                        .collect(),
                    (Some(m), _) => vec![m.clone()],
                    _ => Vec::new(),
                };
                let ask_answer = policy.as_ref().and_then(|p| p.ask_answer);
                out::event(Event::DropEnter {
                    mimes,
                    proxied: false,
                });
                st.drop = Some(Drop {
                    offer,
                    mime,
                    action: None,
                    reads,
                    ask_answer,
                });
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                out::event(Event::DropMotion { x, y });
            }
            wl_data_device::Event::Leave => {
                if let Some(d) = st.drop.take() {
                    st.offer_mimes.remove(&d.offer.id());
                    d.offer.destroy();
                }
                out::event(Event::DropLeave);
            }
            wl_data_device::Event::Drop => {
                let Some(d) = st.drop.take() else { return };
                st.offer_mimes.remove(&d.offer.id());
                read_drop(d, conn.clone());
            }
            _ => {}
        }
    }
}

/// Reads a drop on a thread (every type it reads, in order), then finishes the offer and
/// reports what arrived.
fn read_drop(d: Drop, conn: Connection) {
    if d.mime.is_none() {
        d.offer.destroy();
        return;
    }
    let mut readers = Vec::new();
    for mime in &d.reads {
        let (reader, writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => return out::error(format!("drop pipe: {e}")),
        };
        d.offer.receive(mime.clone(), writer.as_fd());
        drop(writer);
        readers.push((mime.clone(), reader));
    }
    let _ = conn.flush();
    std::thread::spawn(move || {
        let mut got = Vec::new();
        for (mime, mut reader) in readers {
            let mut data = Vec::new();
            if let Err(e) = reader.read_to_end(&mut data) {
                out::error(format!("reading the drop: {e}"));
            }
            got.push((mime, data));
        }
        // An ask drop is settled with one action before it may finish.
        let mut action = d.action;
        if action == Some(Action::Ask)
            && let Some(a) = d.ask_answer
            && d.offer.version() >= 3
        {
            d.offer.set_actions(flags(a), flags(a));
            action = Some(a);
        }
        // finish is only valid once an action (not ask) was chosen.
        if d.offer.version() >= 3 && action.is_some_and(|a| a != Action::Ask) {
            d.offer.finish();
        }
        d.offer.destroy();
        let _ = conn.flush();
        for (mime, data) in got {
            out::event(Event::Dropped {
                mime,
                action,
                data: out::received(&data),
            });
        }
    });
}

impl Dispatch<WlDataOffer, ()> for St {
    fn event(
        st: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                st.offer_mimes
                    .entry(offer.id())
                    .or_default()
                    .push(mime_type);
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(a),
            } => {
                if let Some(d) = st.drop.as_mut().filter(|d| &d.offer == offer) {
                    d.action = action_of(a);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, Source> for St {
    fn event(
        _: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        data: &Source,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => data.send(mime_type, fd),
            wl_data_source::Event::Target { mime_type } => {
                out::event(Event::DragTarget { mime: mime_type });
            }
            wl_data_source::Event::Action {
                dnd_action: WEnum::Value(a),
            } => {
                let action = action_of(a);
                if let Ok(mut last) = data.action.lock() {
                    *last = action;
                }
                out::event(Event::DragAction { action });
            }
            wl_data_source::Event::DndDropPerformed => out::event(Event::DragDropped),
            wl_data_source::Event::DndFinished => {
                let action = data.action.lock().ok().and_then(|a| *a);
                out::event(Event::DragFinished { action });
                source.destroy();
            }
            wl_data_source::Event::Cancelled => {
                data.cancelled();
                source.destroy();
            }
            _ => {}
        }
    }
}

/// The same handlers for both data-control families: the protocols are identical.
macro_rules! data_control {
    ($manager:ty, $device:ty, $device_mod:ident, $offer:ty, $offer_mod:ident, $source:ty,
     $source_mod:ident, $variant:ident) => {
        delegate_noop!(St: ignore $manager);

        impl Dispatch<$device, ()> for St {
            event_created_child!(St, $device, [
                $device_mod::EVT_DATA_OFFER_OPCODE => ($offer, ()),
            ]);

            fn event(
                st: &mut Self,
                _: &$device,
                event: $device_mod::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    $device_mod::Event::DataOffer { id } => {
                        st.offer_mimes.insert(id.id(), Vec::new());
                    }
                    $device_mod::Event::Selection { id } => st.set_current(
                        Via::DataControl,
                        Selection::Clipboard,
                        id.map(OfferRef::$variant),
                    ),
                    $device_mod::Event::PrimarySelection { id } => st.set_current(
                        Via::DataControl,
                        Selection::Primary,
                        id.map(OfferRef::$variant),
                    ),
                    _ => {}
                }
            }
        }

        impl Dispatch<$offer, ()> for St {
            fn event(
                st: &mut Self,
                offer: &$offer,
                event: $offer_mod::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let $offer_mod::Event::Offer { mime_type } = event {
                    st.offer_mimes.entry(offer.id()).or_default().push(mime_type);
                }
            }
        }

        impl Dispatch<$source, Source> for St {
            fn event(
                _: &mut Self,
                source: &$source,
                event: $source_mod::Event,
                data: &Source,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    $source_mod::Event::Send { mime_type, fd } => data.send(mime_type, fd),
                    $source_mod::Event::Cancelled => {
                        data.cancelled();
                        source.destroy();
                    }
                    _ => {}
                }
            }
        }
    };
}

data_control!(
    ExtDataControlManagerV1,
    ExtDataControlDeviceV1,
    ext_data_control_device_v1,
    ExtDataControlOfferV1,
    ext_data_control_offer_v1,
    ExtDataControlSourceV1,
    ext_data_control_source_v1,
    Ext
);

data_control!(
    ZwlrDataControlManagerV1,
    ZwlrDataControlDeviceV1,
    zwlr_data_control_device_v1,
    ZwlrDataControlOfferV1,
    zwlr_data_control_offer_v1,
    ZwlrDataControlSourceV1,
    zwlr_data_control_source_v1,
    Wlr
);
