//! The gallery page for C25 ThumbnailStore: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c25-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C25-thumbnail-store.md`.
//!
//! The page is a folder of 600 entries in a C1e list whose icons are C25 thumbnails: images,
//! a transparent model render, files that fail to render, text files and folders (which have
//! no thumbnail key). A fake worker on the virtual clock renders what the store asks for:
//! four lanes, each job due after a delay fixed by its key, at most eight results handed over
//! a frame, so goldens and scripts see the same results every run. A job within 50 ms of done
//! when it is cancelled can't be stopped, and its late result reaches the store, which drops
//! it.
//!
//! Thumbnails land with no view commit. The status lines above the list are built from the
//! state, so they change only when the view runs: the page runs it once each time the worker
//! goes idle, never per thumbnail. They are what the scripts read:
//!
//! | Key | Says |
//! | --- | --- |
//! | `c25-status` | What the rows on screen show, and the bytes held against the budget |
//! | `c25-order` | The first requests since the worker last went idle, and the direction |
//! | `c25-span` | The rows asked for since then, around the rows on screen |
//! | `c25-work` | Totals (renders, failures, cancellations, late results, stops), and the |
//! | | view's runs while the last thumbnails landed |
//! | `c25-snapshot` | `c25-status` and `c25-work` as S last copied them (two moments a script) |
//!
//! | Key | Does |
//! | --- | --- |
//! | T | Show thumbnails off, or on |
//! | N | Rename the first image on screen, keeping its content: its thumbnail moves with it |
//! | O | A file operation starts: renders in progress end stopped, and nothing more renders |
//! | P | The file operation ends: stopped thumbnails are asked for again |
//! | F | Filter to images only, or back |
//! | S | Refresh the status lines now, and copy them to the snapshot line |
//!
//! `STRATA_C25_BUDGET_WARMUP=1` makes the page's second poll wait 1.1 s of real time once,
//! then print [`WARMED`] on stderr, as C1's page does: Rune's `RUNE_COMMIT_BUDGET_MS` prints at
//! most one line a real second (Rune draft U-C1e-5), so the 100k scene's over-budget first
//! mount would hide the lines of a fast scripted scroll. Only the hand run of the commit
//! budget sets it (the module docs of `strata_components::thumbnails`).

use std::cell::Cell;

use rune::{Cx, El, Key, KeyEvent, Tx, col, row, text};
use strata_components::collection::element::Collection;
use strata_components::collection::element::CollectionState;
use strata_components::contract::{
    EntryId, EntryKind, EntrySource, ListEntry, ListSource, ThumbKey,
};
use strata_components::thumbnails::{
    Direction, Landed, Outcome, Request, Settings, State, Step, ThumbnailStore, ViewerId, Viewport,
    Why, icon_slot,
};

use crate::{Gallery, Scene};

/// The variable that turns on the budget warm-up (see the module docs).
pub const WARMUP: &str = "STRATA_C25_BUDGET_WARMUP";

/// What the page prints on stderr once the warm-up is over: budget lines after it are the
/// script's.
pub const WARMED: &str = "c25: budget warm-up done";

/// The list's key: its rows and scroll requests start here.
pub const SCOPE: &str = "c25-list";

/// The second viewer's list key (`c25-two-views-*`).
pub const SCOPE_B: &str = "c25-list-b";

/// The thumbnail box, in points.
const PT: f32 = 48.0;
/// A row: the box and its padding.
const ROW: f32 = 56.0;
/// Entries in the folder.
const COUNT: usize = 600;
/// Results handed to the store a frame at most.
const PER_FRAME: usize = 8;
/// A cancelled job this close to done finishes anyway: its result arrives late.
const UNSTOPPABLE: f64 = 0.05;
/// Thumbnail keys are the entry id plus this.
const KEY_BASE: u64 = 50_000;

/// How fast the fake worker renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speed {
    /// 50–200 ms a job.
    #[default]
    Fast,
    /// 500 ms – 1.4 s a job: a golden at 1 s catches it part way.
    Slow,
}

struct Job {
    due: f64,
    request: Request,
}

