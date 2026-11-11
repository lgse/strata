//! The process's clipboard, on the UI thread: P0's substrate, the bookkeeping, every
//! [`super::Clipboard`]'s way in, and the hand-off at shutdown.
//!
//! Everything here runs on the UI thread: inside an app's call, or inside an
//! `AppHandle::update_if` that runs a [`Job`]. Other threads (P0's Wayland reader, the X11
//! actor, read workers) only post jobs. No app callback runs while the hub is borrowed:
//! changes are queued on each handle, then delivered by that handle's job.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rune::WindowHandle;

use crate::transfer::{
    self, SelectionOffer, SourceId, SourceSpec, Subscription, Transfer, Turn, Wants,
};

use super::api::{
    Change, Closed, CopyId, CopyState, Data, Error, HandOff, Limits, Offered, ReadError, Route,
    Selection, Status,
};
use super::book::Book;
use super::files::Files;
use super::handle::TestRoute;

/// Work for one [`super::Clipboard`], posted from any thread and run on the UI thread.
pub(super) enum Job {
    /// Take in what the platform reported, then deliver this handle's changes.
    Pump,
    /// P0's turn: dispatch its queue (see `transfer::Turn`).
    Run(Box<dyn FnOnce() + Send>),
    /// Read `id` finished.
    Read {
        id: u64,
        result: Result<Out, ReadError>,
    },
}

pub(super) enum Out {
    Data(Data),
    Files(Files),
}

pub(super) type Poster = Arc<dyn Fn(Job) + Send + Sync>;

type Diag = Rc<dyn Fn(&str)>;
type AnswerData = Box<dyn FnOnce(Result<Data, ReadError>) + Send>;
type AnswerFiles = Box<dyn FnOnce(Result<Files, ReadError>) + Send>;

/// A handle, to the hub.
pub(super) trait Sink {
    fn queue(&self, change: Change);
}

/// Every handle's poster: anything any thread hears wakes them all, and the first to run
/// takes it in.
#[derive(Clone, Default)]
struct Posters(Arc<Mutex<Vec<(u64, Poster)>>>);

impl Posters {
    fn set(&self, id: u64, poster: Poster) {
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        all.retain(|(i, _)| *i != id);
        all.push((id, poster));
    }

    fn wake_all(&self) {
        let all: Vec<Poster> = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, p)| p.clone())
            .collect();
        for p in all {
            p(Job::Pump);
        }
    }
}

struct Hub {
    transfer: Transfer,
    sub: Subscription,
    route: Route,
    status: Status,
    book: Book,
    /// Each selection's current offer on the route, to read from.
    offers: HashMap<Selection, SelectionOffer>,
    /// The live copies' content (for reads of our own copy, served in process) and how long
    /// the exit waits for a clipboard manager to take it.
    specs: HashMap<CopyId, (SourceSpec, Option<Duration>)>,
    sinks: Vec<(u64, Weak<dyn Sink>)>,
    posters: Posters,
    /// The platform connection failed: nothing more is written.
    failed: bool,
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
    static LAST_HAND_OFF: Cell<Option<HandOff>> = const { Cell::new(None) };
    static DIAG: RefCell<Option<Diag>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Hub) -> R) -> Option<R> {
    HUB.with(|h| h.borrow_mut().as_mut().map(f))
}

/// Reports what went wrong outside any call that could return it.
pub(super) fn diagnostic(message: &str) {
    let f = DIAG.with(|d| d.borrow().clone());
    match f {
        Some(f) => f(message),
        None => eprintln!("strata-platform clipboard: {message}"),
    }
}

pub(super) fn set_diagnostics(f: Diag) {
    DIAG.with(|d| *d.borrow_mut() = Some(f));
}

/// The route the clipboard takes, best first, and what it can do.
fn route_of(t: &Transfer) -> Result<(Route, Status), Error> {
    let s = t.status();
    let route = if s.data_control.is_some() {
        Route::DataControl
    } else if s.shared {
        Route::DataDevice
    } else if s.backend == "x11" || s.x11 {
        Route::X11
    } else {
        return Err(Error::Unsupported(s.unsupported.as_ref().map_or_else(
            || "no clipboard route".to_owned(),
            ToString::to_string,
        )));
    };
    let status = Status {
        backend: s.backend,
        route,
        hears_unfocused: route != Route::DataDevice,
        primary: route != Route::DataDevice,
        keeps_after_exit: route == Route::X11,
        xwayland: s.x11,
        unsupported: s.unsupported.as_ref().map(ToString::to_string),
    };
    Ok((route, status))
}

