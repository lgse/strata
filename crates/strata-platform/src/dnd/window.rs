// SPDX-License-Identifier: MIT

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rune::{AppHandle, Drag, DragOutcome, El, Key, KeyEvent, Mods, Pick, Picked, Tx, WindowHandle};

use super::api::{
    AskReply, DragAction, DragItem, DragSource, DropData, DropOffer, DropPoint, DropTarget, EndFn,
    Error, Hover, NextReadFn, Status,
};
use super::hub::{self, InEvent, InPort, Job, Poster, Routed, Sink};
use super::local;
use super::negotiate;
use super::registry::Registry;
use super::session::{self, Decision, Guard, Landing, Release, Step};
use crate::transfer::{ReadError, mime};

pub struct Dnd<S: 'static>(Rc<Core<S>>);

impl<S: 'static> Clone for Dnd<S> {
    fn clone(&self) -> Self {
        Dnd(self.0.clone())
    }
}

impl<S: 'static> Default for Dnd<S> {
    fn default() -> Self {
        Dnd::new()
    }
}

/// The drag over this window.
struct InDrag {
    port: Rc<dyn InPort>,
    offer: DropOffer,
    local: Option<(String, Rc<dyn Any>)>,
    landing: Landing,
    at: (f32, f32),
    peeks: HashSet<String>,
}

/// Each type a drop read, as the reads finish.
type Results = Arc<Mutex<Vec<(String, Result<Vec<u8>, ReadError>)>>>;

/// A drop whose reads are running.
struct Pending<S> {
    target: DropTarget<S>,
    data: DropData,
    reader: Reader,
}

/// What reads a drop's types: the drag's port, and the reads' results as they come in.
#[derive(Clone)]
struct Reader {
    port: Rc<dyn InPort>,
    results: Results,
}

impl Reader {
    /// Reads `reads` for drop `id`, posting [`Job::Dropped`] to `poster` once they are all in.
    fn start(&self, id: u64, poster: Option<Poster>, reads: Vec<String>) {
        let left = Arc::new(AtomicUsize::new(reads.len()));
        for mime in reads {
            let (results, left, poster, m) = (
                self.results.clone(),
                left.clone(),
                poster.clone(),
                mime.clone(),
            );
            self.port.read(
                &mime,
                Box::new(move |r| {
                    results
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((m, r));
                    if left.fetch_sub(1, Ordering::AcqRel) == 1
                        && let Some(p) = poster
                    {
                        p(Job::Dropped(id));
                    }
                }),
            );
        }
    }

    /// The reads that are in, into `data`, in the order of `preference`.
    fn take_into(&self, data: &mut DropData, preference: &[String]) {
        let mut results =
            std::mem::take(&mut *self.results.lock().unwrap_or_else(PoisonError::into_inner));
        results.sort_by_key(|(m, _)| preference.iter().position(|t| mime::names(t, m)));
        for (mime, r) in results {
            match r {
                Ok(bytes) => data.items.push((mime, bytes)),
                Err(e) => data.failed.push((mime, e)),
            }
        }
    }
}

pub(crate) struct Core<S: 'static> {
    key: RefCell<Option<String>>,
    poster: RefCell<Option<Poster>>,
    registry: RefCell<Registry<S>>,
    guard: RefCell<Guard>,
    queue: RefCell<VecDeque<Routed>>,
    drag: RefCell<Option<InDrag>>,
    pending: RefCell<HashMap<u64, Pending<S>>>,
    ends: RefCell<HashMap<u64, EndFn<S>>>,
    /// The hit test whose answer counts: older ones are stale.
    seq: Cell<u64>,
    /// The drag started from this window, and whether its guard waits for a real release.
    out: Cell<Option<(u64, Release)>>,
    source: RefCell<Option<SourceAt>>,
    /// Test switches for strata-platform's own tests ([`Dnd::test_route`]).
    test: Cell<hub::TestRoute>,
}

#[derive(Clone, Debug)]
struct SourceAt {
    id: u64,
    /// Shared with the pick's callback, so a check copies no string.
    path: Option<Rc<str>>,
    at: (f32, f32),
}

fn selector(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect::<Vec<_>>()
        .join("/")
}

fn through(path: &str, key: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').collect();
    let i = segs.iter().position(|s| *s == key)?;
    Some(segs[..=i].join("/"))
}