/// C25's demo state, held in `Gallery::c25`.
pub struct Demo {
    pub listing: ListSource,
    /// Every entry, for the filter to go back to.
    pub all: Vec<ListEntry>,
    pub coll: CollectionState,
    pub store: ThumbnailStore,
    pub viewer: ViewerId,
    /// The second list, in `c25-two-views-*`.
    pub second: Option<(CollectionState, ViewerId)>,
    /// The listing version: bumped on every change at the same length.
    pub version: u64,
    pub clock: f64,
    pub speed: Speed,
    /// The worker's lanes.
    pub lanes: usize,
    jobs: Vec<Job>,
    /// A file operation is running: renders end stopped.
    pub operation: bool,
    /// Images only.
    pub filtered: bool,
    /// The view runs the page's `view` made, counted.
    pub views: Cell<usize>,
    /// Refresh the status lines when the worker goes idle (off in the commit test).
    pub status_on_idle: bool,
    /// The worker has nothing to do.
    pub idle: bool,
    /// Polls run since the page opened, for the budget warm-up.
    polls: u32,
    /// Refresh the status lines once the list has reported its rows (a scene with nothing to
    /// load, so no idle would).
    refresh_once: bool,
    /// The view runs when the worker last went busy, and the runs that round made once it
    /// went idle.
    busy_views: usize,
    round_runs: Option<usize>,
    /// Since the worker last went busy: requests in order, and results held.
    batch: Vec<usize>,
    landed: usize,
    /// Totals.
    pub decoded: usize,
    pub failures: usize,
    pub cancelled: usize,
    pub late: usize,
    /// The entry last renamed, whether its thumbnail moved, and the renders before it.
    renamed: Option<(String, bool, usize)>,
    /// The status lines, as last refreshed.
    pub status: [String; 4],
    pub snapshot: String,
}

impl Default for Demo {
    fn default() -> Self {
        Self::new(Settings::default().budget(8 * 1024 * 1024))
    }
}

/// The folder: images in four shapes, a model render, failing files, text files, folders.
fn folder(count: usize) -> Vec<ListEntry> {
    (0..count)
        .map(|i| {
            let id = EntryId::new(i as u64 + 1);
            let n = i + 1;
            let (name, kind, icon, keyed) = if i % 10 == 5 {
                (format!("Album {n:04}"), EntryKind::Folder, "folder", false)
            } else if i == 4 {
                ("model.stl".to_owned(), EntryKind::File, "file", true)
            } else if i % 37 == 7 {
                (
                    format!("broken_{n:04}.heic"),
                    EntryKind::File,
                    "file_image",
                    true,
                )
            } else if i % 23 == 2 {
                (
                    format!("notes_{n:04}.txt"),
                    EntryKind::File,
                    "file_text",
                    false,
                )
            } else {
                (
                    format!("IMG_{n:04}.jpg"),
                    EntryKind::File,
                    "file_image",
                    true,
                )
            };
            let mut e = ListEntry::new(id, name, kind, icon);
            if keyed {
                e.thumbnail = Some(ThumbKey::new(KEY_BASE + id.get()));
            }
            e
        })
        .collect()
}

impl Demo {
    fn new(settings: Settings) -> Self {
        Self::with(settings, COUNT)
    }

    fn with(settings: Settings, count: usize) -> Self {
        let mut store = ThumbnailStore::new(settings);
        let viewer = store.viewer();
        let all = folder(count);
        Self {
            listing: ListSource::new(all.clone()),
            all,
            coll: CollectionState::new(),
            store,
            viewer,
            second: None,
            version: 1,
            clock: 0.0,
            speed: Speed::Fast,
            lanes: 4,
            jobs: Vec::new(),
            operation: false,
            filtered: false,
            views: Cell::new(0),
            status_on_idle: true,
            idle: true,
            refresh_once: false,
            polls: 0,
            busy_views: 0,
            round_runs: None,
            batch: Vec::new(),
            landed: 0,
            decoded: 0,
            failures: 0,
            cancelled: 0,
            late: 0,
            renamed: None,
            status: [
                "Loading…".to_owned(),
                String::new(),
                String::new(),
                String::new(),
            ],
            snapshot: String::new(),
        }
    }

