// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use rune::Color;
use syntect::highlighting::Theme;
use syntect::parsing::{ParseState, ScopeStack, ScopeStackOp, SyntaxReference, SyntaxSet};

use super::detect;
use super::palette::{Class, Interner};
use super::{LineEdit, Mode, Options};

/// A run of a line: its byte range within the line's content (no line break) and its class.
pub(crate) type Token = (u32, u32, Class);

pub(crate) type LineTokens = Box<[Token]>;

/// How often a time-bounded slice checks the clock, in lines.
const CLOCK_EVERY: usize = 16;

/// What the engine sends the UI side.
#[derive(Debug)]
pub(crate) enum Reply {
    /// The grammar detection picked.
    Opened {
        syntax: String,
    },
    /// Colours of classes `from..`; `from == 0` replaces the palette (a theme switch).
    Palette {
        from: usize,
        colors: Vec<Option<Color>>,
    },
    Lines {
        generation: u64,
        first: usize,
        lines: Vec<LineTokens>,
        exact: bool,
    },
    Settled {
        generation: u64,
        from: usize,
    },
    /// The document has no work left after request `seq`.
    Idle {
        seq: u64,
    },
    Block {
        key: u64,
        code: Arc<str>,
        tokens: Vec<Token>,
    },
    /// Highlighting failed (a grammar error or a panic): draw the text plain.
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct Lines {
    pub(crate) text: Arc<str>,
    starts: Vec<usize>,
}

impl Lines {
    pub(crate) fn new(text: Arc<str>) -> Lines {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Where line `i` starts in the text.
    pub(crate) fn start(&self, i: usize) -> usize {
        self.starts.get(i).copied().unwrap_or(self.text.len())
    }

    /// Line `i` without its `\n` or `\r\n`.
    pub(crate) fn content(&self, i: usize) -> &str {
        let start = self.start(i);
        let end = self.starts.get(i + 1).map_or(self.text.len(), |&e| e - 1);
        let line = self.text.get(start..end).unwrap_or("");
        line.strip_suffix('\r').unwrap_or(line)
    }
}

/// The parser's state at a line start, with the scope stack and its class.
#[derive(Clone, PartialEq)]
pub(crate) struct State {
    parse: ParseState,
    stack: ScopeStack,
    class: Class,
}

impl State {
    fn new(syntax: &SyntaxReference) -> State {
        State {
            parse: ParseState::new(syntax),
            stack: ScopeStack::new(),
            class: 0,
        }
    }
}

/// What a line parse needs besides its state.
#[derive(Clone, Copy)]
struct Ctx {
    set: &'static SyntaxSet,
    long_line: usize,
}

/// Grammar errors end highlighting for that text.
struct ParseFailed;

fn parse_line(
    ctx: Ctx,
    state: &mut State,
    interner: &mut Interner,
    content: &str,
    scratch: &mut String,
) -> Result<LineTokens, ParseFailed> {
    if content.len() > ctx.long_line {
        return Ok(Box::default());
    }
    scratch.clear();
    scratch.push_str(content);
    scratch.push('\n');
    let ops = state
        .parse
        .parse_line(scratch, ctx.set)
        .map_err(|_| ParseFailed)?;
    let end = content.len();
    let mut tokens: Vec<Token> = Vec::new();
    let mut last = 0;
    let emit = |from: usize, to: usize, class: Class, tokens: &mut Vec<Token>| {
        if to <= from {
            return;
        }
        let (from, to) = (from as u32, to as u32);
        match tokens.last_mut() {
            Some(t) if t.1 == from && t.2 == class => t.1 = to,
            _ => tokens.push((from, to, class)),
        }
    };
    for (pos, op) in &ops {
        let pos = (*pos).min(end);
        emit(last, pos, state.class, &mut tokens);
        last = last.max(pos);
        state.stack.apply(op).map_err(|_| ParseFailed)?;
        state.class = match op {
            ScopeStackOp::Push(scope) => interner.push(state.class, *scope, state.stack.as_slice()),
            ScopeStackOp::Pop(n) => (0..*n).fold(state.class, |c, _| interner.parent(c)),
            _ => interner.class_of(state.stack.as_slice()),
        };
    }
    emit(last, end, state.class, &mut tokens);
    Ok(tokens.into_boxed_slice())
}

/// One open text.
pub(crate) struct Doc {
    lines: Lines,
    syntax: &'static SyntaxReference,
    /// Checkpoint interval: 1 for an editable text (a state per line), else S4's 64.
    every: usize,
    states: Vec<State>,
    /// The pass has parsed lines `..exact` in order.
    exact: usize,
    /// The pass's state at line `exact` when `every > 1`.
    cursor: Option<State>,
    dirty_until: usize,
    /// True between an edit and the end of its background pass.
    provisional: bool,
    interner: Interner,
    /// Classes whose colours the UI has.
    sent: usize,
    generation: u64,
    /// Lines of the UI's window the pass still owes it.
    pending: Option<Range<usize>>,
    /// A show request not yet served.
    show: Option<Range<usize>>,
    /// An edit's foreground target, not yet run.
    foreground: Option<Range<usize>>,
    seq: u64,
    idle_owed: bool,
    failed: bool,
    outbox: Vec<Reply>,
    scratch: String,
    /// Lines parsed, for tests.
    pub(crate) parsed: u64,
}

impl Doc {
    fn new(lines: Lines, syntax: &'static SyntaxReference, every: usize) -> Doc {
        let initial = State::new(syntax);
        Doc {
            lines,
            syntax,
            every: every.max(1),
            cursor: (every > 1).then(|| initial.clone()),
            states: vec![initial],
            exact: 0,
            dirty_until: 0,
            provisional: false,
            interner: Interner::new(),
            sent: 0,
            generation: 0,
            pending: None,
            show: None,
            foreground: None,
            seq: 0,
            idle_owed: true,
            failed: false,
            outbox: Vec::new(),
            scratch: String::new(),
            parsed: 0,
        }
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn pass_active(&self) -> bool {
        !self.failed && self.exact < self.line_count()
    }

    fn has_work(&self) -> bool {
        !self.failed && (self.foreground.is_some() || self.show.is_some() || self.pass_active())
    }

    /// Queues `tokens` for line `line` in the reply outbox, joining a run of lines.
    fn emit(&mut self, line: usize, tokens: LineTokens, exact: bool) {
        let generation = self.generation;
        if let Some(Reply::Lines {
            generation: g,
            first,
            lines,
            exact: e,
        }) = self.outbox.last_mut()
            && *g == generation
            && *e == exact
            && *first + lines.len() == line
        {
            lines.push(tokens);
            return;
        }
        self.outbox.push(Reply::Lines {
            generation,
            first: line,
            lines: vec![tokens],
            exact,
        });
    }

    fn wanted(&self, line: usize) -> bool {
        self.pending.as_ref().is_some_and(|p| p.contains(&line))
    }

    fn pass_step(&mut self, ctx: Ctx) -> Result<(), ParseFailed> {
        let line = self.exact;
        let mut state = match self.cursor.take() {
            Some(cursor) => cursor,
            None => self.states[line].clone(),
        };
        let tokens = parse_line(
            ctx,
            &mut state,
            &mut self.interner,
            self.lines.content(line),
            &mut self.scratch,
        )?;
        self.parsed += 1;
        if self.wanted(line) {
            self.emit(line, tokens, true);
        }
        let next = line + 1;
        self.exact = next;
        if self.every > 1 {
            if next.is_multiple_of(self.every) {
                self.states.push(state.clone());
            }
            self.cursor = Some(state);
        } else if next < self.states.len() {
            if next >= self.dirty_until && self.states[next] == state {
                self.exact = self.states.len() - 1;
                self.settle(next);
            } else {
                self.states[next] = state;
            }
        } else {
            self.states.push(state);
        }
        if self.provisional && self.exact >= self.line_count() {
            self.settle(self.line_count());
        }
        Ok(())
    }

    fn settle(&mut self, from: usize) {
        if self.provisional {
            self.provisional = false;
            self.dirty_until = 0;
            self.outbox.push(Reply::Settled {
                generation: self.generation,
                from,
            });
        }
    }

    /// Runs the pass until it reaches `until`, or the clock passes `deadline`.
    fn run_pass(
        &mut self,
        ctx: Ctx,
        until: usize,
        deadline: Option<Instant>,
    ) -> Result<(), ParseFailed> {
        let mut n = 0usize;
        while self.exact < until.min(self.line_count()) {
            self.pass_step(ctx)?;
            n += 1;
            if n.is_multiple_of(CLOCK_EVERY) && deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
        }
        Ok(())
    }

    /// Parses `range` from a state, emitting every line, without touching the cache.
    fn emit_from(
        &mut self,
        ctx: Ctx,
        mut state: State,
        range: Range<usize>,
        exact: bool,
    ) -> Result<(), ParseFailed> {
        if !exact {
            self.dirty_until = self.dirty_until.max(range.end);
        }
        for line in range {
            let tokens = parse_line(
                ctx,
                &mut state,
                &mut self.interner,
                self.lines.content(line),
                &mut self.scratch,
            )?;
            self.parsed += 1;
            self.emit(line, tokens, exact);
        }
        Ok(())
    }

    /// Serves a viewport: lines `range` and, for free, those from its checkpoint down to it.
    fn serve_show(&mut self, ctx: Ctx, range: Range<usize>) -> Result<(), ParseFailed> {
        if self.every > 1 {
            let k = range.start / self.every;
            match self.states.get(k) {
                // Parsing on from an exact checkpoint is exact, even past the pass.
                Some(state) => {
                    let state = state.clone();
                    self.pending = None;
                    self.emit_from(ctx, state, k * self.every..range.end, true)
                }
                // The pass hasn't built the checkpoint yet: it sends the lines on arrival.
                None => {
                    self.pending = Some(range);
                    Ok(())
                }
            }
        } else if range.start <= self.exact {
            // Lines the pass has passed come from their own exact states; it parses the rest.
            let done = range.end.min(self.exact);
            if range.start < done {
                let state = self.states[range.start].clone();
                self.emit_from(ctx, state, range.start..done, true)?;
            }
            self.pending = Some(done..range.end);
            self.run_pass(ctx, range.end, None)
        } else if let Some(state) = self.states.get(range.start) {
            let state = state.clone();
            self.pending = Some(range.clone());
            self.emit_from(ctx, state, range, false)
        } else {
            self.pending = Some(range);
            Ok(())
        }
    }

    fn apply_edit(
        &mut self,
        text: Arc<str>,
        edit: LineEdit,
        generation: u64,
        visible: Range<usize>,
        overscan: usize,
    ) {
        let old_n = self.line_count();
        let lines = Lines::new(text);
        let new_n = lines.len();
        self.generation = generation;
        self.lines = lines;
        let valid =
            edit.first + edit.removed <= old_n && old_n - edit.removed + edit.inserted == new_n;
        if self.every > 1 || !valid {
            let initial = State::new(self.syntax);
            self.cursor = (self.every > 1).then(|| initial.clone());
            self.states = vec![initial];
            self.exact = 0;
            self.dirty_until = 0;
            self.provisional = false;
            self.pending = Some(window(visible, overscan, new_n));
            self.show = None;
            self.foreground = None;
            return;
        }
        let LineEdit {
            first,
            removed,
            inserted,
        } = normalize(edit, old_n);
        if self.provisional {
            self.dirty_until = self.dirty_until.max(self.exact + 1);
        }
        if first < self.states.len() {
            let interior =
                (first + 1).min(self.states.len())..(first + removed).min(self.states.len());
            let placeholder = self.states[first].clone();
            self.states.splice(
                interior,
                std::iter::repeat_n(placeholder, inserted.saturating_sub(1)),
            );
        }
        let shift = |l: usize| {
            if l > first + removed {
                l + inserted - removed
            } else {
                l.min(first + inserted)
            }
        };
        self.dirty_until = shift(self.dirty_until).max(first + inserted);
        self.exact = self.exact.min(first);
        self.provisional = true;
        let win = window(visible, overscan, new_n);
        let target = (first + inserted).max(win.end).min(new_n);
        self.pending = Some(win.start.min(first)..target);
        self.show = None;
        self.foreground = Some(first..target);
    }

    fn run_foreground(&mut self, ctx: Ctx) -> Result<(), ParseFailed> {
        let Some(target) = self.foreground.take() else {
            return Ok(());
        };
        if self.exact >= target.start {
            return self.run_pass(ctx, target.end, None);
        }
        match self.states.get(target.start) {
            Some(state) => {
                let state = state.clone();
                self.emit_from(ctx, state, target, false)
            }
            None => Ok(()),
        }
    }

    fn take_outbox(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        if self.interner.len() > self.sent
            && self.outbox.iter().any(|r| matches!(r, Reply::Lines { .. }))
        {
            // Colours for new classes always land before the lines that use them.
            out.push(Reply::Palette {
                from: self.sent,
                colors: Vec::new(),
            });
        }
        out.append(&mut self.outbox);
        out
    }
}

/// The viewport plus the overscan below it, within the text.
fn window(visible: Range<usize>, overscan: usize, n: usize) -> Range<usize> {
    visible.start.min(n)..visible.end.saturating_add(overscan).min(n)
}

fn normalize(edit: LineEdit, old_n: usize) -> LineEdit {
    let mut e = edit;
    if e.removed == 0 || e.inserted == 0 {
        if e.first + e.removed < old_n {
            e.removed += 1;
            e.inserted += 1;
        } else if e.first > 0 {
            e.first -= 1;
            e.removed += 1;
            e.inserted += 1;
        }
    }
    e
}

/// A Markdown adapter's block pool: one interner for all its blocks.
struct Pool {
    interner: Interner,
    sent: usize,
    outbox: Vec<Reply>,
}

/// A code block being highlighted, in slices.
struct BlockJob {
    pool: u64,
    key: u64,
    lines: Lines,
    state: State,
    line: usize,
    tokens: Vec<Token>,
}

/// A request to open a text.
pub(crate) struct OpenArgs {
    pub(crate) text: Arc<str>,
    pub(crate) file_name: Option<String>,
    pub(crate) syntax: Option<String>,
    pub(crate) mode: Mode,
}

/// The highlighter thread's state: every document, block pool and block job.
pub(crate) struct Engine {
    /// `None` when the grammar set failed to load: everything answers plain at once.
    set: Option<&'static SyntaxSet>,
    theme: Arc<Theme>,
    opts: Options,
    docs: HashMap<u64, Doc>,
    pools: HashMap<u64, Pool>,
    blocks: VecDeque<BlockJob>,
    /// Replies for each document or pool, in order.
    out: Vec<(u64, Reply)>,
    /// The document the next background slice starts looking from.
    turn: usize,
}

impl Engine {
    pub(crate) fn new(theme: Arc<Theme>, opts: Options) -> Engine {
        Engine::with_syntaxes(theme, opts, detect::syntaxes())
    }

