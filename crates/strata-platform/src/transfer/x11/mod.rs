//! X11: selections on a connection of our own, from one thread.
//!
//! Rune exposes no X connection (draft U9, not filed), and winit answers XDND for its window
//! itself, so everything here runs on a second x11rb connection with an unmapped
//! override-redirect window. One thread (the actor) does all of its I/O: a reply awaited on
//! another thread would read events off the socket that the actor's poll then never sees.
//! Calls from other threads become [`Cmd`]s on a channel, with a byte on a wake pipe.
//!
//! It owns `CLIPBOARD` and `PRIMARY` with `TARGETS`, `TIMESTAMP` and any MIME types, serves
//! large data with INCR, reads others' selections (INCR too), and hears every ownership change
//! through XFixes, so a lost selection shows at once.

mod handoff;
#[cfg(feature = "dnd")]
pub(crate) mod hook;
mod multiple;
mod selection;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, ConnectionExt as _, CreateWindowAux, EventMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::data::{ReadLimits, SourceSpec};
use super::error::{Error, ReadError};
use super::event::{Done, Event, ReadOffer, Selection, SelectionOffer, SourceId, Subscribers};
use super::pipe;
pub use handoff::HandOff;
use selection::{Incoming, Outgoing};

x11rb::atom_manager! {
    pub(crate) Atoms: AtomsCookie {
        CLIPBOARD,
        PRIMARY,
        TARGETS,
        TIMESTAMP,
        MULTIPLE,
        ATOM_PAIR,
        SAVE_TARGETS,
        CLIPBOARD_MANAGER,
        INCR,
        ATOM,
        INTEGER,
        // Our own window's properties: neutral names, as the crate's API carries no app names.
        TRANSFER_TIME: b"_TRANSFER_SUBSTRATE_TIME",
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Cmd {
    Own {
        selection: Selection,
        id: SourceId,
        spec: SourceSpec,
    },
    Read {
        selection: Selection,
        owner: Window,
        mime: String,
        limits: ReadLimits,
        done: Reply,
    },
    /// Give up the selection source `id` holds, if it still holds one.
    Disown {
        id: SourceId,
    },
    /// Hand `CLIPBOARD` to a clipboard manager, answering by `deadline`.
    HandOff {
        deadline: Instant,
        reply: Sender<HandOff>,
    },
    /// The lazy writer of `target`, asked by `MULTIPLE` request `key`, finished on a worker.
    MultiReady {
        key: u64,
        target: Atom,
        bytes: Option<Arc<[u8]>>,
    },
    /// A lazy writer finished on a worker: answer the request that asked for it.
    Ready {
        request: selection::Request,
        bytes: Option<Arc<[u8]>>,
    },
    /// A test switch: has the server kill our own connection, so it fails.
    KillSelf,
    /// Installs the module that runs on the actor thread (P1's XDND).
    #[cfg(feature = "dnd")]
    Install(Box<dyn hook::Hook>),
    /// A command for the installed hook.
    #[cfg(feature = "dnd")]
    Hook(Box<dyn std::any::Any + Send>),
    Quit,
}

/// A read's answer, sent once. Dropped unsent (a command the actor never took, a read it
/// abandoned when it stopped) it answers [`ReadError::Shutdown`], so no reader waits forever.
pub(super) struct Reply(Option<Done>);

impl Reply {
    fn new(done: Done) -> Reply {
        Reply(Some(done))
    }

    pub(super) fn send(mut self, result: Result<Vec<u8>, ReadError>) {
        if let Some(done) = self.0.take() {
            done(result);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            done(Err(ReadError::Shutdown));
        }
    }
}

/// Every command still queued when the actor stops: their reads answer Shutdown.
fn refuse_all(rx: &Receiver<Cmd>) {
    while let Ok(cmd) = rx.try_recv() {
        drop(cmd);
    }
}

/// What other threads may ask without a round trip to the actor.
#[derive(Default)]
struct Shared {
    offers: HashMap<Selection, SelectionOffer>,
    owned: HashSet<u64>,
    /// Takes asked for and not yet settled: the actor waits for their server timestamp.
    taking: HashSet<u64>,
}

/// The way into the actor, from any thread.
#[derive(Clone)]
struct Mailbox {
    tx: Sender<Cmd>,
    wake: Arc<File>,
}

impl Mailbox {
    /// Queues `cmd` and wakes the actor. Gives the command back if the actor is gone.
    fn post(&self, cmd: Cmd) -> Result<(), Cmd> {
        self.tx.send(cmd).map_err(|mpsc::SendError(cmd)| cmd)?;
        // Non-blocking: a full pipe already wakes the actor.
        let _ = (&*self.wake).write(&[1]);
        Ok(())
    }
}

/// The X11 side of the substrate: a handle to the actor thread.
pub(crate) struct X11 {
    mailbox: Mailbox,
    shared: Arc<Mutex<Shared>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl X11 {
    /// Connects to `$DISPLAY` and starts the actor.
    pub(crate) fn start(subs: Subscribers) -> Result<X11, Error> {
        let conn = |e: &dyn std::fmt::Display| Error::Connection(e.to_string());
        let (xconn, screen) = x11rb::connect(None).map_err(|e| conn(&e))?;
        let root = xconn.setup().roots[screen].root;
        let atoms = Atoms::new(&xconn)
            .map_err(|e| conn(&e))?
            .reply()
            .map_err(|e| conn(&e))?;
        let win = xconn.generate_id().map_err(|e| conn(&e))?;
        xconn
            .create_window(
                0,
                win,
                root,
                -10,
                -10,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(|e| conn(&e))?;
        xconn
            .xfixes_query_version(5, 0)
            .map_err(|e| conn(&e))?
            .reply()
            .map_err(|e| conn(&e))?;
        let mask = xfixes::SelectionEventMask::SET_SELECTION_OWNER
            | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
            | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE;
        for sel in [atoms.CLIPBOARD, atoms.PRIMARY] {
            xconn
                .xfixes_select_selection_input(win, sel, mask)
                .map_err(|e| conn(&e))?;
        }
        xconn.flush().map_err(|e| conn(&e))?;

        let (wake_r, wake_w) = pipe::pipe().map_err(|e| conn(&e))?;
        pipe::set_nonblocking(wake_r.as_fd()).map_err(|e| conn(&e))?;
        pipe::set_nonblocking(wake_w.as_fd()).map_err(|e| conn(&e))?;
        let (tx, rx) = mpsc::channel();
        let mailbox = Mailbox {
            tx,
            wake: Arc::new(File::from(wake_w)),
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let chunk = (xconn.maximum_request_bytes() / 2).clamp(4096, 256 * 1024);
        let mut actor = Actor {
            conn: xconn,
            win,
            #[cfg(feature = "dnd")]
            root,
            #[cfg(feature = "dnd")]
            hook: None,
            atoms,
            names: HashMap::new(),
            atom_of: HashMap::new(),
            owner_of: HashMap::new(),
            owned: HashMap::new(),
            owning: selection::Takes::default(),
            outgoing: Vec::new(),
            multis: Vec::new(),
            next_multi: 0,
            handoff: None,
            incoming: Vec::new(),
            free_props: Vec::new(),
            next_prop: 0,
            queued: VecDeque::new(),
            subs,
            shared: shared.clone(),
            mailbox: mailbox.clone(),
            events: Vec::new(),
            chunk,
        };
        let thread = std::thread::Builder::new()
            .name("transfer-x11".into())
            .spawn(move || actor.run(rx, wake_r))
            .map_err(|e| conn(&e))?;
        Ok(X11 {
            mailbox,
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Takes `selection` with `spec`. The actor completes it (after a round trip for the
    /// server's timestamp) without the caller waiting; a refusal arrives as
    /// [`Event::SourceCancelled`], and [`X11::owns`] turns true once it holds. A take still
    /// queued when the actor fails is dropped: its caller hears only [`Event::Failed`].
    pub(crate) fn set_selection(
        &self,
        selection: Selection,
        spec: SourceSpec,
    ) -> Result<SourceId, Error> {
        if spec.is_empty() {
            return Err(Error::Refused("a source with no types"));
        }
        let id = SourceId::next();
        lock(&self.shared).taking.insert(id.get());
        let own = Cmd::Own {
            selection,
            id,
            spec,
        };
        self.mailbox.post(own).map_err(|_| Error::Shutdown)?;
        Ok(id)
    }

    /// Gives up the selection `source` holds, if it still holds it, or cancels its take if it
    /// still waits for its timestamp. True if it held one or was about to when asked; the
    /// actor then gives it up and [`Event::SourceCancelled`] follows.
    pub(crate) fn clear_selection(&self, source: SourceId) -> Result<bool, Error> {
        let pending = lock(&self.shared).taking.contains(&source.get());
        if !self.owns(source) && !pending {
            return Ok(false);
        }
        self.mailbox
            .post(Cmd::Disown { id: source })
            .map_err(|_| Error::Shutdown)?;
        Ok(true)
    }

    /// Hands `CLIPBOARD`, if this process holds it, to a running clipboard manager with every
    /// type, so it outlives the process. Blocks until the manager answers, at most about
    /// `timeout`; the actor serves the manager's requests meanwhile.
    pub(crate) fn hand_off(&self, timeout: std::time::Duration) -> HandOff {
        handoff::hand_off(&self.mailbox, timeout)
    }

    /// Installs `hook` on the actor thread, replacing any earlier one. False if the actor is
    /// gone.
    #[cfg(feature = "dnd")]
    pub(crate) fn install_hook(&self, hook: Box<dyn hook::Hook>) -> bool {
        self.mailbox.post(Cmd::Install(hook)).is_ok()
    }

    /// Posts `cmd` to the installed hook, from any thread. False if the actor is gone.
    #[cfg(feature = "dnd")]
    pub(crate) fn post_hook(&self, cmd: Box<dyn std::any::Any + Send>) -> bool {
        self.mailbox.post(Cmd::Hook(cmd)).is_ok()
    }

    /// A test switch: makes the X server end our connection, to check how the actor's
    /// failure is reported. False if the actor is gone.
    pub(crate) fn provoke_connection_failure(&self) -> bool {
        self.mailbox.post(Cmd::KillSelf).is_ok()
    }

    pub(crate) fn selection(&self, selection: Selection) -> Option<SelectionOffer> {
        lock(&self.shared).offers.get(&selection).cloned()
    }

    pub(crate) fn owns(&self, source: SourceId) -> bool {
        lock(&self.shared).owned.contains(&source.get())
    }

    /// Stops the actor and joins it. Its window goes, which gives up every selection it holds.
    pub(crate) fn shutdown(&self) {
        let _ = self.mailbox.post(Cmd::Quit);
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
    }
}

/// Another client's X11 selection, as of one owner.
struct XOffer {
    selection: Selection,
    owner: Window,
    mimes: Vec<String>,
    mailbox: Mailbox,
}

impl ReadOffer for XOffer {
    fn mimes(&self) -> &[String] {
        &self.mimes
    }

    fn read(&self, mime: &str, limits: ReadLimits, done: Done) {
        let cmd = Cmd::Read {
            selection: self.selection,
            owner: self.owner,
            mime: mime.to_owned(),
            limits,
            done: Reply::new(done),
        };
        // A gone actor drops the command, and its reply answers Shutdown.
        let _ = self.mailbox.post(cmd);
    }
}

/// The actor's state; it lives on the actor thread.
struct Actor {
    conn: RustConnection,
    win: Window,
    #[cfg(feature = "dnd")]
    root: Window,
    /// The module built on the actor (P1's XDND), once installed.
    #[cfg(feature = "dnd")]
    hook: Option<Box<dyn hook::Hook>>,
    atoms: Atoms,
    names: HashMap<Atom, String>,
    atom_of: HashMap<String, Atom>,
    /// Each selection's owner, as XFixes last said.
    owner_of: HashMap<Selection, Window>,
    owned: HashMap<Atom, selection::Owned>,
    /// Ownership requests waiting for a server timestamp, in order.
    owning: selection::Takes,
    outgoing: Vec<Outgoing>,
    /// `MULTIPLE` requests waiting for lazy writers.
    multis: Vec<multiple::Multi>,
    next_multi: u64,
    /// A hand-off to a clipboard manager, waiting for its answer.
    handoff: Option<handoff::Waiting>,
    incoming: Vec<Incoming>,
    free_props: Vec<Atom>,
    next_prop: u32,
    /// Reads waiting for a free property.
    queued: VecDeque<selection::Wanted>,
    subs: Subscribers,
    shared: Arc<Mutex<Shared>>,
    mailbox: Mailbox,
    events: Vec<Event>,
    /// The largest property written at once; more goes by INCR.
    chunk: usize,
}

impl Actor {
    /// Serves until told to quit or the connection fails, then answers every read still
    /// queued. The receiver drops on return, which discards (and so answers) any command sent
    /// after.
    fn run(&mut self, rx: Receiver<Cmd>, wake: OwnedFd) {
        self.serve(&rx, wake);
        refuse_all(&rx);
    }

    fn serve(&mut self, rx: &Receiver<Cmd>, wake: OwnedFd) {
        let mut wake = File::from(wake);
        for selection in [Selection::Clipboard, Selection::Primary] {
            self.check_owner(selection);
        }
        loop {
            let mut busy = false;
            loop {
                match self.conn.poll_for_event() {
                    Ok(Some(event)) => {
                        busy = true;
                        self.handle(event);
                    }
                    Ok(None) => break,
                    Err(e) => return self.fail(e.to_string()),
                }
            }
            while let Ok(cmd) = rx.try_recv() {
                busy = true;
                if matches!(cmd, Cmd::Quit) {
                    return self.quit();
                }
                self.command(cmd);
            }
            self.expire(Instant::now());
            if let Err(e) = self.conn.flush() {
                return self.fail(e.to_string());
            }
            self.subs.emit(std::mem::take(&mut self.events));
            if busy {
                // Handling may have read more events off the socket (replies do): drain them
                // before sleeping.
                continue;
            }
            let timeout = self
                .next_deadline()
                .map(|d| d.saturating_duration_since(Instant::now()));
            let ready = pipe::poll(
                &[
                    (self.conn.stream().as_fd(), libc::POLLIN),
                    (wake.as_fd(), libc::POLLIN),
                ],
                timeout,
            );
            match ready {
                Ok(r) if r[1] != 0 => {
                    let mut buf = [0; 64];
                    while matches!(wake.read(&mut buf), Ok(n) if n > 0) {}
                }
                Ok(_) => {}
                Err(e) => return self.fail(format!("poll: {e}")),
            }
        }
    }

    fn fail(&mut self, message: String) {
        self.end_hand_off(None, HandOff::Gone);
        // Nothing is held or heard any more: owns() turns false for every source.
        {
            let mut shared = lock(&self.shared);
            shared.owned.clear();
            shared.taking.clear();
            shared.offers.clear();
        }
        self.events.push(Event::Failed { message });
        self.subs.emit(std::mem::take(&mut self.events));
        self.abort_reads();
    }

    fn quit(&mut self) {
        self.end_hand_off(None, HandOff::Gone);
        #[cfg(feature = "dnd")]
        self.with_hook(|h, cx| h.stop(cx));
        self.abort_reads();
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.flush();
    }

    fn abort_reads(&mut self) {
        for read in self.incoming.drain(..) {
            read.fail(ReadError::Shutdown);
        }
        for wanted in self.queued.drain(..) {
            wanted.fail(ReadError::Shutdown);
        }
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

    fn atom(&mut self, name: &str) -> Option<Atom> {
        if let Some(a) = self.atom_of.get(name) {
            return Some(*a);
        }
        let atom = self
            .conn
            .intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        self.atom_of.insert(name.to_owned(), atom);
        self.names.insert(atom, name.to_owned());
        Some(atom)
    }

    fn name(&mut self, atom: Atom) -> Option<String> {
        if let Some(n) = self.names.get(&atom) {
            return Some(n.clone());
        }
        let reply = self.conn.get_atom_name(atom).ok()?.reply().ok()?;
        let name = String::from_utf8_lossy(&reply.name).into_owned();
        self.names.insert(atom, name.clone());
        self.atom_of.insert(name.clone(), atom);
        Some(name)
    }

    /// A property of our window to receive one conversion into.
    fn take_prop(&mut self) -> Option<Atom> {
        if let Some(p) = self.free_props.pop() {
            return Some(p);
        }
        let n = self.next_prop;
        self.next_prop += 1;
        self.atom(&format!("_TRANSFER_SUBSTRATE_{n}"))
    }

    fn next_deadline(&self) -> Option<Instant> {
        #[cfg(feature = "dnd")]
        let hook = self.hook.as_ref().and_then(|h| h.deadline());
        #[cfg(not(feature = "dnd"))]
        let hook = None;
        self.incoming
            .iter()
            .map(Incoming::deadline)
            .chain(self.outgoing.iter().map(Outgoing::deadline))
            .chain(self.handoff.as_ref().map(handoff::Waiting::deadline))
            .chain(self.multis.iter().map(multiple::Multi::deadline))
            .chain(hook)
            .min()
    }
}

#[cfg(test)]
mod tests;
