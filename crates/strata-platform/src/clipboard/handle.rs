//! [`Clipboard`]: the app's way to the clipboard, one per state type.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rune::{AppHandle, Tx, WindowHandle};

use super::api::{
    Change, CopyId, CopyState, Data, Error, Limits, Offered, ReadError, Selection, Status,
};
use super::content::Content;
use super::files::Files;
use super::hub::{self, Job, Out, Poster, Sink};

type OnChange<S> = Rc<dyn Fn(&mut S, &Change, &mut Tx)>;
type OnData<S> = Box<dyn FnOnce(&mut S, Result<Data, ReadError>, &mut Tx)>;
type OnFiles<S> = Box<dyn FnOnce(&mut S, Result<Files, ReadError>, &mut Tx)>;

enum Pending<S> {
    Data(OnData<S>),
    Files(OnFiles<S>),
}

struct Core<S: 'static> {
    id: u64,
    poster: RefCell<Option<Poster>>,
    on_change: RefCell<Option<OnChange<S>>>,
    queue: RefCell<VecDeque<Change>>,
    reads: RefCell<HashMap<u64, Pending<S>>>,
    next_read: Cell<u64>,
    test: Cell<TestRoute>,
}

impl<S: 'static> Sink for Core<S> {
    fn queue(&self, change: Change) {
        self.queue.borrow_mut().push_back(change);
    }
}

/// A test switch for strata-platform's own session tests: which route the first attach may
/// bind. Not API.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TestRoute {
    /// Behave as on a compositor with no data control (mutter): a Wayland window takes the
    /// data device, or with no winit display, X11 through Xwayland.
    pub as_if_no_data_control: bool,
    /// Behave as on libwayland < 1.23: winit's display is out of reach.
    pub as_if_libwayland_below_1_23: bool,
}

thread_local! {
    /// Every handle, by id, for the jobs its poster runs.
    static CORES: RefCell<HashMap<u64, Weak<dyn Any>>> = RefCell::new(HashMap::new());
}

fn core<S: 'static>(id: u64) -> Option<Rc<Core<S>>> {
    CORES
        .with(|c| c.borrow().get(&id).and_then(Weak::upgrade))
        .and_then(|c| c.downcast::<Core<S>>().ok())
}

/// The clipboard, for app state `S`: copies with any MIME types, reads by type, and notices
/// when the selection changes hands. One per state type: an app whose windows share a state
/// needs one; another window type with its own state gets its own. They all share the one
/// clipboard of the process.
///
/// ```ignore
/// let clip = Clipboard::<App>::new().on_change(|s, change, _| {
///     if let Change::Lost { .. } = change { s.marks.clear(); }
/// });
/// let app = rune::App::new(spec).init({ let c = clip.clone(); move || App::new(c) });
/// let handle = app.handle();
/// app.on_window(move |w| { let _ = clip.attach(w, &handle); }).run();
/// ```
///
/// It is `Rc` inside: clone it into the state, and use it on the UI thread only.
pub struct Clipboard<S: 'static>(Rc<Core<S>>);

impl<S: 'static> Clone for Clipboard<S> {
    fn clone(&self) -> Self {
        Clipboard(self.0.clone())
    }
}

impl<S: 'static> Default for Clipboard<S> {
    fn default() -> Self {
        Clipboard::new()
    }
}

/// Runs a job on the UI thread, in an update of handle `id`'s app. True if the app heard
/// something (its state may have changed).
fn run<S: 'static>(id: u64, job: Job, s: &mut S, tx: &mut Tx) -> bool {
    if let Job::Run(f) = job {
        f();
        return false;
    }
    let Some(core) = core::<S>(id) else {
        return false;
    };
    match job {
        Job::Pump | Job::Run(_) => {
            hub::pump();
            core.drain(s, tx)
        }
        Job::Read { id, result } => core.deliver(id, result, s, tx),
    }
}

impl<S: 'static> Core<S> {
    /// Hands this handle's queued changes to `on_change`, one at a time: it may copy, clear
    /// or read again.
    fn drain(&self, s: &mut S, tx: &mut Tx) -> bool {
        let f = self.on_change.borrow().clone();
        let mut any = false;
        loop {
            let Some(change) = self.queue.borrow_mut().pop_front() else {
                break;
            };
            if let Some(f) = &f {
                f(s, &change, tx);
                any = true;
            }
        }
        any
    }

    fn deliver(&self, id: u64, result: Result<Out, ReadError>, s: &mut S, tx: &mut Tx) -> bool {
        let Some(pending) = self.reads.borrow_mut().remove(&id) else {
            return false;
        };
        match (pending, result) {
            (Pending::Data(f), Ok(Out::Data(d))) => f(s, Ok(d), tx),
            (Pending::Data(f), Err(e)) => f(s, Err(e), tx),
            (Pending::Files(f), Ok(Out::Files(files))) => f(s, Ok(files), tx),
            (Pending::Files(f), Err(e)) => f(s, Err(e), tx),
            (Pending::Data(f), Ok(Out::Files(_))) => f(s, Err(ReadError::NothingOffered), tx),
            (Pending::Files(f), Ok(Out::Data(_))) => f(s, Err(ReadError::NothingOffered), tx),
        }
        true
    }

    /// A new read: its callback kept here, and how its answer gets back.
    fn pending(&self, p: Pending<S>) -> Result<(u64, Poster), Error> {
        let poster = self.poster.borrow().clone().ok_or(Error::NotAttached)?;
        let id = self.next_read.get();
        self.next_read.set(id + 1);
        self.reads.borrow_mut().insert(id, p);
        Ok((id, poster))
    }
}

