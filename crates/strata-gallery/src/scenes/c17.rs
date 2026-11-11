//! The gallery page for C17 SearchPalette: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c17-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C17-search-palette.md`.
//!
//! The page draws a stand-in window: a header with a search button (`c17-search`), a focusable
//! file list (`c17-files`, where an opened result's focus lands) and a `c17-log` line naming
//! the last thing the host did, which scripts read. The demo plays Strata's part:
//!
//! - Ctrl+K toggles search and Ctrl+Shift+K the folder jump (the window's key map);
//! - a fake index of about 200 paths under `/home/demo`, searched on a fake worker on the
//!   virtual clock: each query's results arrive in three batches 0.05, 0.15 and 0.3 s after it,
//!   each holding what a walk a third, two thirds and all the way through has found, ranked by
//!   W1's `fuzzy` (name matches first), at most 100, the last batch done. An empty query only
//!   ends the spinner at 0.3 s. The worker answers superseded queries too, so the palette's
//!   own stale check is what scripts see. The worker asks for a wake at its next answer, so a
//!   window at rest still gets it;
//! - a fake folder history for the jump, answered at once;
//! - fake thumbnails for images through a C25 store, rendered 0.1 s after they are asked for;
//! - a stand-in for the result menu C21 will give (`c17-menu`, a nested modal with "Open
//!   containing folder").
//!
//! The demo's switches stand in for a slow walk (`slow`: answers 30 s late), a walk that never
//! finishes (`hold`), one that stopped early (`partial`) and no search locations
//! (`unavailable`).

use std::collections::HashMap;
use std::time::Duration;

