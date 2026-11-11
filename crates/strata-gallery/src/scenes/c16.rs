//! The gallery page for C16 ProgressDock: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c16-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C16-progress-dock.md`.
//!
//! The page draws a stand-in window: a header, a listing with a focusable cursor row, buttons
//! that start fake operations and notices, a probe button that proves the window stays usable
//! while cards show, and a stand-in footer with the Jobs button (C19's `Footer` takes its
//! place once C19 merges). Fake operations are workers on the virtual clock: each poll turns
//! the time since they started into a progress sample, so goldens and scripts see the same
//! numbers every run.

use rune::{Cx, El, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::Button;
use strata_components::dialog::{Focus, StrataDialog};
use strata_components::progress_dock::jobs::{JobRow, JobStatus, Jobs, JobsIntent, JobsState};
use strata_components::progress_dock::{
    Dock, DockIntent, DockView, Notice, OpId, OpSpec, Placement, Progress, ProgressDialog,
    REVEAL_DELAY, Sample, Transfer,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// How long a fake operation takes to stop after Cancel.
const STOP_AFTER: f64 = 0.5;

/// How long a fake job takes to stop after Cancel: long enough for a script's final tree,
/// a second after its last event, to see it "Cancelling…".
const JOB_STOP_AFTER: f64 = 2.0;

/// What the page does when a cancelled operation has stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CancelPath {
    /// Strata's: the card closes and an "Operation cancelled" dialog gives the counts.
    #[default]
    Dialog,
    /// The WP's: the card turns into a notice that closes itself.
    Notice,
}

/// A fake operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A copy of `files` files, `total` bytes at `rate` bytes a second.
    Copy { total: u64, files: usize, rate: f64 },
    /// A compression of `items` items, one every `per` seconds.
    Compress { items: usize, per: f64 },
    /// A one-item permanent delete counting `rate` entries a second, `total` in all.
    DeleteOne { rate: f64, total: usize },
}

/// A fake worker: its kind, when it started (stamped by the first poll), and how it ends.
#[derive(Clone, Debug)]
pub struct Fake {
    pub id: OpId,
    pub kind: Kind,
    pub started: Option<f64>,
    /// Cancel was asked at this time.
    pub cancel_at: Option<f64>,
    /// Never stops after Cancel: a stalled device.
    pub stuck: bool,
    /// Stops moving at this fraction: a golden holds it there.
    pub hold: Option<f64>,
}

/// C16's demo state, held in `Gallery::c16`.
#[derive(Default)]
pub struct Demo {
    pub dock: Dock,
    pub fakes: Vec<Fake>,
    pub next_id: OpId,
    pub path: CancelPath,
    /// The blocking dialog's operation.
    pub dialog: Option<OpId>,
    /// Strata's "Operation cancelled" dialog, with its summary.
    pub cancelled: Option<String>,
    pub jobs: Vec<JobRow>,
    pub jobs_ui: JobsState,
    /// What the page heard last: "Cancel 1", "Dismissed 2", "Jobs: Clear finished", …
    pub heard: String,
    /// Presses on the probe button.
    pub probes: u32,
    pub now: f64,
    /// The window's animation clock scale (`Cx::time_scale`): a host that froze its motion.
    pub time_scale: Option<f32>,
}

fn demo(g: &mut Gallery) -> &mut Demo {
    &mut g.c16
}

fn dock(g: &Gallery) -> &Dock {
    &g.c16.dock
}

fn dock_mut(g: &mut Gallery) -> &mut Dock {
    &mut g.c16.dock
}

fn photos(total: u64, files: usize, rate: f64) -> Kind {
    Kind::Copy { total, files, rate }
}

impl Demo {
    fn add(&mut self, spec: OpSpec, kind: Kind) -> OpId {
        self.next_id += 1;
        let id = self.next_id;
        let mut tx = Tx::default();
        let placement = spec.placement;
        self.dock.start(id, spec, &mut tx);
        if placement == Placement::Dialog {
            self.dialog = Some(id);
        }
        self.fakes.push(Fake {
            id,
            kind,
            started: None,
            cancel_at: None,
            stuck: false,
            hold: None,
        });
        id
    }

    /// A copy of the holiday photos into Pictures: 40 MB in 10 files at 8 MB/s.
    pub fn copy(&mut self) -> OpId {
        let mut spec = OpSpec::new(IconName::Copy, "Copying items", "Holiday photos");
        spec.transfer = true;
        spec.destination = Some("→ Pictures".into());
        spec.destination_description = Some("/home/ada/Pictures".into());
        spec.cancel_label = "Cancel copying".into();
        self.add(spec, photos(40_000_000, 10, 8_000_000.0))
    }