/// Attaches `window` for handle `id` (its `sink`, reached through `poster`), binding the
/// substrate on the first call. On the UI thread, from `App::on_window`.
pub(super) fn attach(
    window: &WindowHandle,
    id: u64,
    sink: Weak<dyn Sink>,
    poster: Poster,
    test: TestRoute,
) -> Result<Status, Error> {
    let fresh = HUB.with(|h| h.borrow().is_none());
    if fresh {
        let turn_poster = poster.clone();
        let turn: Turn = Arc::new(move |f| turn_poster(Job::Run(f)));
        let wants = Wants {
            as_if_libwayland_below_1_23: test.as_if_libwayland_below_1_23,
            as_if_no_data_control: test.as_if_no_data_control,
            // Where Wayland has no route, X11 selections through Xwayland.
            x11_fallback: true,
            ..Wants::default()
        };
        // P0 binds once per process: when drag and drop bound it first, these wants don't
        // apply, and its defaults have data control too.
        let (t, _) = Transfer::attach(window, turn, wants)?;
        let (route, status) = route_of(&t)?;
        let posters = Posters::default();
        let wake = posters.clone();
        let sub = t.subscribe(move || wake.wake_all());
        t.before_shutdown(Box::new(teardown));
        let mut hub = Hub {
            transfer: t.clone(),
            sub,
            route,
            status,
            book: Book::default(),
            offers: HashMap::new(),
            specs: HashMap::new(),
            sinks: Vec::new(),
            posters,
            failed: false,
        };
        // What the substrate heard before the subscription: the offers as they are now.
        for selection in [Selection::Clipboard, Selection::Primary] {
            if let Some(offer) = t.selection(selection.into(), route.transfer()) {
                let mimes = offer.mimes().to_vec();
                hub.offers.insert(selection, offer);
                hub.book.selection(selection, Some(mimes), &|_| false);
            }
        }
        HUB.with(|h| *h.borrow_mut() = Some(hub));
        // Events read while binding wait for a dispatch (winit's display).
        let _ = t.dispatch();
    } else {
        // P0 tracks every window: the data device's routing compares their surfaces, and its
        // reader's turns go through any window's app that still runs.
        let turn_poster = poster.clone();
        let turn: Turn = Arc::new(move |f| turn_poster(Job::Run(f)));
        let _ = Transfer::attach(window, turn, Wants::default());
    }
    let (status, now) = with(|hub| {
        hub.sinks.retain(|(i, s)| *i != id && s.strong_count() > 0);
        hub.sinks.push((id, sink.clone()));
        hub.posters.set(id, poster.clone());
        let now: Vec<Change> = [Selection::Clipboard, Selection::Primary]
            .into_iter()
            .filter_map(|selection| {
                let o = hub.book.offered(selection)?;
                Some(match o.ours {
                    Some(copy) => Change::Holding { selection, copy },
                    None => Change::Offered {
                        selection,
                        mimes: o.mimes.clone(),
                    },
                })
            })
            .collect();
        (hub.status.clone(), now)
    })
    .ok_or(Error::NotAttached)?;
    // A handle attached now hears what each selection offers.
    if let Some(sink) = sink.upgrade() {
        for c in now {
            sink.queue(c);
        }
    }
    poster(Job::Pump);
    Ok(status)
}

pub(super) fn status() -> Option<Status> {
    with(|hub| hub.status.clone())
}

/// Takes in what the platform reported since the last pump, and queues the changes on every
/// handle. Diagnostics go out once the hub is no longer borrowed: the app's handler may call
/// the clipboard back.
pub(super) fn pump() {
    let Some((changes, sinks, diagnostics)) = with(|hub| {
        let mut changes = Vec::new();
        let mut diagnostics = Vec::new();
        for event in hub.sub.drain() {
            match event {
                transfer::Event::SelectionChanged {
                    selection,
                    route,
                    offer,
                } if route == hub.route.transfer() => {
                    let selection = Selection::from(selection);
                    let mimes = offer.as_ref().map(|o| o.mimes().to_vec());
                    match offer {
                        Some(o) => hub.offers.insert(selection, o),
                        None => hub.offers.remove(&selection),
                    };
                    let t = &hub.transfer;
                    changes.extend(
                        hub.book
                            .selection(selection, mimes, &|c| t.owns(SourceId::from_raw(c.0))),
                    );
                }
                transfer::Event::SourceCancelled { source } => {
                    let copy = CopyId(source.get());
                    hub.specs.remove(&copy);
                    let refused = hub.transfer.write_was_dropped(source);
                    changes.extend(hub.book.cancelled(copy, refused));
                }
                transfer::Event::Failed { message } => {
                    diagnostics.push(format!("the platform connection failed: {message}"));
                    hub.failed = true;
                    hub.specs.clear();
                    hub.offers.clear();
                    changes.extend(hub.book.failed());
                }
                _ => {}
            }
        }
        hub.sinks.retain(|(_, s)| s.strong_count() > 0);
        let sinks: Vec<Rc<dyn Sink>> = hub.sinks.iter().filter_map(|(_, s)| s.upgrade()).collect();
        (changes, sinks, diagnostics)
    }) else {
        return;
    };
    for message in diagnostics {
        diagnostic(&message);
    }
    for sink in sinks {
        for c in &changes {
            sink.queue(c.clone());
        }
    }
}

