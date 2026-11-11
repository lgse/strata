//! [`Dock`]: the operations, notices and timers the dock and the progress dialog show. The
//! host holds it and feeds it data; the views read it.

use std::time::Duration;

use rune::{Live, LiveText, Tween, Tx};
use strata_kit::icons::IconName;

use super::countdown::{COMPLETE_FOR, Countdown, Since};
use super::format;
use super::rate::RateMeter;
use super::words::Words;

/// An operation's id, the host's: stable while the operation shows.
pub type OpId = u64;

/// The most often committed text and accessibility values change while operations run
/// (seconds between commits). The bars and live labels follow every sample.
pub const COMMIT_EVERY: f64 = 0.25;

/// How long after Cancel a transfer that hasn't stopped reads "Device not responding" (Strata's
/// `STALLED_CANCEL_DELAY`).
pub const STALL_AFTER: f64 = 8.0;

/// How long a notice shows by default (Strata's provider notices: about 4 s).
pub const NOTICE_FOR: f64 = 4.0;

/// An operation on fewer than this many items waits [`REVEAL_DELAY`] before it shows (Strata's
/// `IMMEDIATE_PROGRESS_ITEM_COUNT`).
pub const IMMEDIATE_ITEMS: usize = 16;

/// The wait before a small operation shows (Strata's `FILE_PROGRESS_DELAY`).
pub const REVEAL_DELAY: f64 = 0.35;

const EPS: f64 = 1e-6;

/// Strata's rule for when an operation's progress shows: at once for 16 or more items or an
/// unknown count, otherwise after 350 ms, and never if it finishes first.
pub fn reveal_after(total_items: Option<usize>) -> f64 {
    match total_items {
        Some(n) if n > 0 && n < IMMEDIATE_ITEMS => REVEAL_DELAY,
        _ => 0.0,
    }
}

/// Where an operation shows. Strata's rule (which operations dock) stays with Strata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// A card in the dock; the window stays usable.
    #[default]
    Dock,
    /// The blocking progress dialog ([`ProgressDialog`](super::ProgressDialog)).
    Dialog,
}

/// How far an operation has got, as the host knows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// A copy or a move: items, files and bytes (Strata's transfer snapshot).
    Transfer(Transfer),
    /// Items done of a total: a compression, a multi-item delete, a restore.
    Items { done: usize, total: usize },
    /// A running count without a total: the bar pulses (a one-item permanent delete).
    Count(usize),
    /// Nothing known yet: the bar pulses.
    Unknown,
}

/// A transfer's progress. `total_files` and `total_bytes` are `None` until the transfer has
/// counted them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transfer {
    pub completed_items: usize,
    pub total_items: usize,
    pub completed_files: usize,
    pub total_files: Option<usize>,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
}

/// One progress report.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub progress: Progress,
    /// The file being worked on, which a card shows in place of its info line.
    pub current: Option<String>,
    /// The card footer's left label for an operation that isn't a transfer: a compression's
    /// "1 / 2 files", a delete's "270,325 items deleted", "Stopping…". Empty for a delete.
    pub status: Option<String>,
    /// Pulses the bar and shows this in place of the speed: "Writing to device…".
    pub busy: Option<String>,
}

impl Sample {
    pub fn new(progress: Progress) -> Self {
        Sample {
            progress,
            current: None,
            status: None,
            busy: None,
        }
    }

    pub fn current(mut self, name: impl Into<String>) -> Self {
        self.current = Some(name.into());
        self
    }

    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    pub fn busy(mut self, words: impl Into<String>) -> Self {
        self.busy = Some(words.into());
        self
    }
}

/// What an operation shows, set once when it starts. The words are the host's.
#[derive(Clone, Debug, PartialEq)]
pub struct OpSpec {
    pub icon: IconName,
    /// "Copying items".
    pub title: String,
    /// The dialog's subtitle: "Cancelling will not undo completed changes".
    pub subtitle: Option<String>,
    /// The first item's name, or an archive's: the card's info line until a file is current.
    pub info: String,
    /// "→ Pictures", "→ Trash", "Permanent deletion".
    pub destination: Option<String>,
    /// What assistive technology reads for the destination: its whole path.
    pub destination_description: Option<String>,
    /// The Cancel buttons' accessible name: "Cancel copying".
    pub cancel_label: String,
    /// A copy or a move: Cancel warns that the device may still be writing, and stalls.
    pub transfer: bool,
    pub placement: Placement,
    /// Seconds before it shows ([`reveal_after`]); it never shows if it ends first.
    pub reveal_after: f64,
}