    pub(crate) fn with_syntaxes(
        theme: Arc<Theme>,
        opts: Options,
        set: Option<&'static SyntaxSet>,
    ) -> Engine {
        Engine {
            set,
            theme,
            opts,
            docs: HashMap::new(),
            pools: HashMap::new(),
            blocks: VecDeque::new(),
            out: Vec::new(),
            turn: 0,
        }
    }

    fn ctx(&self) -> Option<Ctx> {
        Some(Ctx {
            set: self.set?,
            long_line: self.opts.long_line,
        })
    }

    /// Opens a text, replacing any open under `id` (a retry after a failure).
    pub(crate) fn open(&mut self, id: u64, seq: u64, args: OpenArgs) {
        let detected = self.set.and_then(|set| {
            catch_unwind(AssertUnwindSafe(|| {
                detect::detect(
                    set,
                    args.syntax.as_deref(),
                    args.file_name.as_deref(),
                    &args.text,
                )
            }))
            .ok()
        });
        let Some(syntax) = detected else {
            self.docs.remove(&id);
            self.out.push((id, Reply::Failed));
            self.out.push((id, Reply::Idle { seq }));
            return;
        };
        let every = match args.mode {
            Mode::Editable => 1,
            Mode::Preview => self.opts.checkpoint_every,
        };
        let mut doc = Doc::new(Lines::new(args.text), syntax, every);
        doc.seq = seq;
        doc.outbox.push(Reply::Opened {
            syntax: syntax.name.clone(),
        });
        self.docs.insert(id, doc);
    }