impl<S: 'static> Clipboard<S> {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let core = Rc::new(Core {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            poster: RefCell::new(None),
            on_change: RefCell::new(None),
            queue: RefCell::new(VecDeque::new()),
            reads: RefCell::new(HashMap::new()),
            next_read: Cell::new(0),
            test: Cell::new(TestRoute::default()),
        });
        let any: Rc<dyn Any> = core.clone();
        CORES.with(|c| {
            let mut c = c.borrow_mut();
            c.retain(|_, w| w.strong_count() > 0);
            c.insert(core.id, Rc::downgrade(&any));
        });
        Clipboard(core)
    }

    /// Hears every [`Change`], in an update of the app: another app's copy, the selection
    /// emptied, and each of this process's copies holding and losing it. A handle attached
    /// later first hears what each selection offers then.
    pub fn on_change(self, f: impl Fn(&mut S, &Change, &mut Tx) + 'static) -> Self {
        *self.0.on_change.borrow_mut() = Some(Rc::new(f));
        self
    }

    #[doc(hidden)]
    pub fn test_route(self, route: TestRoute) -> Self {
        self.0.test.set(route);
        self
    }

    /// Attaches a window: call it on the UI thread from `App::on_window`, every time it runs
    /// (Rune recreates a window's native side after a Wayland hide). The first call binds the
    /// transfer substrate for the process, shared with drag and drop.
    pub fn attach(&self, window: &WindowHandle, app: &AppHandle<S>) -> Result<Status, Error> {
        let id = self.0.id;
        let handle = app.clone();
        let poster: Poster = Arc::new(move |job: Job| {
            handle.update_if(move |s, tx| run::<S>(id, job, s, tx));
        });
        *self.0.poster.borrow_mut() = Some(poster.clone());
        let sink: Rc<dyn Sink> = self.0.clone();
        hub::attach(window, id, Rc::downgrade(&sink), poster, self.0.test.get())
    }

    /// What the clipboard can do here, once attached.
    pub fn status(&self) -> Option<Status> {
        hub::status()
    }

    /// Puts `content` on `selection`, replacing whatever was there (this process's earlier
    /// copy is then [`Lost::Replaced`](super::Lost::Replaced)). Refused for a content that
    /// asked to offer an X11 meta target ([`Content::with`]). On the UI thread. Call it
    /// from the key or click handler that asked for the copy: on the data-device route the
    /// write takes that event's serial.
    pub fn copy(&self, selection: Selection, content: Content) -> Result<CopyId, Error> {
        if content.reserved().is_some() {
            return Err(Error::Refused(
                "an X11 target naming a request (TARGETS, SAVE_TARGETS…) is no type to offer",
            ));
        }
        let (spec, keep) = content.into_parts();
        hub::copy(selection, spec, keep)
    }

    /// Empties the selection if `copy` still holds it: true if it did. A copy another app
    /// has replaced is left alone, and so is that app's copy.
    pub fn clear(&self, copy: CopyId) -> Result<bool, Error> {
        hub::clear(copy)
    }

    /// Where `copy` stands; None for one unknown, or long finished.
    pub fn state(&self, copy: CopyId) -> Option<CopyState> {
        hub::state(copy)
    }

    /// What `selection` offers, as last heard. On the data-device route another app's copy is
    /// heard only at the app's next keyboard focus.
    pub fn offered(&self, selection: Selection) -> Option<Offered> {
        hub::offered(selection)
    }

    /// Reads the first of `mimes` that `selection` offers, off the UI thread, and hands it to
    /// `f` in an update, as `Tx::read_clipboard` does. `f` always runs once attached:
    /// with [`ReadError::NothingOffered`] when none of `mimes` is offered. A read of this
    /// process's own copy is served in process, without the compositor. Err, and `f` never
    /// runs, when the clipboard isn't attached.
    pub fn read(
        &self,
        selection: Selection,
        mimes: &[&str],
        limits: Limits,
        f: impl FnOnce(&mut S, Result<Data, ReadError>, &mut Tx) + 'static,
    ) -> Result<(), Error> {
        let (id, poster) = self.0.pending(Pending::Data(Box::new(f)))?;
        let wanted = mimes.iter().map(|m| (*m).to_owned()).collect();
        hub::read(
            selection,
            wanted,
            limits,
            Box::new(move |r| {
                poster(Job::Read {
                    id,
                    result: r.map(Out::Data),
                });
            }),
        );
        Ok(())
    }

    /// Reads the files on `selection`, in the file managers' formats ([`Files`]): every one
    /// of them offered, at once, decoded by [`Files::decode`]'s rule. As [`Clipboard::read`].
    pub fn read_files(
        &self,
        selection: Selection,
        limits: Limits,
        f: impl FnOnce(&mut S, Result<Files, ReadError>, &mut Tx) + 'static,
    ) -> Result<(), Error> {
        let (id, poster) = self.0.pending(Pending::Files(Box::new(f)))?;
        hub::read_files(
            selection,
            limits,
            Box::new(move |r| {
                poster(Job::Read {
                    id,
                    result: r.map(Out::Files),
                });
            }),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
