//! The Jobs dashboard: a footer button with the jobs' count that opens their list above it
//! (Strata's `src/ui/jobs.rs`). It slots into C19's `Footer::jobs`. Jobs arrive as rows; what
//! the buttons do (cancel, remove, dismiss, clear) is the host's, as intents.

mod text;

use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use rune::{El, ScrollSpec, TextStyle, Tx, col, el, icon, row};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Placement, Progress, popover};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

pub use text::{JobsWords, StatusPhrase, indicator, per_item_fraction, strip_terminal_sequences};

/// A job's id, the host's.
pub type JobId = u64;

/// The most rows the list shows (Strata's `MAX_ROWS`); a note counts the rest.
pub const MAX_ROWS: usize = 12;

/// The list's width and tallest (`width_request(500)`, `max_content_height(420)`).
pub const LIST_WIDTH: f32 = 500.0;
pub const LIST_MAX_HEIGHT: f32 = 420.0;

/// The tallest a job's output gets before it scrolls (`max_content_height(160)`).
pub const OUTPUT_MAX_HEIGHT: f32 = 160.0;

/// The footer button's look: C19's F1 button (`footer::BUTTON_*`), copied until C19 shares its
/// button (a recorded follow-up).
pub const BUTTON_RADIUS: f32 = 4.0;
pub const BUTTON_HOVER_ALPHA: f32 = 0.15;
pub const BUTTON_CHECKED_ALPHA: f32 = 0.22;
pub const BUTTON_RING: f32 = 2.0;

/// The icon bezel's side and radius before roundness (`.job-icon-bezel`).
const BEZEL: f32 = 28.0;
const BEZEL_RADIUS: f32 = 7.0;

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// Running or cancelling: what the footer counts as running.
    pub fn is_running(self) -> bool {
        matches!(self, JobStatus::Running | JobStatus::Cancelling)
    }

    pub fn is_active(self) -> bool {
        self.is_running() || self == JobStatus::Queued
    }

    pub fn is_finished(self) -> bool {
        matches!(
            self,
            JobStatus::Succeeded | JobStatus::Failed | JobStatus::Cancelled
        )
    }
}

/// One job, as the host's job service reports it. Rows come newest first, as Strata lists
/// them (a new job first).
#[derive(Clone, Debug, PartialEq)]
pub struct JobRow {
    pub id: JobId,
    pub name: String,
    pub icon: IconName,
    pub status: JobStatus,
    /// When it started, on the host's clock (the clock the host's poll passes to
    /// [`JobsState::tick`]).
    pub started_at: f64,
    /// When it ended, once finished.
    pub ended_at: Option<f64>,
    /// The job's latest progress message.
    pub message: Option<String>,
    /// Items done of the total (a per-item job, or a script's reported total).
    pub items: Option<(usize, usize)>,
    pub failed: usize,
    /// The invoking folder as Strata writes it ("~/Pictures").
    pub folder: String,
    /// The bar's fraction; `None` pulses it (no total reported). See [`per_item_fraction`].
    pub fraction: Option<f32>,
    /// Captured output; terminal sequences are stripped when shown.
    pub output: String,
    /// Older output was dropped.
    pub output_truncated: bool,
    /// The failure's message, shown before the output.
    pub error: Option<String>,
}

impl JobRow {
    pub fn new(id: JobId, name: impl Into<String>, status: JobStatus, started_at: f64) -> Self {
        JobRow {
            id,
            name: name.into(),
            icon: IconName::Terminal,
            status,
            started_at,
            ended_at: None,
            message: None,
            items: None,
            failed: 0,
            folder: String::new(),
            fraction: None,
            output: String::new(),
            output_truncated: false,
            error: None,
        }
    }
}

/// What the list asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobsIntent {
    /// Cancel a running job.
    Cancel(JobId),
    /// Remove a queued job before it starts.
    Remove(JobId),
    /// Drop one finished job.
    Dismiss(JobId),
    /// Drop every finished job; active ones stay.
    ClearFinished,
}

/// The dashboard's own state: open or not, which rows show their output, and the clock the
/// elapsed times read. The host holds it.
#[derive(Clone, Debug, Default)]
pub struct JobsState {
    open: bool,
    expanded: BTreeSet<JobId>,
    now: f64,
    /// When the next elapsed second starts for an active row, while the list is open.
    due: Option<f64>,
    /// The next tick refreshes the clock (set on open: a resting window's clock may be old).
    stale: bool,
    pub words: JobsWords,
}

