//! Code blocks: the adapter for rune-kit's `Markdown` highlight hook, and for C11's own
//! blocks.
//!
//! rune-kit calls its hook while building a view, on the UI thread, and caches the answer per
//! highlighter, language and code. A block's first call misses: it asks the thread and
//! answers plain. When the colours land, [`CodeBlocks::poll`] bumps an epoch that is part of
//! the hook's cache key, so the next build asks again and gets them.
//!
//! An epoch bump is also where the cache trims: after one, the host builds again and every
//! block on screen is asked for, through any number of hooks. A host may poll any number of
//! times before it builds: a bump with nothing asked since the last one doesn't count as a
//! build. So a block asked for in the
//! last two builds (between bumps) is on screen, or was a moment ago, and is never evicted;
//! the others go, least recently asked first, down to [`CACHE_BYTES`]. A miss never evicts
//! a block the current build may still ask for.
//!
//! Blocks are keyed by a hash with the pool's own random key. Two blocks with one key still
//! take turns: one asked for since the last bump keeps the key, and the other draws plain.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rune::Color;
use rune_kit::syntax::{Highlight, Paint};

use super::engine::{Reply, Token};
use super::spans;
use super::worker::Request;

/// The cache trims to this many bytes of code at each epoch bump, sparing the blocks asked for
/// in the last two builds: the cap stretches to fit what is on screen, so a large document
/// (or two views of one pool) can't evict what it is drawing and ask for it again.
const CACHE_BYTES: usize = 4 << 20;

/// How far the cap stretches (512 blocks at the 32 KiB cut). A miss past it first evicts
/// blocks not asked for since the last bump, down to three quarters of it; a block that would
/// still take the cache past it answers plain and is never asked for, so nothing lands,
/// nothing wakes the host, and memory stays bounded. A host that never polls (one that waits
/// on misses) never trims, so its cache grows to here.
///
/// Both bounds count each block as its code's bytes plus [`ENTRY_BYTES`], so a mass of tiny
/// blocks is bounded by their number too. They don't count a landed block's runs, 12 bytes
/// each: about one run per 5 to 8 bytes of typical code, so 2 to 3 times the code, and at
/// worst (a run per byte) 13 times. rune-kit's hook keeps its own cache besides, within its
/// own budget.
const CACHE_CEILING: usize = 4 * CACHE_BYTES;

/// What an entry costs besides its code, as rune-kit's hook cache counts each entry: the
/// entry, its map slot, its language and the code's `Arc` header.
const ENTRY_BYTES: usize = 96;

/// What the cache counts for a block of `code`.
fn cost(code: &str) -> usize {
    code.len() + ENTRY_BYTES
}

/// A block colours up to its last line end within this many bytes: the cut rune-kit's
/// `Markdown` hands an app's highlighter over (`rune_kit::syntax` cut at 32 KiB at 5d421fd;
/// its built-in highlighter takes 128 KiB at 2a44156). C11's own blocks get the same cut, so a
/// block colours alike either way, and one huge block can't hold the thread or the cache.
const BLOCK_CUT: usize = 32 * 1024;

/// How much of `code` gets colour: all of it, or up to its last line end within
/// [`BLOCK_CUT`] (none when the first line runs past it), as rune-kit cuts.
fn coloured_len(code: &str) -> usize {
    if code.len() <= BLOCK_CUT {
        return code.len();
    }
    code.as_bytes()[..BLOCK_CUT]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap_or(0)
}

/// A pool of code blocks on the highlighter thread, from [`Highlighter::blocks`]. Clones
/// share the pool; it lives on the UI thread.
///
/// [`Highlighter::blocks`]: super::Highlighter::blocks
#[derive(Clone)]
pub struct CodeBlocks {
    state: Rc<RefCell<Blocks>>,
}

struct Entry {
    lang: Box<str>,
    /// The code as sent to the thread: its reply carries the same `Arc`, so a reply for a
    /// block since replaced under the same key is told apart.
    code: Arc<str>,
    /// `None` while the thread works on it.
    tokens: Option<Rc<[Token]>>,
    /// When it was last asked for, in [`Blocks::tick`]s.
    used: u64,
}

/// The cache's entries by key. Every scan of them (`values`, `iter`) goes through here, and
/// in tests each entry a scan yields counts as a visit, so a test can bound the work misses
/// and trims do on the UI thread however a scan is written.
#[derive(Default)]
struct Entries {
    map: HashMap<u64, Entry>,
    #[cfg(test)]
    visits: std::cell::Cell<u64>,
}