    pub(crate) fn show(&mut self, id: u64, seq: u64, lines: Range<usize>) {
        if let Some(doc) = self.docs.get_mut(&id) {
            let n = doc.line_count();
            doc.show = Some(lines.start.min(n)..lines.end.min(n));
            doc.seq = seq;
            doc.idle_owed = true;
        }
    }

    pub(crate) fn edit(
        &mut self,
        id: u64,
        seq: u64,
        generation: u64,
        text: Arc<str>,
        edit: LineEdit,
        visible: Range<usize>,
    ) {
        let overscan = self.opts.overscan;
        if let Some(doc) = self.docs.get_mut(&id) {
            doc.apply_edit(text, edit, generation, visible, overscan);
            doc.seq = seq;
            doc.idle_owed = true;
        }
    }

    pub(crate) fn close(&mut self, id: u64) {
        self.docs.remove(&id);
        if self.pools.remove(&id).is_some() {
            self.blocks.retain(|b| b.pool != id);
        }
    }

    pub(crate) fn open_pool(&mut self, id: u64) {
        self.pools.insert(
            id,
            Pool {
                interner: Interner::new(),
                sent: 0,
                outbox: Vec::new(),
            },
        );
    }

    /// Queues a code block. A block with no grammar for its language answers plain at once.
    pub(crate) fn block(&mut self, pool: u64, key: u64, lang: &str, code: Arc<str>) {
        let Some(p) = self.pools.get_mut(&pool) else {
            return;
        };
        let syntax = self
            .set
            .and_then(|set| catch_unwind(|| detect::by_token(set, lang)).ok().flatten());
        match syntax {
            Some(syntax) => self.blocks.push_back(BlockJob {
                pool,
                key,
                lines: Lines::new(code),
                state: State::new(syntax),
                line: 0,
                tokens: Vec::new(),
            }),
            None => p.outbox.push(Reply::Block {
                key,
                code,
                tokens: Vec::new(),
            }),
        }
    }