impl OpSpec {
    pub fn new(icon: IconName, title: impl Into<String>, info: impl Into<String>) -> Self {
        let title = title.into();
        OpSpec {
            icon,
            cancel_label: "Cancel operation".into(),
            title,
            subtitle: None,
            info: info.into(),
            destination: None,
            destination_description: None,
            transfer: false,
            placement: Placement::Dock,
            reveal_after: 0.0,
        }
    }
}

/// How an operation ended on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndKind {
    /// "Copy complete", with a check, 100 % and the Complete button.
    Complete,
    /// A card that turned into a notice: the WP's cancel path.
    Notice,
}

/// Where a running operation stands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Phase {
    Running,
    Cancelling { since: Since, stalled: bool },
    Ended { kind: EndKind, countdown: Countdown },
}

/// What the views read, refreshed at most every [`COMMIT_EVERY`] while an operation runs,
/// and at once when its phase changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub icon: Option<IconName>,
    pub title: String,
    /// The card's info line: the current file, or the spec's info, or the cancel warning.
    pub info: String,
    /// The card's count shows ("3/10"); its words are live.
    pub count: Option<String>,
    /// The fraction the bar shows; `None` pulses it.
    pub fraction: Option<f32>,
    /// What assistive technology reads as the bar's value: "42%".
    pub value: Option<String>,
    /// What it reads after: the speed or status words.
    pub description: String,
    /// The dialog's item line: "3 of 10 files · photo.jpg".
    pub items_line: Option<String>,
    /// A transfer's dialog shows bytes and speed.
    pub transfer: bool,
    /// A transfer's bytes as committed: "20 MB / 120 MB", which the dialog's bar describes.
    pub bytes: Option<String>,
    pub subtitle: Option<String>,
    pub cancel_enabled: bool,
    /// The dialog's Cancel words.
    pub cancel_words: String,
    /// A finished card: its kind, pin and countdown words.
    pub ended: Option<EndShown>,
}

/// A finished card's countdown, as shown.
#[derive(Clone, Debug, PartialEq)]
pub struct EndShown {
    pub kind: EndKind,
    pub pinned: bool,
    /// "closing in 4s", "paused", "pinned".
    pub words: String,
    /// The countdown bar's fraction while it doesn't move (paused).
    pub left: f32,
    /// Paused by hover or the pin: the countdown bar stands still.
    pub paused: bool,
    pub leaving: bool,
}

/// The labels and bars that follow every sample without a commit.
#[derive(Clone)]
pub struct CardLive {
    /// The bar's fill, `[x, y, w, h]` in the track's points.
    pub fill: Live<[f32; 4]>,
    /// The card's right label and the dialog's big one: "42%", "…".
    pub percent: LiveText,
    /// The dialog's status beside the percentage: "Preparing…".
    pub status: LiveText,
    /// The card's left label: speed and time left, or the status.
    pub meta: LiveText,
    /// The card's count: "3/10".
    pub count: LiveText,
    /// The dialog's bytes: "12 MB / 40 MB".
    pub bytes: LiveText,
}

impl Default for CardLive {
    fn default() -> Self {
        CardLive {
            fill: Live::new([0.0; 4]),
            percent: LiveText::new(""),
            status: LiveText::new(""),
            meta: LiveText::new(""),
            count: LiveText::new(""),
            bytes: LiveText::new(""),
        }
    }
}

impl std::fmt::Debug for CardLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardLive").finish_non_exhaustive()
    }
}

/// An operation the dock or the dialog shows.
pub struct Op {
    pub id: OpId,
    pub(crate) order: u64,
    pub spec: OpSpec,
    reveal: Option<Since>,
    pub(crate) revealed: bool,
    sample: Option<Sample>,
    rate: RateMeter,
    pub(crate) phase: Phase,
    title: String,
    pub shown: Shown,
    pub live: CardLive,
    fraction: Option<f32>,
    /// The bar's track, `[w, h]`, once laid out.
    pub(crate) track: Option<[f32; 2]>,
    /// The countdown bar, from the time left to empty.
    pub(crate) countdown_bar: Option<Tween<[f32; 4]>>,
    /// The fade before the card goes.
    pub(crate) fade: Option<Tween<f32>>,
    /// The pointer is over the card, in every phase: Rune reports a hover only when it
    /// changes, so a card that ends under a resting pointer hears no new enter and starts
    /// its countdown paused from this (Strata's `completion.rs` tracks motion enter
    /// throughout).
    hovered: bool,
}

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Op")
            .field("id", &self.id)
            .field("phase", &self.phase)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// A brief notice in the dock: a row with no controls that closes itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub heading: Option<String>,
    pub text: String,
    /// A success check before the text; off for a status notice ("Text size: 16px"), which
    /// has no minimum width either.
    pub check: bool,
    /// Joins the dock and leaves the notices showing in place (a transfer report); otherwise
    /// it replaces them.
    pub joins: bool,
    /// Seconds it shows.
    pub shown_for: f64,
}

