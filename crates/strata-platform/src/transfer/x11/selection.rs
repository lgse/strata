//! ICCCM selections on the actor thread: owning one (with a server timestamp), answering
//! `SelectionRequest`s (INCR above one chunk), converting another client's selection (INCR
//! too), and following owners through XFixes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xfixes::SelectionNotifyEvent as XfixesNotify;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, PropMode, Property,
    PropertyNotifyEvent, SELECTION_NOTIFY_EVENT, SelectionClearEvent, SelectionNotifyEvent,
    SelectionRequestEvent, Window,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use super::super::data::{ReadLimits, SourceSpec, WRITE_IDLE};
use super::super::error::ReadError;
use super::super::event::{Event, Selection, SelectionOffer, SourceId};
use super::{Actor, Cmd, Reply, XOffer, lock};

/// A selection we hold.
pub(super) struct Owned {
    id: SourceId,
    spec: SourceSpec,
    time: u32,
}

impl Owned {
    #[cfg(feature = "dnd")]
    pub(super) fn id(&self) -> SourceId {
        self.id
    }

    pub(super) fn time(&self) -> u32 {
        self.time
    }

    pub(super) fn mimes(&self) -> Vec<String> {
        self.spec.mimes().map(str::to_owned).collect()
    }
}

/// An ownership request waiting for the server's timestamp.
pub(super) struct Owning {
    /// The selection's atom: `CLIPBOARD`, `PRIMARY`, or another a module built on the actor
    /// owns (P1's `XdndSelection`).
    sel: Atom,
    id: SourceId,
    spec: SourceSpec,
    /// Cleared before its timestamp came: skipped when it does. It keeps its place, as each
    /// take's timestamp arrives in the order the takes were asked.
    cancelled: bool,
}

#[cfg(feature = "dnd")]
impl Owning {
    pub(super) fn new(sel: Atom, id: SourceId, spec: SourceSpec) -> Owning {
        Owning {
            sel,
            id,
            spec,
            cancelled: false,
        }
    }
}

/// Ownership requests waiting for a server timestamp, in the order asked. Each take appends
/// to a property of our window, and the server's `PropertyNotify`s come back in that order, so
/// each notify's time belongs to the take at the front. A cancelled take therefore keeps its
/// place, and is skipped when its time comes: removing it would hand its time to the next.
#[derive(Default)]
pub(super) struct Takes(VecDeque<Owning>);

impl Takes {
    pub(super) fn push(&mut self, o: Owning) {
        self.0.push_back(o);
    }

    /// The take a timestamp that just came belongs to: None if it was cancelled.
    pub(super) fn next(&mut self) -> Option<Owning> {
        let o = self.0.pop_front()?;
        (!o.cancelled).then_some(o)
    }

    /// Cancels the take of `id` still waiting: true if there was one.
    pub(super) fn cancel(&mut self, id: SourceId) -> bool {
        match self.0.iter_mut().find(|o| o.id == id && !o.cancelled) {
            Some(o) => {
                o.cancelled = true;
                true
            }
            None => false,
        }
    }

    /// Cancels every take of selection `sel` still waiting.
    #[cfg(feature = "dnd")]
    pub(super) fn cancel_selection(&mut self, sel: Atom) {
        for o in self.0.iter_mut().filter(|o| o.sel == sel) {
            o.cancelled = true;
        }
    }
}

/// A `SelectionRequest` to answer.
pub(super) struct Request {
    pub(super) requestor: Window,
    selection: Atom,
    pub(super) target: Atom,
    pub(super) property: Atom,
    time: u32,
    /// The request named no property (an obsolete requestor): `property` is the target.
    pub(super) obsolete: bool,
}

/// An answer too large for one property, sent in chunks as the requestor deletes each.
pub(super) struct Outgoing {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Arc<[u8]>,
    offset: usize,
    /// The zero-length chunk that ends it has been written.
    ended: bool,
    deadline: Instant,
}

impl Outgoing {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }
}

