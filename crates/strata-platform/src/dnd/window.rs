//! [`Dnd`]: one window's drag and drop. Its targets, the drag over it, and the drags it
//! starts.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rune::{AppHandle, Drag, DragOutcome, El, Key, KeyEvent, Mods, Pick, Picked, Tx, WindowHandle};

use super::api::{
    AskReply, DragAction, DragItem, DragSource, DropData, DropOffer, DropPoint, DropTarget, EndFn,
    Error, Hover, Status,
};
use super::hub::{self, InEvent, InPort, Job, Poster, Routed, Sink};
use super::local;
use super::negotiate;
use super::registry::Registry;
use super::session::{self, Decision, Guard, Landing, Release, Step};

/// One window's drag and drop: create it with the window's state, attach it from
/// `App::on_window`, and build drop targets and drag sources through it in the view.
///
/// ```ignore
/// let dnd = Dnd::<App>::new();
/// let app = rune::App::new(spec).init({ let d = dnd.clone(); move || App::new(d) });
/// let handle = app.handle();
/// app.on_window(move |w| { let _ = dnd.attach(w, &handle); }).run();
/// ```
///
/// It is `Rc` inside: clone it into the state, and use it on the UI thread only.
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
type Results = Arc<Mutex<Vec<(String, Result<Vec<u8>, String>)>>>;

/// A drop whose reads are running.
struct Pending<S> {
    target: DropTarget<S>,
    data: DropData,
    port: Rc<dyn InPort>,
    results: Results,
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
    /// Where the drag started from this window's source sits, to check before an Escape that
    /// Rune still holds a live element there (see [`SourceAt`]).
    source: RefCell<Option<SourceAt>>,
    /// Test switches for strata-platform's own tests ([`Dnd::test_route`]).
    test: Cell<hub::TestRoute>,
}

/// The element a drag started from: its key, its exact path in Rune's tree (learned by a
/// pick in the handler that started the drag, before any commit), and the press point,
/// which Rune found inside its visible box.
///
/// Rune's capture ends on an Escape only if the captured element is still live at the path
/// it was pressed at: `cancel_drag` finds it by that path, skipping exiting subtrees. An
/// Escape sent otherwise goes on to the app's keys (review round 2, R2-B1). Rune has no
/// public query for that, but `Tx::element_at_in` runs its callback after the handler and
/// before the keys it sent are handled, against the same tree, and skips exiting subtrees
/// too: the Escape is sent from that callback only if what it picks lies at that path.
#[derive(Clone, Debug)]
struct SourceAt {
    id: u64,
    /// Shared with the pick's callback, so a check copies no string.
    path: Option<Rc<str>>,
    at: (f32, f32),
}

/// The keyed segments of `path`, as `Tx::element_at_in` takes a scope: `#n` segments stand
/// for elements without keys.
fn selector(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect::<Vec<_>>()
        .join("/")
}