    pub(crate) fn forget(&mut self, pool: u64, keys: &[u64]) {
        let keys: HashSet<u64> = keys.iter().copied().collect();
        self.blocks
            .retain(|b| b.pool != pool || !keys.contains(&b.key));
    }

    /// Re-colours every document and pool from its classes, with no re-parse.
    pub(crate) fn set_theme(&mut self, theme: Arc<Theme>) {
        self.theme = theme;
        for doc in self.docs.values_mut() {
            doc.sent = 0;
            doc.outbox.push(Reply::Palette {
                from: 0,
                colors: Vec::new(),
            });
        }
        for pool in self.pools.values_mut() {
            pool.sent = 0;
            pool.outbox.push(Reply::Palette {
                from: 0,
                colors: Vec::new(),
            });
        }
    }

    pub(crate) fn has_work(&self) -> bool {
        !self.blocks.is_empty() || self.docs.values().any(Doc::has_work)
    }

    pub(crate) fn step(&mut self) -> bool {
        let Some(ctx) = self.ctx() else {
            // No grammars: open answered `Failed`, blocks answered plain; nothing to do.
            return false;
        };
        let deadline = Some(Instant::now() + self.opts.slice);
        let mut ids: Vec<u64> = self.docs.keys().copied().collect();
        ids.sort_unstable();
        let did = if let Some(&id) = ids.iter().find(|id| self.docs[id].foreground.is_some()) {
            self.guarded(id, |d| d.run_foreground(ctx))
        } else if let Some(&id) = ids.iter().find(|id| self.docs[id].show.is_some()) {
            self.guarded(id, |d| match d.show.take() {
                Some(range) => d.serve_show(ctx, range),
                None => Ok(()),
            })
        } else if !self.blocks.is_empty() {
            self.block_slice(ctx, deadline);
            true
        } else if let Some(id) = self.next_pass(&ids) {
            self.guarded(id, |d| d.run_pass(ctx, usize::MAX, deadline))
        } else {
            false
        };
        self.collect();
        did
    }