impl Notice {
    /// A notice with a check that replaces the others and shows for [`NOTICE_FOR`].
    pub fn new(text: impl Into<String>) -> Self {
        Notice {
            heading: None,
            text: text.into(),
            check: true,
            joins: false,
            shown_for: NOTICE_FOR,
        }
    }

    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.heading = Some(heading.into());
        self
    }

    /// A status notice: no check and no minimum width.
    pub fn status(mut self) -> Self {
        self.check = false;
        self
    }

    pub fn joins(mut self) -> Self {
        self.joins = true;
        self
    }

    pub fn shown_for(mut self, seconds: f64) -> Self {
        self.shown_for = seconds;
        self
    }
}

/// A notice in the dock, with its id and lifetime.
#[derive(Clone, Debug, PartialEq)]
pub struct DockNotice {
    pub id: u64,
    pub(crate) order: u64,
    pub notice: Notice,
    since: Since,
}

/// What the dock asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockIntent {
    /// Cancel this operation. Its card already reads "Cancelling…" with X disabled. When the
    /// operation has stopped, the host removes the card ([`Dock::remove`]) and opens its
    /// "Operation cancelled" dialog (Strata), or turns it into a notice
    /// ([`Dock::close_as_notice`]).
    Cancel(OpId),
    /// The user dismissed this finished card (Complete or X). It is already gone.
    Dismissed(OpId),
}

/// The operations and notices the dock and the progress dialog show, and their timers.
///
/// The host holds it and passes it by accessor. Every timer runs from [`Dock::tick`], called
/// from the host's poll with its clock: it stamps what handlers started, ends what is due,
/// flushes held commits and asks for a wake when the next thing is due, so a resting window
/// polls only then. Calls that start a timer ask for a wake so the next tick comes.
#[derive(Debug, Default)]
pub struct Dock {
    pub(crate) ops: Vec<Op>,
    pub(crate) notices: Vec<DockNotice>,
    next_order: u64,
    next_notice: u64,
    words: Words,
    /// The animations setting is off (so the default, false, has them on).
    still: bool,
    /// A cancel from a handler without a `Tx` ([`Dock::cancel_untimed`]) waits for the tick
    /// that stamps it: the views mount a wake beacon with this generation until then.
    beacon: Option<u64>,
    beacons: u64,
    /// When the views last read what the ops show.
    last_flush: Option<f64>,
    /// Something changed since that the views haven't read.
    dirty: bool,
    /// The wake the last call asked for, in seconds from its clock.
    asked: Option<f64>,
}

impl Dock {
    /// An empty dock with English words and animations on.
    pub fn new() -> Self {
        Dock::default()
    }

    /// The words cards and the dialog write.
    pub fn set_words(&mut self, words: Words) {
        self.words = words;
    }

    pub fn words(&self) -> &Words {
        &self.words
    }

    /// The animations setting.
    pub fn animations(&self) -> bool {
        !self.still
    }

    /// The animations setting (P5's switch, as data): off, a finished card goes without its
    /// fade.
    pub fn set_animations(&mut self, on: bool) {
        self.still = !on;
    }

    /// True with nothing to show.
    pub fn is_empty(&self) -> bool {
        !self
            .ops
            .iter()
            .any(|o| o.revealed && o.spec.placement == Placement::Dock)
            && self.notices.is_empty()
    }

    /// The operation `id`, revealed or not.
    pub fn op(&self, id: OpId) -> Option<&Op> {
        self.ops.iter().find(|o| o.id == id)
    }

    fn op_mut(&mut self, id: OpId) -> Option<&mut Op> {
        self.ops.iter_mut().find(|o| o.id == id)
    }

    /// The notices showing, oldest first.
    pub fn notices(&self) -> impl Iterator<Item = &DockNotice> {
        self.notices.iter()
    }

    /// Whether a transfer is cancelling: Strata then refuses to start another file operation
    /// ("Transfer cancellation pending").
    pub fn cancel_pending(&self) -> bool {
        self.ops
            .iter()
            .any(|o| o.spec.transfer && matches!(o.phase, Phase::Cancelling { .. }))
    }

