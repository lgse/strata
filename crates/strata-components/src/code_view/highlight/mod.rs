// SPDX-License-Identifier: MIT

mod blocks;
mod detect;
mod engine;
mod palette;
mod spans;
mod worker;

#[cfg(test)]
mod tests;

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rune::{Color, Span, Waker};
use syntect::highlighting::Theme;

pub use blocks::CodeBlocks;

/// Number of bundled syntax definitions loaded for code previews.
pub fn language_count() -> usize {
    detect::syntaxes().map_or(0, |set| set.syntaxes().len())
}

use engine::{LineTokens, Lines, OpenArgs, Reply};
use worker::{Request, WakerSlot, Worker};

/// The checkpoint interval for a preview, in lines (S4's decision: the 8 ms reading at p95).
pub const CHECKPOINT_EVERY: usize = 64;

/// Lines highlighted below the viewport (S4's viewport: 80 visible lines plus 40).
pub const OVERSCAN: usize = 40;

/// Lines longer than this, in bytes, draw plain (S4: a 100 KB line costs 64–181 ms).
pub const LONG_LINE: usize = 16 * 1024;

const CACHED_LINES: usize = 4096;

/// The highlighter's tuning. [`Options::default`] has S4's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// A preview's checkpoint interval, in lines.
    pub checkpoint_every: usize,
    /// Lines highlighted below the viewport.
    pub overscan: usize,
    /// The longest line highlighted, in bytes.
    pub long_line: usize,
    /// How long one background slice runs before the thread looks at its queue again.
    pub slice: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            checkpoint_every: CHECKPOINT_EVERY,
            overscan: OVERSCAN,
            long_line: LONG_LINE,
            slice: Duration::from_millis(2),
        }
    }
}

/// How a text is kept on the highlighter thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Preview,
    /// A state per line, for the editable variant (scripts: about 3.5 MiB per 5,000 lines).
    Editable,
}

/// A text to open.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub text: Arc<str>,
    pub file_name: Option<String>,
    /// A grammar to use instead of detection: a token (`rs`, `bash`) or a grammar's name.
    pub syntax: Option<String>,
    pub mode: Mode,
}

impl Source {
    pub fn new(text: impl Into<Arc<str>>) -> Source {
        Source {
            text: text.into(),
            ..Source::default()
        }
    }

    pub fn file_name(mut self, name: impl Into<String>) -> Source {
        self.file_name = Some(name.into());
        self
    }

    pub fn syntax(mut self, syntax: impl Into<String>) -> Source {
        self.syntax = Some(syntax.into());
        self
    }