    fn delay(&self, key: ThumbKey) -> f64 {
        let h = key.get().wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 54; // 0..1024
        let f = h as f64 / 1023.0;
        match self.speed {
            Speed::Fast => 0.05 + 0.15 * f,
            Speed::Slow => 0.5 + 0.9 * f,
        }
    }

    fn name_of_key(&self, key: ThumbKey) -> Option<String> {
        let i = self.index_of_key(key)?;
        Some(self.listing.entry(i).name.to_owned())
    }

    fn index_of_key(&self, key: ThumbKey) -> Option<usize> {
        let id = EntryId::new(key.get().checked_sub(KEY_BASE)?);
        self.listing
            .index_of(id)
            .filter(|&i| self.listing.entry(i).thumbnail == Some(key))
            .or_else(|| {
                // A renamed entry keeps its id and gets a new key.
                (0..self.listing.len()).find(|&i| self.listing.entry(i).thumbnail == Some(key))
            })
    }

    /// The worker's render of `r`: what the file is decides it.
    fn render(&self, r: &Request) -> Outcome {
        if self.operation {
            return Outcome::Stopped;
        }
        let name = self.name_of_key(r.key).unwrap_or_default();
        if name.starts_with("broken_") {
            return Outcome::Failed;
        }
        let px = r.step.px();
        let seed = r.key.get();
        let (w, h) = match (name.as_str(), seed % 4) {
            ("model.stl", _) => (px, px),
            (_, 0) => (px, px),
            (_, 1) => (px, px * 3 / 4),
            (_, 2) => (px * 3 / 4, px),
            _ => (px, px / 2),
        };
        Outcome::Ready {
            width: w,
            height: h,
            rgba: picture(w, h, seed, name == "model.stl"),
        }
    }

    /// The worker's frame: drop cancelled jobs, hand over what is due, take new requests.
    fn work(&mut self, t: f64) {
        for ticket in self.store.take_cancelled() {
            self.cancelled += 1;
            // A job about to finish can't be stopped: its result comes late.
            self.jobs
                .retain(|j| j.request.ticket != ticket || j.due - t <= UNSTOPPABLE);
        }
        self.jobs.sort_by(|a, b| a.due.total_cmp(&b.due));
        let due = self
            .jobs
            .iter()
            .take(PER_FRAME)
            .take_while(|j| j.due <= t)
            .count();
        let done: Vec<Job> = self.jobs.drain(..due).collect();
        for job in done {
            let outcome = self.render(&job.request);
            match outcome {
                Outcome::Ready { .. } => self.decoded += 1,
                Outcome::Failed => self.failures += 1,
                Outcome::Stopped => {}
            }
            match self.store.complete(job.request.ticket, outcome, t) {
                Landed::Visible | Landed::Offscreen => self.landed += 1,
                Landed::Dropped(Why::Cancelled) => self.late += 1,
                _ => {}
            }
        }
        // Lanes stay busy with late jobs, as a worker that couldn't stop them is.
        while !self.operation && self.jobs.len() < self.lanes {
            let Some(r) = self.store.pop(t) else { break };
            if self.idle {
                // The worker goes busy: a new loading round, which the next idle reports.
                self.begin_round();
                self.idle = false;
            }
            let due = t + self.delay(r.key);
            if let Some(i) = self.index_of_key(r.key) {
                self.batch.push(i);
            }
            self.jobs.push(Job { due, request: r });
        }
    }

    fn step(&self, scale: f32, look: &strata_kit::look::Look) -> Step {
        Step::for_tile(look, PT, scale)
    }