    /// A move in the blocking dialog: 120 MB in 30 files at 20 MB/s.
    pub fn move_in_dialog(&mut self) -> OpId {
        let mut spec = OpSpec::new(IconName::Folder, "Moving items", "Raw footage");
        spec.transfer = true;
        spec.placement = Placement::Dialog;
        spec.subtitle = Some("Cancelling will not undo completed changes".into());
        spec.cancel_label = "Cancel moving".into();
        self.add(spec, photos(120_000_000, 30, 20_000_000.0))
    }

    /// A compression of two items into photos.zip.
    pub fn compress(&mut self) -> OpId {
        let mut spec = OpSpec::new(IconName::Archive, "Compressing items", "photos.zip");
        spec.destination = Some("→ Documents".into());
        spec.cancel_label = "Cancel compressing".into();
        self.add(spec, Kind::Compress { items: 2, per: 1.6 })
    }

    /// A permanent delete of one large folder.
    pub fn delete_one(&mut self) -> OpId {
        let mut spec = OpSpec::new(IconName::Trash, "Deleting items", "node_modules");
        spec.destination = Some("Permanent deletion".into());
        spec.cancel_label = "Cancel deleting".into();
        self.add(
            spec,
            Kind::DeleteOne {
                rate: 90_000.0,
                total: 600_000,
            },
        )
    }

    /// A small copy of three files: it waits 350 ms before it shows, and is done in `secs`.
    pub fn small(&mut self, secs: f64) -> OpId {
        let mut spec = OpSpec::new(IconName::Copy, "Copying items", "notes.md");
        spec.transfer = true;
        spec.reveal_after = REVEAL_DELAY;
        spec.destination = Some("→ Documents".into());
        self.add(spec, photos(3_000_000, 3, 3_000_000.0 / secs))
    }

    fn fake(&mut self, id: OpId) -> Option<&mut Fake> {
        self.fakes.iter_mut().find(|f| f.id == id)
    }

    /// Operation `id` never stops after Cancel: a stalled device.
    fn stick(&mut self, id: OpId) {
        if let Some(f) = self.fake(id) {
            f.stuck = true;
        }
    }

    fn hear(&mut self, intent: DockIntent, now: f64) {
        match intent {
            DockIntent::Cancel(id) => {
                self.heard = format!("Cancel {id}");
                if let Some(f) = self.fake(id) {
                    f.cancel_at = Some(now);
                }
            }
            DockIntent::Dismissed(id) => self.heard = format!("Dismissed {id}"),
        }
    }

    fn hear_jobs(&mut self, intent: JobsIntent) {
        self.heard = format!("Jobs: {intent:?}");
        let now = self.now;
        match intent {
            JobsIntent::Cancel(id) | JobsIntent::Remove(id) => {
                if let Some(r) = self.jobs.iter_mut().find(|r| r.id == id) {
                    if r.status == JobStatus::Queued {
                        r.status = JobStatus::Cancelled;
                        r.ended_at = Some(r.started_at);
                        r.message = Some("Removed before starting".into());
                    } else {
                        r.status = JobStatus::Cancelling;
                    }
                }
            }
            JobsIntent::Dismiss(id) => self.jobs.retain(|r| r.id != id),
            JobsIntent::ClearFinished => self.jobs.retain(|r| !r.status.is_finished()),
        }
        let _ = now;
    }