pub(super) enum Then {
    /// An app's read.
    User(Reply),
    /// The `TARGETS` of a new owner, to announce its offer.
    Targets(Selection, Window),
}

/// A read waiting for a free property.
pub(super) struct Wanted {
    /// The selection's atom.
    sel: Atom,
    /// The conversion's time: `CurrentTime` for `CLIPBOARD` and `PRIMARY`, the drop's time
    /// for `XdndSelection`.
    time: u32,
    target: Atom,
    limits: ReadLimits,
    then: Then,
}

impl Wanted {
    #[cfg(feature = "dnd")]
    pub(super) fn new(
        sel: Atom,
        time: u32,
        target: Atom,
        limits: ReadLimits,
        then: Then,
    ) -> Wanted {
        Wanted {
            sel,
            time,
            target,
            limits,
            then,
        }
    }

    pub(super) fn fail(self, e: ReadError) {
        if let Then::User(done) = self.then {
            done.send(Err(e));
        }
    }
}

/// A conversion in flight, into one property of our window.
pub(super) struct Incoming {
    want: Wanted,
    property: Atom,
    /// Started INCR: chunks arrive as new values of the property.
    incr: bool,
    buf: Vec<u8>,
    deadline: Instant,
    /// The type and format the data last came in, which `TARGETS` must have right.
    got: (Atom, u8),
}

impl Incoming {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) fn fail(self, e: ReadError) {
        self.want.fail(e);
    }
}

/// ICCCM 2.2: whether request `r` was made before we took the selection, and so must be
/// refused. `CurrentTime` is never early. Server times are 32-bit milliseconds that wrap
/// (about every 49.7 days), so they compare as a signed difference.
fn predates(r: &SelectionRequestEvent, owned: &Owned) -> bool {
    r.time != CURRENT_TIME && (r.time.wrapping_sub(owned.time) as i32) < 0
}