pub(super) fn copy(
    selection: Selection,
    spec: SourceSpec,
    keep: Option<Duration>,
) -> Result<CopyId, Error> {
    if spec.is_empty() {
        return Err(Error::Empty);
    }
    let (t, route) = with(|hub| {
        if hub.failed {
            return Err(Error::Platform("the platform connection failed".into()));
        }
        Ok((hub.transfer.clone(), hub.route))
    })
    .ok_or(Error::NotAttached)??;
    if route == Route::DataDevice {
        // The key or click that asked for the copy must be in before its serial is read.
        let _ = t.dispatch();
    }
    let source = t.set_selection(selection.into(), route.transfer(), spec.clone())?;
    let copy = CopyId(source.get());
    with(|hub| {
        hub.book
            .copied(selection, copy, spec.mimes().map(str::to_owned).collect());
        hub.specs.insert(copy, (spec, keep));
    });
    Ok(copy)
}

pub(super) fn clear(copy: CopyId) -> Result<bool, Error> {
    let t = with(|hub| hub.book.clearing(copy, true).then(|| hub.transfer.clone()))
        .ok_or(Error::NotAttached)?;
    let Some(t) = t else {
        return Ok(false);
    };
    match t.clear_selection(SourceId::from_raw(copy.0)) {
        Ok(true) => Ok(true),
        other => {
            with(|hub| hub.book.clearing(copy, false));
            other.map_err(Into::into)
        }
    }
}

pub(super) fn state(copy: CopyId) -> Option<CopyState> {
    with(|hub| hub.book.state(copy)).flatten()
}

pub(super) fn offered(selection: Selection) -> Option<Offered> {
    with(|hub| hub.book.offered(selection).cloned()).flatten()
}

/// Where a read's bytes come from.
enum Src {
    /// Our own copy: its writers, in process.
    Ours(SourceSpec),
    Theirs(SelectionOffer),
}

impl Src {
    fn mimes(&self) -> Vec<String> {
        match self {
            Src::Ours(spec) => spec.mimes().map(str::to_owned).collect(),
            Src::Theirs(o) => o.mimes().to_vec(),
        }
    }
}

type Done = Box<dyn FnOnce(Result<Vec<u8>, ReadError>) + Send>;

/// The newest copy of ours on `selection` that isn't lost, holding or still on its way, is
/// what the selection has (or will have once the platform answers): a read right after a
/// copy must not get the copy it replaced. Else the other app's offer.
fn source_of(selection: Selection) -> Result<Option<Src>, ReadError> {
    with(|hub| {
        if let Some(spec) = hub
            .book
            .serving(selection)
            .and_then(|copy| hub.specs.get(&copy).map(|(spec, _)| spec.clone()))
        {
            return Some(Src::Ours(spec));
        }
        hub.offers.get(&selection).cloned().map(Src::Theirs)
    })
    .ok_or(ReadError::NotAttached)
}

/// Reads `mime` from `src`; `done` runs once, on any thread (on this one at once when the
/// read can't start).
fn read_one(src: &Src, mime: &str, limits: Limits, done: Done) {
    match src {
        Src::Ours(spec) => {
            let Some(writer) = spec.writer(mime).cloned() else {
                return done(Err(ReadError::NothingOffered));
            };
            let done = Arc::new(Mutex::new(Some(done)));
            let worker = done.clone();
            let take = |d: &Mutex<Option<Done>>| d.lock().ok().and_then(|mut d| d.take());
            let spawned = std::thread::Builder::new()
                .name("clipboard-read".into())
                .spawn(move || {
                    // A lazy writer may run here; it blocks this worker only.
                    let result = match writer.get() {
                        Some(b) if b.len() <= limits.max_bytes => Ok(b.to_vec()),
                        Some(_) => Err(ReadError::TooLarge),
                        None => Err(ReadError::Platform("the copy's writer panicked".into())),
                    };
                    if let Some(done) = take(&worker) {
                        done(result);
                    }
                });
            if let Err(e) = spawned
                && let Some(done) = take(&done)
            {
                done(Err(ReadError::Platform(format!(
                    "no thread for the read: {e}"
                ))));
            }
        }
        Src::Theirs(offer) => offer.read(mime, limits.into(), move |r| done(r.map_err(Into::into))),
    }
}