    /// Fake workers one poll on: samples, completions, and the cancels that have stopped.
    fn step(&mut self, t: f64, tx: &mut Tx) -> bool {
        let mut changed = false;
        let mut ended = Vec::new();
        let mut running = false;
        for f in &mut self.fakes {
            let start = *f.started.get_or_insert(t);
            let elapsed = t - start;
            if let Some(at) = f.cancel_at {
                if !f.stuck && t - at >= STOP_AFTER {
                    ended.push((f.id, false));
                }
                running |= !f.stuck;
                continue;
            }
            let (sample, done) = sample(&f.kind, elapsed, f.hold);
            changed |= self.dock.progress(f.id, sample, t, tx);
            if done {
                ended.push((f.id, true));
            } else {
                running = true;
            }
        }
        for (id, completed) in ended {
            self.fakes.retain(|f| f.id != id);
            changed = true;
            if completed {
                let title = match self.dock.op(id).map(|o| o.spec.icon) {
                    Some(IconName::Archive) => "Compression complete",
                    Some(IconName::Trash) => "Deletion complete",
                    _ => "Copy complete",
                };
                self.dock.complete(id, title, tx);
            } else {
                let summary = "3 items completed, 0 failed, 7 not attempted.\n\nCompleted \
                               changes were not reverted.";
                match self.path {
                    CancelPath::Dialog => {
                        self.dock.remove(id);
                        self.cancelled = Some(summary.into());
                    }
                    CancelPath::Notice => {
                        self.dock.close_as_notice(
                            id,
                            "Operation cancelled",
                            "3 items completed, 7 not attempted",
                            4.0,
                            tx,
                        );
                    }
                }
            }
            if self.dialog == Some(id) {
                self.dialog = None;
            }
        }
        // A real worker reports as it goes; the fake one polls each frame while it runs.
        if running {
            tx.wake_after(std::time::Duration::ZERO);
        }
        changed
    }
}

/// A fake worker's sample `elapsed` seconds in, and whether it is done.
fn sample(kind: &Kind, elapsed: f64, hold: Option<f64>) -> (Sample, bool) {
    let cap = |f: f64| hold.map_or(f, |h| f.min(h)).clamp(0.0, 1.0);
    match *kind {
        Kind::Copy { total, files, rate } => {
            let f = cap(elapsed * rate / total as f64);
            let bytes = (f * total as f64) as u64;
            let done_files = ((f * files as f64).floor() as usize).min(files);
            let current = format!("IMG_{:04}.jpg", (done_files + 1).min(files));
            let s = Sample::new(Progress::Transfer(Transfer {
                completed_items: if f >= 1.0 { 1 } else { 0 },
                total_items: 1,
                completed_files: done_files,
                total_files: Some(files),
                bytes,
                total_bytes: Some(total),
            }))
            .current(current);
            (s, f >= 1.0)
        }
        Kind::Compress { items, per } => {
            let f = cap(elapsed / (per * items as f64));
            let done = ((f * items as f64).floor() as usize).min(items);
            let s = Sample::new(Progress::Items { done, total: items })
                .status(format!("{} / {items} files", (done + 1).min(items)));
            (s, f >= 1.0)
        }
        Kind::DeleteOne { rate, total } => {
            let f = cap(elapsed * rate / total as f64);
            let n = (f * total as f64) as u64;
            let s = Sample::new(Progress::Count(n as usize)).status(format!(
                "{} items deleted",
                strata_components::progress_dock::format::integer(n)
            ));
            (s, f >= 1.0)
        }
    }
}

/// The jobs the dashboard scenes list: one running per item, one queued, two done (one with
/// output, one without), one failed.
fn sample_jobs() -> Vec<JobRow> {
    let mut resize = JobRow::new(4, "Resize images", JobStatus::Running, -12.4);
    resize.icon = IconName::Pictures;
    resize.items = Some((4, 10));
    resize.fraction = Some(0.45);
    resize.folder = "~/Pictures".into();
    resize.output = "resized IMG_0001.jpg\nresized IMG_0002.jpg\n".into();
    let mut rename = JobRow::new(3, "Batch rename", JobStatus::Queued, 0.0);
    rename.icon = IconName::Rename;
    rename.folder = "~/Documents".into();
    let mut sums = JobRow::new(2, "SHA-256 checksums", JobStatus::Succeeded, -95.0);
    sums.ended_at = Some(-30.0);
    sums.folder = "~/Downloads".into();
    sums.output =
        "\u{1b}[32mok\u{1b}[0m iso.img.sha256\n\u{1b}[32mok\u{1b}[0m notes.txt.sha256".into();
    let mut terminal = JobRow::new(5, "Open terminal here", JobStatus::Succeeded, -60.0);
    terminal.ended_at = Some(-59.0);
    terminal.folder = "~/Projects".into();
    let mut strip = JobRow::new(1, "Strip EXIF", JobStatus::Failed, -200.0);
    strip.ended_at = Some(-190.0);
    strip.folder = "Home".into();
    strip.error = Some("exiftool was not found".into());
    vec![resize, rename, terminal, sums, strip]
}

macro_rules! pair {
    ($name:literal, $f:expr) => {
        (concat!($name, "-light"), |g: &mut Gallery| {
            g.open("c16", false);
            ($f)(g)
        })
    };
    ($name:literal, $f:expr, dark) => {
        (concat!($name, "-dark"), |g: &mut Gallery| {
            g.open("c16", true);
            ($f)(g)
        })
    };
}