impl Actor {
    pub(super) fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Own {
                selection,
                id,
                spec,
            } => {
                let sel = self.selection_atom(selection);
                self.take_with_server_time(Owning {
                    sel,
                    id,
                    spec,
                    cancelled: false,
                });
            }
            Cmd::Read {
                selection,
                owner,
                mime,
                limits,
                done,
            } => {
                if self.owner_of.get(&selection) != Some(&owner) {
                    return done.send(Err(ReadError::Gone));
                }
                let Some(target) = self.atom(&mime) else {
                    return done.send(Err(ReadError::Io("couldn't intern the type".into())));
                };
                let sel = self.selection_atom(selection);
                self.start_read(Wanted {
                    sel,
                    time: CURRENT_TIME,
                    target,
                    limits,
                    then: Then::User(done),
                });
            }
            Cmd::Disown { id } => self.disown(id),
            Cmd::HandOff { deadline, reply } => self.start_hand_off(deadline, reply),
            Cmd::Ready { request, bytes } => self.answer(request, bytes),
            Cmd::MultiReady { key, target, bytes } => self.multi_ready(key, target, bytes),
            Cmd::KillSelf => {
                // KillClient on a resource of ours closes our own connection.
                let _ = self.conn.kill_client(self.win);
            }
            #[cfg(feature = "dnd")]
            Cmd::Install(hook) => self.hook = Some(hook),
            #[cfg(feature = "dnd")]
            Cmd::Hook(cmd) => self.with_hook(|h, cx| h.command(cx, cmd)),
            Cmd::Quit => {}
        }
    }

    pub(super) fn handle(&mut self, event: XEvent) {
        match event {
            XEvent::PropertyNotify(e) => self.property(e),
            XEvent::SelectionRequest(r) => self.request(r),
            XEvent::SelectionNotify(n) => self.converted(n),
            XEvent::SelectionClear(c) => self.cleared(c),
            XEvent::XfixesSelectionNotify(n) => self.owner_changed(n),
            #[cfg(feature = "dnd")]
            other => self.with_hook(|h, cx| h.event(cx, &other)),
            #[cfg(not(feature = "dnd"))]
            _ => {}
        }
    }

    fn property(&mut self, e: PropertyNotifyEvent) {
        if e.window == self.win && e.atom == self.atoms.TRANSFER_TIME {
            if let Some(owning) = self.owning.next() {
                self.own(owning, e.time);
            }
            return;
        }
        if e.window == self.win && e.state == Property::NEW_VALUE {
            if let Some(i) = self
                .incoming
                .iter()
                .position(|r| r.incr && r.property == e.atom)
            {
                self.incr_chunk(i);
            }
            return;
        }
        if e.state == Property::DELETE
            && let Some(i) = self
                .outgoing
                .iter()
                .position(|o| o.requestor == e.window && o.property == e.atom)
        {
            self.next_chunk(i);
        }
    }

    /// ICCCM: own with a real timestamp, never CurrentTime. A zero-length append to our own
    /// window makes the server send a PropertyNotify carrying its time, and [`Actor::own`]
    /// then takes the selection.
    pub(super) fn take_with_server_time(&mut self, owning: Owning) {
        let _ = self.conn.change_property8(
            PropMode::APPEND,
            self.win,
            self.atoms.TRANSFER_TIME,
            AtomEnum::STRING,
            &[],
        );
        self.owning.push(owning);
    }

    fn own(&mut self, o: Owning, time: u32) {
        lock(&self.shared).taking.remove(&o.id.get());
        let sel = o.sel;
        let _ = self.conn.set_selection_owner(self.win, sel, time);
        let ours = self
            .conn
            .get_selection_owner(sel)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.owner == self.win);
        if !ours {
            // Another client took it in between: the take failed, as a loss would show.
            self.events.push(Event::SourceCancelled { source: o.id });
            return;
        }
        if let Some(old) = self.owned.insert(
            sel,
            Owned {
                id: o.id,
                spec: o.spec,
                time,
            },
        ) {
            // Our own earlier source on this selection is replaced: it is cancelled, as on
            // Wayland.
            lock(&self.shared).owned.remove(&old.id.get());
            self.events.push(Event::SourceCancelled { source: old.id });
        }
        lock(&self.shared).owned.insert(o.id.get());
    }

    /// Gives up whichever selection source `id` holds, with the time it was taken (ICCCM:
    /// never `CurrentTime`).
    fn disown(&mut self, id: SourceId) {
        // A take still waiting for its timestamp is cancelled, and never happens.
        if self.owning.cancel(id) {
            lock(&self.shared).taking.remove(&id.get());
            self.events.push(Event::SourceCancelled { source: id });
            return;
        }
        let Some(sel) = self
            .owned
            .iter()
            .find(|(_, o)| o.id == id)
            .map(|(sel, _)| *sel)
        else {
            return;
        };
        if let Some(owned) = self.owned.remove(&sel) {
            let _ = self.conn.set_selection_owner(NONE, sel, owned.time);
            lock(&self.shared).owned.remove(&id.get());
            self.events.push(Event::SourceCancelled { source: id });
        }
    }

    fn cleared(&mut self, c: SelectionClearEvent) {
        if c.owner != self.win {
            return;
        }
        if let Some(owned) = self.owned.remove(&c.selection) {
            lock(&self.shared).owned.remove(&owned.id.get());
            self.events
                .push(Event::SourceCancelled { source: owned.id });
        }
    }

    fn owner_changed(&mut self, n: XfixesNotify) {
        if let Some(selection) = self.selection_of(n.selection) {
            self.new_owner(selection, n.owner);
        }
    }

    pub(super) fn check_owner(&mut self, selection: Selection) {
        let sel = self.selection_atom(selection);
        let owner = self
            .conn
            .get_selection_owner(sel)
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or(NONE, |r| r.owner);
        if owner != NONE {
            self.new_owner(selection, owner);
        }
    }

    fn new_owner(&mut self, selection: Selection, owner: Window) {
        self.owner_of.insert(selection, owner);
        if owner == NONE {
            lock(&self.shared).offers.remove(&selection);
            self.events.push(Event::SelectionChanged {
                selection,
                route: super::super::event::Route::X11,
                offer: None,
            });
            return;
        }
        if owner == self.win {
            // Our own: no need to ask ourselves for TARGETS.
            let sel = self.selection_atom(selection);
            let mimes = self
                .owned
                .get(&sel)
                .map(|o| o.spec.mimes().map(str::to_owned).collect())
                .unwrap_or_default();
            self.announce(selection, owner, mimes);
            return;
        }
        let targets = self.atoms.TARGETS;
        let sel = self.selection_atom(selection);
        self.start_read(Wanted {
            sel,
            time: CURRENT_TIME,
            target: targets,
            limits: ReadLimits::default(),
            then: Then::Targets(selection, owner),
        });
    }

    fn announce(&mut self, selection: Selection, owner: Window, mimes: Vec<String>) {
        let offer = SelectionOffer(Arc::new(XOffer {
            selection,
            owner,
            mimes,
            mailbox: self.mailbox.clone(),
        }));
        lock(&self.shared).offers.insert(selection, offer.clone());
        self.events.push(Event::SelectionChanged {
            selection,
            route: super::super::event::Route::X11,
            offer: Some(offer),
        });
    }

    // ---- reading another client's selection ----

    pub(super) fn start_read(&mut self, want: Wanted) {
        let Some(property) = self.take_prop() else {
            self.queued.push_back(want);
            return;
        };
        let _ = self
            .conn
            .convert_selection(self.win, want.sel, want.target, property, want.time);
        let deadline = Instant::now() + want.limits.idle;
        self.incoming.push(Incoming {
            want,
            property,
            incr: false,
            buf: Vec::new(),
            deadline,
            got: (NONE, 0),
        });
    }

    fn converted(&mut self, n: SelectionNotifyEvent) {
        if n.requestor != self.win || self.hand_off_answered(&n) {
            return;
        }
        let selection = n.selection;
        let found = if n.property == NONE {
            // A refusal names no property: match the waiting conversion by what it asked.
            self.incoming
                .iter()
                .position(|r| !r.incr && r.want.sel == selection && r.want.target == n.target)
        } else {
            self.incoming
                .iter()
                .position(|r| !r.incr && r.property == n.property)
        };
        let Some(i) = found else {
            return;
        };
        if n.property == NONE {
            let read = self.incoming.swap_remove(i);
            self.free_props.push(read.property);
            return self.finish(read, Err(ReadError::NotOffered));
        }
        let property = self.incoming[i].property;
        let max = self.incoming[i].want.limits.max_bytes;
        match self.take_property(property, max) {
            Ok((ty, _, _size)) if ty == self.atoms.INCR => {
                // The property held the size; deleting it (done) asks for the first chunk.
                let read = &mut self.incoming[i];
                read.incr = true;
                read.deadline = Instant::now() + read.want.limits.idle;
            }
            Ok((ty, format, data)) => {
                let mut read = self.incoming.swap_remove(i);
                self.free_props.push(read.property);
                read.got = (ty, format);
                self.finish(read, Ok(data));
            }
            Err(e) => {
                let read = self.incoming.swap_remove(i);
                self.free_props.push(read.property);
                self.finish(read, Err(e));
            }
        }
    }

    fn incr_chunk(&mut self, i: usize) {
        let property = self.incoming[i].property;
        let left = self.incoming[i]
            .want
            .limits
            .max_bytes
            .saturating_sub(self.incoming[i].buf.len());
        match self.take_property(property, left) {
            Ok((_, _, chunk)) if chunk.is_empty() => {
                let mut read = self.incoming.swap_remove(i);
                self.free_props.push(read.property);
                let buf = std::mem::take(&mut read.buf);
                self.finish(read, Ok(buf));
            }
            Ok((ty, format, chunk)) => {
                let read = &mut self.incoming[i];
                read.got = (ty, format);
                read.buf.extend_from_slice(&chunk);
                read.deadline = Instant::now() + read.want.limits.idle;
            }
            Err(e) => {
                let read = self.incoming.swap_remove(i);
                // Chunks may still arrive into it: don't reuse it.
                self.finish(read, Err(e));
            }
        }
    }

    /// Reads a whole property of our window and deletes it: its type, format and bytes.
    fn take_property(
        &mut self,
        property: Atom,
        max: usize,
    ) -> Result<(Atom, u8, Vec<u8>), ReadError> {
        let mut data = Vec::new();
        let mut offset = 0u32;
        let ty = loop {
            let reply = self
                .conn
                .get_property(false, self.win, property, AtomEnum::ANY, offset, 1 << 16)
                .map_err(|e| ReadError::Io(e.to_string()))?
                .reply()
                .map_err(|e| ReadError::Io(e.to_string()))?;
            data.extend_from_slice(&reply.value);
            if data.len() > max {
                let _ = self.conn.delete_property(self.win, property);
                return Err(ReadError::TooLarge);
            }
            if reply.bytes_after == 0 {
                break (reply.type_, reply.format);
            }
            // Offsets count 32-bit units.
            offset += (reply.value.len() / 4) as u32;
        };
        let _ = self.conn.delete_property(self.win, property);
        Ok((ty.0, ty.1, data))
    }

    fn finish(&mut self, read: Incoming, result: Result<Vec<u8>, ReadError>) {
        match read.want.then {
            Then::User(done) => done.send(result),
            Then::Targets(selection, owner) => {
                // Only the owner we asked still counts; a newer one has its own request.
                if self.owner_of.get(&selection) != Some(&owner) {
                    return self.pump();
                }
                // ICCCM: TARGETS is a list of atoms, format 32. Anything else lists nothing.
                let (ty, format) = read.got;
                let well_formed =
                    format == 32 && (ty == self.atoms.ATOM || ty == self.atoms.TARGETS);
                let bytes = result.ok().filter(|_| well_formed).unwrap_or_default();
                let atoms: Vec<Atom> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_ne_bytes(*c))
                    .collect();
                let meta = [
                    self.atoms.TARGETS,
                    self.atoms.TIMESTAMP,
                    self.atoms.MULTIPLE,
                ];
                let mimes = atoms
                    .into_iter()
                    .filter(|a| *a != NONE && !meta.contains(a))
                    .filter_map(|a| self.name(a))
                    .filter(|n| !super::super::mime::is_x11_meta(n))
                    .collect();
                self.announce(selection, owner, mimes);
            }
        }
        self.pump();
    }

    /// Starts queued reads now that a property is free.
    fn pump(&mut self) {
        while !self.free_props.is_empty() {
            let Some(want) = self.queued.pop_front() else {
                break;
            };
            self.start_read(want);
        }
    }

    // ---- answering requests for our selections ----

    fn request(&mut self, r: SelectionRequestEvent) {
        // Obsolete clients send no property: answer in the target's name.
        let property = if r.property == NONE {
            r.target
        } else {
            r.property
        };
        let req = Request {
            requestor: r.requestor,
            selection: r.selection,
            target: r.target,
            property,
            time: r.time,
            obsolete: r.property == NONE,
        };
        let Some(owned) = self.owned.get(&r.selection).filter(|_| r.owner == self.win) else {
            return self.notify(&req, NONE);
        };
        if predates(&r, owned) {
            return self.notify(&req, NONE);
        }
        if r.target == self.atoms.TARGETS {
            let mimes: Vec<String> = owned.spec.mimes().map(str::to_owned).collect();
            let mut list = vec![
                self.atoms.TARGETS,
                self.atoms.TIMESTAMP,
                self.atoms.MULTIPLE,
            ];
            list.extend(mimes.iter().filter_map(|m| self.atom(m)));
            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                req.property,
                self.atoms.ATOM,
                &list,
            );
            return self.notify(&req, req.property);
        }
        if r.target == self.atoms.TIMESTAMP {
            let time = owned.time;
            let _ = self.conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                req.property,
                self.atoms.INTEGER,
                &[time],
            );
            return self.notify(&req, req.property);
        }
        if r.target == self.atoms.MULTIPLE {
            let (id, spec, time) = (owned.id, owned.spec.clone(), owned.time);
            return self.multiple(req, id, spec, time);
        }
        let (id, spec) = (owned.id, owned.spec.clone());
        let Some(mime) = self.name(r.target) else {
            return self.notify(&req, NONE);
        };
        // Unknown types are refused.
        let Some(writer) = spec.writer(&mime).cloned() else {
            return self.notify(&req, NONE);
        };
        self.events.push(Event::SourceAsked { source: id, mime });
        if let Some(bytes) = writer.ready() {
            return self.answer(req, bytes);
        }
        // The writer runs on a worker; its bytes come back as a command.
        let mailbox = self.mailbox.clone();
        let spawned = std::thread::Builder::new()
            .name("transfer-write".into())
            .spawn(move || {
                let bytes = writer.get();
                let _ = mailbox.post(Cmd::Ready {
                    request: req,
                    bytes,
                });
            });
        if let Err(e) = spawned {
            eprintln!("strata-platform transfer: no thread for an X11 write: {e}");
        }
    }

    fn answer(&mut self, req: Request, bytes: Option<Arc<[u8]>>) {
        let Some(data) = bytes else {
            return self.notify(&req, NONE);
        };
        self.put(req.requestor, req.property, req.target, data);
        self.notify(&req, req.property);
    }

    /// Writes `data` into `property` of `requestor` as type `target`: at once, or by INCR when
    /// it is larger than one chunk (the size now, then a chunk each time the requestor
    /// deletes the property, ending with an empty one).
    pub(super) fn put(&mut self, requestor: Window, property: Atom, target: Atom, data: Arc<[u8]>) {
        if data.len() <= self.chunk {
            let _ =
                self.conn
                    .change_property8(PropMode::REPLACE, requestor, property, target, &data);
            return;
        }
        let _ = self.conn.change_window_attributes(
            requestor,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        );
        let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            requestor,
            property,
            self.atoms.INCR,
            &[len],
        );
        self.outgoing.push(Outgoing {
            requestor,
            property,
            target,
            data,
            offset: 0,
            ended: false,
            deadline: Instant::now() + WRITE_IDLE,
        });
    }

    fn next_chunk(&mut self, i: usize) {
        let out = &mut self.outgoing[i];
        if out.ended {
            self.outgoing.swap_remove(i);
            return;
        }
        let end = (out.offset + self.chunk).min(out.data.len());
        let chunk = &out.data[out.offset..end];
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            out.requestor,
            out.property,
            out.target,
            chunk,
        );
        out.ended = chunk.is_empty();
        out.offset = end;
        out.deadline = Instant::now() + WRITE_IDLE;
    }

    pub(super) fn notify(&mut self, req: &Request, property: Atom) {
        let event = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property,
        };
        let _ = self
            .conn
            .send_event(false, req.requestor, EventMask::NO_EVENT, event);
    }

    // ---- timeouts ----

    pub(super) fn expire(&mut self, now: Instant) {
        self.end_hand_off(Some(now), super::HandOff::TimedOut);
        self.expire_multis(now);
        #[cfg(feature = "dnd")]
        if self
            .hook
            .as_ref()
            .and_then(|h| h.deadline())
            .is_some_and(|d| d <= now)
        {
            self.with_hook(|h, cx| h.tick(cx, now));
        }
        self.outgoing.retain(|o| o.deadline > now);
        let mut i = 0;
        while i < self.incoming.len() {
            if self.incoming[i].deadline <= now {
                // Its property is dropped from the pool: late data may still land in it.
                let read = self.incoming.swap_remove(i);
                self.finish(read, Err(ReadError::TimedOut));
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests;