impl JobsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the list: the footer button, or the host when an action launches (Strata opens
    /// Jobs in the launching window). Progress updates never open it.
    pub fn open(&mut self, tx: &mut Tx) {
        self.open = true;
        self.stale = true;
        tx.wake_after(Duration::ZERO);
    }

    /// Hides the list; no job is cancelled. A list closed with no jobs stays closed when a
    /// job comes later.
    pub fn close(&mut self) {
        self.open = false;
        self.due = None;
    }

    pub fn is_expanded(&self, id: JobId) -> bool {
        self.expanded.contains(&id)
    }

    /// Details or Hide on row `id`.
    pub fn toggle_details(&mut self, id: JobId) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
    }

    /// When the next elapsed second starts, while the list is open with a running job: the
    /// wake the last tick asked for, on the host's clock.
    pub fn due(&self) -> Option<f64> {
        self.due
    }

    /// The clock the elapsed times read.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// One frame at `now`: while the list is open with an active job, the elapsed times move
    /// each second, and a wake is asked for the next one. True when what the list shows
    /// changed.
    pub fn tick(&mut self, now: f64, rows: &[JobRow], tx: &mut Tx) -> bool {
        let mut changed = std::mem::take(&mut self.stale);
        if self.due.is_some_and(|d| now + 1e-6 >= d) {
            changed = true;
        }
        self.now = now;
        self.due = None;
        if !self.open {
            return changed;
        }
        let next = rows
            .iter()
            .filter(|r| r.status.is_running() && r.ended_at.is_none())
            .map(|r| {
                let elapsed = (now - r.started_at).max(0.0);
                elapsed.floor() + 1.0 - elapsed
            })
            .min_by(f64::total_cmp);
        if let Some(w) = next {
            self.due = Some(now + w);
            tx.wake_after(Duration::from_secs_f64(w.max(1e-3)));
        }
        changed
    }
}

type Intent<S> = Rc<dyn Fn(&mut S, JobsIntent, &mut Tx)>;

/// The footer button and its list.
///
/// ```ignore
/// let jobs = Jobs::new("jobs", |s: &App| &s.jobs_ui, |s: &mut App| &mut s.jobs_ui, |s: &App| &s.jobs)
///     .on_intent(|s: &mut App, intent, tx| s.job_service.handle(intent, tx))
///     .build(s, &s.look, &mut layer);
/// footer.jobs(jobs) // C19
/// ```
///
/// Keys: `<key>` (the button), `<key>-list` (the list), `<key>-clear`, `<key>-minimize`,
/// `<key>-row-<id>` with `-cancel`, `-details`, `-dismiss`, `-bar` and `-output` after it,
/// and `<key>-note`.
pub struct Jobs<S> {
    key: String,
    get: fn(&S) -> &JobsState,
    get_mut: fn(&mut S) -> &mut JobsState,
    rows: fn(&S) -> &[JobRow],
    on_intent: Intent<S>,
    fallback: Option<String>,
}