fn card(g: &mut Gallery) {
    let d = demo(g);
    d.copy();
}

fn stack(g: &mut Gallery) {
    let d = demo(g);
    d.compress();
    d.delete_one();
}

fn cancelling(g: &mut Gallery) {
    let d = demo(g);
    let id = d.copy();
    d.stick(id);
    let mut tx = Tx::default();
    d.dock.progress(
        id,
        sample(&photos(40_000_000, 10, 8_000_000.0), 2.0, None).0,
        0.0,
        &mut tx,
    );
    d.dock.cancel(id, &mut tx);
    if let Some(f) = d.fake(id) {
        f.cancel_at = Some(0.0);
    }
}

fn complete(g: &mut Gallery) {
    let d = demo(g);
    let mut tx = Tx::default();
    for _ in 0..2 {
        let id = d.copy();
        d.fakes.retain(|f| f.id != id);
        d.dock.progress(
            id,
            sample(&photos(40_000_000, 10, 8_000_000.0), 2.0, None).0,
            0.0,
            &mut tx,
        );
        d.dock.complete(id, "Copy complete", &mut tx);
    }
    d.dock.toggle_pin(d.next_id, &mut tx);
}

fn notices(g: &mut Gallery) {
    let d = demo(g);
    d.compress();
    let mut tx = Tx::default();
    d.dock.notice(
        Notice::new(
            "The provider accepted 12 of 12 files for upload. They will appear in your cloud \
             folder once the transfer on the provider's side has finished, which can take a \
             while for large files on a slow connection.",
        )
        .heading("Nextcloud")
        .shown_for(60.0),
        &mut tx,
    );
    d.dock.notice(
        Notice::new("Text size: 16px")
            .status()
            .joins()
            .shown_for(60.0),
        &mut tx,
    );
}

fn dialog(g: &mut Gallery) {
    demo(g).move_in_dialog();
}

fn jobs(g: &mut Gallery) {
    let d = demo(g);
    d.jobs = sample_jobs();
    let mut tx = Tx::default();
    d.jobs_ui.open(&mut tx);
    d.jobs_ui.toggle_details(2);
}

fn jobs_closed(g: &mut Gallery) {
    demo(g).jobs = sample_jobs();
}

fn idle(_g: &mut Gallery) {}

/// Eight cards that differ: copies, compressions and deletes, each started 0.6 s before the
/// next, so each shows its own progress.
fn many(g: &mut Gallery) {
    let d = demo(g);
    for i in 0..8 {
        let id = match i % 3 {
            0 => d.copy(),
            1 => d.compress(),
            _ => d.delete_one(),
        };
        if let Some(f) = d.fake(id) {
            f.started = Some(-0.6 * f64::from(8 - i));
        }
    }
}

fn jobs_finished(g: &mut Gallery) {
    let d = demo(g);
    d.jobs = sample_jobs();
    d.jobs.retain(|r| r.status.is_finished());
    let mut tx = Tx::default();
    d.jobs_ui.open(&mut tx);
}

fn many_jobs(g: &mut Gallery) {
    let d = demo(g);
    d.jobs = (1..=15)
        .map(|i| {
            let mut r = JobRow::new(i, format!("Job {i}"), JobStatus::Succeeded, -10.0);
            r.ended_at = Some(-5.0);
            r
        })
        .collect();
    let mut tx = Tx::default();
    d.jobs_ui.open(&mut tx);
}

fn cancel_notice(g: &mut Gallery) {
    let d = demo(g);
    d.path = CancelPath::Notice;
    d.copy();
}

fn dialog_preparing(g: &mut Gallery) {
    let d = demo(g);
    let id = d.move_in_dialog();
    d.fakes.retain(|f| f.id != id);
    let mut tx = Tx::default();
    d.dock.progress(
        id,
        Sample::new(Progress::Transfer(Transfer::default())),
        0.0,
        &mut tx,
    );
}