    /// Starts showing operation `id` (after `spec.reveal_after`). Starting an id that shows
    /// replaces it.
    pub fn start(&mut self, id: OpId, spec: OpSpec, tx: &mut Tx) {
        self.ops.retain(|o| o.id != id);
        let order = self.take_order();
        let title = spec.title.clone();
        let revealed = spec.reveal_after <= 0.0;
        let mut op = Op {
            id,
            order,
            reveal: (!revealed).then_some(Since::Pending),
            revealed,
            spec,
            sample: None,
            rate: RateMeter::new(),
            phase: Phase::Running,
            hovered: false,
            title,
            shown: Shown::default(),
            live: CardLive::default(),
            fraction: None,
            track: None,
            countdown_bar: None,
            fade: None,
        };
        let words = self.words.clone();
        op.publish(&words);
        op.shown = op.derive(&words);
        self.ops.push(op);
        tx.wake_after(Duration::ZERO);
    }

    /// Takes a progress report for `id` at `now` on the host's clock. The bar and the live
    /// labels follow at once; what is committed (the file line, the accessibility values)
    /// follows at most every [`COMMIT_EVERY`], at once when what the card shows changes shape.
    /// True when the view needs to run again; a held change asks for a wake so it lands even if
    /// no report follows.
    pub fn progress(&mut self, id: OpId, sample: Sample, now: f64, tx: &mut Tx) -> bool {
        let words = self.words.clone();
        let Some(op) = self.op_mut(id) else {
            return false;
        };
        if matches!(op.phase, Phase::Ended { .. }) {
            return false;
        }
        if let Progress::Transfer(t) = sample.progress {
            op.rate.sample(t.bytes, now);
        }
        op.sample = Some(sample);
        op.publish(&words);
        let next = op.derive(&words);
        let reshaped = next.fraction.is_some() != op.shown.fraction.is_some()
            || next.count.is_some() != op.shown.count.is_some()
            || next.items_line.is_some() != op.shown.items_line.is_some()
            || next.title != op.shown.title;
        if !op.revealed {
            // Nothing shows yet: the view reads it when it does.
            op.shown = next;
            return false;
        }
        if next == op.shown {
            return false;
        }
        self.dirty = true;
        if reshaped
            || self
                .last_flush
                .is_none_or(|t| now - t + EPS >= COMMIT_EVERY)
        {
            self.flush(now);
            return true;
        }
        let at = self.last_flush.unwrap_or(now) + COMMIT_EVERY;
        self.asked = Some((at - now).max(0.0));
        tx.wake_after(Duration::from_secs_f64((at - now).max(0.0)));
        false
    }

    /// Operation `id` finished: its card reads `title` ("Copy complete") with a check, 100 %
    /// and Complete, and closes itself after [`COMPLETE_FOR`]. A dialog operation just goes.
    /// False when it never showed (it ended before its reveal): it is gone, and nothing shows.
    /// A card that has already ended keeps its ending: a second one is ignored.
    pub fn complete(&mut self, id: OpId, title: impl Into<String>, tx: &mut Tx) -> bool {
        self.end(id, EndKind::Complete, title.into(), None, COMPLETE_FOR, tx)
    }

    /// Operation `id` stopped, and its card turns into a notice that reads `title` and `text`
    /// and closes itself after `shown_for` seconds (the WP's cancel path; Strata's is
    /// [`Dock::remove`] and a dialog). False when it never showed.
    pub fn close_as_notice(
        &mut self,
        id: OpId,
        title: impl Into<String>,
        text: impl Into<String>,
        shown_for: f64,
        tx: &mut Tx,
    ) -> bool {
        self.end(
            id,
            EndKind::Notice,
            title.into(),
            Some(text.into()),
            shown_for,
            tx,
        )
    }

    fn end(
        &mut self,
        id: OpId,
        kind: EndKind,
        title: String,
        text: Option<String>,
        shown_for: f64,
        tx: &mut Tx,
    ) -> bool {
        let animations = !self.still;
        let words = self.words.clone();
        let Some(op) = self.op_mut(id) else {
            return false;
        };
        if !op.revealed || op.spec.placement == Placement::Dialog {
            self.remove(id);
            return false;
        }
        if matches!(op.phase, Phase::Ended { .. }) {
            // Already finished: a second ending changes nothing, as Strata's `complete` ignores
            // a card already completed.
            return true;
        }
        op.title = title;
        if let Some(text) = text {
            op.spec.info = text;
            op.spec.destination = None;
            op.sample = None;
        }
        if kind == EndKind::Complete {
            op.spec.icon = IconName::Check;
            op.complete_progress();
        }
        op.phase = Phase::Ended {
            kind,
            countdown: {
                let mut c = Countdown::new(shown_for, animations);
                c.set_hovered(op.hovered);
                c
            },
        };
        op.publish(&words);
        op.shown = op.derive(&words);
        tx.wake_after(Duration::ZERO);
        true
    }