/// Reads the first of `wanted` that `selection` offers.
pub(super) fn read(selection: Selection, wanted: Vec<String>, limits: Limits, answer: AnswerData) {
    let src = match source_of(selection) {
        Ok(Some(src)) => src,
        Ok(None) => return answer(Err(ReadError::NothingOffered)),
        Err(e) => return answer(Err(e)),
    };
    let offered = src.mimes();
    let Some(mime) = wanted.into_iter().find(|w| offered.contains(w)) else {
        return answer(Err(ReadError::NothingOffered));
    };
    let m = mime.clone();
    read_one(
        &src,
        &mime,
        limits,
        Box::new(move |r| answer(r.map(|bytes| Data { mime: m, bytes }))),
    );
}

/// Reads the file-manager formats `selection` offers, all at once, and decodes them.
pub(super) fn read_files(selection: Selection, limits: Limits, answer: AnswerFiles) {
    let src = match source_of(selection) {
        Ok(Some(src)) => src,
        Ok(None) => return answer(Err(ReadError::NothingOffered)),
        Err(e) => return answer(Err(e)),
    };
    let offered = src.mimes();
    if !Files::offered(&offered) {
        return answer(Err(ReadError::NothingOffered));
    }
    let types: Vec<usize> = (0..Files::TYPES.len())
        .filter(|&i| offered.iter().any(|m| m == Files::TYPES[i]))
        .collect();
    struct Join {
        left: usize,
        got: [Option<Result<Vec<u8>, ReadError>>; 3],
        answer: Option<AnswerFiles>,
    }
    let join = Arc::new(Mutex::new(Join {
        left: types.len(),
        got: [None, None, None],
        answer: Some(answer),
    }));
    for i in types {
        let join = join.clone();
        read_one(
            &src,
            Files::TYPES[i],
            limits,
            Box::new(move |r| {
                let finished = {
                    let mut j = join.lock().unwrap_or_else(PoisonError::into_inner);
                    j.got[i] = Some(r);
                    j.left -= 1;
                    if j.left > 0 {
                        return;
                    }
                    let ok = |k: usize| {
                        j.got[k]
                            .as_ref()
                            .and_then(|r| r.as_ref().ok())
                            .map(Vec::as_slice)
                    };
                    let files = Files::decode(ok(0), ok(1), ok(2));
                    let first_error = j
                        .got
                        .iter()
                        .flatten()
                        .find_map(|r| r.as_ref().err().cloned());
                    let result = files.ok_or(first_error.unwrap_or(ReadError::NothingOffered));
                    j.answer.take().map(|a| (a, result))
                };
                // Outside the lock: the answer posts a job.
                if let Some((answer, result)) = finished {
                    answer(result);
                }
            }),
        );
    }
}

/// Before P0's teardown, on the UI thread: on X11 the clipboard copy that still holds goes to
/// a clipboard manager, if one runs and the copy asked to be kept, then the hub goes.
fn teardown(t: &Transfer) {
    let Some(hub) = HUB.with(|h| h.borrow_mut().take()) else {
        return;
    };
    if hub.route != Route::X11 {
        return;
    }
    let held = hub
        .book
        .live(Selection::Clipboard)
        .into_iter()
        .find(|c| t.owns(SourceId::from_raw(c.0)));
    let Some(copy) = held else {
        return;
    };
    let result = match hub.specs.get(&copy).and_then(|(_, keep)| *keep) {
        Some(wait) => HandOff::from(t.hand_off_clipboard(wait)),
        None => HandOff::NotHeld,
    };
    if matches!(result, HandOff::TimedOut | HandOff::Refused | HandOff::Gone) {
        diagnostic(&format!(
            "the clipboard manager didn't take the clipboard ({result:?}): it goes with the app"
        ));
    }
    LAST_HAND_OFF.with(|l| l.set(Some(result)));
}

pub(super) fn shutdown() -> Result<Closed, Error> {
    LAST_HAND_OFF.with(|l| l.set(None));
    let bound = Transfer::shutdown()?;
    // Without our hook (never attached), the hub may still be here.
    HUB.with(|h| h.borrow_mut().take());
    Ok(Closed {
        bound,
        hand_off: LAST_HAND_OFF.with(Cell::take),
    })
}
