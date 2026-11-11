//! The store: one handle per thumbnail key, held under a byte budget, with each key's state.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use rune::{Live, Pixels};

use super::queue::{Queued, Viewer};
use super::step::Step;
use crate::contract::ThumbKey;

/// The largest side Rune draws a `Pixels` at (`ImageAtlas::place` at the pinned rev).
const MAX_SIDE: u32 = 8192;

/// How many empty handles (nothing held, nothing asked) the store keeps before it prunes the
/// ones no viewer is near.
const PRUNE_SLACK: usize = 4096;

/// What the store is held to. The defaults are Strata's numbers (`src/ui/thumbnail/cache.rs`,
/// `src/ui/thumbnail/viewport.rs` at `upstream/main` 35ee677ab): Strata passes its own, and a
/// test or the gallery passes others.
///
/// Fields may be added: start from [`Settings::default`] and set the ones that differ.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Settings {
    /// The bytes held images may take (`w × h × 4` each): 64 MiB.
    pub budget_bytes: usize,
    /// How far past the viewport thumbnails load, in viewports per side: 0.25.
    pub margin: f32,
    /// How long a failed thumbnail waits before it is asked for again, in seconds: 30.
    pub retry_after: f64,
}

impl Settings {
    /// With a budget of `bytes`.
    pub fn budget(mut self, bytes: usize) -> Self {
        self.budget_bytes = bytes;
        self
    }

    /// With a margin of `viewports` per side.
    pub fn margin(mut self, viewports: f32) -> Self {
        self.margin = viewports;
        self
    }

    /// With failed thumbnails asked again after `seconds`.
    pub fn retry_after(mut self, seconds: f64) -> Self {
        self.retry_after = seconds;
        self
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            budget_bytes: 64 * 1024 * 1024,
            margin: 0.25,
            retry_after: 30.0,
        }
    }
}

/// A key's handles: the image and the placeholder icon's opacity, each updated in place.
///
/// The image ([`Thumb::pixels`]) is empty until a thumbnail lands, and empty again once it is
/// evicted; the icon's opacity ([`Thumb::icon_alpha`]) is 0 while an image is held and 1
/// otherwise. Both wake only the windows that draw them, with no view commit, so a view draws
/// a key's handles once and never builds again when its thumbnail lands, changes size or goes.
///
/// Used by: C3, C4, C25 ([`picture`](super::picture)).
#[derive(Clone)]
pub struct Thumb {
    pixels: Pixels,
    icon_alpha: Live<f32>,
    epoch: u64,
}

impl std::fmt::Debug for Thumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumb")
            .field("pixels", &self.pixels)
            .field("icon_alpha", &self.icon_alpha.get())
            .field("epoch", &self.epoch)
            .finish()
    }
}

impl Thumb {
    /// The image: draw it with `El::pixels_fit`.
    pub fn pixels(&self) -> &Pixels {
        &self.pixels
    }

    /// The placeholder icon's opacity: bind it with `El::live_opacity`.
    pub fn icon_alpha(&self) -> &Live<f32> {
        &self.icon_alpha
    }

    /// Which handles these are. A key's handles keep their epoch while the store holds the
    /// key; a key the store pruned and sees again gets new handles and a new epoch, which
    /// [`ThumbnailStore::version`] carries so a row built on the old ones builds again.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// A key's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Nothing held, nothing asked: a view draws the placeholder icon.
    Empty,
    /// Asked for, with nothing held yet.
    Requested,
    /// An image is held at `step` (a request at another step may be out).
    Ready {
        /// The step the image was rendered for.
        step: Step,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
    },
    /// Rendering failed: not asked for again before `until` (seconds on the host's clock).
    Failed {
        /// When it may be asked for again.
        until: f64,
    },
    /// A file operation stopped it: not asked for again until
    /// [`ThumbnailStore::requeue_stopped`].
    Stopped,
}

