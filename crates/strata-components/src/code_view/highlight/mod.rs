//! The syntect highlighter on its own thread, with checkpoints and colour spans (C10a).
//!
//! Work package: C10a, `docs/work/phase-1-leaves/C10a-highlighter.md`. Backend and numbers:
//! `docs/decisions/S4-syntect-backend.md`.
//!
//! One [`Highlighter`] serves the app. It colours whole texts ([`CodeDoc`], for C10b's source
//! preview and its editable variant) and code blocks ([`CodeBlocks`], for rune-kit's
//! `Markdown` hook and C11). Text arrives as data; nothing here reads files.
//!
//! # The thread
//!
//! syntect runs on `onig`, whose `ParseState` is neither `Send` nor `Sync` (S4, B1). So one
//! thread, started on first use, owns every parse state, and the UI side gets only colour
//! data back. Requests and replies go over channels, and the thread wakes the window with the
//! `Waker` from [`Highlighter::set_waker`]; the host calls each document's `poll` from its
//! own. The grammar set (two-face's `extra_newlines`) loads once, lazily, on that thread. The
//! thread exits when the last handle, document and block pool drop.
//!
//! # How a text is coloured
//!
//! - **Detection:** an explicit grammar, else the file name and extension, else the first
//!   line (shebangs, and the modelines grammars declare), else a Vim or Emacs modeline, else
//!   plain text.
//! - **Checkpoints:** a preview keeps the parse state every [`CHECKPOINT_EVERY`] lines,
//!   built by a background pass after open. A viewport resumes from the nearest one, so a
//!   jump to the end never parses the file on the UI thread; lines draw plain until their
//!   colours arrive, and highlighting changes only colour, so nothing moves.
//! - **Colours:** each token's colour depends only on its scope stack, so stacks are kept as
//!   classes and a theme turns classes into a palette. [`Highlighter::set_theme`] re-colours
//!   every open text without re-parsing it.
//! - **Edits** ([`Mode::Editable`], a state per line; S4's workaround 1): an edit re-parses
//!   in the foreground from its line to the bottom of the viewport plus [`OVERSCAN`], then a
//!   background pass carries on until the parse state equals its cached one again or the
//!   text ends. With `onig` an edit inside a heredoc or a fenced block runs to that block's
//!   end (S4, B2). Until the pass ends, lines past it are provisional
//!   ([`CodeDoc::provisional_from`]); once it ends every line is exact.
//! - **Long lines:** a line longer than [`LONG_LINE`] bytes draws plain, and the parse state
//!   carries over it.
//!
//! # Perf tests
//!
//! The perf tests on the S4 fixtures are `#[ignore]`d. Run them with the workspace's `perf`
//! profile (release, no LTO), one at a time:
//!
//! ```sh
//! CARGO_BUILD_JOBS=8 cargo test --profile perf -p strata-components --lib \
//!     code_view::highlight::tests::perf -- --ignored --test-threads=1 --nocapture
//! ```

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
pub use spans::MAX_SPANS;

use engine::{LineTokens, Lines, OpenArgs, Reply};
use worker::{Request, WakerSlot, Worker};

/// The checkpoint interval for a preview, in lines (S4's decision: the 8 ms reading at p95).
pub const CHECKPOINT_EVERY: usize = 64;

/// Lines highlighted below the viewport (S4's viewport: 80 visible lines plus 40).
pub const OVERSCAN: usize = 40;

/// Lines longer than this, in bytes, draw plain (S4: a 100 KB line costs 64–181 ms).
pub const LONG_LINE: usize = 16 * 1024;

/// A preview's UI side keeps about this many lines' colours, and drops those farthest from
/// the viewport beyond it. An editable text keeps them all.
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
    /// Read-only: a checkpoint every [`Options::checkpoint_every`] lines. An edit
    /// re-highlights it from the start.
    #[default]
    Preview,
    /// A state per line, for the editable variant (scripts: about 3.5 MiB per 5,000 lines).
    Editable,
}

/// A text to open.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub text: Arc<str>,
    /// The file name, for detection by name and extension (`PKGBUILD`, `main.rs`). A path
    /// works too; only its last part is read.
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

/// Which lines an edit changed: lines `first..first + removed` of the old text became lines
/// `first..first + inserted` of the new one. A change inside one line is
/// `{ first: line, removed: 1, inserted: 1 }`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineEdit {
    pub first: usize,
    pub removed: usize,
    pub inserted: usize,
}

impl LineEdit {
    /// The smallest edit that turns `old` into `new`: the lines between their common first
    /// and last lines.
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

/// Locks `m`, poisoned or not: a panic elsewhere must not stop highlighting (the data behind
/// these locks is replaced whole, never left half-written).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The app's highlighter: a handle to the thread that owns every parse state. Cloning is
/// cheap and shares the thread. Creating one starts nothing; the thread starts on the first
/// [`open`](Self::open) or block request.
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
    /// A highlighter colouring with `theme`: strata-kit's `theme::syntax::syntect_theme` for the
    /// look.
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