impl Entries {
    fn get(&self, key: &u64) -> Option<&Entry> {
        self.map.get(key)
    }

    fn get_mut(&mut self, key: &u64) -> Option<&mut Entry> {
        self.map.get_mut(key)
    }

    fn insert(&mut self, key: u64, entry: Entry) {
        self.map.insert(key, entry);
    }

    fn remove(&mut self, key: &u64) -> Option<Entry> {
        self.map.remove(key)
    }

    fn values(&self) -> impl Iterator<Item = &Entry> {
        self.map.values().inspect(|_| self.visit())
    }

    fn iter(&self) -> impl Iterator<Item = (&u64, &Entry)> {
        self.map.iter().inspect(|_| self.visit())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    fn visit(&self) {
        self.visits.set(self.visits.get() + 1);
    }

    #[cfg(not(test))]
    fn visit(&self) {}
}

struct Blocks {
    id: u64,
    tx: Option<Sender<Request>>,
    rx: Receiver<Reply>,
    entries: Entries,
    bytes: usize,
    /// The bytes of the blocks asked for since the last bump, kept as they are asked.
    current: usize,
    /// Counts `highlight` calls.
    tick: u64,
    /// The tick at the last epoch bump: blocks asked for after it belong to the current build.
    build_start: u64,
    /// The tick at the bump before: blocks asked for after it were in the last two builds.
    prev_start: u64,
    palette: Vec<Option<Color>>,
    epoch: u64,
    wait: Option<Duration>,
    /// The pool's own random hash key, so no document can be crafted to collide its blocks.
    hasher: RandomState,
    /// Tests force keys (to collide them) with this.
    #[cfg(test)]
    key_fn: Option<fn(&str, &str) -> u64>,
}

impl CodeBlocks {
    pub(crate) fn new(tx: Option<Sender<Request>>) -> CodeBlocks {
        let id = super::next_id();
        let (reply, rx) = mpsc::channel();
        if let Some(tx) = &tx {
            let _ = tx.send(Request::OpenPool { id, reply });
        }
        CodeBlocks {
            state: Rc::new(RefCell::new(Blocks {
                id,
                tx,
                rx,
                entries: Entries::default(),
                bytes: 0,
                current: 0,
                tick: 0,
                build_start: 0,
                prev_start: 0,
                palette: Vec::new(),
                epoch: 0,
                wait: None,
                hasher: RandomState::new(),
                #[cfg(test)]
                key_fn: None,
            })),
        }
    }

    /// A highlighter for `Markdown::highlighter`. Build a fresh one each time the view
    /// builds: its cache key changes when colours land or the theme changes.
    ///
    /// A host that memoises a view holding such a `Markdown` (`rune::memo`) must make the
    /// memo's version change with each [`poll`](Self::poll) that returns true, counting them
    /// itself; otherwise the memo keeps the plain answers of the first build and the colours
    /// never show.
    pub fn markdown(&self) -> rune_kit::syntax::Highlighter {
        let (id, epoch) = {
            let s = self.state.borrow();
            (s.id, s.epoch)
        };
        let state = self.state.clone();
        rune_kit::syntax::Highlighter::new(move |lang, code| {
            state.borrow_mut().highlight(lang, code).unwrap_or_default()
        })
        .key((id, epoch))
    }

    /// `code`'s colours as rune-kit highlights (byte ranges into `code`), or `None` while
    /// the thread works on it: draw it plain meanwhile. A block with no language, or one no
    /// grammar knows, is plain: `Some` of nothing. Colour stops at the last line end within
    /// 32 KiB (`BLOCK_CUT`), and a block past the cache's ceiling for one
    /// build is plain.
    pub fn highlight(&self, lang: Option<&str>, code: &str) -> Option<Vec<Highlight>> {
        self.state.borrow_mut().highlight(lang, code)
    }

    /// Keys blocks with `f` instead of the hash, so a test can make two blocks share a key.
    #[cfg(test)]
    pub(crate) fn key_with(&self, f: fn(&str, &str) -> u64) {
        self.state.borrow_mut().key_fn = Some(f);
    }

    /// Entries the cache's misses and trims have looked at so far.
    #[cfg(test)]
    pub(crate) fn visits(&self) -> u64 {
        self.state.borrow().entries.visits.get()
    }

    /// The key this pool gives a block.
    #[cfg(test)]
    pub(crate) fn key_for(&self, lang: &str, code: &str) -> u64 {
        self.state.borrow().key(lang, code)
    }