/// One request the host is working on. Hand it back to [`ThumbnailStore::complete`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket {
    pub(super) key: ThumbKey,
    pub(super) step: Step,
    pub(super) n: u64,
}

impl Ticket {
    /// The key asked for.
    pub fn key(&self) -> ThumbKey {
        self.key
    }

    /// The step to render at: the image's long edge must not exceed it.
    pub fn step(&self) -> Step {
        self.step
    }
}

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Rendered: `width × height` straight-alpha sRGB RGBA pixels, rows top first, the long
    /// edge at most the ticket's step.
    Ready {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// `width × height × 4` bytes.
        rgba: Vec<u8>,
    },
    /// Rendering failed: the key waits [`Settings`]' `retry_after` before it is asked again.
    Failed,
    /// Stopped by a file operation, not failed: asked again after
    /// [`ThumbnailStore::requeue_stopped`].
    Stopped,
}

/// What [`ThumbnailStore::complete`] did with a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Held, for a row a viewer shows: it draws on the next frame.
    Visible,
    /// Held, for a row no viewer shows (the margin, or scrolled away since).
    Offscreen,
    /// Recorded as failed.
    Failed,
    /// Recorded as stopped.
    Stopped,
    /// Not held.
    Dropped(Why),
}

/// Why a result was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The request was cancelled (scrolled away, thumbnails off, the key relocated) or is not
    /// the key's latest.
    Cancelled,
    /// The image doesn't match its size, is empty, or is larger than the ticket's step: the
    /// key is recorded as failed.
    Invalid,
    /// It wouldn't fit: rows on screen fill the budget, or a margin image could only land by
    /// evicting an image some viewer still wants. The key isn't asked for again until room may
    /// have appeared: a sync to another viewport, new settings, a viewer going, a relocation, a
    /// wanted image evicted or replaced by a smaller one, or a margin request ending.
    Budget,
}

/// What the store holds, for a status line or a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stats {
    /// Bytes held.
    pub bytes: usize,
    /// The budget.
    pub budget: usize,
    /// Bytes held for rows a viewer shows (or showed at its previous sync).
    pub protected_bytes: usize,
    /// Keys with an image held.
    pub ready: usize,
    /// Requests out.
    pub requested: usize,
    /// Keys recorded as failed.
    pub failed: usize,
    /// Keys recorded as stopped.
    pub stopped: usize,
    /// Keys the store has handles for.
    pub entries: usize,
}

/// Thumbnails as `rune::Pixels` handles under a byte budget, and the requests to load next.
///
/// See the [module docs](super) for how a host and a view use it.
///
/// Used by: C3, C4, C2, hosts.
pub struct ThumbnailStore {
    pub(super) inner: RefCell<Inner>,
}

impl std::fmt::Debug for ThumbnailStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbnailStore")
            .field("stats", &self.stats())
            .finish()
    }
}