    /// The next document with a background pass, taking turns.
    fn next_pass(&mut self, ids: &[u64]) -> Option<u64> {
        let n = ids.len();
        let found = (0..n)
            .map(|i| ids[(self.turn + i) % n])
            .find(|id| self.docs[id].pass_active())?;
        self.turn = self.turn.wrapping_add(1);
        Some(found)
    }

    /// Runs `f` on a document. A grammar error or a panic ends highlighting for it.
    fn guarded(&mut self, id: u64, f: impl FnOnce(&mut Doc) -> Result<(), ParseFailed>) -> bool {
        let Some(doc) = self.docs.get_mut(&id) else {
            return false;
        };
        doc.idle_owed = true;
        let seq = doc.seq;
        let ok = matches!(catch_unwind(AssertUnwindSafe(|| f(doc))), Ok(Ok(())));
        if !ok {
            // The state may be half-updated: drop it, and let the text draw plain.
            self.docs.remove(&id);
            self.out.push((id, Reply::Failed));
            self.out.push((id, Reply::Idle { seq }));
        }
        true
    }

    fn block_slice(&mut self, ctx: Ctx, deadline: Option<Instant>) {
        let Some(mut job) = self.blocks.pop_front() else {
            return;
        };
        let Some(pool) = self.pools.get_mut(&job.pool) else {
            return;
        };
        let mut scratch = String::new();
        let mut n = 0usize;
        // A grammar error or a panic leaves the block plain; the rest go on.
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            while job.line < job.lines.len() {
                let line = job.line;
                let offset = job.lines.start(line) as u32;
                let tokens = parse_line(
                    ctx,
                    &mut job.state,
                    &mut pool.interner,
                    job.lines.content(line),
                    &mut scratch,
                )?;
                job.tokens
                    .extend(tokens.iter().map(|&(a, b, c)| (a + offset, b + offset, c)));
                job.line += 1;
                n += 1;
                if n.is_multiple_of(CLOCK_EVERY) && deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
            }
            Ok::<(), ParseFailed>(())
        }));
        let failed = !matches!(parsed, Ok(Ok(())));
        if failed {
            job.tokens.clear();
        } else if job.line < job.lines.len() {
            self.blocks.push_front(job);
            return;
        }
        pool.outbox.push(Reply::Block {
            key: job.key,
            code: job.lines.text,
            tokens: job.tokens,
        });
    }