/// Whether `picked` is the element at `path` or inside it.
fn under(picked: &str, path: &str) -> bool {
    picked == path
        || picked
            .strip_prefix(path)
            .is_some_and(|r| r.starts_with('/'))
}

impl<S: 'static> Sink for Core<S> {
    fn queue(&self, r: Routed) {
        self.queue.borrow_mut().push_back(r);
    }

    fn post(&self, job: Job) {
        let poster = self.poster.borrow().clone();
        if let Some(p) = poster {
            p(job);
        }
    }
}

/// Runs `job` for window `key`, on the UI thread, in its update. True if app code ran.
fn run<S: 'static>(key: &str, job: Job, s: &mut S, tx: &mut Tx) -> bool {
    if let Job::Run(f) = job {
        f();
        return false;
    }
    hub::route();
    let Some(core) = hub::core::<Core<S>>(key) else {
        return false;
    };
    core.run_job(job, s, tx)
}

impl<S: 'static> Core<S> {
    /// A job for this window, once routed.
    fn run_job(self: &Rc<Self>, job: Job, s: &mut S, tx: &mut Tx) -> bool {
        let core = self;
        match job {
            Job::Pump | Job::Run(_) => core.drain(s, tx),
            Job::Dropped(id) => core.deliver(id, s, tx),
            Job::Peeked {
                serial,
                mime,
                bytes,
            } => core.peeked(serial, mime, bytes, s, tx),
            Job::Icon { id, image } => {
                hub::icon_ready(id, image);
                false
            }
        }
    }
}

