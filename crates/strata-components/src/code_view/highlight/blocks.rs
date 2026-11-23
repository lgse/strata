// SPDX-License-Identifier: MIT

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

const CACHE_BYTES: usize = 4 << 20;

const CACHE_CEILING: usize = 4 * CACHE_BYTES;

const ENTRY_BYTES: usize = 96;

/// What the cache counts for a block of `code`.
fn cost(code: &str) -> usize {
    code.len() + ENTRY_BYTES
}

const BLOCK_CUT: usize = 32 * 1024;

fn coloured_len(code: &str) -> usize {
    if code.len() <= BLOCK_CUT {
        return code.len();
    }
    code.as_bytes()[..BLOCK_CUT]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct CodeBlocks {
    state: Rc<RefCell<Blocks>>,
}

struct Entry {
    lang: Box<str>,
    code: Arc<str>,
    /// `None` while the thread works on it.
    tokens: Option<Rc<[Token]>>,
    /// When it was last asked for, in [`Blocks::tick`]s.
    used: u64,
}

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

    pub fn wait_on_miss(&mut self, wait: Option<Duration>) {
        self.state.borrow_mut().wait = wait;
    }

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

    fn bump(&mut self) {
        self.epoch += 1;
        let spare_after = self.prev_start;
        self.evict(CACHE_BYTES, 0, CACHE_BYTES, spare_after);
        if self.tick > self.build_start {
            self.prev_start = self.build_start;
            self.build_start = self.tick;
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

    fn request(&mut self, key: u64, lang: &str, code: &str) -> bool {
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