use rune::{Chord, Cx, El, Key, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::Button;
use strata_components::contract::{
    EntryId, EntryKind, ListEntry, ListSource, Point, ThumbKey, fuzzy,
};
use strata_components::header;
use strata_components::search_palette::{
    Answer, By, PaletteIntent, PaletteItem, PaletteMode, PaletteQuery, PaletteState, Polled,
    ResultKind, SearchPalette,
};
use strata_components::thumbnails::{
    Outcome, Request, Settings, Step, ThumbnailStore, ViewerId, Viewport,
};
use strata_kit::density::PALETTE_THUMB;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The palette's key.
pub const KEY: &str = "c17-palette";
const HOME: &str = "/home/demo";
/// When each of a query's three batches comes due, after it.
const BATCHES: [f64; 3] = [0.05, 0.15, 0.3];
/// How late a slow walk answers.
const SLOW: f64 = 30.0;
/// How long a fake thumbnail takes.
const RENDER: f64 = 0.1;
/// The most results a query shows (Strata's cap).
const CAP: usize = 100;

/// C17's demo state, held in `Gallery::c17`.
pub struct Demo {
    pub palette: PaletteState,
    /// The fake index, in walk order: absolute paths, folders ending in `/`.
    pub index: Vec<String>,
    /// The fake folder history, most frequent first.
    pub history: Vec<String>,
    /// What the host did last, shown in `c17-log`.
    pub log: String,
    /// The clock at the last poll.
    pub now: f64,
    /// The fake worker: answers, each due at a clock time.
    pub worker: Vec<(f64, Answer)>,
    pub slow: bool,
    pub hold: bool,
    pub partial: bool,
    pub unavailable: bool,
    /// The stand-in result menu: the result and where it opened.
    pub menu: Option<(String, Point)>,
    pub thumbs: ThumbnailStore,
    viewer: ViewerId,
    ids: HashMap<String, EntryId>,
    jobs: Vec<(f64, Request)>,
}

impl Default for Demo {
    fn default() -> Demo {
        let mut thumbs = ThumbnailStore::new(Settings::default());
        let viewer = thumbs.viewer();
        Demo {
            palette: PaletteState::new(),
            index: fake_index(),
            history: fake_history(),
            log: String::new(),
            now: 0.0,
            worker: Vec::new(),
            slow: false,
            hold: false,
            partial: false,
            unavailable: false,
            menu: None,
            thumbs,
            viewer,
            ids: HashMap::new(),
            jobs: Vec::new(),
        }
    }
}

/// The fake home folder, in the order a walk finds it.
fn fake_index() -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |dir: &str, names: &[&str]| {
        out.push(format!("{HOME}/{dir}/"));
        for n in names {
            out.push(format!("{HOME}/{dir}/{n}"));
        }
    };
    add(
        "Documents",
        &[
            "notes.md",
            "quarterly-summary.txt",
            "quarterly-summary.md",
            "budget-2026.ods",
            "letter-to-landlord.odt",
            "résumé.pdf",
            "reading-list.txt",
        ],
    );
    add(
        "Documents/Reports",
        &[
            "report-january.pdf",
            "report-february.pdf",
            "report-march.pdf",
            "annual-report-2025.pdf",
            "report-notes.md",
        ],
    );
    add(
        "Documents/Notes",
        &["meeting-notes.md", "ideas.md", "todo.txt"],
    );
    add(
        "Documents/Archive/2019/tax-returns-and-supporting-documents-for-the-household/scanned-originals-from-the-accountant",
        &["receipt-bundle-final-version-two-signed-and-countersigned-by-everyone-involved.pdf"],
    );
    add(
        "Downloads",
        &["installer.run", "notes-export.zip", "wallpaper.png"],
    );
    add("Music", &["playlist.m3u", "voice-memo.ogg"]);
    add(
        "Pictures",
        &[
            "avatar.png",
            "screenshot-2026-11-01.png",
            "receipt-scan.jpg",
        ],
    );
    let trips: Vec<String> = (1..=60).map(|i| format!("trip-photo-{i:03}.jpg")).collect();
    let trips: Vec<&str> = trips.iter().map(String::as_str).collect();
    add("Pictures/Trips", &trips);
    add(
        "Projects/strata",
        &[
            "README.md",
            "Cargo.toml",
            "CHANGELOG.md",
            "notes-for-release.md",
        ],
    );
    add(
        "Projects/strata/src",
        &[
            "main.rs",
            "search.rs",
            "search_session.rs",
            "folder_picker.rs",
        ],
    );
    add(
        "Projects/strata/docs",
        &["keymap.md", "preferences.md", "testing.md"],
    );
    add("Projects/rune", &["README.md", "notes.txt"]);
    let logs: Vec<String> = (1..=80).map(|i| format!("build-{i:03}.log")).collect();
    let logs: Vec<&str> = logs.iter().map(String::as_str).collect();
    add("Projects/strata/target/logs", &logs);
    out
}

/// The fake folder history, most frequent and recent first.
fn fake_history() -> Vec<String> {
    [
        "Projects/strata",
        "Downloads",
        "Documents",
        "Projects/strata/src",
        "Pictures/Trips",
        "Documents/Reports",
        "Projects/rune",
        "Music",
        "Documents/Notes",
        "Projects/strata/docs",
    ]
    .into_iter()
    .map(|d| format!("{HOME}/{d}"))
    .collect()
}

/// The folders Ctrl+K searches, for the field's description.
pub const LOCATIONS: [&str; 2] = [HOME, "/run/media/demo/USB"];