    /// Removes operation `id` at once: Strata's cancel and failure paths, where the host opens
    /// a dialog instead.
    pub fn remove(&mut self, id: OpId) {
        self.ops.retain(|o| o.id != id);
    }

    /// Shows `notice`. Unless it joins, it replaces the notices showing; cards stay.
    pub fn notice(&mut self, notice: Notice, tx: &mut Tx) -> u64 {
        if !notice.joins {
            self.notices.clear();
        }
        let order = self.take_order();
        self.next_notice += 1;
        let id = self.next_notice;
        self.notices.push(DockNotice {
            id,
            order,
            notice,
            since: Since::Pending,
        });
        tx.wake_after(Duration::ZERO);
        id
    }

    /// The user pressed a running card's X, or the dialog's Cancel: the card reads
    /// "Cancelling…" with Cancel disabled. True when that started a cancel (the caller then
    /// sends [`DockIntent::Cancel`]); a second press does nothing.
    pub fn cancel(&mut self, id: OpId, tx: &mut Tx) -> bool {
        let words = self.words.clone();
        let Some(op) = self.op_mut(id) else {
            return false;
        };
        if op.phase != Phase::Running {
            return false;
        }
        op.phase = Phase::Cancelling {
            since: Since::Pending,
            stalled: false,
        };
        op.title = if op.spec.transfer {
            words.cancelling_transfer.clone()
        } else {
            words.cancelling_operation.clone()
        };
        op.publish(&words);
        op.shown = op.derive(&words);
        tx.wake_after(Duration::ZERO);
        true
    }

    /// [`Dock::cancel`] from a handler without a `Tx` (C15's buttons and Escape). Rune runs no
    /// poll on its own after the commit the cancel makes, so nothing would bring the tick that
    /// starts the stall's clock: until that tick the views mount a wake beacon
    /// ([`Dock::wants_wake`]), a zero-size element whose `on_layout_tx` (the one place a view
    /// gets a `Tx`) asks for a wake when the commit lays it out.
    pub fn cancel_untimed(&mut self, id: OpId) -> bool {
        let started = self.cancel(id, &mut Tx::default());
        if started {
            self.beacons += 1;
            self.beacon = Some(self.beacons);
        }
        started
    }

    /// The wake beacon's generation while one is due ([`Dock::cancel_untimed`]); a new
    /// generation mounts a new beacon, whose first layout asks for the wake.
    pub fn wants_wake(&self) -> Option<u64> {
        self.beacon
    }

    /// The user dismissed finished card `id`. True when it was finished (and is now gone).
    pub fn dismiss(&mut self, id: OpId) -> bool {
        let ended = self
            .op(id)
            .is_some_and(|o| matches!(o.phase, Phase::Ended { .. }));
        if ended {
            self.remove(id);
        }
        ended
    }

    /// The pointer moved onto or off card `id`, in any phase: a finished card's countdown
    /// pauses, and a card that finishes under the pointer starts paused. True when a
    /// countdown changed.
    pub fn hover(&mut self, id: OpId, hovered: bool, tx: &mut Tx) -> bool {
        let Some(op) = self.op_mut(id) else {
            return false;
        };
        op.hovered = hovered;
        let Phase::Ended { countdown, .. } = &mut op.phase else {
            return false;
        };
        let changed = countdown.set_hovered(hovered);
        if changed {
            tx.wake_after(Duration::ZERO);
        }
        changed
    }

    /// Pins or unpins finished card `id` ("Keep notification").
    pub fn toggle_pin(&mut self, id: OpId, tx: &mut Tx) {
        let words = self.words.clone();
        if let Some(op) = self.op_mut(id)
            && let Phase::Ended { countdown, .. } = &mut op.phase
        {
            countdown.toggle_pin();
            op.shown = op.derive(&words);
            tx.wake_after(Duration::ZERO);
        }
    }

    /// The bar's track laid out at `w` × `h` points. True when the view must run again (the
    /// first size, or one that restarts a countdown bar); otherwise only live values moved.
    pub fn set_track(&mut self, id: OpId, w: f32, h: f32) -> bool {
        let Some(op) = self.op_mut(id) else {
            return false;
        };
        // A scroll moves the track without resizing it, and on_layout reports every move:
        // only a new size changes the fill or a countdown bar.
        if op
            .track
            .is_some_and(|[ow, oh]| (ow - w).abs() < 0.5 && (oh - h).abs() < 0.5)
        {
            return false;
        }
        let first = op.track.is_none();
        op.track = Some([w, h]);
        op.publish_fill();
        let restart = match &op.phase {
            Phase::Ended { countdown, .. } if !countdown.paused() => {
                op.countdown_bar = Some(countdown_bar(countdown, w, h));
                true
            }
            _ => false,
        };
        first || restart
    }