    /// Rebuilds the status lines from the state.
    fn refresh_status(&mut self) {
        let shown = self.coll.visible_range();
        let on = self.store.enabled();
        let (mut thumbs, mut loading, mut failed, mut keyless, mut folders) = (0, 0, 0, 0, 0);
        for i in shown.clone().filter(|&i| i < self.listing.len()) {
            let e = self.listing.entry(i);
            match e.thumbnail {
                None => {
                    keyless += 1;
                    folders += usize::from(e.kind == EntryKind::Folder);
                }
                Some(k) => match self.store.state(k) {
                    State::Ready { .. } if on => thumbs += 1,
                    State::Failed { .. } => failed += 1,
                    _ => loading += 1,
                },
            }
        }
        let icons = loading + failed + keyless;
        let stats = self.store.stats();
        let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
        let held = format!(
            "{:.2} MiB of {:.2} MiB held",
            mib(stats.bytes),
            mib(stats.budget)
        );
        self.status[0] = if on {
            format!(
                "{} rows on screen: {thumbs} show thumbnails, {icons} show icons ({loading} \
                 waiting, {failed} failed, {keyless} without thumbnails, {folders} of them \
                 folders) · {held}",
                shown.len()
            )
        } else {
            let rows = shown.len();
            format!(
                "Thumbnails off · {rows} rows on screen: 0 show thumbnails, {icons} show icons \
                 · {held}"
            )
        };
        let direction = match self.store.direction(self.viewer) {
            Direction::Down => "down",
            Direction::Up => "up",
        };
        let names: Vec<String> = self
            .batch
            .iter()
            .take(6)
            .filter(|&&i| i < self.listing.len())
            .map(|&i| self.listing.entry(i).name.to_owned())
            .collect();
        self.status[1] = if names.is_empty() {
            format!("No requests since the last idle · scrolled {direction}")
        } else {
            format!(
                "First requests: {} · scrolled {direction}",
                names.join(", ")
            )
        };
        self.status[2] = match (self.batch.iter().min(), self.batch.iter().max()) {
            (Some(lo), Some(hi)) => format!(
                "Asked for rows {}–{} around rows {}–{} on screen",
                lo + 1,
                hi + 1,
                shown.start + 1,
                shown.end
            ),
            _ => format!(
                "Nothing asked for around rows {}–{} on screen",
                shown.start + 1,
                shown.end
            ),
        };
        let runs = self
            .round_runs
            .unwrap_or_else(|| self.views.get().saturating_sub(self.busy_views));
        let renamed = self
            .renamed
            .as_ref()
            .map(|(name, kept, before)| {
                let kept = if *kept { "kept" } else { "lost" };
                format!(
                    " · renamed {name}: thumbnail {kept}, {} renders since",
                    self.decoded - before
                )
            })
            .unwrap_or_default();
        self.status[3] = format!(
            "Rendered {} · failed {} · cancelled {} · late results dropped {} · stopped {} · \
             view ran {runs} times while {} thumbnails landed{renamed}",
            self.decoded, self.failures, self.cancelled, self.late, stats.stopped, self.landed
        );
    }

    /// Starts a new loading round: the counts the next idle reports start here.
    fn begin_round(&mut self) {
        self.batch.clear();
        self.landed = 0;
        self.busy_views = self.views.get();
        self.round_runs = None;
    }

    fn sync_main(&mut self, range: std::ops::Range<usize>, step: Step) -> bool {
        let view = Viewport::new(range, step).listing(self.version);
        self.store.sync(self.viewer, &self.listing, view)
    }

    fn toggle_filter(&mut self, tx: &mut Tx) {
        self.filtered = !self.filtered;
        let entries: Vec<ListEntry> = if self.filtered {
            self.all
                .iter()
                .filter(|e| e.name.starts_with("IMG_"))
                .cloned()
                .collect()
        } else {
            self.all.clone()
        };
        self.listing.replace(entries);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            strata_components::collection::selection::SyncRule::complete(
                strata_components::collection::selection::CursorGone::Neighbor,
            ),
            tx,
        );
        self.store.resync(self.viewer, &self.listing, self.version);
    }

    /// Renames the first image on screen, keeping its content, as an inline rename that
    /// keeps the extension, size and time does: the host gives it a new thumbnail key and
    /// moves the old key's thumbnail to it.
    fn rename_first(&mut self) {
        let shown = self.coll.visible_range();
        let Some(i) = shown
            .filter(|&i| i < self.listing.len())
            .find(|&i| self.listing.entry(i).name.starts_with("IMG_"))
        else {
            return;
        };
        let mut entries = self.listing.entries().to_vec();
        let e = &mut entries[i];
        let Some(from) = e.thumbnail else { return };
        let to = ThumbKey::new(from.get() + 1_000_000);
        let old = std::mem::replace(&mut e.name, format!("Renamed_{:04}.jpg", i + 1));
        e.thumbnail = Some(to);
        let id = e.id;
        if let Some(a) = self.all.iter_mut().find(|a| a.id == id) {
            *a = entries[i].clone();
        }
        self.listing.replace(entries);
        self.version += 1;
        let moved = self.store.relocate(from, to);
        self.store.resync(self.viewer, &self.listing, self.version);
        self.renamed = Some((old, moved, self.decoded));
    }
}