    /// The window's waker (`AppSpec::setup`'s): the thread calls it after sending replies,
    /// so the host's `poll` runs and calls each document's `poll`.
    pub fn set_waker(&self, waker: Waker) {
        *lock(&self.inner.waker) = Some(waker);
    }

    /// Colours every open text and block with `theme` from now on, re-colouring cached lines
    /// from their classes, with no re-parse.
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

    /// The thread's queue, starting the thread on first use. `None` when it couldn't start:
    /// texts then draw plain.
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

    /// A pool of code blocks, for rune-kit's `Markdown` hook and C11's own blocks.
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
    /// `Some(g)` while the colours may still change: since edit generation `g` marked them
    /// provisional, or they came from a state past that generation's frontier.
    provisional: Option<u64>,
}

/// The most edits a [`CodeDoc`] remembers for mapping an older generation's `Settled` onto
/// today's lines. Past it, such a `Settled` drops every line it could concern.
const EDIT_HISTORY: usize = 256;

/// Where line boundary `at` of the text before `edit` is in the text after it. A boundary
/// inside the replaced lines maps to the start of the edit.
fn map_boundary(at: usize, edit: &LineEdit) -> usize {
    if at < edit.first {
        at
    } else if at >= edit.first + edit.removed {
        at - edit.removed + edit.inserted
    } else {
        edit.first
    }
}

/// One text's highlighting, on the UI side: the colours that have landed, by line.
///
/// The host keeps it in its state and calls [`show`](Self::show) with the lines in view,
/// [`poll`](Self::poll) from its own poll, and reads [`line_spans`](Self::line_spans) (one
/// rich text per line, C10b) or [`text_spans`](Self::text_spans) (a TextArea) when it
/// builds. Dropping it closes the text on the thread.
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
    /// The edits since the last `Settled` of the current generation, by generation, to map
    /// an older `Settled` onto today's lines.
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

    /// Asks for the colours of `visible` and [`Options::overscan`] lines below it. Call it
    /// whenever the viewport moves; it asks only for lines it lacks, and the newest request
    /// wins.
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
            // Asked for already; part may have landed while the rest is on its way (a pass
            // crossing the view sends its lines as it goes).
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
                // The pass ended: provisional lines from `from` on are exact, and those
                // before it that weren't sent again are stale.
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

    /// A pass of an older generation ended, but the host only sees it after a newer edit.
    /// It still drops the lines that pass left stale, which no later reply would drop. It
    /// marks nothing exact: a newer edit's pass is running, and its own `Settled` decides.
    ///
    /// Only lines provisional since `generation` or before are its business: a line made
    /// provisional by a later edit, or by a later pass's provisional colours, was exact or
    /// sent again by then. `from` is in that generation's line numbers, so it is carried
    /// through the edits since. Lines at or past it were exact for that generation, so they
    /// stay, provisional as of the next edit, as if they had been marked exact and then
    /// provisional again. This keeps the lines in view after a quick edit, where dropping
    /// every provisional line would draw them plain for a frame.
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

    /// Line `line`'s colours as colour-only spans, with byte offsets into the line. `None`
    /// until its colours land: draw it plain meanwhile.
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

    /// Colour-only spans for the lines in `lines`, with byte offsets into the whole text, for
    /// a TextArea (`TextArea::spans`). At most [`MAX_SPANS`].
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
            if runs.len() >= MAX_SPANS {
                runs.truncate(MAX_SPANS);
                break;
            }
        }
        runs.into_iter().map(|(r, c)| spans::span(r, c)).collect()
    }

    /// The text changed: `text` is the new text and `edit` the lines that changed
    /// ([`LineEdit::diff`] finds them). `visible` is the viewport in the new text.
    ///
    /// An editable text re-parses in the foreground down to the viewport's bottom plus the
    /// overscan, then in the background; cached lines after the edit keep their colours,
    /// marked provisional, until the pass reaches them. The foreground has no deadline: an
    /// edit far above the viewport (a scripted or programmatic one) parses every line from
    /// the edit to the viewport's bottom before the viewport's colours come back. A preview
    /// starts over. A text whose highlighting failed is opened again, from scratch.
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
            // Replies to the failed open arrive on the old channel, which `send_open`
            // replaces; the new open starts counting idleness afresh.
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

    /// The first line whose cached colours may still change after an edit, or `None` once
    /// the background pass has ended and every line is exact.
    pub fn provisional_from(&self) -> Option<usize> {
        self.provisional_from
    }

    /// Whether the thread has finished every request sent so far. A text that failed, or has
    /// no thread, is idle: it stays plain.
    pub fn is_idle(&self) -> bool {
        self.failed || self.idle_seq >= self.seq || self.tx.is_none()
    }

    /// Blocks until the thread has finished every request sent so far (its background passes
    /// too), or `timeout` passes. True when it finished. For tests and headless snapshots
    /// only: never call it while building or polling a window.
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