impl<S: 'static> Dnd<S> {
    pub fn new() -> Self {
        Dnd(Rc::new(Core {
            key: RefCell::new(None),
            poster: RefCell::new(None),
            registry: RefCell::new(Registry::default()),
            guard: RefCell::new(Guard::default()),
            queue: RefCell::new(VecDeque::new()),
            drag: RefCell::new(None),
            pending: RefCell::new(HashMap::new()),
            ends: RefCell::new(HashMap::new()),
            seq: Cell::new(0),
            out: Cell::new(None),
            source: RefCell::new(None),
            test: Cell::new(hub::TestRoute::default()),
        }))
    }

    #[doc(hidden)]
    pub fn test_route(self, route: hub::TestRoute) -> Self {
        self.0.test.set(route);
        self
    }

    pub fn attach(&self, window: &WindowHandle, app: &AppHandle<S>) -> Result<Status, Error> {
        let key = window.key().to_owned();
        let handle = app.clone();
        let k = key.clone();
        let poster: Poster = Arc::new(move |job: Job| {
            let k = k.clone();
            handle.update_if(move |s, tx| run::<S>(&k, job, s, tx));
        });
        *self.0.key.borrow_mut() = Some(key);
        *self.0.poster.borrow_mut() = Some(poster.clone());
        let sink: Rc<dyn Sink> = self.0.clone();
        let core: Rc<dyn Any> = self.0.clone();
        let status = hub::attach(window, sink, core, poster.clone(), self.0.test.get())?;
        // Anything heard while binding waits for a pump.
        poster(Job::Pump);
        Ok(status)
    }

    /// What drag and drop can do here, once attached.
    pub fn status(&self) -> Option<Status> {
        hub::current_status()
    }

    pub fn drop_target(&self, el: El, key: &str, target: DropTarget<S>) -> El {
        let rect = self.0.registry.borrow_mut().put(key, target);
        el.key(key).drawn_rect_into(&rect)
    }

    pub fn drop_target_in(&self, el: El, scope: &str, key: &str, target: DropTarget<S>) -> El {
        let id = super::registry::scoped(scope, key);
        let rect = self.0.registry.borrow_mut().put(&id, target);
        el.key(key).drawn_rect_into(&rect)
    }

    pub fn drag_source(&self, el: El, key: &str, source: DragSource<S>) -> El {
        let me = self.clone();
        let source_key = key.to_owned();
        el.key(key)
            .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                let decision = me.0.guard.borrow_mut().phase(Step::of(d), source.threshold);
                let pass = |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                    if let Some(f) = &source.inner {
                        f(s, d, tx);
                    }
                };
                match decision {
                    Decision::Pass => pass(s, d, tx),
                    Decision::Own => {}
                    Decision::Cancel => hub::cancel_drag(),
                    Decision::Released => hub::released(),
                    Decision::Start => {
                        let item = (source.item)(s, tx.mods());
                        let Some(item) = item.filter(|i| !i.is_empty()) else {
                            return pass(s, d, tx);
                        };
                        let end = source.on_end.clone();
                        match me.begin(tx, d, &source_key, item, end) {
                            Ok(()) => {
                                if let Some(f) = &source.on_start {
                                    f(s, tx);
                                }
                            }
                            Err(e) => {
                                hub::diagnostic(&format!("the drag didn't start: {e}"));
                                pass(s, d, tx);
                            }
                        }
                    }
                }
            })
    }

    pub fn start_drag(
        &self,
        tx: &mut Tx,
        drag: &Drag,
        key: &str,
        item: DragItem,
        on_end: impl Fn(&mut S, DragOutcome, &mut Tx) + 'static,
    ) -> Result<(), Error> {
        self.begin(tx, drag, key, item, Some(Rc::new(on_end)))
    }

    pub fn owns(&self, drag: &Drag) -> bool {
        let decision = self.0.guard.borrow_mut().owns(Step::of(drag));
        match decision {
            Decision::Pass | Decision::Start => false,
            Decision::Own => true,
            Decision::Cancel => {
                hub::cancel_drag();
                true
            }
            Decision::Released => {
                hub::released();
                true
            }
        }
    }

    pub fn cancel_drag(&self) {
        hub::cancel_drag();
    }

    fn begin(
        &self,
        tx: &mut Tx,
        drag: &Drag,
        source_key: &str,
        item: DragItem,
        end: Option<EndFn<S>>,
    ) -> Result<(), Error> {
        #[cfg(test)]
        let key = self.0.key.borrow().clone().unwrap_or_default();
        #[cfg(not(test))]
        let key = self.0.key.borrow().clone().ok_or(Error::NotAttached)?;
        let (id, release) = hub::start_drag(&key, item, tx.mods())?;
        if let Some(end) = end {
            self.0.ends.borrow_mut().insert(id, end);
        }
        self.0.out.set(Some((id, release)));
        self.0.guard.borrow_mut().started(release);
        let [wx, wy] = drag.window_point();
        let at = (wx - drag.dx, wy - drag.dy);
        *self.0.source.borrow_mut() = Some(SourceAt { id, path: None, at });
        let core = Rc::downgrade(&self.0);
        let k = source_key.to_owned();
        tx.element_at_in_with(
            source_key.to_owned(),
            at.0,
            at.1,
            Pick::new().transparent(true),
            move |_: &mut S, picked: Option<Picked>, _: &mut Tx| {
                let path = picked.and_then(|p| through(p.path(), &k)).map(Rc::from);
                if let Some(core) = core.upgrade()
                    && let Some(src) = core.source.borrow_mut().as_mut().filter(|s| s.id == id)
                {
                    src.path = path;
                }
            },
        );
        Ok(())
    }

    /// For the headless tests: the compositor's proof arrives, with no platform to ask.
    #[cfg(test)]
    pub(crate) fn holding_for_test(&self, tx: &mut Tx) {
        if let Some((id, _)) = self.0.out.get() {
            self.0.holding(id, tx, || true);
        }
    }

    /// For the headless tests: `r` is routed to this window, as the hub would.
    #[cfg(test)]
    pub(crate) fn route_for_test(&self, r: Routed, s: &mut S, tx: &mut Tx) {
        self.0.queue.borrow_mut().push_back(r);
        self.0.drain(s, tx);
    }

    #[cfg(test)]
    pub(crate) fn set_poster_for_test(&self, poster: Poster) {
        *self.0.poster.borrow_mut() = Some(poster);
    }

    /// For the headless tests: runs a posted job, as the window's update would.
    #[cfg(test)]
    pub(crate) fn run_job_for_test(&self, job: Job, s: &mut S, tx: &mut Tx) -> bool {
        self.0.run_job(job, s, tx)
    }

    #[cfg(test)]
    pub(crate) fn drop_on_for_test(&self, drop: TestDrop, s: &mut S, tx: &mut Tx) -> bool {
        let TestDrop { port, mimes, key } = drop;
        let offer = DropOffer {
            mimes,
            actions: super::api::Actions::COPY,
            local: false,
            mods: None,
            serial: hub::next_id(),
            peeked: Vec::new(),
        };
        *self.0.drag.borrow_mut() = Some(InDrag {
            port,
            offer,
            local: None,
            landing: Landing::Accepting {
                key,
                action: DragAction::Copy,
                accept: String::new(),
            },
            at: (0.0, 0.0),
            peeks: HashSet::new(),
        });
        self.0.dropped(s, tx)
    }

    /// For the headless tests: the drag this window started, if any.
    #[cfg(test)]
    pub(crate) fn drag_id_for_test(&self) -> Option<u64> {
        self.0.out.get().map(|(id, _)| id)
    }
}