/// A fake thumbnail: a gradient hued by `seed`, with diagonal bands; a model render is a
/// shaded disc on a transparent background.
fn picture(w: u32, h: u32, seed: u64, transparent: bool) -> Vec<u8> {
    let hue = (seed.wrapping_mul(2_654_435_761) % 360) as f32 / 360.0 * std::f32::consts::TAU;
    let base = [hue.cos(), (hue + 2.1).cos(), (hue + 4.2).cos()];
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            if transparent {
                let (dx, dy) = (u - 0.5, v - 0.5);
                let d = (dx * dx + dy * dy).sqrt();
                if d > 0.42 {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let shade = (0.95 - d - dx * 0.6).clamp(0.2, 1.0);
                    let c = (204.0 * shade) as u8;
                    out.extend_from_slice(&[c, c, c, 255]);
                }
                continue;
            }
            let band = if ((u + v) * 6.0).fract() < 0.5 {
                1.0
            } else {
                0.85
            };
            let light = (0.45 + 0.4 * (1.0 - v)) * band;
            for c in base {
                out.push(((0.5 + 0.45 * c) * light * 255.0).clamp(0.0, 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

fn open(g: &mut Gallery, dark: bool, speed: Speed) -> &mut Demo {
    g.open("c25", dark);
    g.c25 = Demo::default();
    g.c25.speed = speed;
    &mut g.c25
}

fn off(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Speed::Fast);
    d.store.set_enabled(false);
    d.refresh_once = true;
}

fn budget(g: &mut Gallery, dark: bool) {
    g.open("c25", dark);
    g.c25 = Demo::new(Settings::default().budget(1024 * 1024));
}

fn two_views(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Speed::Fast);
    let v = d.store.viewer();
    d.second = Some((CollectionState::new(), v));
}

fn hundred_k(g: &mut Gallery, dark: bool) {
    g.open("c25", dark);
    g.c25 = Demo::with(Settings::default(), 100_000);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c25", dark)`, then fills in `g.c25`.
pub const SCENES: &[Scene] = &[
    ("c25-loading-light", |g| {
        open(g, false, Speed::Slow);
    }),
    ("c25-loading-dark", |g| {
        open(g, true, Speed::Slow);
    }),
    ("c25-filled-light", |g| {
        open(g, false, Speed::Fast);
    }),
    ("c25-filled-dark", |g| {
        open(g, true, Speed::Fast);
    }),
    ("c25-off-light", |g| off(g, false)),
    ("c25-off-dark", |g| off(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c25-budget-light", |g| budget(g, false)),
    ("c25-budget-dark", |g| budget(g, true)),
    ("c25-two-views-light", |g| two_views(g, false)),
    ("c25-two-views-dark", |g| two_views(g, true)),
    ("c25-hundred-k-light", |g| hundred_k(g, false)),
    ("c25-hundred-k-dark", |g| hundred_k(g, true)),
];

fn list(g: &Gallery, scope: &'static str, step: Step, second: bool) -> El {
    let c = if second {
        Collection::new(
            scope,
            |g: &Gallery| {
                let d = &g.c25;
                (&d.listing, &d.second.as_ref().expect("two views").0)
            },
            |g: &mut Gallery| {
                let d = &mut g.c25;
                (&d.listing, &mut d.second.as_mut().expect("two views").0)
            },
            |g: &Gallery| g.look(),
            |g: &Gallery| g.c25.version,
        )
        .on_visible(move |g: &mut Gallery, r| {
            let d = &mut g.c25;
            let v = d.second.as_ref().expect("two views").1;
            let view = Viewport::new(r, step).listing(d.version);
            d.store.sync(v, &d.listing, view)
        })
    } else {
        Collection::new(
            scope,
            |g: &Gallery| (&g.c25.listing, &g.c25.coll),
            |g: &mut Gallery| (&g.c25.listing, &mut g.c25.coll),
            |g: &Gallery| g.look(),
            |g: &Gallery| g.c25.version,
        )
        .on_visible(move |g: &mut Gallery, r| g.c25.sync_main(r, step))
    };
    c.name("Photos")
        .row_height(|_, _| ROW)
        .icon(icon_slot(|g: &Gallery| &g.c25.store, step, PT))
        .build(g)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c25;
    d.views.set(d.views.get() + 1);
    let look = g.look();
    let kit = &look.kit;
    let step = d.step(cx.env().scale, look);
    let line = |key: &'static str, s: &str| {
        text(s.to_owned())
            .key(key)
            .font(kit.ty.ui)
            .color(kit.c.text_muted)
            .truncate()
            .min_w(0.0)
    };
    let header = col()
        .w_full()
        .px(look.space.lg)
        .py(look.space.sm)
        .gap(look.space.xxs)
        .child(
            text(format!("Photos · {} items", d.listing.len()))
                .key("c25-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(line("c25-status", &d.status[0]))
        .child(line("c25-order", &d.status[1]))
        .child(line("c25-span", &d.status[2]))
        .child(line("c25-work", &d.status[3]))
        .child(line("c25-snapshot", &d.snapshot));
    let mut lists = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(look.space.lg)
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .h_full()
                .child(list(g, SCOPE, step, false)),
        );
    if d.second.is_some() {
        lists = lists.child(
            col()
                .grow(1.0)
                .w(0.0)
                .h_full()
                .child(list(g, SCOPE_B, step, true)),
        );
    }
    col()
        .key("c25-page")
        .w_full()
        .h_full()
        .child(header)
        .child(lists)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c25;
    let Key::Text(t) = &key.key else {
        return false;
    };
    match t.as_str() {
        "t" => {
            let on = !d.store.enabled();
            d.store.set_enabled(on);
            if !on {
                d.work(d.clock);
            }
        }
        "n" => d.rename_first(),
        "o" => d.operation = true,
        "p" => {
            d.operation = false;
            d.store.requeue_stopped();
        }
        "f" => d.toggle_filter(tx),
        "s" => {
            d.refresh_status();
            d.snapshot = format!("Snapshot: {} | {}", d.status[0], d.status[3]);
        }
        _ => return false,
    }
    // A key's own effect shows at once; the next idle refreshes the rest.
    d.refresh_status();
    true
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Hands the fake worker's results to the store, and takes its next requests. True only when
/// the worker went idle: the status lines refresh then (one view run a loading round).
pub fn poll(g: &mut Gallery, t: f64, _tx: &mut Tx) -> bool {
    let d = &mut g.c25;
    d.clock = t;
    d.polls = d.polls.saturating_add(1);
    if d.polls == 2 && std::env::var_os(WARMUP).is_some_and(|v| !v.is_empty()) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        eprintln!("{WARMED}");
    }
    let was_idle = d.idle;
    d.work(t);
    if d.refresh_once && !d.coll.visible_range().is_empty() {
        d.refresh_once = false;
        d.refresh_status();
        return true;
    }
    let busy = !d.jobs.is_empty();
    d.idle = !busy;
    // Still loading, or idle all along: nothing the status lines show changed.
    if busy || was_idle {
        return false;
    }
    d.round_runs = Some(d.views.get().saturating_sub(d.busy_views));
    if d.status_on_idle {
        d.refresh_status();
        return true;
    }
    false
}