    /// The running total of the current build's bytes, and the same counted afresh.
    #[cfg(test)]
    pub(crate) fn current_check(&self) -> (usize, usize) {
        let s = self.state.borrow();
        let counted = s
            .entries
            .map
            .values()
            .filter(|e| e.used > s.build_start)
            .map(|e| cost(&e.code))
            .sum();
        (s.current, counted)
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.state.borrow().entries.len()
    }

    #[cfg(test)]
    pub(crate) fn cached_bytes(&self) -> usize {
        self.state.borrow().bytes()
    }

    /// Whether `code`'s colours (cut as `highlight` cuts) are cached and landed.
    #[cfg(test)]
    pub(crate) fn has_landed(&self, lang: &str, code: &str) -> bool {
        let s = self.state.borrow();
        let code = &code[..coloured_len(code)];
        s.entries
            .map
            .get(&s.key(lang, code))
            .is_some_and(|e| *e.code == *code && e.tokens.is_some())
    }

    /// The keys of the blocks asked for that haven't landed.
    #[cfg(test)]
    pub(crate) fn pending_keys(&self) -> Vec<u64> {
        let s = self.state.borrow();
        s.entries
            .map
            .iter()
            .filter(|(_, e)| e.tokens.is_none())
            .map(|(&k, _)| k)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn pool_id(&self) -> u64 {
        self.state.borrow().id
    }

    /// Takes the blocks that have landed. True when colours changed: the host builds again,
    /// asking for every block on screen (the next `markdown()` hook has a new cache key).
    pub fn poll(&mut self) -> bool {
        let mut s = self.state.borrow_mut();
        let mut changed = false;
        while let Ok(reply) = s.rx.try_recv() {
            changed |= s.apply(reply);
        }
        if changed {
            s.bump();
        }
        changed
    }

    /// Makes a miss wait up to `wait` for the thread instead of answering plain. **For
    /// headless runs and goldens only**, so a scene's first build is already coloured: in a
    /// window it would stall the frame.
    pub fn wait_on_miss(&mut self, wait: Option<Duration>) {
        self.state.borrow_mut().wait = wait;
    }

    /// Blocks until every block asked for has landed, or `timeout` passes. True when they
    /// all did. For tests and headless snapshots only.
    pub fn settle(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.borrow_mut();
        let mut changed = false;
        while s.pending() {
            let left = deadline.saturating_duration_since(Instant::now());
            match s.rx.recv_timeout(left) {
                Ok(reply) => changed |= s.apply(reply),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        if changed {
            s.bump();
        }
        !s.pending()
    }
}

impl Blocks {
    fn key(&self, lang: &str, code: &str) -> u64 {
        #[cfg(test)]
        if let Some(f) = self.key_fn {
            return f(lang, code);
        }
        self.hasher.hash_one((lang, code))
    }

    fn pending(&self) -> bool {
        self.tx.is_some() && self.entries.values().any(|e| e.tokens.is_none())
    }

    /// A new epoch: the next build asks for every block on screen. Trims the cache to
    /// [`CACHE_BYTES`], sparing the blocks asked for in the last two builds.
    ///
    /// A bump with nothing asked since the last one (a host polling several times before it
    /// builds) leaves the build marks where they are: moving them would push the last real
    /// build out of the last two, and its blocks, still on screen, would be trimmed.
    fn bump(&mut self) {
        self.epoch += 1;
        let spare_after = self.prev_start;
        self.evict(CACHE_BYTES, 0, CACHE_BYTES, spare_after);
        if self.tick > self.build_start {
            self.prev_start = self.build_start;
            self.build_start = self.tick;
            // Every block is now from an earlier build. (Trims and misses only ever drop
            // blocks from earlier builds, so `current` needs no other upkeep.)
            self.current = 0;
        }
    }

    fn apply(&mut self, reply: Reply) -> bool {
        match reply {
            Reply::Palette { from, colors } => {
                self.palette.truncate(from);
                self.palette.extend(colors);
                true
            }
            Reply::Block { key, code, tokens } => match self.entries.get_mut(&key) {
                Some(e) if Arc::ptr_eq(&e.code, &code) => {
                    e.tokens = Some(tokens.into());
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn highlight(&mut self, lang: Option<&str>, code: &str) -> Option<Vec<Highlight>> {
        // A fence with no language is as often output or prose as code (rune-kit's rule).
        let Some(lang) = lang.map(str::trim).filter(|l| !l.is_empty()) else {
            return Some(Vec::new());
        };
        // No highlighter thread (it couldn't start, or it stopped): plain, as documented.
        if self.tx.is_none() {
            return Some(Vec::new());
        }
        let code = &code[..coloured_len(code)];
        if code.is_empty() {
            return Some(Vec::new());
        }
        self.tick += 1;
        let key = self.key(lang, code);
        match self.entries.get_mut(&key) {
            Some(e) if *e.lang == *lang && *e.code == *code => {
                if e.used <= self.build_start {
                    self.current += cost(&e.code);
                }
                e.used = self.tick;
            }
            _ => {
                if !self.request(key, lang, code) {
                    return Some(Vec::new());
                }
            }
        }
        if self.entries.get(&key).is_some_and(|e| e.tokens.is_none())
            && let Some(wait) = self.wait
        {
            self.wait_for(key, wait);
        }
        let tokens = self.entries.get(&key)?.tokens.clone()?;
        let mut runs: Vec<(Range<usize>, Color)> = Vec::new();
        spans::colored(&tokens, &self.palette, 0, &mut runs);
        Some(
            runs.into_iter()
                .map(|(r, c)| (r, Paint::Color(c)))
                .collect(),
        )
    }

    /// Asks the thread for a block and keeps a pending entry. False, and nothing asked, when
    /// the block would take the cache past [`CACHE_CEILING`] even without the blocks not
    /// asked for since the last bump, when another block holds its key and was asked for since
    /// then, or there is no thread: it is then plain.
    fn request(&mut self, key: u64, lang: &str, code: &str) -> bool {
        // A hash collision. The other block, if the current build asked for it, keeps the key
        // and this one draws plain: replacing it would ask again on every landing, and two
        // such blocks on screen would never come to rest. Otherwise it gives way: count only
        // the one kept, and drop its job.
        if self
            .entries
            .get(&key)
            .is_some_and(|e| e.used > self.build_start)
        {
            return false;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(cost(&old.code));
            if old.tokens.is_none() {
                self.forget(vec![key]);
            }
        }
        if self.current + cost(code) > CACHE_CEILING {
            return false;
        }
        // Past the ceiling, free a quarter of it at once, as `Budget` does, so a host that
        // never polls doesn't sort the cache again on every miss.
        self.evict(
            CACHE_CEILING,
            cost(code),
            CACHE_CEILING / 4 * 3,
            self.build_start,
        );
        let Some(tx) = &self.tx else {
            return false;
        };
        let code: Arc<str> = Arc::from(code);
        let sent = tx.send(Request::Block {
            pool: self.id,
            key,
            lang: lang.into(),
            code: code.clone(),
        });
        if sent.is_err() {
            // The thread is gone: every block is plain from now on.
            self.tx = None;
            return false;
        }
        self.bytes += cost(&code);
        self.current += cost(&code);
        let entry = Entry {
            lang: lang.into(),
            code,
            tokens: None,
            used: self.tick,
        };
        self.entries.insert(key, entry);
        true
    }

    /// When `extra` more bytes don't fit under `limit`, drops the least recently asked blocks
    /// asked for at or before tick `spare_after` until they fit under `target`. The thread
    /// drops the jobs of those still pending.
    ///
    /// It sorts the candidates once, as rune-kit's `Budget::evict` does, so a trim of `n`
    /// blocks costs `n log n` on the UI thread rather than `n` per block dropped.
    fn evict(&mut self, limit: usize, extra: usize, target: usize, spare_after: u64) {
        if self.bytes + extra <= limit {
            return;
        }
        let mut old: Vec<(u64, u64)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.used <= spare_after)
            .map(|(&k, e)| (e.used, k))
            .collect();
        old.sort_unstable();
        let mut forgotten = Vec::new();
        for (_, key) in old {
            if self.bytes + extra <= target {
                break;
            }
            if let Some(e) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(cost(&e.code));
                if e.tokens.is_none() {
                    forgotten.push(key);
                }
            }
        }
        if !forgotten.is_empty() {
            self.forget(forgotten);
        }
    }

    /// Tells the thread to drop the jobs of blocks no longer wanted.
    fn forget(&mut self, keys: Vec<u64>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Request::Forget {
                pool: self.id,
                keys,
            });
        }
    }

    fn wait_for(&mut self, key: u64, wait: Duration) {
        let deadline = Instant::now() + wait;
        while self.entries.get(&key).is_some_and(|e| e.tokens.is_none()) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(reply) => {
                    // A new theme (a palette replacing colours already given) must reach
                    // rune-kit's cache too: bump the epoch, as `poll` would.
                    let theme =
                        matches!(&reply, Reply::Palette { from, .. } if *from < self.palette.len());
                    self.apply(reply);
                    if theme {
                        self.bump();
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for Blocks {
    fn drop(&mut self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Request::Close { id: self.id });
        }
    }
}