    /// One frame at `now` seconds: reveals, stalls, countdowns, notices and held commits. True
    /// when the views need to run again. Asks `tx` for a wake when the next thing is due.
    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> bool {
        let words = self.words.clone();
        let animations = !self.still;
        let mut changed = self.beacon.take().is_some();
        let mut wake: Option<f64> = None;
        let mut want = |w: f64| wake = Some(wake.map_or(w, |v: f64| v.min(w)));
        let mut gone = Vec::new();
        for op in &mut self.ops {
            if let Some(since) = &mut op.reveal {
                let at = since.at(now) + op.spec.reveal_after;
                if now + EPS >= at {
                    op.reveal = None;
                    op.revealed = true;
                    op.shown = op.derive(&words);
                    changed = true;
                } else {
                    want(at - now);
                }
            }
            match &mut op.phase {
                Phase::Cancelling { since, stalled } if op.spec.transfer && !*stalled => {
                    let at = since.at(now) + STALL_AFTER;
                    if now + EPS >= at {
                        *stalled = true;
                        op.title = words.not_responding.clone();
                        op.shown = op.derive(&words);
                        changed = true;
                    } else {
                        want(at - now);
                    }
                }
                Phase::Ended { countdown, .. } => {
                    let step = countdown.tick(now);
                    if step.done {
                        gone.push(op.id);
                        changed = true;
                        continue;
                    }
                    if step.restarted {
                        op.countdown_bar = op.track.map(|[w, h]| countdown_bar(countdown, w, h));
                    }
                    if step.leaving {
                        op.fade = (animations && countdown.fade() > 0.0).then(|| {
                            Tween::linear(1.0, 0.0, Duration::from_secs_f64(countdown.fade()))
                        });
                    } else if !countdown.leaving() {
                        op.fade = None;
                    }
                    if step.changed {
                        op.shown = op.derive(&words);
                        changed = true;
                    }
                    if let Some(w) = step.wake {
                        want(w);
                    }
                }
                _ => {}
            }
        }
        self.ops.retain(|o| !gone.contains(&o.id));
        let before = self.notices.len();
        self.notices.retain_mut(|n| {
            let at = n.since.at(now) + n.notice.shown_for;
            if now + EPS >= at {
                false
            } else {
                want(at - now);
                true
            }
        });
        changed |= self.notices.len() != before;
        if self.dirty {
            let due = self.last_flush.map_or(now, |t| t + COMMIT_EVERY);
            if now + EPS >= due {
                self.flush(now);
                changed = true;
            } else {
                want(due - now);
            }
        }
        self.asked = wake;
        if let Some(w) = wake {
            tx.wake_after(Duration::from_secs_f64(w.max(0.0)));
        }
        changed
    }

    /// The wake the last [`Dock::tick`] (or a held [`Dock::progress`]) asked for, in seconds
    /// from its clock: `None` when nothing is due, so a resting window sleeps.
    pub fn asked_wake(&self) -> Option<f64> {
        self.asked
    }

    /// The views read what every op shows now.
    fn flush(&mut self, now: f64) {
        let words = self.words.clone();
        for op in &mut self.ops {
            op.shown = op.derive(&words);
        }
        self.last_flush = Some(now);
        self.dirty = false;
    }

    fn take_order(&mut self) -> u64 {
        self.next_order += 1;
        self.next_order
    }
}

/// The countdown bar from the time left to empty, over the time left.
fn countdown_bar(c: &Countdown, w: f32, h: f32) -> Tween<[f32; 4]> {
    let left = c.left().max(0.0);
    let frac = (left / c.duration().max(EPS)).clamp(0.0, 1.0) as f32;
    Tween::linear(
        [0.0, 0.0, w * frac, h],
        [0.0, 0.0, 0.0, h],
        Duration::from_secs_f64(left),
    )
}

impl Op {
    /// The phase's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Running: neither cancelling nor finished.
    pub fn running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// Cancel was pressed.
    pub fn cancelling(&self) -> bool {
        matches!(self.phase, Phase::Cancelling { .. })
    }

    /// Finished and counting down (or pinned).
    pub fn ended(&self) -> Option<EndKind> {
        match self.phase {
            Phase::Ended { kind, .. } => Some(kind),
            _ => None,
        }
    }

    /// Fills the transfer to its end, as Strata's completed card shows "N/N" and 100 %.
    fn complete_progress(&mut self) {
        if let Some(s) = &mut self.sample {
            match &mut s.progress {
                Progress::Transfer(t) => {
                    let files = t.total_files.unwrap_or(t.total_items);
                    t.completed_files = files;
                    t.total_files = Some(files);
                    t.completed_items = t.total_items;
                    if let Some(total) = t.total_bytes {
                        t.bytes = total;
                    }
                }
                Progress::Items { done, total } => *done = *total,
                _ => {}
            }
            s.busy = None;
            s.status = None;
        }
    }