fn reduced(g: &mut Gallery) {
    g.look.animations = false;
    g.c16.dock.set_animations(false);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c16", dark)`, then fills in `g.c16`.
pub const SCENES: &[Scene] = &[
    pair!("c16-card", card),
    pair!("c16-card", card, dark),
    pair!("c16-stack", stack),
    pair!("c16-stack", stack, dark),
    pair!("c16-cancelling", cancelling),
    pair!("c16-cancelling", cancelling, dark),
    pair!("c16-complete", complete),
    pair!("c16-complete", complete, dark),
    pair!("c16-notices", notices),
    pair!("c16-notices", notices, dark),
    pair!("c16-dialog", dialog),
    pair!("c16-dialog", dialog, dark),
    pair!("c16-jobs", jobs),
    pair!("c16-jobs", jobs, dark),
];

/// Scenes for review with `--scene` only, kept out of the goldens. Scripts start from these
/// too: an idle window with the openers, and the jobs button with its list closed.
pub const REVIEW: &[Scene] = &[
    pair!("c16-idle", idle),
    pair!("c16-idle", idle, dark),
    pair!("c16-jobs-closed", jobs_closed),
    pair!("c16-jobs-closed", jobs_closed, dark),
    pair!("c16-many", many),
    pair!("c16-many", many, dark),
    pair!("c16-jobs-finished", jobs_finished),
    pair!("c16-jobs-finished", jobs_finished, dark),
    pair!("c16-jobs-many", many_jobs),
    pair!("c16-jobs-many", many_jobs, dark),
    pair!("c16-cancel-notice", cancel_notice),
    pair!("c16-cancel-notice", cancel_notice, dark),
    pair!("c16-dialog-preparing", dialog_preparing),
    pair!("c16-dialog-preparing", dialog_preparing, dark),
    pair!("c16-reduced", |g: &mut Gallery| {
        card(g);
        reduced(g)
    }),
    pair!(
        "c16-reduced",
        |g: &mut Gallery| {
            card(g);
            reduced(g)
        },
        dark
    ),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    if let Some(k) = g.c16.time_scale {
        cx.time_scale(k);
    }
    let look = g.look();
    let mut layer = Vec::new();
    let jobs = Jobs::new(
        "c16-jobs",
        |g: &Gallery| &g.c16.jobs_ui,
        |g: &mut Gallery| &mut g.c16.jobs_ui,
        |g: &Gallery| &g.c16.jobs,
    )
    .on_intent(|g: &mut Gallery, intent, _tx: &mut Tx| g.c16.hear_jobs(intent))
    .return_focus("c16-cursor")
    .build(g, look, &mut layer);
    let base = window(g, look, jobs);
    DockView::new("c16-dock", dock, dock_mut)
        .on_intent(|g: &mut Gallery, intent, _tx: &mut Tx| {
            let now = g.c16.now;
            g.c16.hear(intent, now)
        })
        .build(g, look, &mut layer);
    if let Some(id) = g.c16.dialog {
        ProgressDialog::new("c16-progress", dock, dock_mut, id)
            .return_focus("c16-cursor")
            .on_intent(|g: &mut Gallery, intent| {
                let now = g.c16.now;
                g.c16.hear(intent, now)
            })
            .build(g, look, &mut layer);
    }
    if let Some(summary) = &g.c16.cancelled {
        StrataDialog::new("c16-cancelled", "Operation cancelled", |g: &mut Gallery| {
            g.c16.cancelled = None
        })
        .open(true)
        .icon(IconName::Info)
        .message(summary.clone())
        .primary("Close", |g: &mut Gallery| g.c16.cancelled = None)
        .initial_focus(Focus::Primary)
        .return_focus("c16-cursor")
        .build(look, &mut layer);
    }
    rune_kit::root(base, layer)
}

fn window(g: &Gallery, look: &Look, jobs: Option<El>) -> El {
    let k = &look.kit;
    let space = look.space;
    let d = &g.c16;
    let header = row()
        .w_full()
        .h(look.frame.bar)
        .px(space.lg)
        .gap(space.md)
        .center_y()
        .fill(k.c.bg_deep)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
        .child(text("Pictures").font(look.ty.title).color(k.c.text_base));
    let names = [
        "Holiday photos",
        "Raw footage",
        "notes.md",
        "photos.zip",
        "node_modules",
    ];
    let listing = col()
        .gap(space.xxs)
        .padding(space.lg)
        .children(names.iter().enumerate().map(|(i, name)| {
            let r = row()
                .h(look.row.list)
                .px(space.md)
                .gap(space.md)
                .center_y()
                .radius(k.radius.sm)
                .child(icon(look.icon(IconName::Folder), look.icon_size.md).color(k.c.icon_muted))
                .child(text(*name).font(k.ty.ui).color(k.c.text_base));
            if i == 0 {
                r.key("c16-cursor")
                    .role("button")
                    .label(*name)
                    .focusable(true)
                    .fill(k.c.layer_02)
            } else {
                r
            }
        }));
    let opener = |key: &str, label: &str, f: fn(&mut Demo)| {
        Button::new(label)
            .on_press(move |g: &mut Gallery| f(&mut g.c16))
            .build(k)
            .key(format!("c16-start-{key}"))
    };
    let openers = row()
        .flex_wrap()
        .gap(space.md)
        .padding(space.lg)
        .child(opener("copy", "Copy", |d| {
            d.copy();
        }))
        .child(opener("move", "Move (dialog)", |d| {
            d.move_in_dialog();
        }))
        .child(opener("move-stuck", "Move from a stalled device", |d| {
            let id = d.move_in_dialog();
            d.stick(id);
        }))
        .child(opener("compress", "Compress", |d| {
            d.compress();
        }))
        .child(opener("delete", "Delete one folder", |d| {
            d.delete_one();
        }))
        .child(opener("delete-stuck", "Delete on a stalled device", |d| {
            let id = d.delete_one();
            d.stick(id);
        }))
        .child(opener("small-fast", "Small copy, 0.2 s", |d| {
            d.small(0.2);
        }))
        .child(opener("small-slow", "Small copy, 2 s", |d| {
            d.small(2.0);
        }))
        .child(opener("stuck", "Copy to a stalled device", |d| {
            let id = d.copy();
            d.stick(id);
        }))
        .child(opener("notice", "Notice", |d| {
            let mut tx = Tx::default();
            d.dock.notice(
                Notice::new("Sent to Pixel 8").heading("KDE Connect"),
                &mut tx,
            );
        }))
        .child(opener("notice-2", "Another notice", |d| {
            let mut tx = Tx::default();
            d.dock.notice(Notice::new("Uploaded to Nextcloud"), &mut tx);
        }))
        .child(opener("report", "Transfer report", |d| {
            let mut tx = Tx::default();
            d.dock.notice(
                Notice::new("2 items were renamed to fit the device").joins(),
                &mut tx,
            );
        }))
        .child(opener("job", "Run an action", |d| {
            let id = 100 + d.jobs.len() as u64;
            let mut r = JobRow::new(id, "Resize images", JobStatus::Running, d.now);
            r.folder = "~/Pictures".into();
            d.jobs.insert(0, r);
            let mut tx = Tx::default();
            d.jobs_ui.open(&mut tx);
        }))
        .child(
            Button::new("Probe")
                .on_press(|g: &mut Gallery| g.c16.probes += 1)
                .build(k)
                .key("c16-probe"),
        );
    let status = text(format!("{} · probes {}", d.heard, d.probes))
        .key("c16-heard")
        .label(format!("{} · probes {}", d.heard, d.probes))
        .font(k.ty.small)
        .color(k.c.text_muted)
        .px(space.lg);
    // C19's footer, until it merges: its height (frame.control + 2·space.xs) and its fill.
    let footer = row()
        .key("c16-footer")
        .w_full()
        .h(look.frame.control + 2.0 * space.xs)
        .py(space.xs)
        .pl(space.xs)
        .pr(space.lg)
        .gap(space.lg)
        .items_center()
        .fill(k.c.bg_deep)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base)
        .child(
            text("5 items")
                .font(k.ty.small)
                .color(k.c.text_base)
                .pl(space.sm),
        )
        .child(el().grow(1.0).min_w(0.0))
        .children(jobs);
    col()
        .w_full()
        .h_full()
        .child(header)
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .items_start()
                .child(listing)
                .child(col().grow(1.0).child(openers).child(status)),
        )
        .child(footer)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds: the
/// fake workers report, then the dock's and the dashboard's timers run.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = demo(g);
    d.now = t;
    let mut changed = d.step(t, tx);
    changed |= d.dock.tick(t, tx);
    changed |= d.jobs_ui.tick(t, &d.jobs, tx);
    // A cancelled job stops JOB_STOP_AFTER after Cancel.
    for r in &mut d.jobs {
        if r.status == JobStatus::Cancelling {
            let since = *r.ended_at.get_or_insert(t + JOB_STOP_AFTER);
            if t >= since {
                r.status = JobStatus::Cancelled;
                changed = true;
            } else {
                tx.wake_after(std::time::Duration::from_secs_f64(since - t));
            }
        }
    }
    changed
}
