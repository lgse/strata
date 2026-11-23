// SPDX-License-Identifier: MIT

use std::fmt;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

use wayland_backend::client::{ObjectId, WeakBackend};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::{Connection, Proxy};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::ExtDataControlOfferV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1::ZwlrDataControlOfferV1;

use super::super::data::ReadLimits;
use super::super::error::{Error, ReadError};
use super::super::event::{Action, Done, ReadOffer};
use super::super::pipe;

/// Why a drag offer refused a request from the wrong thread.
pub(super) const OFF_UI: &str =
    "a drag offer on winit's display takes requests on the UI thread only";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The connection an offer belongs to, and the thread its requests must come from.
#[derive(Clone)]
pub(super) struct Home {
    pub(super) backend: WeakBackend,
    /// winit's display: the UI thread. A connection of our own: None, any thread.
    pub(super) ui: Option<ThreadId>,
}

impl Home {
    /// Whether this thread may send requests on the connection.
    pub(super) fn here(&self) -> bool {
        self.ui.is_none_or(|ui| ui == std::thread::current().id())
    }

    pub(super) fn flush(&self) {
        if let Some(b) = self.backend.upgrade() {
            let _ = Connection::from_backend(b).flush();
        }
    }
}

/// An offer proxy of any of the three protocols.
pub(super) enum AnyOffer {
    Device(WlDataOffer),
    Ext(ExtDataControlOfferV1),
    Wlr(ZwlrDataControlOfferV1),
}

impl AnyOffer {
    fn receive(&self, mime: &str, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            AnyOffer::Device(o) => o.receive(mime.to_owned(), fd),
            AnyOffer::Ext(o) => o.receive(mime.to_owned(), fd),
            AnyOffer::Wlr(o) => o.receive(mime.to_owned(), fd),
        }
    }

    fn destroy(&self) {
        match self {
            AnyOffer::Device(o) => o.destroy(),
            AnyOffer::Ext(o) => o.destroy(),
            AnyOffer::Wlr(o) => o.destroy(),
        }
    }
}

fn receive(offer: &AnyOffer, home: &Home, mime: &str) -> Result<OwnedFd, ReadError> {
    let (r, w) = pipe::pipe().map_err(|e| ReadError::Io(e.to_string()))?;
    offer.receive(mime, w.as_fd());
    home.flush();
    // Our copy of the write end must close, or the read never sees EOF.
    drop(w);
    Ok(r)
}

/// Starts the read of a pipe from [`receive`], or reports why there is none. No lock held.
fn finish_read(r: Result<OwnedFd, ReadError>, limits: ReadLimits, done: Done) {
    match r {
        Ok(fd) => pipe::spawn_read(fd, limits, done),
        Err(e) => done(Err(e)),
    }
}

pub(super) struct WlSelection {
    mimes: Vec<String>,
    proxy: Mutex<Option<AnyOffer>>,
    home: Home,
}

impl WlSelection {
    pub(super) fn new(proxy: Option<AnyOffer>, mimes: Vec<String>, home: Home) -> Arc<Self> {
        Arc::new(WlSelection {
            mimes,
            proxy: Mutex::new(proxy),
            home,
        })
    }

    /// Destroys the proxy; later reads fail with `Gone`.
    pub(super) fn close(&self) {
        if let Some(p) = lock(&self.proxy).take() {
            p.destroy();
        }
    }
}

impl ReadOffer for WlSelection {
    fn mimes(&self) -> &[String] {
        &self.mimes
    }

    fn read(&self, mime: &str, limits: ReadLimits, done: Done) {
        if !self.home.here() {
            return done(Err(ReadError::OffUiThread));
        }
        let r = match lock(&self.proxy).as_ref() {
            Some(p) => receive(p, &self.home, mime),
            None => Err(ReadError::Gone),
        };
        finish_read(r, limits, done);
    }
}

/// What a drag offer has done so far, to refuse the requests `wl_data_offer` forbids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DropState {
    /// The offer's protocol version: actions and `finish` need 3.
    pub(crate) version: u32,
    accepted: Option<String>,
    /// The action the compositor settled on (`wl_data_offer.action`).
    action: Option<u32>,
    source_actions: Option<u32>,
    dropped: bool,
    finished: bool,
}

impl DropState {
    pub(crate) fn new(version: u32) -> Self {
        DropState {
            version,
            ..DropState::default()
        }
    }