impl<S: 'static> Jobs<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &JobsState,
        get_mut: fn(&mut S) -> &mut JobsState,
        rows: fn(&S) -> &[JobRow],
    ) -> Self {
        Jobs {
            key: key.into(),
            get,
            get_mut,
            rows,
            on_intent: Rc::new(|_, _, _| {}),
            fallback: None,
        }
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, JobsIntent, &mut Tx) + 'static) -> Self {
        self.on_intent = Rc::new(f);
        self
    }

    /// Where focus goes when the list closes with no jobs left: the button goes with the last
    /// job, so it can't take focus back. Strata's window falls back to the file list's cursor
    /// row; pass its key. Without it Rune leaves nothing focused.
    pub fn return_focus(mut self, key: impl Into<String>) -> Self {
        self.fallback = Some(key.into());
        self
    }

    /// The footer button, or `None` with no jobs (Strata hides it). It is the look's
    /// `frame.control` tall, as C19's F1 button, so it sits in C19's footer
    /// (`frame.control + 2·space.xs`) with the footer's own inset above and below. While open,
    /// the list goes onto `layer`, above the button and aligned to its end.
    pub fn build(&self, s: &S, look: &Look, layer: &mut Vec<El>) -> Option<El> {
        let state = (self.get)(s);
        let rows = (self.rows)(s);
        let words = &state.words;
        // While the list is open the button stays, so the list keeps its anchor: emptied by
        // Clear finished, it reads "No background jobs." until it is closed.
        let label = indicator(rows, words).or_else(|| state.open.then(|| words.title.clone()))?;
        let k = &look.kit;
        let accent = k.c.text_accent;
        let open = state.open;
        let get_mut = self.get_mut;
        let mut button = row()
            .key(self.key.clone())
            .role("button")
            .label(label.clone())
            .description(words.show.clone())
            .expanded(open)
            .focusable(true)
            .h(look.frame.control)
            .px(look.space.sm)
            .items_center()
            .radius(look.round(BUTTON_RADIUS))
            .border(
                1.0,
                if open {
                    accent
                } else {
                    rune::Color::TRANSPARENT
                },
            )
            .hover(k.alpha(accent, BUTTON_HOVER_ALPHA))
            .press(k.alpha(accent, BUTTON_CHECKED_ALPHA))
            .focus_ring(BUTTON_RING, -BUTTON_RING, accent)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let st = get_mut(s);
                if st.open {
                    st.close();
                } else {
                    st.open(tx);
                }
            })
            .child(
                rune::text(label)
                    .font(k.ty.small)
                    .color(k.c.text_base)
                    .a11y_hidden(),
            );
        if open {
            // C19's F1 button fills itself with the checked tint while its panel shows.
            button = button.fill(k.alpha(accent, BUTTON_CHECKED_ALPHA));
            layer.push(popover::anchored(
                k,
                popover::raised(k, self.list(s, look), k.radius.md),
                &self.key,
                Placement::TopEnd,
                look.space.lg,
            ));
        }
        Some(button)
    }

    fn list(&self, s: &S, look: &Look) -> El {
        let state = (self.get)(s);
        let rows = (self.rows)(s);
        let words = &state.words;
        let k = &look.kit;
        let c = &k.c;
        let space = look.space;
        let key = &self.key;
        let get_mut = self.get_mut;
        let intent = self.on_intent.clone();
        let (active, finished) = text::activity(rows, words);
        let small = k.ty.small;

        let mut header = row()
            .w_full()
            .gap(space.md)
            .items_center()
            .px(space.xl)
            .py(space.lg)
            .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
            .child(bezel(look, IconName::ListChecks))
            .child(
                rune::text(words.title.clone())
                    .font(TextStyle {
                        weight: 700.0,
                        ..k.ty.ui
                    })
                    .color(c.text_base)
                    .a11y_hidden(),
            );
        let both = active.is_some();
        if let Some(a) = active {
            header = header.child(
                rune::text(a)
                    .key(format!("{key}-running"))
                    .font(small)
                    .color(c.text_accent),
            );
        }
        if let Some(f) = finished {
            // The separator is drawn, not read.
            let shown = if both { format!("· {f}") } else { f.clone() };
            header = header.child(
                rune::text(shown)
                    .key(format!("{key}-finished"))
                    .label(f)
                    .font(small)
                    .color(c.text_muted),
            );
        }
        header = header.child(el().grow(1.0));
        if rows.iter().any(|r| r.status.is_finished()) {
            let intent = intent.clone();
            header = header.child(
                rune_kit::Button::new(words.clear.clone())
                    .variant(rune_kit::ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .icon(look.icon(IconName::Trash))
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        intent(s, JobsIntent::ClearFinished, tx)
                    })
                    .build(k)
                    .key(format!("{key}-clear")),
            );
        }
        header = header.child(
            small_button(look, IconName::Minimize, &words.minimize)
                .on_press(move |s: &mut S| get_mut(s).close())
                .build(k)
                .key(format!("{key}-minimize")),
        );

        let mut list = col().w_full();
        for (i, r) in rows.iter().take(MAX_ROWS).enumerate() {
            list = list.child(self.row(r, i, state, look));
        }
        let note = if rows.is_empty() {
            Some(words.none.clone())
        } else if rows.len() > MAX_ROWS {
            Some((words.hidden)(rows.len() - MAX_ROWS))
        } else {
            None
        };
        if let Some(n) = note {
            list = list.child(
                rune::text(n)
                    .key(format!("{key}-note"))
                    .font(small)
                    .color(c.text_muted)
                    .px(space.xl)
                    .py(space.lg),
            );
        }
        col()
            .key(format!("{key}-list"))
            .role("dialog")
            .label(words.title.clone())
            .w(LIST_WIDTH)
            .max_w(LIST_WIDTH)
            .max_h(LIST_MAX_HEIGHT)
            .min_h(0.0)
            // With no jobs left the button goes once the list closes: focus goes to the host's
            // fallback instead.
            .focus_return(match (&self.fallback, rows.is_empty()) {
                (Some(f), true) => f.clone(),
                _ => key.clone(),
            })
            // Escape and a press outside hide it, as Minimize does; no job is cancelled.
            .modal(true, move |s: &mut S| get_mut(s).close())
            .child(header)
            .child(
                col()
                    .w_full()
                    .shrink()
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(list),
            )
    }

    fn row(&self, r: &JobRow, index: usize, state: &JobsState, look: &Look) -> El {
        let words = &state.words;
        let k = &look.kit;
        let c = &k.c;
        let space = look.space;
        let small = k.ty.small;
        let key = format!("{}-row-{}", self.key, r.id);
        let id = r.id;
        let get_mut = self.get_mut;
        let status_words = text::status(r, state.now, words);

        let mut head = row()
            .w_full()
            .gap(space.md)
            .items_center()
            .child(
                rune::text(r.name.clone())
                    .font(TextStyle {
                        weight: 700.0,
                        ..k.ty.ui
                    })
                    .color(c.text_base)
                    .truncate()
                    .shrink()
                    .min_w(0.0)
                    .a11y_hidden(),
            )
            .child(
                rune::text(status_words.clone())
                    .key(format!("{key}-status"))
                    .font(small)
                    .color(c.text_accent)
                    .a11y_hidden(),
            );
        let mark = match r.status {
            JobStatus::Succeeded => Some(IconName::Check),
            JobStatus::Failed | JobStatus::Cancelled => Some(IconName::Close),
            _ => None,
        };
        if let Some(m) = mark {
            head = head.child(
                icon(look.icon(m), look.icon_size.sm)
                    .color(c.icon_base)
                    .a11y_hidden(),
            );
        }
        head = head.child(el().grow(1.0));
        if matches!(r.status, JobStatus::Running | JobStatus::Queued) {
            let queued = r.status == JobStatus::Queued;
            let intent = self.on_intent.clone();
            let label = if queued { &words.remove } else { &words.cancel };
            head = head.child(
                small_button(look, IconName::Close, label)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        intent(
                            s,
                            if queued {
                                JobsIntent::Remove(id)
                            } else {
                                JobsIntent::Cancel(id)
                            },
                            tx,
                        )
                    })
                    .build(k)
                    .key(format!("{key}-cancel")),
            );
        }
        let expanded = state.is_expanded(id);
        if r.status != JobStatus::Queued {
            let (glyph, label) = if expanded {
                (IconName::EyeOff, &words.hide)
            } else {
                (IconName::Eye, &words.details)
            };
            head = head.child(
                small_button(look, glyph, label)
                    .expanded(expanded)
                    .on_press(move |s: &mut S| get_mut(s).toggle_details(id))
                    .build(k)
                    .key(format!("{key}-details")),
            );
        }
        if r.status.is_finished() {
            let intent = self.on_intent.clone();
            head = head.child(
                small_button(look, IconName::Close, &words.dismiss)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        intent(s, JobsIntent::Dismiss(id), tx)
                    })
                    .build(k)
                    .key(format!("{key}-dismiss")),
            );
        }
        let meta = text::meta(r, words);
        let mut content = col().grow(1.0).min_w(0.0).gap(space.sm).child(head).child(
            rune::text(meta.clone())
                .font(small)
                .color(c.text_muted)
                .truncate()
                .a11y_hidden(),
        );
        if r.status.is_active() {
            // Named on the built bar: Progress's own label would draw the name again.
            let bar = Progress::new(r.fraction).build(k).label(r.name.clone());
            content = content.child(bar.key(format!("{key}-bar")));
        }
        if expanded {
            let mut output = rune::text(text::details(r, words))
                .key(format!("{key}-output"))
                .font(k.ty.mono)
                .color(c.text_base)
                .wrap()
                .w_full()
                .selectable(true)
                .selection_color(k.selection());
            if r.output_truncated {
                output = output.description(words.truncated.clone());
            }
            content = content.child(
                col()
                    .w_full()
                    .max_h(OUTPUT_MAX_HEIGHT)
                    .min_h(0.0)
                    // Follows the output as it arrives until scrolled up, and again once
                    // scrolled back to the end (Rune's stick_bottom, Strata's follow_log_output).
                    .scroll(ScrollSpec { stick_bottom: true })
                    .child(output),
            );
        }
        let mut row_el = row()
            .key(key)
            .role("group")
            .label(r.name.clone())
            .description(format!("{status_words} · {meta}"))
            .w_full()
            .gap(space.lg)
            .items_start()
            .px(space.xl)
            .py(space.lg)
            .child(bezel(look, r.icon))
            .child(content);
        if index > 0 {
            row_el = row_el.border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted);
        }
        row_el
    }
}

/// An icon in a rounded bezel (`.job-icon-bezel`).
fn bezel(look: &Look, name: IconName) -> El {
    let c = &look.kit.c;
    el().size(BEZEL, BEZEL)
        .radius(look.round(BEZEL_RADIUS))
        .fill(c.layer_02)
        .border(1.0, c.border_base)
        .center()
        .a11y_hidden()
        .child(icon(look.icon(name), look.icon_size.md).color(c.icon_accent))
}

/// A row action: an icon button named and tooltipped by `label` (Strata's `job_button`).
fn small_button(look: &Look, name: IconName, label: &str) -> IconButton {
    IconButton::new(look.icon(name))
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .label(label.to_owned())
        .help(label.to_owned())
}