/// `path` up to and including its segment `key`: the keyed element's own path, from a pick
/// that may have found one of its descendants.
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

    /// A test switch for strata-platform's own tests: what the first attach behaves as if it
    /// found. Not API.
    #[doc(hidden)]
    pub fn test_route(self, route: hub::TestRoute) -> Self {
        self.0.test.set(route);
        self
    }

    /// Attaches this window: call it on the UI thread from `App::on_window`, every time it
    /// runs (Rune recreates a window's native side after a Wayland hide). The first window
    /// binds P0's substrate for the process; on X11 each call points the window's
    /// `XdndProxy` at P1's window again.
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

    /// `el`, keyed `key`, as a drop target. The key must be unique in the window: drops find
    /// their target by it (draft U-P1-1, not filed). It also takes the element's
    /// `drawn_rect_into` slot, for points in the target's own coordinates.
    pub fn drop_target(&self, el: El, key: &str, target: DropTarget<S>) -> El {
        let rect = self.0.registry.borrow_mut().put(key, target);
        el.key(key).drawn_rect_into(&rect)
    }

    /// `el`, keyed `key`, as a drop target whose key is unique only inside the keyed element
    /// `scope`, one of its ancestors: rows keyed by an id inside a list keyed by the list, so
    /// two lists showing the same ids keep their targets apart. A drag finds it under the
    /// nearest ancestor keyed `scope`. Otherwise as [`Dnd::drop_target`].
    pub fn drop_target_in(&self, el: El, scope: &str, key: &str, target: DropTarget<S>) -> El {
        let id = super::registry::scoped(scope, key);
        let rect = self.0.registry.borrow_mut().put(&id, target);
        el.key(key).drawn_rect_into(&rect)
    }

    /// `el`, keyed `key`, as a drag source. It takes the element's `on_drag`: give the
    /// element's own handler to [`DragSource::on_drag`], which hears every phase that isn't
    /// the system drag's. For an element whose `on_drag` belongs to someone else, call
    /// [`Dnd::start_drag`] and [`Dnd::owns`] from that handler instead.
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

    /// Starts a system drag from the press the app's own `on_drag` is handling, once it has
    /// decided the press is a drag: `drag` is the phase it is handling, and `key` the key of
    /// the element whose `on_drag` that is (unique in the window). Call [`Dnd::owns`] first
    /// in that handler on every phase: the phases after this belong to the system drag.
    ///
    /// Keep that element mounted at the same place in the tree, and not exiting, until the
    /// drag ends: on Wayland, P1 ends Rune's capture of the press only while it is (see the
    /// module docs).
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

    /// Whether `drag` (a phase of an app's own `on_drag`) belongs to a system drag this
    /// window started: if so, the handler should ignore it. It also tracks the press, so
    /// call it on every phase.
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

    /// Cancels the drag this process started. On Wayland it is the only way: under the
    /// compositor's drag grab no key reaches the app, and wlroots compositors (sway) don't
    /// cancel on Escape themselves.
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
        // On Wayland the Escape that ends Rune's capture waits for the compositor's proof
        // that it took the pointer (`Routed::Holding`): see `session`.
        self.0.guard.borrow_mut().started(release);
        // The press point, in window points: Rune hit-tested it inside the source's visible
        // box, which its centre may not be (a row more than half scrolled out, review round
        // 3, SF1). And the source's path, picked now, while Rune's tree is the one this phase
        // reached it in.
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

    /// For the headless tests: jobs go to `poster`, as an attached window's go to its
    /// update; [`Dnd::run_job_for_test`] runs them.
    #[cfg(test)]
    pub(crate) fn set_poster_for_test(&self, poster: Poster) {
        *self.0.poster.borrow_mut() = Some(poster);
    }

    /// For the headless tests: runs a posted job, as the window's update would.
    #[cfg(test)]
    pub(crate) fn run_job_for_test(&self, job: Job, s: &mut S, tx: &mut Tx) -> bool {
        self.0.run_job(job, s, tx)
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

    /// The compositor took the pointer for drag `id`: end Rune's capture of its press with an
    /// Escape, if the guard still holds that press and `live` confirms the drag hasn't
    /// dropped or ended (see `session`).
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

    /// Sends the Escape, from a pick's callback, only if the drag's source is still live at
    /// the path it started from (see [`SourceAt`]), and the guard still allows it.
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
                    // Nothing ends Rune's capture after the drag (review round 3, B1): Rune
                    // drops a capture without an Up when a press replaces it or a modal
                    // mounts, so a later Escape could reach the app. A drag no proof reached
                    // keeps it until the next press or focus change (module docs, path 3).
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
        // A rejecting target (one with `on_reject` that refuses the drag) hears moves too:
        // autoscroll follows any drag over a view, not only one it would take.
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
            if !d.offer.offers(mime) || !d.peeks.insert(mime.clone()) {
                continue;
            }
            let (poster, serial, m) = (self.poster.borrow().clone(), d.offer.serial, mime.clone());
            d.port.read(
                mime,
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
        let reads = if local.is_some() {
            Vec::new()
        } else {
            target.reads(&d.offer)
        };
        let data = DropData {
            offer: d.offer.clone(),
            items: Vec::new(),
            failed: Vec::new(),
            action,
            point,
            local,
        };
        let id = hub::next_id();
        let results: Results = Arc::default();
        self.pending.borrow_mut().insert(
            id,
            Pending {
                target,
                data,
                port: d.port.clone(),
                results: results.clone(),
            },
        );
        if reads.is_empty() {
            return self.deliver(id, s, tx) || changed;
        }
        let left = Arc::new(AtomicUsize::new(reads.len()));
        let poster = self.poster.borrow().clone();
        for mime in reads {
            let (results, left, poster, m) =
                (results.clone(), left.clone(), poster.clone(), mime.clone());
            d.port.read(
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
        changed
    }

    /// The reads of drop `id` are in: the target hears the drop, and the source its end.
    fn deliver(&self, id: u64, s: &mut S, tx: &mut Tx) -> bool {
        let Some(p) = self.pending.borrow_mut().remove(&id) else {
            return false;
        };
        let mut data = p.data;
        let mut results =
            std::mem::take(&mut *p.results.lock().unwrap_or_else(PoisonError::into_inner));
        // In the order the target prefers.
        results.sort_by_key(|(m, _)| p.target.mimes.iter().position(|t| t == m));
        for (mime, r) in results {
            match r {
                Ok(bytes) => data.items.push((mime, bytes)),
                Err(e) => data.failed.push((mime, e)),
            }
        }
        if data.action == DragAction::Ask {
            if let Some(f) = &p.target.on_ask {
                let port = p.port.clone();
                let reply = AskReply(Some(Box::new(move |a| port.finish(a))));
                f(s, data, reply, tx);
                return true;
            }
            // No one to ask: the rule's action.
            match negotiate::drop_action(None, session::allowed(&data.offer), data.offer.mods) {
                Some(a) => data.action = a,
                None => {
                    p.port.finish(None);
                    return false;
                }
            }
        }
        let action = data.action;
        (p.target.on_drop)(s, data, tx);
        p.port.finish(Some(action));
        true
    }
}