    pub fn mode(mut self, mode: Mode) -> Source {
        self.mode = mode;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineEdit {
    pub first: usize,
    pub removed: usize,
    pub inserted: usize,
}

impl LineEdit {
    pub fn diff(old: &str, new: &str) -> LineEdit {
        let a: Vec<&str> = old.split('\n').collect();
        let b: Vec<&str> = new.split('\n').collect();
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let room = a.len().min(b.len()) - prefix;
        let suffix = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(room)
            .take_while(|(x, y)| x == y)
            .count();
        LineEdit {
            first: prefix,
            removed: a.len() - prefix - suffix,
            inserted: b.len() - prefix - suffix,
        }
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub struct Highlighter {
    inner: Arc<Inner>,
}

struct Inner {
    opts: Options,
    theme: Mutex<Arc<Theme>>,
    waker: WakerSlot,
    thread: Mutex<Thread>,
}

#[derive(Default)]
struct Thread {
    /// Set once the thread has been started (or failed to start).
    started: bool,
    tx: Option<Sender<Request>>,
    handle: Option<JoinHandle<()>>,
}

impl Highlighter {
    pub fn new(theme: Arc<Theme>) -> Highlighter {
        Highlighter::with_options(theme, Options::default())
    }

    pub fn with_options(theme: Arc<Theme>, opts: Options) -> Highlighter {
        Highlighter {
            inner: Arc::new(Inner {
                opts,
                theme: Mutex::new(theme),
                waker: Arc::default(),
                thread: Mutex::default(),
            }),
        }
    }

    pub fn set_waker(&self, waker: Waker) {
        *lock(&self.inner.waker) = Some(waker);
    }

    pub fn set_theme(&self, theme: Arc<Theme>) {
        *lock(&self.inner.theme) = theme.clone();
        let tx = lock(&self.inner.thread).tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(Request::SetTheme(theme));
        }
    }

    pub fn options(&self) -> Options {
        self.inner.opts
    }

    fn sender(&self) -> Option<Sender<Request>> {
        let mut t = lock(&self.inner.thread);
        if !t.started {
            t.started = true;
            let (tx, rx) = mpsc::channel();
            let theme = lock(&self.inner.theme).clone();
            let (opts, waker) = (self.inner.opts, self.inner.waker.clone());
            // The worker is built on the thread: it holds parse states, which can't cross.
            let spawned = std::thread::Builder::new()
                .name("strata-highlight".into())
                .spawn(move || worker::run(rx, Worker::new(theme, opts, waker)));
            if let Ok(handle) = spawned {
                t.tx = Some(tx);
                t.handle = Some(handle);
            }
        }
        t.tx.clone()
    }

    /// Starts highlighting a text. Ask for lines with [`CodeDoc::show`].
    pub fn open(&self, src: Source) -> CodeDoc {
        CodeDoc::open(self.sender(), src, self.inner.opts)
    }

    pub fn blocks(&self) -> CodeBlocks {
        CodeBlocks::new(self.sender())
    }

    /// Ends the thread and waits for it, once every document and pool is dropped.
    #[cfg(test)]
    pub(crate) fn join(&self) -> bool {
        let handle = {
            let mut t = lock(&self.inner.thread);
            t.tx = None;
            t.handle.take()
        };
        handle.is_some_and(|h| h.join().is_ok())
    }
}

/// A line's colours as the UI side keeps them.
#[derive(Clone, Debug)]
struct Cached {
    tokens: LineTokens,
    provisional: Option<u64>,
}

const EDIT_HISTORY: usize = 256;

fn map_boundary(at: usize, edit: &LineEdit) -> usize {
    if at < edit.first {
        at
    } else if at >= edit.first + edit.removed {
        at - edit.removed + edit.inserted
    } else {
        edit.first
    }
}

pub struct CodeDoc {
    id: u64,
    tx: Option<Sender<Request>>,
    rx: Receiver<Reply>,
    lines: Lines,
    mode: Mode,
    overscan: usize,
    long_line: usize,
    cache: Vec<Option<Cached>>,
    cached: usize,
    palette: Vec<Option<Color>>,
    generation: u64,
    /// The last request sent, and the last one the thread finished with.
    seq: u64,
    idle_seq: u64,
    syntax: Option<String>,
    /// What [`Source`] said besides the text, for a retry after a failure.
    file_name: Option<String>,
    syntax_hint: Option<String>,
    /// The lines in view (with the overscan), as of the last `show`.
    view: Option<Range<usize>>,
    /// The last lines asked for, and the request that asked.
    last_show: Option<Range<usize>>,
    last_show_seq: u64,
    provisional_from: Option<usize>,
    history: Vec<(u64, LineEdit)>,
    failed: bool,
}

impl CodeDoc {
    /// Opens `src` on the thread behind `tx` (none: it stays plain).
    fn open(tx: Option<Sender<Request>>, src: Source, opts: Options) -> CodeDoc {
        let lines = Lines::new(src.text);
        let n = lines.len();
        let mut doc = CodeDoc {
            id: next_id(),
            tx,
            rx: mpsc::channel().1,
            lines,
            mode: src.mode,
            overscan: opts.overscan,
            long_line: opts.long_line,
            cache: vec![None; n],
            cached: 0,
            palette: Vec::new(),
            generation: 0,
            seq: 0,
            idle_seq: 0,
            syntax: None,
            file_name: src.file_name,
            syntax_hint: src.syntax,
            view: None,
            last_show: None,
            last_show_seq: 0,
            provisional_from: None,
            history: Vec::new(),
            failed: false,
        };
        doc.send_open();
        doc
    }

    /// Opens the current text on the thread, from scratch, with a fresh reply channel.
    fn send_open(&mut self) {
        let (reply, rx) = mpsc::channel();
        self.rx = rx;
        self.seq += 1;
        self.send(Request::Open {
            id: self.id,
            seq: self.seq,
            args: OpenArgs {
                text: self.lines.text.clone(),
                file_name: self.file_name.clone(),
                syntax: self.syntax_hint.clone(),
                mode: self.mode,
            },
            reply,
        });
    }

    fn send(&mut self, req: Request) {
        if let Some(tx) = &self.tx
            && tx.send(req).is_err()
        {
            // The thread is gone: the text stays plain.
            self.tx = None;
        }
    }

    /// The grammar detection picked, once the thread has answered.
    pub fn syntax(&self) -> Option<&str> {
        self.syntax.as_deref()
    }

    /// The text being highlighted.
    pub fn text(&self) -> &Arc<str> {
        &self.lines.text
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn show(&mut self, visible: Range<usize>) {
        let n = self.lines.len();
        let range = visible.start.min(n)..visible.end.saturating_add(self.overscan).min(n);
        if range.is_empty() || self.failed {
            return;
        }
        self.view = Some(range.clone());
        let missing = |c: &Option<Cached>| c.as_ref().is_none_or(|c| c.provisional.is_some());
        let first = range.clone().find(|&i| missing(&self.cache[i]));
        let Some(first) = first else { return };
        let last = range
            .clone()
            .rev()
            .find(|&i| missing(&self.cache[i]))
            .unwrap_or(first);
        let want = first..last + 1;
        if let Some(last) = &self.last_show {
            let within = last.start <= want.start && want.end <= last.end;
            if *last == want || (within && self.idle_seq < self.last_show_seq) {
                return;
            }
        }
        self.last_show = Some(want.clone());
        self.seq += 1;
        self.last_show_seq = self.seq;
        self.send(Request::Show {
            id: self.id,
            seq: self.seq,
            lines: want,
        });
    }

    /// Takes the replies that have landed. True when colours changed, so the host rebuilds.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(reply) = self.rx.try_recv() {
            changed |= self.apply(reply);
        }
        if changed {
            self.evict();
        }
        changed
    }

    fn apply(&mut self, reply: Reply) -> bool {
        match reply {
            Reply::Opened { syntax } => {
                self.syntax = Some(syntax);
                false
            }
            Reply::Palette { from, colors } => {
                self.palette.truncate(from);
                self.palette.extend(colors);
                self.cached > 0
            }
            Reply::Lines {
                generation,
                first,
                lines,
                exact,
            } => {
                if generation != self.generation {
                    return false;
                }
                for (i, tokens) in lines.into_iter().enumerate() {
                    if let Some(slot) = self.cache.get_mut(first + i) {
                        if slot.is_none() {
                            self.cached += 1;
                        }
                        *slot = Some(Cached {
                            tokens,
                            provisional: (!exact).then_some(generation),
                        });
                    }
                }
                true
            }
            Reply::Settled { generation, from } if generation == self.generation => {
                for (i, slot) in self.cache.iter_mut().enumerate() {
                    let Some(c) = slot.as_mut().filter(|c| c.provisional.is_some()) else {
                        continue;
                    };
                    if i >= from {
                        c.provisional = None;
                    } else {
                        *slot = None;
                        self.cached -= 1;
                    }
                }
                self.provisional_from = None;
                self.last_show = None;
                self.history.clear();
                true
            }
            Reply::Settled { generation, from } => self.settled_before(generation, from),
            Reply::Idle { seq } => {
                self.idle_seq = self.idle_seq.max(seq);
                false
            }
            Reply::Block { .. } => false,
            Reply::Failed => {
                self.failed = true;
                self.cache.iter_mut().for_each(|c| *c = None);
                self.cached = 0;
                self.provisional_from = None;
                self.history.clear();
                true
            }
        }
    }

    fn settled_before(&mut self, generation: u64, from: usize) -> bool {
        let first_kept = self.history.first().map(|(g, _)| *g);
        let from = if first_kept.is_some_and(|g| g <= generation + 1) {
            self.history
                .iter()
                .filter(|(g, _)| *g > generation)
                .fold(from, |at, (_, edit)| map_boundary(at, edit))
        } else {
            // The edits since have been forgotten: treat every line as stale.
            usize::MAX
        };
        let mut changed = false;
        for (i, slot) in self.cache.iter_mut().enumerate() {
            let Some(c) = slot.as_mut() else { continue };
            if !c.provisional.is_some_and(|g| g <= generation) {
                continue;
            }
            changed = true;
            if i < from {
                *slot = None;
                self.cached -= 1;
            } else {
                c.provisional = Some(generation + 1);
            }
        }
        self.history.retain(|(g, _)| *g > generation);
        if changed {
            self.last_show = None;
        }
        changed
    }

    /// Keeps a preview's cache near the viewport.
    fn evict(&mut self) {
        if self.mode == Mode::Editable || self.cached <= CACHED_LINES {
            return;
        }
        let Some(view) = self.view.clone() else {
            return;
        };
        let keep = view.start.saturating_sub(CACHED_LINES / 4)..view.end + CACHED_LINES / 4;
        let mut asked_dropped = false;
        for (i, slot) in self.cache.iter_mut().enumerate() {
            if !keep.contains(&i) && slot.take().is_some() {
                self.cached -= 1;
                asked_dropped |= self.last_show.as_ref().is_some_and(|r| r.contains(&i));
            }
        }
        if asked_dropped {
            // Lines of the last request are gone: asking for it again must not be skipped.
            self.last_show = None;
        }
    }

    pub fn line_spans(&self, line: usize) -> Option<Vec<Span>> {
        let cached = self.cache.get(line)?.as_ref()?;
        let mut runs = Vec::new();
        spans::colored(&cached.tokens, &self.palette, 0, &mut runs);
        Some(runs.into_iter().map(|(r, c)| spans::span(r, c)).collect())
    }

    /// Whether line `line`'s colours came from a state past an edit's frontier.
    pub fn is_provisional(&self, line: usize) -> bool {
        self.cache
            .get(line)
            .and_then(Option::as_ref)
            .is_some_and(|c| c.provisional.is_some())
    }

    pub fn text_spans(&self, lines: Range<usize>) -> Vec<Span> {
        let mut runs = Vec::new();
        for i in lines.start..lines.end.min(self.lines.len()) {
            // Rune's span offsets are `u32`: a text past 4 GiB draws plain from there.
            if self.lines.start(i) + self.long_line > u32::MAX as usize {
                break;
            }
            if let Some(Some(c)) = self.cache.get(i) {
                spans::colored(&c.tokens, &self.palette, self.lines.start(i), &mut runs);
            }
        }
        runs.into_iter().map(|(r, c)| spans::span(r, c)).collect()
    }

    pub fn edit(&mut self, text: Arc<str>, edit: LineEdit, visible: Range<usize>) {
        if self.failed {
            self.lines = Lines::new(text);
            self.cache = vec![None; self.lines.len()];
            self.cached = 0;
            self.generation = 0;
            self.view = None;
            self.last_show = None;
            self.provisional_from = None;
            self.history.clear();
            self.failed = false;
            self.idle_seq = 0;
            self.send_open();
            return;
        }
        let old_n = self.lines.len();
        self.lines = Lines::new(text.clone());
        let new_n = self.lines.len();
        self.generation += 1;
        self.last_show = None;
        let valid =
            edit.first + edit.removed <= old_n && old_n - edit.removed + edit.inserted == new_n;
        if self.mode == Mode::Preview || !valid {
            self.cache = vec![None; new_n];
            self.cached = 0;
            self.provisional_from = None;
            self.history.clear();
        } else {
            let removed = self
                .cache
                .splice(
                    edit.first..edit.first + edit.removed,
                    std::iter::repeat_n(None, edit.inserted),
                )
                .filter(Option::is_some)
                .count();
            self.cached -= removed;
            for c in self.cache[edit.first + edit.inserted..]
                .iter_mut()
                .flatten()
            {
                c.provisional.get_or_insert(self.generation);
            }
            if self.history.len() == EDIT_HISTORY {
                self.history.remove(0);
            }
            self.history.push((self.generation, edit));
            self.provisional_from = Some(
                self.provisional_from
                    .map_or(edit.first, |p| p.min(edit.first)),
            );
        }
        self.seq += 1;
        self.send(Request::Edit {
            id: self.id,
            seq: self.seq,
            generation: self.generation,
            text,
            edit,
            visible,
        });
    }

    pub fn provisional_from(&self) -> Option<usize> {
        self.provisional_from
    }

    pub fn is_idle(&self) -> bool {
        self.failed || self.idle_seq >= self.seq || self.tx.is_none()
    }

    pub fn settle(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll();
            if self.is_idle() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(reply) => {
                    self.apply(reply);
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    self.poll();
                    return self.is_idle();
                }
            }
        }
    }
}

impl Drop for CodeDoc {
    fn drop(&mut self) {
        self.send(Request::Close { id: self.id });
    }
}