impl Default for ThumbnailStore {
    fn default() -> Self {
        Self::new(Settings::default())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Held {
    pub step: Step,
    pub width: u32,
    pub height: u32,
}

impl Held {
    pub fn bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Job {
    Idle,
    Requested { n: u64, step: Step },
    Failed { until: f64 },
    Stopped,
}

pub(super) struct Entry {
    pub thumb: Thumb,
    pub held: Option<Held>,
    pub job: Job,
    /// When it was last touched (built or shown), on the store's touch counter.
    pub stamp: u64,
}

pub(super) struct Inner {
    pub settings: Settings,
    pub enabled: bool,
    pub entries: HashMap<ThumbKey, Entry>,
    /// Keys with an image held, least recently touched first: the eviction order.
    pub lru: BTreeMap<u64, ThumbKey>,
    pub touches: u64,
    pub next_epoch: u64,
    pub next_ticket: u64,
    pub bytes: usize,
    pub in_flight: HashSet<ThumbKey>,
    pub cancelled: Vec<Ticket>,
    pub viewers: Vec<Option<Viewer>>,
    /// Keys every viewer shows now or showed at its previous sync: never evicted.
    pub protected: HashSet<ThumbKey>,
    /// Keys some viewer wants (shown or in a margin), at the largest step any wants.
    pub wanted: HashMap<ThumbKey, Step>,
    /// Keys whose result the budget dropped: not asked again until the next sync.
    pub deferred: HashSet<ThumbKey>,
    /// Bytes held for keys some viewer wants or protects: what a margin result may not evict.
    pub pinned: usize,
    /// The bytes margin requests out may bring (each at its step's largest), by key.
    pub margin_out: HashMap<ThumbKey, usize>,
    pub margin_out_bytes: usize,
    /// `pop`'s ranked keys, and how far it has gone through them. Rebuilt only when `dirty`.
    pub queue: Vec<Queued>,
    pub cursor: usize,
    pub dirty: bool,
    /// The earliest time a failed key skipped by the queue may be asked again.
    pub next_expiry: f64,
    /// A margin key waited for room since the queue was built.
    pub gated: bool,
    pub prune_slack: usize,
    /// Keys `pop` has looked at, for the tests that hold its cost.
    #[cfg(test)]
    pub scans: usize,
}

impl ThumbnailStore {
    /// An empty store, thumbnails on.
    pub fn new(settings: Settings) -> Self {
        Self {
            inner: RefCell::new(Inner {
                settings,
                enabled: true,
                entries: HashMap::new(),
                lru: BTreeMap::new(),
                touches: 0,
                next_epoch: 1,
                next_ticket: 1,
                bytes: 0,
                in_flight: HashSet::new(),
                cancelled: Vec::new(),
                viewers: Vec::new(),
                protected: HashSet::new(),
                wanted: HashMap::new(),
                deferred: HashSet::new(),
                pinned: 0,
                margin_out: HashMap::new(),
                margin_out_bytes: 0,
                queue: Vec::new(),
                cursor: 0,
                dirty: true,
                next_expiry: f64::INFINITY,
                gated: false,
                prune_slack: PRUNE_SLACK,
                #[cfg(test)]
                scans: 0,
            }),
        }
    }

    /// The settings.
    pub fn settings(&self) -> Settings {
        self.inner.borrow().settings
    }

    /// Changes the settings. A smaller budget evicts at once (rows on screen aside). A new
    /// margin applies at each viewer's next sync, even of an unchanged viewport.
    pub fn set_settings(&mut self, settings: Settings) {
        let inner = self.inner.get_mut();
        inner.settings = settings;
        for w in inner.viewers.iter_mut().flatten() {
            w.stale();
        }
        inner.enforce();
        // A larger budget may fit margins it dropped.
        inner.deferred.clear();
        inner.dirty = true;
    }

    /// The key's handles, made the first time anything asks: call it while building the row
    /// or tile that draws them. It counts as drawn (least-recently-used order).
    pub fn thumb(&self, key: ThumbKey) -> Thumb {
        let mut inner = self.inner.borrow_mut();
        inner.ensure(key);
        inner.touch(key);
        inner.entries[&key].thumb.clone()
    }

    /// A memo version for a part that draws `key`'s handles: it changes when the key's
    /// handles are replaced (after a prune) or thumbnails are turned on or off, and never when
    /// a thumbnail lands, changes size or is evicted.
    pub fn version(&self, key: ThumbKey) -> u64 {
        let mut inner = self.inner.borrow_mut();
        inner.ensure(key);
        let mut h = std::hash::DefaultHasher::new();
        inner.entries[&key].thumb.epoch.hash(&mut h);
        inner.enabled.hash(&mut h);
        h.finish()
    }

    /// The key's state; [`State::Empty`] for a key the store has never seen.
    pub fn state(&self, key: ThumbKey) -> State {
        let inner = self.inner.borrow();
        let Some(e) = inner.entries.get(&key) else {
            return State::Empty;
        };
        match (e.held, e.job) {
            (Some(h), _) => State::Ready {
                step: h.step,
                width: h.width,
                height: h.height,
            },
            (None, Job::Requested { .. }) => State::Requested,
            (None, Job::Failed { until }) => State::Failed { until },
            (None, Job::Stopped) => State::Stopped,
            (None, Job::Idle) => State::Empty,
        }
    }

    /// Whether thumbnails are on.
    pub fn enabled(&self) -> bool {
        self.inner.borrow().enabled
    }

    /// What the store holds.
    pub fn stats(&self) -> Stats {
        let inner = self.inner.borrow();
        let mut s = Stats {
            bytes: inner.bytes,
            budget: inner.settings.budget_bytes,
            protected_bytes: inner.protected_bytes(),
            entries: inner.entries.len(),
            ready: inner.lru.len(),
            requested: inner.in_flight.len(),
            ..Stats::default()
        };
        for e in inner.entries.values() {
            match e.job {
                Job::Failed { .. } => s.failed += 1,
                Job::Stopped => s.stopped += 1,
                _ => {}
            }
        }
        s
    }

    /// Hands back a request's result. Call it from the host's `poll`, and don't commit for
    /// it: a held image wakes the windows that draw it, with no view commit.
    pub fn complete(&mut self, ticket: Ticket, outcome: Outcome, now: f64) -> Landed {
        let inner = self.inner.get_mut();
        let live = inner.entries.get(&ticket.key).is_some_and(|e| {
            e.job
                == Job::Requested {
                    n: ticket.n,
                    step: ticket.step,
                }
        });
        if !live {
            return Landed::Dropped(Why::Cancelled);
        }
        inner.in_flight.remove(&ticket.key);
        inner.unmargin(ticket.key);
        let retry = now + inner.settings.retry_after;
        let entry = inner.entries.get_mut(&ticket.key).expect("checked above");
        let (width, height, rgba) = match outcome {
            Outcome::Failed => {
                entry.job = Job::Failed { until: retry };
                inner.next_expiry = inner.next_expiry.min(retry);
                return Landed::Failed;
            }
            Outcome::Stopped => {
                entry.job = Job::Stopped;
                return Landed::Stopped;
            }
            Outcome::Ready {
                width,
                height,
                rgba,
            } => (width, height, rgba),
        };
        let fits = width > 0
            && height > 0
            && width.max(height) <= ticket.step.px().min(MAX_SIDE)
            && (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4))
                == Some(rgba.len());
        if !fits {
            entry.job = Job::Failed { until: retry };
            inner.next_expiry = inner.next_expiry.min(retry);
            return Landed::Dropped(Why::Invalid);
        }
        entry.job = Job::Idle;
        let protected = inner.protected.contains(&ticket.key);
        let new = Held {
            step: ticket.step,
            width,
            height,
        };
        let old = inner.entries[&ticket.key].held.map_or(0, Held::bytes);
        let pinned = inner.is_pinned(ticket.key);
        // Rows on screen win over the budget. Anything else lands only if it fits once the
        // images nobody wants are evicted: it never evicts a key some viewer still wants, which
        // would only be asked for again (and evicts nothing when it won't fit anyway).
        if !protected {
            let free = inner.bytes - inner.pinned - if pinned { 0 } else { old };
            let after = inner.bytes - old + new.bytes();
            if after.saturating_sub(free) > inner.settings.budget_bytes {
                inner.deferred.insert(ticket.key);
                return Landed::Dropped(Why::Budget);
            }
        }
        inner.make_room(ticket.key, old, new.bytes(), protected);
        inner.bytes = inner.bytes - old + new.bytes();
        if pinned {
            inner.pinned = inner.pinned - old + new.bytes();
            if new.bytes() < old {
                // A wanted image replaced by a smaller one (a zoom out) frees room.
                inner.room_freed();
            }
        }
        let entry = inner.entries.get_mut(&ticket.key).expect("checked above");
        entry.held = Some(new);
        entry.thumb.pixels.set(width, height, rgba);
        entry.thumb.icon_alpha.set(0.0);
        inner.touch(ticket.key);
        if inner
            .wanted
            .get(&ticket.key)
            .is_some_and(|w| *w != ticket.step)
        {
            // Landed at a step no longer wanted (a zoom, or a viewer at another step, while it
            // was out): it shows until the image at the wanted step lands, so ask for that.
            inner.dirty = true;
        }
        if protected {
            Landed::Visible
        } else {
            Landed::Offscreen
        }
    }

    /// The requests cancelled since the last call: the host stops working on them. A result
    /// for one is dropped anyway.
    pub fn take_cancelled(&mut self) -> Vec<Ticket> {
        std::mem::take(&mut self.inner.get_mut().cancelled)
    }

    /// The file operation that stopped thumbnails ended: stopped keys are asked for again.
    pub fn requeue_stopped(&mut self) {
        let inner = self.inner.get_mut();
        for e in inner.entries.values_mut() {
            if e.job == Job::Stopped {
                e.job = Job::Idle;
            }
        }
        inner.dirty = true;
    }

    /// Moves what `from` holds to `to`, keeping its handles (an inline rename that kept the
    /// thumbnail's content, as the host judges it). True if `from` was there to move. A
    /// request out for `from` is cancelled, and what `to` held is replaced. Viewers that showed
    /// `from` show `to` until their next sync, so its protection moves with it.
    pub fn relocate(&mut self, from: ThumbKey, to: ThumbKey) -> bool {
        let inner = self.inner.get_mut();
        if from == to || !inner.entries.contains_key(&from) {
            return false;
        }
        inner.cancel(from);
        if inner.entries.contains_key(&to) {
            inner.cancel(to);
            inner.evict(to);
            inner.entries.remove(&to);
        }
        let entry = inner.entries.remove(&from).expect("checked above");
        if entry.held.is_some() {
            inner.lru.insert(entry.stamp, to);
        }
        inner.entries.insert(to, entry);
        for w in inner.viewers.iter_mut().flatten() {
            w.rename(from, to);
        }
        inner.rewant();
        inner.enforce();
        // `to`'s old image (if any) went: margins the budget dropped may fit now.
        inner.deferred.clear();
        true
    }

    /// Turns thumbnails on or off. Off cancels every request and asks for none; images
    /// already held stay, so turning it back on draws them with no decode. True when it
    /// changed: views draw icons only while off, so the host commits once.
    pub fn set_enabled(&mut self, on: bool) -> bool {
        let inner = self.inner.get_mut();
        if inner.enabled == on {
            return false;
        }
        inner.enabled = on;
        inner.dirty = true;
        if !on {
            let keys: Vec<ThumbKey> = inner.in_flight.iter().copied().collect();
            for key in keys {
                inner.cancel(key);
            }
        }
        true
    }

    #[cfg(test)]
    pub(super) fn set_prune_slack(&mut self, n: usize) {
        self.inner.get_mut().prune_slack = n;
    }
}

impl Inner {
    pub fn ensure(&mut self, key: ThumbKey) {
        if self.entries.contains_key(&key) {
            return;
        }
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        self.entries.insert(
            key,
            Entry {
                thumb: Thumb {
                    pixels: Pixels::new(),
                    icon_alpha: Live::new(1.0),
                    epoch,
                },
                held: None,
                job: Job::Idle,
                stamp: 0,
            },
        );
    }

    pub fn touch(&mut self, key: ThumbKey) {
        self.touches += 1;
        let stamp = self.touches;
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        if e.held.is_some() {
            self.lru.remove(&e.stamp);
            self.lru.insert(stamp, key);
        }
        e.stamp = stamp;
    }

    pub fn protected_bytes(&self) -> usize {
        self.protected
            .iter()
            .filter_map(|k| self.entries.get(k)?.held)
            .map(Held::bytes)
            .sum()
    }

    /// Whether some viewer wants or protects `key`.
    pub fn is_pinned(&self, key: ThumbKey) -> bool {
        self.wanted.contains_key(&key) || self.protected.contains(&key)
    }

    /// Recounts the bytes held for keys some viewer wants or protects.
    pub fn repin(&mut self) {
        let held = |k: &ThumbKey| {
            self.entries
                .get(k)
                .and_then(|e| e.held)
                .map_or(0, Held::bytes)
        };
        let wanted: usize = self.wanted.keys().map(held).sum();
        let only_protected: usize = self
            .protected
            .iter()
            .filter(|k| !self.wanted.contains_key(k))
            .map(held)
            .sum();
        self.pinned = wanted + only_protected;
    }

    /// Empties `key`'s image in place (its handles stay, so a row still drawing them shows
    /// the icon again).
    pub fn evict(&mut self, key: ThumbKey) {
        let pinned = self.is_pinned(key);
        let wanted = self.wanted.contains_key(&key);
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        let Some(held) = e.held.take() else {
            return;
        };
        self.lru.remove(&e.stamp);
        self.bytes -= held.bytes();
        e.thumb.pixels.set(0, 0, Vec::new());
        e.thumb.icon_alpha.set(1.0);
        if pinned {
            self.pinned -= held.bytes();
            self.room_freed();
        }
        if wanted {
            // Wanted and empty again: the queue asks for it.
            self.dirty = true;
        }
    }

    /// The least recently touched held key that isn't protected and isn't `except`: one no
    /// viewer wants first, else (when `wanted_too`) one in a margin.
    fn victim(&self, except: Option<ThumbKey>, wanted_too: bool) -> Option<ThumbKey> {
        let evictable = |k: &ThumbKey| Some(*k) != except && !self.protected.contains(k);
        self.lru
            .values()
            .copied()
            .find(|k| evictable(k) && !self.wanted.contains_key(k))
            .or_else(|| {
                wanted_too
                    .then(|| self.lru.values().copied().find(evictable))
                    .flatten()
            })
    }

    /// Evicts until `key`'s image, growing from `old` to `new` bytes, fits the budget, or
    /// nothing evictable is left. Margin images go only for a row on screen (`for_screen`).
    fn make_room(&mut self, key: ThumbKey, old: usize, new: usize, for_screen: bool) {
        while self.bytes - old + new > self.settings.budget_bytes {
            let Some(victim) = self.victim(Some(key), for_screen) else {
                return;
            };
            self.evict(victim);
        }
    }

    /// Evicts until the budget holds, or only protected images are left: images no viewer
    /// wants first, then margins.
    pub fn enforce(&mut self) {
        while self.bytes > self.settings.budget_bytes {
            let Some(victim) = self.victim(None, true) else {
                return;
            };
            self.evict(victim);
        }
    }

    /// Forgets a margin request's room, once it ended.
    pub fn unmargin(&mut self, key: ThumbKey) {
        if let Some(b) = self.margin_out.remove(&key) {
            self.margin_out_bytes -= b;
            self.room_freed();
        }
    }

    /// Room may have appeared (the pinned bytes or the margin room out dropped): keys the
    /// budget deferred may fit now, and margin keys `pop` passed for want of room may too. If
    /// any wait, `pop` ranks again. Anything else that frees room (a sync, a viewer going, new
    /// settings, a relocation) forgets the deferrals and ranks again itself.
    pub fn room_freed(&mut self) {
        if self.gated || !self.deferred.is_empty() {
            self.deferred.clear();
            self.dirty = true;
        }
    }

    /// Cancels `key`'s request, if one is out.
    pub fn cancel(&mut self, key: ThumbKey) {
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        if let Job::Requested { n, step } = e.job {
            e.job = Job::Idle;
            self.in_flight.remove(&key);
            self.cancelled.push(Ticket { key, step, n });
            self.unmargin(key);
            self.dirty = true;
        }
    }
}