/// The name of `path`: its last part.
fn name_of(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// `path` as a result, matched against `query`; `None` when it doesn't match. The rank is
/// the name's score, then the path's below every name match.
fn matched(path: &str, query: &str) -> Option<(i64, PaletteItem)> {
    let folder = path.ends_with('/');
    let full = path.trim_end_matches('/');
    let name = name_of(path);
    let on_name = fuzzy(query, name);
    let on_path = fuzzy(query, full);
    let rank = match (&on_name, &on_path) {
        (Some(n), _) => 1_000_000 + i64::from(n.score),
        (None, Some(p)) => i64::from(p.score),
        (None, None) => return None,
    };
    let mut item = PaletteItem::new(full, name, full).icon(icon_for(path));
    if let Some(n) = on_name {
        item = item.matched(n.ranges);
    }
    if let Some(p) = on_path {
        item = item.detail_matched(p.ranges);
    }
    if !folder && is_image(name) {
        item = item.thumb(thumb_key(full));
    }
    Some((rank, item))
}

fn is_image(name: &str) -> bool {
    name.ends_with(".jpg") || name.ends_with(".png")
}

fn icon_for(path: &str) -> IconName {
    let name = name_of(path);
    if path.ends_with('/') {
        IconName::Folder
    } else if is_image(name) {
        IconName::FileImage
    } else if name.ends_with(".md") || name.ends_with(".txt") || name.ends_with(".log") {
        IconName::FileText
    } else if name.ends_with(".rs") || name.ends_with(".toml") {
        IconName::FileCode
    } else {
        IconName::File
    }
}

/// A stable thumbnail key for `path`.
fn thumb_key(path: &str) -> ThumbKey {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    ThumbKey::new(h)
}

/// The best `CAP` matches of `query` among `paths`, best first, then by name and path
/// (Strata's tie order).
fn rank<'a>(paths: impl Iterator<Item = &'a String>, query: &str) -> Vec<PaletteItem> {
    let mut hits: Vec<(i64, PaletteItem)> = paths.filter_map(|p| matched(p, query)).collect();
    hits.sort_by(|(ra, a), (rb, b)| {
        rb.cmp(ra)
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    hits.into_iter().take(CAP).map(|(_, it)| it).collect()
}

impl Demo {
    fn is_jump(&self) -> bool {
        self.palette
            .mode()
            .is_some_and(|m| m.kind == ResultKind::Folders)
    }

    /// Answers `q`: the jump at once, as Strata's history search does, the index on the fake
    /// worker.
    fn schedule(&mut self, q: PaletteQuery, tx: &mut Tx) {
        let late = if self.slow { SLOW } else { 0.0 };
        let query = q.text.trim().to_owned();
        if self.is_jump() {
            let items = if query.is_empty() {
                self.history
                    .iter()
                    .map(|p| {
                        PaletteItem::new(p.clone(), name_of(p), p.clone()).icon(IconName::Folder)
                    })
                    .collect()
            } else {
                let folders: Vec<String> = self.history.iter().map(|p| format!("{p}/")).collect();
                rank(folders.iter(), &query)
            };
            let answer = Answer::new(q.generation, items).done();
            if self.slow {
                self.worker.push((self.now + late, answer));
            } else {
                let _ = self.palette.deliver(answer, tx);
            }
            return;
        }
        if query.is_empty() {
            // The index walk ends: only the spinner goes.
            self.worker.push((
                self.now + BATCHES[2] + late,
                Answer::new(q.generation, Vec::new()).done(),
            ));
            return;
        }
        let n = self.index.len();
        for (k, due) in BATCHES.iter().enumerate() {
            let walked = n * (k + 1) / BATCHES.len();
            let mut answer = Answer::new(q.generation, rank(self.index[..walked].iter(), &query));
            if k + 1 == BATCHES.len() {
                if !self.hold {
                    answer = answer.done();
                }
                if self.partial {
                    answer = answer.partial("Some folders could not be read: ~/Private");
                }
            }
            self.worker.push((self.now + due + late, answer));
        }
    }

    /// Routes the palette's intents as Strata's window would. True when state changed.
    fn route(&mut self, polled: Polled, tx: &mut Tx) -> bool {
        let mut changed = polled.changed;
        for intent in polled.intents {
            changed = true;
            match intent {
                PaletteIntent::Query(q) => {
                    self.schedule(q, tx);
                    self.wake(tx);
                }
                PaletteIntent::Open { id, new_tab, .. } => {
                    self.log = if new_tab {
                        format!("Opened {id} in a new tab")
                    } else {
                        format!("Opened {id}")
                    };
                }
                PaletteIntent::Reveal { id } => self.log = format!("Revealed {id} in its folder"),
                PaletteIntent::ResultMenu { id, at } => self.menu = Some((id, at)),
                PaletteIntent::FieldMenu { .. } => self.log = "Field menu".into(),
                PaletteIntent::Closed { by } => {
                    if !matches!(by, By::Open | By::Reveal) {
                        self.log = format!("Closed ({by:?})");
                    }
                    self.menu = None;
                }
                _ => {}
            }
        }
        changed
    }

    /// Asks for a wake at the fake workers' next due answer: a window at rest polls only at
    /// the wakes it asked for.
    fn wake(&self, tx: &mut Tx) {
        let next = self
            .worker
            .iter()
            .map(|(at, _)| *at)
            .chain(self.jobs.iter().map(|(at, _)| *at))
            .fold(f64::INFINITY, f64::min);
        if next.is_finite() {
            tx.wake_after(Duration::from_secs_f64((next - self.now).max(0.0)));
        }
    }

    /// Opens the palette in `mode`, as Ctrl+K or Ctrl+Shift+K does.
    fn toggle(&mut self, mode: PaletteMode, tx: &mut Tx) -> bool {
        let polled = self.palette.toggle(mode);
        let changed = self.route(polled, tx);
        if self.palette.is_open() && self.unavailable && !self.is_jump() {
            self.palette
                .unavailable("No local search locations available.");
        }
        changed
    }

    /// The palette's rows `rows` are on screen: sync the thumbnail viewer.
    fn sync_thumbs(&mut self, rows: std::ops::Range<usize>, step: Step) -> bool {
        let items = self.palette.items();
        let mut next = self.ids.len() as u64 + 1;
        let mut entries = Vec::with_capacity(items.len());
        for it in items {
            let id = *self.ids.entry(it.id.clone()).or_insert_with(|| {
                next += 1;
                EntryId::new(next)
            });
            let mut e = ListEntry::new(id, it.label.clone(), EntryKind::File, "");
            e.thumbnail = it.thumb;
            entries.push(e);
        }
        let source = ListSource::new(entries);
        self.thumbs
            .sync(self.viewer, &source, Viewport::new(rows, step))
    }

    /// The fake thumbnail worker: hands over what is due, takes new requests.
    fn work_thumbs(&mut self, t: f64) {
        for ticket in self.thumbs.take_cancelled() {
            self.jobs.retain(|(_, r)| r.ticket != ticket);
        }
        let (due, later): (Vec<_>, Vec<_>) = self.jobs.drain(..).partition(|(at, _)| *at <= t);
        self.jobs = later;
        for (_, r) in due {
            let px = r.step.px();
            let outcome = Outcome::Ready {
                width: px,
                height: px,
                rgba: picture(px, r.key.get()),
            };
            let _ = self.thumbs.complete(r.ticket, outcome, t);
        }
        while let Some(r) = self.thumbs.pop(t) {
            self.jobs.push((t + RENDER, r));
        }
    }
}

/// A square stripy gradient seeded by `seed`.
fn picture(px: u32, seed: u64) -> Vec<u8> {
    let hue = (seed % 360) as f32 / 360.0 * std::f32::consts::TAU;
    let base = [hue.cos(), (hue + 2.1).cos(), (hue + 4.2).cos()];
    let mut out = Vec::with_capacity((px * px * 4) as usize);
    for y in 0..px {
        for x in 0..px {
            let (u, v) = (x as f32 / px as f32, y as f32 / px as f32);
            let band = if ((u + v) * 4.0).fract() < 0.5 {
                1.0
            } else {
                0.85
            };
            let light = (0.45 + 0.4 * (1.0 - v)) * band;
            for c in base {
                out.push(((0.5 + 0.5 * c) * light * 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

fn demo(g: &mut Gallery) -> &mut Demo {
    &mut g.c17
}

/// The search mode the page opens.
pub fn search_mode() -> PaletteMode {
    PaletteMode::search(&LOCATIONS)
}

/// Opens `mode` with `query` typed, its answers due from the clock's start.
fn opened(g: &mut Gallery, mode: PaletteMode, query: &str) {
    // A scene runs before the window does: nothing to scroll or wake yet.
    let tx = &mut Tx::default();
    let d = demo(g);
    d.toggle(mode, tx);
    if !query.is_empty() {
        let polled = d.palette.set_query(query);
        d.route(polled, tx);
    }
}

fn idle(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, search_mode(), "");
}

fn loading(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    g.c17.slow = true;
    opened(g, search_mode(), "report");
}

fn results(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    g.c17.hold = true;
    opened(g, search_mode(), "re");
}

fn no_results(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, search_mode(), "zzqx");
}

fn error(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    g.c17.unavailable = true;
    opened(g, search_mode(), "");
}

fn partial(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    g.c17.partial = true;
    opened(g, search_mode(), "trip");
}

fn jump(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, PaletteMode::jump(), "");
}

fn jump_empty(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    g.c17.history.clear();
    opened(g, PaletteMode::jump(), "");
}

fn jump_no_match(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, PaletteMode::jump(), "zzqx");
}

fn cleared(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, search_mode(), "notes");
    let d = demo(g);
    let polled = d.palette.set_query("");
    d.route(polled, &mut Tx::default());
}

fn hundred(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, search_mode(), "o");
}

fn long(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
    opened(g, search_mode(), "bundle");
}

fn closed(g: &mut Gallery, dark: bool) {
    g.open("c17", dark);
}

macro_rules! scenes {
    ($($name:literal => $f:ident),* $(,)?) => {
        &[$(
            (concat!($name, "-light"), |g: &mut Gallery| $f(g, false)),
            (concat!($name, "-dark"), |g: &mut Gallery| $f(g, true)),
        )*]
    };
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c17", dark)`, then fills in `g.c17`.
pub const SCENES: &[Scene] = scenes![
    "c17-idle" => idle,
    "c17-loading" => loading,
    "c17-results" => results,
    "c17-no-results" => no_results,
    "c17-error" => error,
    "c17-partial" => partial,
    "c17-jump" => jump,
    "c17-jump-empty" => jump_empty,
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = scenes![
    "c17-closed" => closed,
    "c17-jump-no-match" => jump_no_match,
    "c17-cleared" => cleared,
    "c17-hundred" => hundred,
    "c17-long" => long,
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let mut layer = Vec::new();
    let step = Step::for_tile(look, PALETTE_THUMB, cx.env().scale);
    SearchPalette::new(
        KEY,
        |g: &Gallery| &g.c17.palette,
        |g: &mut Gallery| &mut g.c17.palette,
    )
    .thumbnails(|g: &Gallery| &g.c17.thumbs, step)
    .on_visible(move |g: &mut Gallery, rows| g.c17.sync_thumbs(rows, step))
    .return_focus_on_open("c17-files")
    .reduced_motion(cx.is_reduced_motion())
    .on_intent(|g: &mut Gallery, intent, tx: &mut Tx| {
        g.c17.route(
            Polled {
                changed: true,
                intents: vec![intent],
            },
            tx,
        );
    })
    .build(look, g, &mut layer);
    if let Some(menu) = result_menu(g, look) {
        layer.push(menu);
    }
    rune_kit::root(window(g, look), layer)
}

/// The stand-in for the result menu C21 will give: one item, in a modal of its own over the
/// palette's.
fn result_menu(g: &Gallery, look: &Look) -> Option<El> {
    let (id, at) = g.c17.menu.clone()?;
    let k = &look.kit;
    let item = Button::new("Open containing folder")
        .on_press_tx(move |g: &mut Gallery, tx: &mut Tx| {
            let d = demo(g);
            d.menu = None;
            let polled = d.palette.reveal(&id, tx);
            d.route(polled, tx);
        })
        .build(k)
        .key("c17-menu-reveal");
    Some(
        col()
            .key("c17-menu")
            .role("menu")
            .label("Result")
            .absolute(at.x, at.y)
            .padding(look.space.xxs)
            .radius(k.radius.md)
            .fill(k.c.layer_02)
            .border(1.0, k.c.border_base)
            .modal(true, |g: &mut Gallery| g.c17.menu = None)
            .child(item),
    )
}

/// The stand-in window behind the palette.
fn window(g: &Gallery, look: &Look) -> El {
    let k = &look.kit;
    let d = &g.c17;
    let search = header::action(look, IconName::Search, "Search (Ctrl+K)")
        .on_press_tx(|g: &mut Gallery, tx: &mut Tx| {
            demo(g).toggle(search_mode(), tx);
        })
        .build(k)
        .key("c17-search");
    let header = row()
        .key("c17-header")
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .gap(look.space.md)
        .center_y()
        .fill(k.c.layer_01)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(text("demo").font(look.ty.title).color(k.c.text_base))
        .child(el().grow(1.0))
        .child(search);
    let names = ["Documents", "Downloads", "Music", "Pictures", "Projects"];
    let files = col()
        .key("c17-files")
        .role("list")
        .label("Files")
        .focusable(true)
        .focus_ring(2.0, -2.0, k.c.border_focus)
        .w(look.row.list * 10.0)
        .padding(look.space.lg)
        .gap(look.space.xxs)
        .children(names.iter().map(|name| {
            row()
                .h(look.row.list)
                .px(look.space.md)
                .gap(look.space.md)
                .center_y()
                .child(
                    icon(look.icon(IconName::Folder), look.icon_size.md)
                        .color(k.c.icon_muted)
                        .a11y_hidden(),
                )
                .child(text(*name).font(k.ty.ui).color(k.c.text_base))
        }));
    let log = text(if d.log.is_empty() {
        "Ready"
    } else {
        d.log.as_str()
    })
    .key("c17-log")
    .font(k.ty.ui)
    .color(k.c.text_base);
    col()
        .key("c17-window")
        .w_full()
        .h_full()
        .child(header)
        .child(
            row()
                .w_full()
                .grow(1.0)
                .child(files)
                .child(col().grow(1.0).padding(look.space.lg).child(log)),
        )
}

/// Keys no focused element took while this page is open: Ctrl+K and Ctrl+Shift+K, the
/// window's key map. True consumes the key.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let chord = |c: &str| Chord::parse(c).is_ok_and(|c| c.matches_tx(key, tx));
    if chord("mod+k") {
        demo(g).toggle(search_mode(), tx);
        return true;
    }
    if chord("mod+shift+k") {
        demo(g).toggle(PaletteMode::jump(), tx);
        return true;
    }
    // Escape with the palette or its menu open is theirs; it never goes back to the index.
    key.key == Key::Escape && (g.c17.palette.is_open() || g.c17.menu.is_some())
}

/// Runs at the start of every frame while this page is open, with the clock in seconds: the
/// palette's tick, the fake worker's due answers and the thumbnails. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c17;
    d.now = t;
    d.palette.tick();
    let mut changed = false;
    let (due, later): (Vec<_>, Vec<_>) = d.worker.drain(..).partition(|(at, _)| *at <= t);
    d.worker = later;
    for (_, answer) in due {
        let polled = d.palette.deliver(answer, tx);
        changed |= d.route(polled, tx);
    }
    // Thumbnails landing fill their handles in place: no view change.
    d.work_thumbs(t);
    d.wake(tx);
    changed
}