    fn pulsing(&self) -> bool {
        self.cancelling() || self.sample.as_ref().is_some_and(|s| s.busy.is_some())
    }

    /// The bar's fraction: `None` pulses it.
    fn fraction(&self) -> Option<f32> {
        match &self.phase {
            Phase::Ended {
                kind: EndKind::Complete,
                ..
            } => return Some(1.0),
            Phase::Ended { .. } => return None,
            _ => {}
        }
        if self.pulsing() {
            return None;
        }
        let progress = self.sample.as_ref()?.progress;
        match progress {
            Progress::Transfer(t) => transfer_fraction(&t).map(|f| f as f32),
            Progress::Items { done, total } if total > 0 => {
                Some((done as f64 / total as f64).clamp(0.0, 1.0) as f32)
            }
            _ => None,
        }
    }

    /// The live labels and the bar, every sample.
    fn publish(&mut self, words: &Words) {
        self.fraction = self.fraction();
        self.publish_fill();
        let d = self.labels(words);
        self.live.percent.set(d.right);
        self.live.status.set(d.status);
        self.live.meta.set(d.meta);
        self.live.count.set(d.count.unwrap_or_default());
        self.live.bytes.set(d.bytes);
    }

    fn publish_fill(&self) {
        if let (Some([w, h]), Some(f)) = (self.track, self.fraction) {
            self.live.fill.set([0.0, 0.0, w * f, h]);
        }
    }

    /// The label words this sample gives, Strata's `transfer_progress_status` and
    /// `sync_compact`.
    fn labels(&self, words: &Words) -> Labels {
        let mut out = Labels::default();
        let percent_of = |f: f64, started: bool| (words.percent)(format::whole_percent(f, started));
        let ended = matches!(self.phase, Phase::Ended { .. });
        let sample = self.sample.as_ref();
        match sample.map(|s| s.progress) {
            Some(Progress::Transfer(t)) => {
                let (status, fraction) = transfer_status(&t, words);
                out.status = status.clone();
                out.right = match fraction {
                    Some(_) => status,
                    None => words.busy_mark.clone(),
                };
                out.bytes = match t.total_bytes {
                    Some(total) => (words.bytes_of)(&(words.size)(t.bytes), &(words.size)(total)),
                    None => (words.size)(t.bytes),
                };
                let count = transfer_count(&t);
                out.count = count.map(|(done, total, _)| {
                    (words.count_of)(
                        &(words.integer)(done as u64),
                        &(words.integer)(total as u64),
                    )
                });
                out.meta = if count.is_none() {
                    words.preparing_items.clone()
                } else {
                    rate_status(self.rate.rate(), t.bytes, t.total_bytes, words)
                };
            }
            Some(Progress::Items { done, total }) => {
                let f = if total > 0 {
                    done as f64 / total as f64
                } else {
                    0.0
                };
                out.right = percent_of(f, false);
                out.status = out.right.clone();
                out.meta = sample.and_then(|s| s.status.clone()).unwrap_or_default();
            }
            Some(Progress::Count(_)) | Some(Progress::Unknown) => {
                out.right = words.busy_mark.clone();
                out.status = sample.and_then(|s| s.status.clone()).unwrap_or_default();
                out.meta = out.status.clone();
            }
            None => {
                out.right = words.busy_mark.clone();
                out.status = words.preparing.clone();
            }
        }
        if let Some(busy) = sample.and_then(|s| s.busy.clone()) {
            out.right = words.busy_mark.clone();
            out.meta = busy.clone();
            out.status = busy;
        }
        if self.cancelling() {
            out.right = words.busy_mark.clone();
            if self.spec.transfer {
                out.meta = words.waiting_for_device.clone();
                out.status = words.waiting_for_device.clone();
            }
        }
        if ended {
            out.right = match self.ended() {
                Some(EndKind::Complete) => (words.percent)(100),
                _ => String::new(),
            };
            out.meta.clear();
        }
        out
    }