impl<S: 'static> Core<S> {
    fn key(&self) -> String {
        self.key.borrow().clone().unwrap_or_default()
    }

    fn holding(self: &Rc<Self>, id: u64, tx: &mut Tx, live: impl FnOnce() -> bool) {
        if self.out.get() != Some((id, Release::Escape)) {
            return;
        }
        let may = self.guard.borrow().may_escape();
        if !may || !live() {
            return;
        }
        self.escape_if_live(id, tx);
    }

    fn escape_if_live(self: &Rc<Self>, id: u64, tx: &mut Tx) {
        let (path, at) = {
            let src = self.source.borrow();
            let Some(src) = src.as_ref().filter(|s| s.id == id) else {
                return;
            };
            // No path: the start's pick found no source, and nothing says the capture is live.
            let Some(path) = src.path.clone() else {
                return;
            };
            (path, src.at)
        };
        let core = Rc::downgrade(self);
        tx.element_at_in_with(
            selector(&path),
            at.0,
            at.1,
            Pick::new().transparent(true),
            move |_: &mut S, picked: Option<Picked>, tx: &mut Tx| {
                let live = picked.is_some_and(|p| under(p.path(), &path));
                let Some(core) = core.upgrade() else {
                    return;
                };
                if live && core.guard.borrow_mut().take_escape() {
                    // The cancelled Up this brings is the guard's to swallow.
                    tx.send_key(KeyEvent {
                        key: Key::Escape,
                        mods: Mods::default(),
                    });
                }
            },
        );
    }

    /// Handles what was routed to this window. True if app code ran.
    fn drain(self: &Rc<Self>, s: &mut S, tx: &mut Tx) -> bool {
        let mut changed = false;
        loop {
            let next = self.queue.borrow_mut().pop_front();
            let Some(r) = next else {
                break;
            };
            changed |= match r {
                Routed::In(ev) => self.on_in(ev, s, tx),
                Routed::Holding { id } => {
                    self.holding(id, tx, || hub::still_holding(id));
                    false
                }
                Routed::Ended { id, outcome } => {
                    let f = self.ends.borrow_mut().remove(&id);
                    match f {
                        Some(f) => {
                            f(s, outcome, tx);
                            true
                        }
                        None => false,
                    }
                }
            };
        }
        changed
    }

    fn on_in(self: &Rc<Self>, ev: InEvent, s: &mut S, tx: &mut Tx) -> bool {
        match ev {
            InEvent::Enter {
                port,
                mimes,
                actions,
                x,
                y,
                mods,
            } => {
                let changed = self.end_hover(s, tx);
                let local = hub::local_payload(&mimes);
                let offer = DropOffer {
                    mimes: local::visible(&mimes),
                    actions,
                    local: local.is_some(),
                    mods,
                    serial: hub::next_id(),
                    peeked: Vec::new(),
                };
                *self.drag.borrow_mut() = Some(InDrag {
                    port,
                    offer,
                    local,
                    landing: Landing::Nowhere,
                    at: (x as f32, y as f32),
                    peeks: HashSet::new(),
                });
                self.pick(tx);
                changed
            }
            InEvent::Actions(actions) => {
                if let Some(d) = self.drag.borrow_mut().as_mut() {
                    d.offer.actions = actions;
                }
                self.pick(tx);
                false
            }
            InEvent::Motion { x, y, mods } => {
                if let Some(d) = self.drag.borrow_mut().as_mut() {
                    d.at = (x as f32, y as f32);
                    if mods.is_some() {
                        d.offer.mods = mods;
                    }
                }
                self.pick(tx);
                false
            }
            // After a drop the drag is taken already: the leave that follows it is no news.
            InEvent::Leave | InEvent::Failed => self.end_hover(s, tx),
            InEvent::Drop => self.dropped(s, tx),
        }
    }

    /// Hit-tests the drag's point: the answer lands in [`Core::landed`].
    fn pick(self: &Rc<Self>, tx: &mut Tx) {
        let Some((x, y)) = self.drag.borrow().as_ref().map(|d| d.at) else {
            return;
        };
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let key = self.key();
        tx.element_at_with(
            x,
            y,
            Pick::new().transparent(true),
            move |s: &mut S, picked: Option<Picked>, tx: &mut Tx| {
                if let Some(core) = hub::core::<Core<S>>(&key) {
                    core.landed(seq, picked.as_ref().map(Picked::path), s, tx);
                }
            },
        );
    }

    fn landed(self: &Rc<Self>, seq: u64, path: Option<&str>, s: &mut S, tx: &mut Tx) {
        if seq != self.seq.get() {
            return;
        }
        let (landing, old, offer, port, at) = {
            let drag = self.drag.borrow();
            let Some(d) = drag.as_ref() else {
                return;
            };
            let registry = self.registry.borrow();
            let local = d.local.as_ref().map(|(t, p)| (t.as_str(), p));
            let landing = session::land_at(
                &registry,
                path,
                s,
                &d.offer,
                local,
                d.offer.mods,
                Some(d.at),
            );
            (
                landing,
                d.landing.clone(),
                d.offer.clone(),
                d.port.clone(),
                d.at,
            )
        };
        match &landing {
            Landing::Accepting { action, accept, .. } => port.status(Some(accept), Some(*action)),
            _ => port.status(None, None),
        }
        if old != landing {
            if let Some(d) = self.drag.borrow_mut().as_mut() {
                d.landing = landing.clone();
            }
            if old.key() != landing.key()
                && let Some(t) = old.key().and_then(|k| self.target(k))
                && let Some(f) = &t.on_hover
            {
                f(s, Hover::Left, &offer, tx);
            }
            if let Some(t) = landing.key().and_then(|k| self.target(k))
                && let Some(f) = &t.on_hover
            {
                f(s, landing.hover(), &offer, tx);
            }
        }
        if let Landing::Accepting { key, .. } | Landing::Rejecting { key } = &landing
            && let Some(f) = self.target(key).and_then(|t| t.on_move.clone())
        {
            let point = self.point(key, at);
            f(s, point, tx);
        }
        if let Landing::Accepting { key, .. } = &landing {
            self.start_peeks(key);
        }
    }

    fn target(&self, key: &str) -> Option<DropTarget<S>> {
        self.registry.borrow().get(key).map(|e| e.target.clone())
    }

    /// `at` (window points) in the target's own points, from where it was last drawn.
    fn point(&self, key: &str, at: (f32, f32)) -> DropPoint {
        let [rx, ry, ..] = self
            .registry
            .borrow()
            .get(key)
            .map_or([0.0; 4], |e| e.rect.get());
        DropPoint {
            x: at.0 - rx,
            y: at.1 - ry,
            window_x: at.0,
            window_y: at.1,
        }
    }

    fn start_peeks(&self, key: &str) {
        let Some(t) = self.target(key) else {
            return;
        };
        let mut drag = self.drag.borrow_mut();
        let Some(d) = drag.as_mut() else {
            return;
        };
        for mime in &t.peek {
            let Some(offered) = d.offer.offered(mime).map(str::to_owned) else {
                continue;
            };
            if !d.peeks.insert(mime.clone()) {
                continue;
            }
            let (poster, serial, m) = (self.poster.borrow().clone(), d.offer.serial, mime.clone());
            d.port.read(
                &offered,
                Box::new(move |r| match (r, poster) {
                    (Ok(bytes), Some(p)) => p(Job::Peeked {
                        serial,
                        mime: m,
                        bytes,
                    }),
                    (Err(e), _) => hub::diagnostic(&format!("peeking {m}: {e}")),
                    (Ok(_), None) => {}
                }),
            );
        }
    }

    fn peeked(&self, serial: u64, mime: String, bytes: Vec<u8>, s: &mut S, tx: &mut Tx) -> bool {
        let (key, hover, offer) = {
            let mut drag = self.drag.borrow_mut();
            let Some(d) = drag.as_mut().filter(|d| d.offer.serial == serial) else {
                return false;
            };
            d.offer.peeked.push((mime, bytes));
            let Some(key) = d.landing.key().map(str::to_owned) else {
                return false;
            };
            (key, d.landing.hover(), d.offer.clone())
        };
        match self.target(&key).and_then(|t| t.on_hover) {
            Some(f) => {
                f(s, hover, &offer, tx);
                true
            }
            None => false,
        }
    }

    /// The drag left, ended or failed: its target hears it leave.
    fn end_hover(&self, s: &mut S, tx: &mut Tx) -> bool {
        let Some(d) = self.drag.borrow_mut().take() else {
            return false;
        };
        match d
            .landing
            .key()
            .and_then(|k| self.target(k))
            .and_then(|t| t.on_hover)
        {
            Some(f) => {
                f(s, Hover::Left, &d.offer, tx);
                true
            }
            None => false,
        }
    }

    fn dropped(self: &Rc<Self>, s: &mut S, tx: &mut Tx) -> bool {
        let Some(d) = self.drag.borrow_mut().take() else {
            return false;
        };
        let target = d.landing.key().and_then(|k| self.target(k));
        let mut changed = false;
        if let Some(t) = &target
            && let Some(f) = &t.on_hover
        {
            f(s, Hover::Left, &d.offer, tx);
            changed = true;
        }
        let (key, chosen) = match (&d.landing, target) {
            (Landing::Accepting { key, action, .. }, Some(t)) => ((key.clone(), t), *action),
            (Landing::Rejecting { .. }, Some(t)) => {
                if let Some(f) = &t.on_reject {
                    f(s, &d.offer, tx);
                    changed = true;
                }
                d.port.finish(None);
                return changed;
            }
            _ => {
                d.port.finish(None);
                return changed;
            }
        };
        let (key, target) = key;
        let action = d.port.dropped_action(chosen);
        let point = Some(self.point(&key, d.at));
        let local = d
            .local
            .as_ref()
            .filter(|(_, p)| target.takes_local(Some(p)))
            .map(|(_, p)| p.clone());
        let reads_nothing = local.is_some();
        let data = DropData {
            offer: d.offer.clone(),
            items: Vec::new(),
            failed: Vec::new(),
            action,
            point,
            local,
        };
        let reads = match &target.next {
            _ if reads_nothing => Vec::new(),
            Some(next) => next_read(next, &data).into_iter().collect(),
            None => target.reads(&d.offer),
        };
        let id = hub::next_id();
        let reader = Reader {
            port: d.port.clone(),
            results: Arc::default(),
        };
        let pending = Pending {
            target,
            data,
            reader: reader.clone(),
        };
        self.pending.borrow_mut().insert(id, pending);
        if reads.is_empty() {
            return self.deliver(id, s, tx) || changed;
        }
        reader.start(id, self.poster.borrow().clone(), reads);
        changed
    }

    fn deliver(&self, id: u64, s: &mut S, tx: &mut Tx) -> bool {
        let Some(mut p) = self.pending.borrow_mut().remove(&id) else {
            return false;
        };
        p.reader.take_into(&mut p.data, &p.target.mimes);
        let next = p.target.next.as_ref();
        if let Some(mime) = next.and_then(|next| next_read(next, &p.data)) {
            let reader = p.reader.clone();
            self.pending.borrow_mut().insert(id, p);
            reader.start(id, self.poster.borrow().clone(), vec![mime]);
            return false;
        }
        Self::hand_over(p, s, tx)
    }

    /// The target hears drop `p`, every read in, and the source its end.
    fn hand_over(p: Pending<S>, s: &mut S, tx: &mut Tx) -> bool {
        let port = &p.reader.port;
        let mut data = p.data;
        if data.action == DragAction::Ask {
            if let Some(f) = &p.target.on_ask {
                let port = port.clone();
                let reply = AskReply(Some(Box::new(move |a| port.finish(a))));
                f(s, data, reply, tx);
                return true;
            }
            // No one to ask: the rule's action.
            match negotiate::drop_action(None, session::allowed(&data.offer), data.offer.mods) {
                Some(a) => data.action = a,
                None => {
                    port.finish(None);
                    return false;
                }
            }
        }
        let action = data.action;
        (p.target.on_drop)(s, data, tx);
        port.finish(Some(action));
        true
    }
}

#[cfg(test)]
pub(crate) struct TestDrop {
    pub(crate) port: Rc<dyn InPort>,
    pub(crate) mimes: Vec<String>,
    pub(crate) key: String,
}

fn next_read(next: &NextReadFn, data: &DropData) -> Option<String> {
    let mime = next(data)?;
    let read = data.items.iter().map(|(m, _)| m);
    let failed = data.failed.iter().map(|(m, _)| m);
    let unread = !read.chain(failed).any(|m| *m == mime);
    (unread && data.offer.mimes.contains(&mime)).then_some(mime)
}