    pub(crate) fn accept(
        &mut self,
        mime: Option<&str>,
        offered: &[String],
    ) -> Result<(), &'static str> {
        if self.finished {
            return Err("the drop is finished");
        }
        if let Some(m) = mime
            && !offered.iter().any(|o| o == m)
        {
            return Err("a type the drag doesn't offer");
        }
        self.accepted = mime.map(str::to_owned);
        Ok(())
    }

    /// `invalid_action_mask` and `invalid_action`, and the version.
    pub(crate) fn set_actions(&mut self, actions: u32, preferred: u32) -> Result<(), &'static str> {
        if self.finished {
            return Err("the drop is finished");
        }
        if self.version < 3 {
            return Err("set_actions needs wl_data_offer version 3");
        }
        if actions & !7 != 0 {
            return Err("an action outside copy, move and ask");
        }
        if preferred != 0 && (preferred.count_ones() != 1 || preferred & actions == 0) {
            return Err("a preferred action that isn't one of the actions");
        }
        // After the drop, the compositor's answer to this request may never reach us: it sends
        // `leave` right after `drop`, and its new action can be none or ask, either of which
        // makes `finish` a protocol error that ends winit's connection. So after the drop
        // only one request is let through: the answer to an ask with one action, not ask, that
        // the source offers. wlroots takes the preferred action only if the source offers it,
        // else the lowest of the actions both offer, so with one such action that action is
        // the one taken, and no event need come. Anything else is refused, and the action
        // stays what the compositor said (ask stays ask, so `finish` is refused for good).
        if self.dropped {
            let single = actions.count_ones() == 1 && actions != Action::Ask.bits();
            let offered = self.source_actions.is_some_and(|s| s & actions != 0);
            if self.action == Some(Action::Ask.bits()) && single && offered {
                self.action = Some(actions);
                return Ok(());
            }
            if single && offered && preferred == actions && self.action == Some(actions) {
                return Ok(());
            }
            return Err(
                "after the drop, set_actions only answers an ask: one action the source offers",
            );
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<bool, &'static str> {
        if self.finished {
            return Err("the drop is finished");
        }
        if !self.dropped {
            return Err("finish before the drop");
        }
        if self.accepted.is_none() {
            return Err("finish after accepting no type");
        }
        if self.version >= 3 {
            match self.action {
                None | Some(0) => return Err("finish before the compositor chose an action"),
                Some(a) if a == Action::Ask.bits() => {
                    return Err("finish while the action is still ask");
                }
                Some(_) => {}
            }
        }
        self.finished = true;
        Ok(self.version >= 3)
    }

    pub(crate) fn on_source_actions(&mut self, actions: u32) {
        self.source_actions = Some(actions);
    }

    pub(crate) fn on_action(&mut self, action: u32) {
        self.action = Some(action);
    }

    pub(crate) fn on_drop(&mut self) {
        self.dropped = true;
    }

    pub(crate) fn dropped(&self) -> bool {
        self.dropped
    }

    pub(crate) fn settled(&self) -> Option<Action> {
        self.action
            .filter(|a| *a != Action::Ask.bits())
            .and_then(Action::from_bits)
    }

    /// The source's actions, when known.
    pub(crate) fn source_actions(&self) -> Option<u32> {
        self.source_actions
    }
}

struct DragCell {
    proxy: Option<WlDataOffer>,
    state: DropState,
}

struct DragInner {
    id: ObjectId,
    mimes: Vec<String>,
    serial: u32,
    cell: Mutex<DragCell>,
    home: Home,
}

#[derive(Clone)]
pub struct DragOffer(Arc<DragInner>);

impl DragOffer {
    pub(super) fn new(proxy: WlDataOffer, mimes: Vec<String>, serial: u32, home: Home) -> Self {
        let (id, version) = (proxy.id(), proxy.version());
        DragOffer::with(Some(proxy), id, version, mimes, serial, home)
    }

    fn with(
        proxy: Option<WlDataOffer>,
        id: ObjectId,
        version: u32,
        mimes: Vec<String>,
        serial: u32,
        home: Home,
    ) -> Self {
        DragOffer(Arc::new(DragInner {
            id,
            mimes,
            serial,
            cell: Mutex::new(DragCell {
                state: DropState::new(version),
                proxy,
            }),
            home,
        }))
    }

