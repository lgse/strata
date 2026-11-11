//! The windows attached to the substrate, each with the [`Turn`] its app gave at attach.
//!
//! The Wayland reader asks the UI thread to dispatch through a window's app
//! (`AppHandle::update_if`). A window that closes takes its app's runtime with it, and a turn
//! posted there never runs. Rune tells P0 neither when a window closes nor whether a turn
//! ran, and a hidden window (whose native side Rune destroys on Wayland) still runs its
//! app's updates, so a closed native window doesn't mean a dead turn. Hence:
//!
//! - **Every window keeps its own turn**, and a window attached again (Rune recreated it)
//!   replaces its entry.
//! - **The reader posts each round to every window with no turn outstanding.** A live app
//!   runs it soon, so its window is posted to again next round; a closed one keeps the one
//!   turn it was given, and is skipped from then on. Dispatching more than once a round is
//!   harmless: a dispatch with nothing queued does nothing.
//! - **Set aside on the UI thread**, at each attach and dispatch: a window whose native side
//!   is gone and whose turn stayed outstanding while two later rounds ran. That marks a
//!   closed window, but not only one: Rune runs one app's updates in the order they were
//!   posted, not one app's against another's, so a live window's app can run three rounds'
//!   turns on its own wakes before a hidden window's app runs its first. So a window set
//!   aside is dormant, not dropped: it gets no turns, and if its outstanding turn runs after
//!   all, it is back ([`Windows::ran`]). A dormant window goes for good when its key attaches
//!   again or is [`Windows::detach`]ed, and beyond [`DORMANT`] the oldest go: a closed
//!   window that never detaches can't be told from a hidden one whose app is slow. A window
//!   detached (its `on_close` ran) goes at once.
//!
//! So while any attached window lives, a turn the reader posts runs. With none attached the
//! reader waits, its events queued, until the next attach wakes it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use rune::WindowHandle;

use super::Turn;

/// What the list needs of a window: [`WindowHandle`], or a test's stand-in.
pub(crate) trait Window: Clone + Send + 'static {
    fn key(&self) -> &str;
    /// Whether its native window exists. On the UI thread.
    fn is_open(&self) -> bool;
}

impl Window for WindowHandle {
    fn key(&self) -> &str {
        WindowHandle::key(self)
    }

    fn is_open(&self) -> bool {
        self.with_raw_handle(|_| ()).is_ok()
    }
}

struct Entry<W> {
    id: u64,
    window: W,
    turn: Turn,
    /// The round of the turn posted here and not yet run.
    outstanding: Option<u64>,
}

struct Inner<W> {
    entries: Vec<Entry<W>>,
    /// Set aside by [`Windows::prune`], oldest first, each with its turn still outstanding.
    dormant: Vec<Entry<W>>,
    next_id: u64,
    /// The last round posted.
    round: u64,
    /// The newest round a turn ran for.
    ran: u64,
}

pub(crate) struct Windows<W: Window = WindowHandle> {
    inner: Mutex<Inner<W>>,
}

impl<W: Window> Default for Windows<W> {
    fn default() -> Self {
        Windows {
            inner: Mutex::new(Inner {
                entries: Vec::new(),
                dormant: Vec::new(),
                next_id: 0,
                round: 0,
                ran: 0,
            }),
        }
    }
}

/// Rounds a closed window's turn stays outstanding before it is set aside.
const ROUNDS_TO_PRUNE: u64 = 2;

/// How many windows stay set aside, waiting for a late turn.
const DORMANT: usize = 8;

impl<W: Window> Windows<W> {
    fn lock(&self) -> MutexGuard<'_, Inner<W>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds `window` with `turn`, replacing an entry of the same key, and prunes. On the UI
    /// thread.
    pub(crate) fn attach(&self, window: W, turn: Turn) {
        {
            let mut inner = self.lock();
            let id = inner.next_id;
            inner.next_id += 1;
            inner.entries.retain(|e| e.window.key() != window.key());
            inner.dormant.retain(|e| e.window.key() != window.key());
            inner.entries.push(Entry {
                id,
                window,
                turn,
                outstanding: None,
            });
        }
        self.prune();
    }

    /// Drops the window keyed `key`, set aside or not: it closed. True if it was attached.
    #[cfg_attr(
        not(any(test, feature = "foreign-parent")),
        expect(dead_code, reason = "P4 detaches a window as it closes")
    )]
    pub(crate) fn detach(&self, key: &str) -> bool {
        let mut inner = self.lock();
        let before = inner.entries.len() + inner.dormant.len();
        inner.entries.retain(|e| e.window.key() != key);
        inner.dormant.retain(|e| e.window.key() != key);
        inner.entries.len() + inner.dormant.len() != before
    }

    /// The windows attached, not counting those set aside.
    pub(crate) fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// The key of the first window that matches. On the UI thread, if `f` needs it.
    pub(crate) fn find(&self, f: impl Fn(&W) -> bool) -> Option<String> {
        self.lock()
            .entries
            .iter()
            .find(|e| f(&e.window))
            .map(|e| e.window.key().to_owned())
    }

    /// Sets aside the windows that look closed: native side gone, turn outstanding for
    /// [`ROUNDS_TO_PRUNE`] rounds. On the UI thread.
    pub(crate) fn prune(&self) {
        let mut inner = self.lock();
        let ran = inner.ran;
        let (stale, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut inner.entries)
            .into_iter()
            .partition(|e| {
                e.outstanding
                    .is_some_and(|r| ran >= r.saturating_add(ROUNDS_TO_PRUNE))
                    && !e.window.is_open()
            });
        inner.entries = kept;
        inner.dormant.extend(stale);
        let over = inner.dormant.len().saturating_sub(DORMANT);
        inner.dormant.drain(..over);
    }

    /// Posts a turn running `job()` to each window with none outstanding. Any thread; the
    /// turns are called with no lock held. How many were posted.
    pub(crate) fn post(self: &Arc<Self>, job: impl Fn() -> Box<dyn FnOnce() + Send>) -> usize {
        let targets: Vec<(u64, u64, Turn)> = {
            let mut inner = self.lock();
            inner.round += 1;
            let round = inner.round;
            inner
                .entries
                .iter_mut()
                .filter(|e| e.outstanding.is_none())
                .map(|e| {
                    e.outstanding = Some(round);
                    (e.id, round, e.turn.clone())
                })
                .collect()
        };
        let n = targets.len();
        for (id, round, turn) in targets {
            let weak: Weak<Self> = Arc::downgrade(self);
            let f = job();
            turn(Box::new(move || {
                if let Some(w) = weak.upgrade() {
                    w.ran(id, round);
                }
                f();
            }));
        }
        n
    }

    /// Entry `id`'s turn for `round` ran. One set aside is back: its app runs.
    fn ran(&self, id: u64, round: u64) {
        let mut inner = self.lock();
        inner.ran = inner.ran.max(round);
        if let Some(e) = inner.entries.iter_mut().find(|e| e.id == id) {
            if e.outstanding == Some(round) {
                e.outstanding = None;
            }
        } else if let Some(i) = inner.dormant.iter().position(|e| e.id == id) {
            let mut e = inner.dormant.remove(i);
            e.outstanding = None;
            inner.entries.push(e);
        }
    }
}

#[cfg(test)]
mod tests;