    fn collect(&mut self) {
        let theme = &self.theme;
        let mut ids: Vec<u64> = self.docs.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(doc) = self.docs.get_mut(&id) else {
                continue;
            };
            for reply in doc.take_outbox() {
                let reply = match reply {
                    Reply::Palette { from, .. } => {
                        let from = from.min(doc.sent);
                        let colors = doc.interner.resolve_from(theme, from);
                        doc.sent = doc.interner.len();
                        Reply::Palette { from, colors }
                    }
                    other => other,
                };
                self.out.push((id, reply));
            }
            if doc.idle_owed && !doc.has_work() {
                doc.idle_owed = false;
                self.out.push((id, Reply::Idle { seq: doc.seq }));
            }
        }
        let mut pools: Vec<u64> = self.pools.keys().copied().collect();
        pools.sort_unstable();
        for id in pools {
            let Some(pool) = self.pools.get_mut(&id) else {
                continue;
            };
            if pool.outbox.is_empty() {
                continue;
            }
            if pool.interner.len() > pool.sent
                || pool
                    .outbox
                    .iter()
                    .any(|r| matches!(r, Reply::Palette { .. }))
            {
                let from = if pool
                    .outbox
                    .iter()
                    .any(|r| matches!(r, Reply::Palette { .. }))
                {
                    0
                } else {
                    pool.sent
                };
                let colors = pool.interner.resolve_from(theme, from);
                pool.sent = pool.interner.len();
                self.out.push((id, Reply::Palette { from, colors }));
            }
            for reply in pool.outbox.drain(..) {
                if !matches!(reply, Reply::Palette { .. }) {
                    self.out.push((id, reply));
                }
            }
        }
    }

    pub(crate) fn fail_all(&mut self) {
        let mut ids: Vec<(u64, u64)> = self.docs.iter().map(|(&id, d)| (id, d.seq)).collect();
        ids.sort_unstable();
        for (id, seq) in ids {
            self.out.push((id, Reply::Failed));
            self.out.push((id, Reply::Idle { seq }));
        }
        self.docs.clear();
        for job in self.blocks.drain(..) {
            self.out.push((
                job.pool,
                Reply::Block {
                    key: job.key,
                    code: job.lines.text,
                    tokens: Vec::new(),
                },
            ));
        }
        for (&id, pool) in &mut self.pools {
            for reply in pool.outbox.drain(..) {
                if let Reply::Block { key, code, .. } = reply {
                    let tokens = Vec::new();
                    self.out.push((id, Reply::Block { key, code, tokens }));
                }
            }
        }
    }

    /// The replies collected so far, without collecting more (after [`Engine::fail_all`]).
    pub(crate) fn take_raw(&mut self) -> Vec<(u64, Reply)> {
        std::mem::take(&mut self.out)
    }

    /// Takes the replies collected so far, after a request or a step.
    pub(crate) fn take_out(&mut self) -> Vec<(u64, Reply)> {
        self.collect();
        std::mem::take(&mut self.out)
    }

    /// The keys of `pool`'s block jobs still queued.
    #[cfg(test)]
    pub(crate) fn queued_blocks(&self, pool: u64) -> Vec<u64> {
        self.blocks
            .iter()
            .filter(|b| b.pool == pool)
            .map(|b| b.key)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn doc(&self, id: u64) -> Option<&Doc> {
        self.docs.get(&id)
    }
}