    fn on_ui(&self) -> Result<(), Error> {
        if self.0.home.here() {
            Ok(())
        } else {
            Err(Error::Refused(OFF_UI))
        }
    }

    pub fn mimes(&self) -> &[String] {
        &self.0.mimes
    }

    pub fn accept(&self, mime: Option<&str>) -> Result<(), Error> {
        self.on_ui()?;
        let mut cell = lock(&self.0.cell);
        cell.state
            .accept(mime, &self.0.mimes)
            .map_err(Error::Refused)?;
        let proxy = cell
            .proxy
            .as_ref()
            .ok_or(Error::Refused("the drag is gone"))?;
        proxy.accept(self.0.serial, mime.map(str::to_owned));
        drop(cell);
        self.0.home.flush();
        Ok(())
    }

    pub fn set_actions(&self, actions: &[Action], preferred: Option<Action>) -> Result<(), Error> {
        let mask = actions.iter().fold(0, |m, a| m | a.bits());
        let pref = preferred.map_or(0, Action::bits);
        self.on_ui()?;
        let mut cell = lock(&self.0.cell);
        cell.state.set_actions(mask, pref).map_err(Error::Refused)?;
        let proxy = cell
            .proxy
            .as_ref()
            .ok_or(Error::Refused("the drag is gone"))?;
        proxy.set_actions(
            DndAction::from_bits_truncate(mask),
            DndAction::from_bits_truncate(pref),
        );
        drop(cell);
        self.0.home.flush();
        Ok(())
    }

    pub fn read(
        &self,
        mime: &str,
        limits: ReadLimits,
        done: impl FnOnce(Result<Vec<u8>, ReadError>) + Send + 'static,
    ) {
        if !self.0.mimes.iter().any(|m| m == mime) {
            return done(Err(ReadError::NotOffered));
        }
        if !self.0.home.here() {
            return done(Err(ReadError::OffUiThread));
        }
        let r = match lock(&self.0.cell).proxy.as_ref() {
            Some(p) => receive(&AnyOffer::Device(p.clone()), &self.0.home, mime),
            None => Err(ReadError::Gone),
        };
        finish_read(r, limits, Box::new(done));
    }

    pub fn settled_action(&self) -> Option<Action> {
        self.with_state(|s| s.settled())
    }

    pub fn confirm_action(&self) -> Result<Option<Action>, Error> {
        let settled = self.with_state(|s| {
            s.settled().filter(|a| {
                s.version >= 3
                    && s.dropped()
                    && s.source_actions().is_some_and(|x| x & a.bits() != 0)
            })
        });
        let Some(a) = settled else {
            return Ok(None);
        };
        self.set_actions(&[a], Some(a))?;
        Ok(Some(a))
    }

    /// Ends the drop once its reads are done: the source learns the drop succeeded.
    pub fn finish(&self) -> Result<(), Error> {
        self.on_ui()?;
        let mut cell = lock(&self.0.cell);
        let send_finish = cell.state.finish().map_err(Error::Refused)?;
        if let Some(p) = cell.proxy.take() {
            if send_finish {
                p.finish();
            }
            p.destroy();
        }
        drop(cell);
        self.0.home.flush();
        Ok(())
    }

    #[cfg(feature = "dnd")]
    pub fn abandon(&self) {
        if self.0.home.here() {
            self.close();
            self.0.home.flush();
        }
    }

    pub(super) fn is(&self, id: &ObjectId) -> bool {
        self.0.id == *id
    }

    pub(super) fn with_state<R>(&self, f: impl FnOnce(&mut DropState) -> R) -> R {
        f(&mut lock(&self.0.cell).state)
    }

    pub(super) fn is_open(&self) -> bool {
        lock(&self.0.cell).proxy.is_some()
    }

    /// Destroys the proxy without finishing: the drag left, or the substrate shuts down.
    pub(super) fn close(&self) {
        if let Some(p) = lock(&self.0.cell).proxy.take() {
            p.destroy();
        }
    }
}

#[cfg(test)]
impl DragOffer {
    /// A drag offer with no proxy, as one is once it is gone.
    pub(super) fn closed(mimes: Vec<String>, home: Home) -> Self {
        DragOffer::with(None, ObjectId::null(), 3, mimes, 0, home)
    }
}

impl fmt::Debug for DragOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DragOffer").field(&self.0.mimes).finish()
    }
}

#[cfg(test)]
mod tests;