    /// What the views read: Strata's `sync_compact` and the dialog's labels.
    pub(crate) fn derive(&self, words: &Words) -> Shown {
        let labels = self.labels(words);
        let transfer = matches!(
            self.sample.as_ref().map(|s| s.progress),
            Some(Progress::Transfer(_))
        );
        let current = self
            .sample
            .as_ref()
            .and_then(|s| s.current.clone())
            .filter(|c| !c.is_empty());
        let info = if self.cancelling() && self.spec.transfer {
            words.still_writing.clone()
        } else {
            current.clone().unwrap_or_else(|| self.spec.info.clone())
        };
        let items_line = match self.sample.as_ref().map(|s| s.progress) {
            Some(Progress::Transfer(t)) => {
                let items = match transfer_count(&t) {
                    Some((done, total, files)) => (words.of_total)(done, total, files),
                    None => words.preparing_items.clone(),
                };
                Some(match &current {
                    Some(name) => (words.with_current)(&items, name),
                    None => items,
                })
            }
            _ => None,
        };
        let fraction = self.fraction();
        let ended = match &self.phase {
            Phase::Ended { kind, countdown } => Some(EndShown {
                kind: *kind,
                pinned: countdown.pinned(),
                words: if countdown.pinned() {
                    words.pinned.clone()
                } else if countdown.paused() {
                    words.paused.clone()
                } else {
                    (words.closing_in)(&(words.duration)(countdown.left().ceil() as u64))
                },
                left: (countdown.left() / countdown.duration().max(EPS)).clamp(0.0, 1.0) as f32,
                paused: countdown.paused(),
                leaving: countdown.leaving(),
            }),
            _ => None,
        };
        let subtitle = match &self.phase {
            Phase::Cancelling { stalled: true, .. } => Some(words.not_responding_detail.clone()),
            Phase::Cancelling { .. } if self.spec.transfer => Some(words.waiting_for_write.clone()),
            _ => self.spec.subtitle.clone(),
        };
        Shown {
            icon: Some(self.spec.icon),
            title: self.title.clone(),
            info,
            count: labels.count.clone(),
            fraction,
            value: fraction
                .map(|_| match self.ended() {
                    Some(EndKind::Complete) => (words.percent)(100),
                    _ => labels.right.clone(),
                })
                .filter(|v| !v.is_empty()),
            description: if ended.is_some() {
                String::new()
            } else {
                labels.meta.clone()
            },
            items_line,
            transfer,
            bytes: transfer.then(|| labels.bytes.clone()),
            subtitle,
            cancel_enabled: self.running(),
            cancel_words: if self.running() {
                words.cancel.clone()
            } else {
                words.cancel_requested.clone()
            },
            ended,
        }
    }
}

#[derive(Default)]
struct Labels {
    /// The card's right label: a percentage or "…".
    right: String,
    /// The dialog's status: the percentage, or "Preparing…" while the bar pulses.
    status: String,
    meta: String,
    count: Option<String>,
    bytes: String,
}

/// The count a transfer shows: files once their total is known, else items; `None` while
/// it still prepares (Strata's `transfer_count`). The flag is true for files.
fn transfer_count(t: &Transfer) -> Option<(usize, usize, bool)> {
    match t.total_files.filter(|&n| n > 0) {
        Some(total) => Some((t.completed_files, total, true)),
        None => (t.total_items > 0).then_some((t.completed_items, t.total_items, false)),
    }
}

/// A transfer's fraction (Strata's `transfer_fraction_status`): by bytes once their total is
/// known, by items for a tree of empty files, `None` (pulsing) before either.
fn transfer_fraction(t: &Transfer) -> Option<f64> {
    match t.total_bytes {
        Some(0) if t.total_items > 0 => {
            Some((t.completed_items as f64 / t.total_items as f64).clamp(0.0, 1.0))
        }
        Some(0) | None => None,
        Some(total) => Some((t.bytes as f64 / total as f64).clamp(0.0, 1.0)),
    }
}

/// The dialog's status words and the fraction behind them.
fn transfer_status(t: &Transfer, words: &Words) -> (String, Option<f64>) {
    let fraction = transfer_fraction(t);
    let status = match (fraction, t.total_bytes) {
        (Some(f), Some(0)) => (words.percent)(format::whole_percent(f, false)),
        (Some(f), _) => (words.percent)(format::whole_percent(f, t.bytes > 0)),
        (None, _) if t.bytes == 0 && t.completed_items == 0 => words.preparing.clone(),
        (None, _) => words.transferring.clone(),
    };
    (status, fraction)
}

/// The speed with the time left: "Calculating speed…" until a rate is measured (Strata's
/// `transfer_rate_status`).
fn rate_status(rate: Option<f64>, bytes: u64, total: Option<u64>, words: &Words) -> String {
    let Some(r) = rate.filter(|r| r.is_finite() && *r > 0.0) else {
        return words.calculating_speed.clone();
    };
    let speed = (words.rate)(r as u64);
    match format::seconds_left(Some(r), total.and_then(|t| t.checked_sub(bytes))) {
        Some(secs) => (words.speed_left)(&speed, &(words.duration)(secs)),
        None => speed,
    }
}
