//! The gallery page for C3 IconsGrid: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c3-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-3-views/C3-icons-grid.md`.
//!
//! The page is a stand-in Icons pane over five fake folders: Pictures (40 entries: folders,
//! images and videos with thumbnails, audio, documents and two long names), Short (10, a short
//! last row), Long names, Archive (100,000) and Empty. A fake worker renders thumbnails and a
//! fake metadata worker streams the details line in, both on the virtual clock. The header
//! shows what the host heard: the size notice (`c3-size`), the last intent (`c3-intent`) and,
//! once both workers are idle, how many visible tiles are filled (`c3-filled`).
//!
//! The page stands in for Strata's policy: it keeps one size (Remember sort and icon size per
//! folder off), so a reset goes back to the look's default, 64; it describes each entry by its
//! kind (Strata's `entry_kind_name`); and it sets the tooltip delay to Strata's 900 ms for the
//! whole window, as Rune's delay is the window's (`Cx::help_look`). C24's autoscroll runs on
//! the grid from its background (no tile, or a tile's blank space; the shield mounts last on
//! the page), and `c3-bar` shows the grid's scroll bar.
//!
//! | Key | Does |
//! | --- | --- |
//! | Ctrl+Alt+plus, Ctrl+Alt+=, Ctrl+Alt+minus, Ctrl+Alt+0 | The size keys, from anywhere on the page |
//! | Ctrl+T | 10xer mode on or off: plain +, = and − then step the size with the grid focused |
//! | F5 | Refresh |
//! | Escape | Ends a rename: the page draws a stand-in for C5's editor |

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rune::{Cx, El, HelpLook, Key, KeyEvent, Tx, col, row, text};
use strata_components::collection::element::{
    Collection, CollectionState, Controllers, Intent, RowMarks,
};
use strata_components::collection::keyboard::{KeySettings, Keyboard};
use strata_components::collection::selection::{CursorGone, SyncRule};
use strata_components::contract::{
    EntryId, EntryKind, EntrySource, ListEntry, ListSource, ThumbKey,
};
use strata_components::icons_grid::{
    IconsGrid, IconsGridState, IconsParts, SizeKeyed, SizeKeys, SizeRequest, autoscroll_background,
    size_command,
};
use strata_components::scroll::{
    Autoscroll, AutoscrollPointer, AutoscrollSettings, BarMode, Shield,
};
use strata_components::thumbnails::{
    Landed, Outcome, Request, Settings, State, ThumbnailStore, ViewerId,
};

use crate::{Gallery, Scene};

/// The collection's key.
pub const SCOPE: &str = "c3-grid";

/// Thumbnail keys are the entry's id past this.
const KEY_BASE: u64 = 7_000_000;

/// The fake thumbnail worker's lanes.
const LANES: usize = 6;

/// The tooltip delay the page sets, Strata's 900 ms (`name_tooltip.rs`).
pub const HELP_DELAY: f64 = 0.9;

/// The fake folders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Folder {
    #[default]
    Pictures,
    Short,
    Long,
    Archive,
    Empty,
}

impl Folder {
    fn title(self) -> &'static str {
        match self {
            Folder::Pictures => "Pictures",
            Folder::Short => "Short",
            Folder::Long => "Long names",
            Folder::Archive => "Archive",
            Folder::Empty => "Empty",
        }
    }

    fn listing(self) -> Vec<ListEntry> {
        let entry = |id: u64, name: &str, kind: EntryKind, icon: &str| {
            let mut e = ListEntry::new(EntryId::new(id), name.to_owned(), kind, icon);
            if matches!(icon, "file_image" | "file_video") {
                e.thumbnail = Some(ThumbKey::new(KEY_BASE + id));
            }
            e
        };
        let named = |base: u64, names: &[(&str, EntryKind, &str)]| -> Vec<ListEntry> {
            names
                .iter()
                .enumerate()
                .map(|(i, (n, k, icon))| entry(base + i as u64, n, *k, icon))
                .collect()
        };
        use EntryKind::{File, Folder as Dir};
        match self {
            Folder::Pictures => {
                let mut v = named(
                    1,
                    &[
                        ("Holidays", Dir, "folder"),
                        ("Screenshots", Dir, "folder"),
                        ("Wallpapers", Dir, "folder"),
                        ("Empty album", Dir, "folder"),
                        ("Scans", Dir, "folder"),
                        ("Raw", Dir, "folder"),
                        ("beach.jpg", File, "file_image"),
                        ("sunset over the bay.jpg", File, "file_image"),
                        ("cat.png", File, "file_image"),
                        ("IMG_2041.heic", File, "file_image"),
                        (
                            "2026-10-07-release-candidate-build-final-v3-with-a-very-long-tail.tar.gz",
                            File,
                            "archive",
                        ),
                        ("clip.mp4", File, "file_video"),
                        ("song.flac", File, "file_audio"),
                        ("notes.md", File, "file_text"),
                        (
                            "Quarterly report for the infrastructure committee, final revision.pdf",
                            File,
                            "file",
                        ),
                        ("a.txt", File, "file_text"),
                    ],
                );
                let more = (0..24u64).map(|i| {
                    let (icon, ext) = match i % 4 {
                        0 | 1 => ("file_image", "jpg"),
                        2 => ("file_video", "mkv"),
                        _ => ("file", "bin"),
                    };
                    entry(100 + i, &format!("photo-{:03}.{ext}", i + 1), File, icon)
                });
                v.extend(more);
                v
            }
            Folder::Short => (0..10u64)
                .map(|i| entry(200 + i, &format!("tile-{i}.jpg"), File, "file_image"))
                .collect(),
            Folder::Long => named(
                300,
                &[
                    ("a", File, "file_text"),
                    ("short.md", File, "file_text"),
                    ("two words.txt", File, "file_text"),
                    (
                        "a-name-without-any-spaces-that-wraps-inside-its-word-and-is-cut.tar.gz",
                        File,
                        "archive",
                    ),
                    (
                        "Meeting notes from the planning session about the next release.odt",
                        File,
                        "file_text",
                    ),
                    ("résumé – final (2).pdf", File, "file"),
                ],
            ),
            Folder::Archive => (0..100_000u64)
                .map(|i| {
                    let (kind, icon) = if i % 25 == 0 {
                        (Dir, "folder")
                    } else if i % 3 == 0 {
                        (File, "file_image")
                    } else {
                        (File, "file")
                    };
                    entry(
                        1_000_000 + i,
                        &format!("entry-{:06}.dat", i + 1),
                        kind,
                        icon,
                    )
                })
                .collect(),
            Folder::Empty => Vec::new(),
        }
    }
}

/// What the fake metadata says of an entry: Strata's four kinds of details line.
fn details_text(kind: EntryKind, icon: &str, id: EntryId) -> String {
    let n = id.get().wrapping_mul(2_654_435_761);
    if kind.is_folder_like() {
        return match n % 5 {
            0 => "No items".to_owned(),
            1 => "1 item".to_owned(),
            k => format!("{} items", k * 7),
        };
    }
    match icon {
        "file_image" => format!("{}×{}", 320 + n % 4 * 640, 180 + n % 3 * 360),
        "file_video" | "file_audio" => format!("{}:{:02}", 1 + n % 9, n % 60),
        _ => {
            let b = n % 5_000_000;
            if b >= 1_000_000 {
                format!("{:.1} MB", b as f64 / 1e6)
            } else {
                format!("{:.1} kB", b as f64 / 1e3)
            }
        }
    }
}

/// What Strata calls an entry's kind (`entry_kind_name`): the page's description.
fn kind_name(kind: EntryKind, icon: &str) -> &'static str {
    if kind.is_folder_like() {
        return "Folder";
    }
    match icon {
        "file_image" => "Image",
        "file_video" => "Video",
        "file_audio" => "Audio",
        "file_text" => "Text document",
        "archive" => "Archive",
        _ => "File",
    }
}

struct Job {
    due: f64,
    request: Request,
}

/// C3's demo state, held in `Gallery::c3`.
pub struct Demo {
    pub folder: Folder,
    pub listing: ListSource,
    pub coll: CollectionState,
    pub grid: IconsGridState,
    pub store: ThumbnailStore,
    pub viewer: ViewerId,
    jobs: Vec<Job>,
    /// Details the fake metadata worker has delivered, and when the ones asked for are due.
    pub details: HashMap<EntryId, Arc<str>>,
    asked: HashSet<EntryId>,
    due: Vec<(f64, EntryId)>,
    /// The tile size (the page keeps one, for every folder).
    pub size: f32,
    pub tenxer: bool,
    pub version: u64,
    pub clock: f64,
    /// The pane's width, when narrower than the page.
    pub pane: Option<f32>,
    /// A vertical scroll to make once laid out.
    pub scroll_y: Option<f32>,
    pub notice: String,
    pub intent: String,
    pub filled: String,
    /// Whether the workers had work at the last poll.
    busy: bool,
    /// Thumbnails show (Strata's Show thumbnails).
    pub thumbs_on: bool,
    /// The details line shows.
    pub with_details: bool,
    /// The entry being renamed: a stand-in editor shows in its name's place (C5's comes
    /// later). Escape ends it.
    pub renaming: Option<EntryId>,
    /// The host's marks on tiles (cut, dragging, the filter's Enter target).
    pub marks: HashMap<EntryId, RowMarks>,
    /// The window's autoscroll (C24).
    pub auto: Autoscroll,
}

/// The page as the index opens it: Pictures, as the default scene shows.
impl Default for Demo {
    fn default() -> Self {
        Demo::at(Folder::Pictures)
    }
}

impl Demo {
    /// Nothing listed yet: what [`Demo::at`] fills in.
    fn blank() -> Self {
        let mut coll = CollectionState::new();
        let grid = IconsGridState::new();
        let auto = Autoscroll::new();
        install(&mut coll.controllers, &auto, &grid);
        let mut store = ThumbnailStore::new(Settings::default());
        let viewer = store.viewer();
        Self {
            folder: Folder::Pictures,
            listing: ListSource::new(Vec::new()),
            coll,
            grid,
            auto,
            store,
            viewer,
            jobs: Vec::new(),
            details: HashMap::new(),
            asked: HashSet::new(),
            due: Vec::new(),
            size: 64.0,
            tenxer: false,
            version: 1,
            clock: 0.0,
            pane: None,
            scroll_y: None,
            notice: String::new(),
            intent: String::new(),
            filled: String::new(),
            busy: false,
            thumbs_on: true,
            with_details: true,
            renaming: None,
            marks: HashMap::new(),
        }
    }
}

/// C1b's pointer intent behind C24's autoscroll (a tile's blank space is background), C3's
/// size keys over C1c's, and C1d's marquee.
fn install(controllers: &mut Controllers, auto: &Autoscroll, grid: &IconsGridState) {
    controllers.pointer = Some(Box::new(
        AutoscrollPointer::new(
            Some(Box::new(
                strata_components::collection::pointer::PointerIntent::new(),
            )),
            auto.clone(),
            SCOPE,
            grid.feed().clone(),
        )
        .background(autoscroll_background),
    ));
    controllers.keyboard = Some(Box::new(SizeKeyed::new(Box::<Keyboard>::default())));
    controllers.marquee = Some(Box::new(
        strata_components::collection::marquee::Marquee::new(),
    ));
}

impl Demo {
    fn at(folder: Folder) -> Self {
        let mut d = Demo {
            folder,
            ..Demo::blank()
        };
        d.listing = ListSource::new(folder.listing());
        d
    }

    fn name_of(&self, id: EntryId) -> String {
        self.listing
            .index_of(id)
            .map(|i| self.listing.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    fn heard(&mut self, intent: Intent) -> Option<Intent> {
        self.intent = match &intent {
            Intent::Clicked { id } => format!("Clicked {}", self.name_of(*id)),
            Intent::Activate { id, .. } => format!("Opened {}", self.name_of(*id)),
            Intent::BeginDrag { ids, .. } => format!("Dragging {} items", ids.len()),
            Intent::BeginRename { id } => {
                self.renaming = Some(*id);
                format!("Renaming {}", self.name_of(*id))
            }
            Intent::ContextMenu { target, .. } => match target {
                Some(id) => format!("Menu on {}", self.name_of(*id)),
                None => "Menu on the background".to_owned(),
            },
            Intent::OpenInNew { id, window } => format!(
                "Opened {} in a new {}",
                self.name_of(*id),
                if *window { "window" } else { "tab" }
            ),
            Intent::Edge(d) => format!("Edge {d:?}"),
            Intent::Other(_) => return Some(intent),
            _ => "Other".to_owned(),
        };
        None
    }

    fn heard_size(&mut self, request: SizeRequest, default: f32) {
        self.size = match request {
            SizeRequest::Step { to, .. } => to,
            SizeRequest::Reset => default,
        };
        self.notice = format!("Thumbnail size: {}px", self.size);
    }

    /// Asks the fake metadata worker for the entries in `range` it hasn't been asked for.
    fn ask_details(&mut self, range: std::ops::Range<usize>) {
        if !self.with_details {
            return;
        }
        for i in range {
            let id = self.listing.id_at(i);
            if self.asked.insert(id) {
                let delay = 0.05 + (id.get() % 7) as f64 * 0.02;
                self.due.push((self.clock + delay, id));
            }
        }
    }

    /// The fake metadata worker's frame: true when details landed.
    fn deliver_details(&mut self, t: f64) -> bool {
        let mut landed = false;
        let listing = &self.listing;
        let details = &mut self.details;
        self.due.retain(|&(due, id)| {
            if due > t {
                return true;
            }
            if let Some(i) = listing.index_of(id) {
                let e = listing.entry(i);
                details.insert(id, details_text(e.kind, e.icon, id).into());
                landed = true;
            }
            false
        });
        landed
    }

    fn render(&self, r: &Request) -> Outcome {
        let px = r.step.px();
        let seed = r.key.get();
        let (w, h) = match seed % 3 {
            0 => (px, px * 3 / 4),
            1 => (px * 3 / 4, px),
            _ => (px, px),
        };
        Outcome::Ready {
            width: w,
            height: h,
            rgba: picture(w, h, seed),
        }
    }

    /// The fake thumbnail worker's frame.
    fn work(&mut self, t: f64) {
        for ticket in self.store.take_cancelled() {
            self.jobs.retain(|j| j.request.ticket != ticket);
        }
        let (due, rest): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.jobs)
            .into_iter()
            .partition(|j| j.due <= t);
        self.jobs = rest;
        for job in due {
            let outcome = self.render(&job.request);
            let _: Landed = self.store.complete(job.request.ticket, outcome, t);
        }
        while self.jobs.len() < LANES {
            let Some(r) = self.store.pop(t) else { break };
            let delay = 0.04 + (r.key.get() % 5) as f64 * 0.02;
            self.jobs.push(Job {
                due: t + delay,
                request: r,
            });
        }
    }

    /// The status line of what the visible tiles show, once the workers are idle.
    fn refresh_filled(&mut self) {
        let shown = self.coll.visible_range();
        let (mut thumbs, mut wanted, mut details) = (0, 0, 0);
        for i in shown.clone() {
            let e = self.listing.entry(i);
            if let Some(key) = e.thumbnail {
                wanted += 1;
                if matches!(self.store.state(key), State::Ready { .. }) {
                    thumbs += 1;
                }
            }
            if self.details.contains_key(&e.id) {
                details += 1;
            }
        }
        self.filled = format!(
            "Visible {}: thumbnails {thumbs} of {wanted}, details {details}",
            shown.len()
        );
    }

    pub fn refresh(&mut self, tx: &mut Tx) {
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
        self.store.resync(self.viewer, &self.listing, self.version);
    }
}

/// A fake thumbnail: a gradient hued by `seed`, with diagonal bands.
fn picture(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let hue = (seed.wrapping_mul(2_654_435_761) % 360) as f32 / 360.0 * std::f32::consts::TAU;
    let base = [hue.cos(), (hue + 2.1).cos(), (hue + 4.2).cos()];
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let band = if ((u + v) * 5.0).fract() < 0.5 {
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

/// The fallback look with Strata's defaults but for what `f` sets (K2's parameters).
fn params(
    dark: bool,
    f: impl FnOnce(&mut strata_kit::shape::LookParams),
) -> strata_kit::look::Look {
    let mut p = strata_kit::shape::LookParams::default();
    f(&mut p);
    strata_kit::look::Look::fallback(dark).with_params(&p)
}

fn open(g: &mut Gallery, dark: bool, folder: Folder) -> &mut Demo {
    g.open("c3", dark);
    g.c3 = Demo::at(folder);
    &mut g.c3
}

fn default_scene(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Pictures);
}

/// 100,000 tiles with the scroll bar shown (`BarMode::Always`), at the top.
fn bar(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Archive)
        .grid
        .set_bar_mode(BarMode::Always);
}

fn loading(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Pictures);
    d.store.set_enabled(false);
    d.thumbs_on = false;
    d.with_details = false;
}

fn selection(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Pictures);
    d.coll.selection.select_only(&d.listing, 6);
    for i in [7, 9] {
        d.coll.selection.toggle(&d.listing, i);
    }
    d.coll.keyboard_moved();
    d.coll.note_selection(&d.listing);
}

fn long_names(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Long);
}

fn size_32(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Pictures).size = 32.0;
}

fn size_256(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Pictures).size = 256.0;
}

fn size_100(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Short).size = 100.0;
}

fn airy(g: &mut Gallery, dark: bool) {
    g.open("c3", dark);
    g.look = params(dark, |p| p.content_density = 8);
    g.c3 = Demo::at(Folder::Archive);
    g.c3.size = 32.0;
    // Wider than the window, so the column cap shows (16 at Airy).
    g.c3.pane = Some(3000.0);
}

fn wide(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Archive);
    d.size = 32.0;
    // Wider than the window, so the column cap shows (20 at Compact).
    d.pane = Some(3000.0);
}

fn narrow(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Short);
    d.pane = Some(150.0);
}

fn short_rows(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Short);
    d.pane = Some(400.0);
}

/// A drop target, a cut tile, a dragged tile and the filter's Enter target, side by side.
fn marks(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Pictures);
    d.coll
        .set_drop_over(Some(strata_components::collection::element::DropOver::Row(
            EntryId::new(1),
        )));
    let mut mark = |id: u64, set: fn(&mut RowMarks)| {
        let mut m = RowMarks::default();
        set(&mut m);
        d.marks.insert(EntryId::new(id), m);
    };
    mark(3, |m| m.cut = true);
    mark(4, |m| m.dragging = true);
    mark(5, |m| m.enter_target = true);
}

fn text_size(g: &mut Gallery, dark: bool) {
    g.open("c3", dark);
    g.look = params(dark, |p| p.text_size = 20);
    g.c3 = Demo::at(Folder::Pictures);
}

fn hundred_k(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Archive);
    d.scroll_y = Some(400_000.0);
}

fn empty(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Empty);
}

fn grouped(g: &mut Gallery, dark: bool) {
    let d = open(g, dark, Folder::Pictures);
    // Groups a host might keep for List: the grid draws none.
    d.coll
        .set_groups(strata_components::collection::element::Groups::new([
            strata_components::collection::element::Group::new(1, "Folders", 6),
            strata_components::collection::element::Group::new(2, "Files", 34),
        ]));
}

fn tenxer(g: &mut Gallery, dark: bool) {
    open(g, dark, Folder::Pictures).tenxer = true;
}

pub const SCENES: &[Scene] = &[
    ("c3-default-light", |g| default_scene(g, false)),
    ("c3-default-dark", |g| default_scene(g, true)),
    ("c3-loading-light", |g| loading(g, false)),
    ("c3-loading-dark", |g| loading(g, true)),
    ("c3-selection-light", |g| selection(g, false)),
    ("c3-selection-dark", |g| selection(g, true)),
    ("c3-long-names-light", |g| long_names(g, false)),
    ("c3-long-names-dark", |g| long_names(g, true)),
    ("c3-size-32-light", |g| size_32(g, false)),
    ("c3-size-32-dark", |g| size_32(g, true)),
    ("c3-size-256-light", |g| size_256(g, false)),
    ("c3-size-256-dark", |g| size_256(g, true)),
    ("c3-airy-light", |g| airy(g, false)),
    ("c3-airy-dark", |g| airy(g, true)),
    ("c3-narrow-light", |g| narrow(g, false)),
    ("c3-narrow-dark", |g| narrow(g, true)),
    ("c3-short-rows-light", |g| short_rows(g, false)),
    ("c3-short-rows-dark", |g| short_rows(g, true)),
    ("c3-marks-light", |g| marks(g, false)),
    ("c3-marks-dark", |g| marks(g, true)),
    ("c3-text-size-light", |g| text_size(g, false)),
    ("c3-text-size-dark", |g| text_size(g, true)),
    ("c3-hundred-k-light", |g| hundred_k(g, false)),
    ("c3-hundred-k-dark", |g| hundred_k(g, true)),
    ("c3-empty-light", |g| empty(g, false)),
    ("c3-empty-dark", |g| empty(g, true)),
    ("c3-bar-light", |g| bar(g, false)),
    ("c3-bar-dark", |g| bar(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: a size between two steps
/// (100 px), 10xer mode and a compact grid wider than the window, for the size and column
/// scripts, and a listing with groups the grid ignores.
pub const REVIEW: &[Scene] = &[
    ("c3-size-100-light", |g| size_100(g, false)),
    ("c3-size-100-dark", |g| size_100(g, true)),
    ("c3-tenxer-light", |g| tenxer(g, false)),
    ("c3-tenxer-dark", |g| tenxer(g, true)),
    ("c3-wide-light", |g| wide(g, false)),
    ("c3-wide-dark", |g| wide(g, true)),
    ("c3-grouped-light", |g| grouped(g, false)),
    ("c3-grouped-dark", |g| grouped(g, true)),
];

/// The page's collection, before the grid adds its parts.
pub fn collection(g: &Gallery) -> Collection<Gallery> {
    let d = &g.c3;
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c3.listing, &g.c3.coll),
        |g: &mut Gallery| (&g.c3.listing, &mut g.c3.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c3.version,
    )
    .name(d.folder.title())
    .marks(|g: &Gallery, id| g.c3.marks.get(&id).copied().unwrap_or_default())
    .settings(KeySettings::default().wrap_rows(true))
    .settings({
        let mut auto = AutoscrollSettings::default();
        auto.margin = strata_kit::slots::collection::row_inset(g.look());
        auto
    })
    .settings(SizeKeys::default_map().with_tenxer(d.tenxer))
    .on_intent(|g: &mut Gallery, intent, tx| {
        if let Some(intent) = g.c3.heard(intent)
            && let Some(command) = size_command(&intent)
        {
            grid().apply_size(g, command, tx);
        }
    })
    .rename(
        |g: &Gallery| g.c3.renaming,
        strata_components::collection::element::Slot::pure(|g: &Gallery, rcx| {
            let kit = g.kit();
            text(rcx.entry.name.to_owned())
                .key("c3-rename")
                .font(kit.ty.ui_compact)
                .color(kit.c.text_base)
                .fill(kit.c.layer_01)
                .outline(1.0, 0.0, kit.c.border_focus)
                .truncate()
                .w_full()
        }),
    )
    .describe(strata_components::collection::element::Describe::pure(
        |_: &Gallery, rcx| Some(kind_name(rcx.entry.kind, rcx.entry.icon).to_owned()),
    ))
    .empty(|g: &Gallery| {
        let kit = g.kit();
        Some(
            col()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(
                    text("Folder is empty")
                        .key("c3-empty-note")
                        .font(kit.ty.ui)
                        .color(kit.c.text_muted),
                ),
        )
    })
}

/// The page's grid.
pub fn grid() -> IconsGrid<Gallery> {
    IconsGrid::new(
        SCOPE,
        collection,
        |g: &Gallery| IconsParts {
            source: &g.c3.listing,
            collection: &g.c3.coll,
            grid: &g.c3.grid,
            size: g.c3.size,
            listing: g.c3.version,
            look: g.look(),
            thumbs: Some((&g.c3.store, g.c3.viewer)),
        },
        |g: &mut Gallery| &mut g.c3.grid,
    )
    .thumbs_mut(|g: &mut Gallery| {
        let d = &mut g.c3;
        Some((&mut d.store, d.viewer, &d.listing as &dyn EntrySource))
    })
    .details(
        |g: &Gallery, rcx| {
            g.c3.details
                .get(&rcx.entry.id)
                .map_or(0, |d| d.len() as u64 + 1)
        },
        |g: &Gallery, rcx| g.c3.details.get(&rcx.entry.id).cloned(),
    )
    .on_size(|g: &mut Gallery, request, _| {
        let default = g.look.tiles.default();
        g.c3.heard_size(request, default);
    })
    .on_visible(|g: &mut Gallery, range| {
        g.c3.ask_details(range);
        false
    })
}

fn label(key: &str, t: String, g: &Gallery) -> El {
    let kit = g.kit();
    text(t).key(key).font(kit.ty.ui).color(kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c3;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    cx.help_look(HelpLook::system(g.dark).delay(HELP_DELAY));
    let kit = g.kit();
    let header = col()
        .w_full()
        .px(look.space.lg)
        .py(look.space.xs)
        .gap(look.space.xxs)
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .center_y()
                .child(
                    text(format!("{} · {} items", d.folder.title(), d.listing.len()))
                        .key("c3-title")
                        .font(kit.ty.ui_medium)
                        .color(kit.c.text_base),
                )
                .child(label("c3-size", d.notice.clone(), g))
                .child(label("c3-intent", d.intent.clone(), g)),
        )
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .child(label(
                    "c3-columns",
                    format!("{} a row", d.coll.per_line()),
                    g,
                ))
                .child(label("c3-filled", d.filled.clone(), g)),
        );
    let body = grid().scale(cx.env().scale).build(g);
    let pane = match d.pane {
        Some(w) => col().w(w).h_full().min_h(0.0).child(body),
        None => col().w_full().h_full().min_h(0.0).child(body),
    };
    let mut page = col().key("c3-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(pane),
    );
    if let Some(shield) = Shield::new(|g: &Gallery| &g.c3.auto, |g: &Gallery| g.look()).build(g) {
        page = page.child(shield);
    }
    page
}

pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let keys = SizeKeys::default_map().with_tenxer(g.c3.tenxer);
    if let Some(command) = keys.window_command(key, tx) {
        grid().apply_size(g, command, tx);
        return true;
    }
    match &key.key {
        Key::Function(5) => g.c3.refresh(tx),
        Key::Escape if g.c3.renaming.is_some() => g.c3.renaming = None,
        _ if key.is_command('t') => g.c3.tenxer = !g.c3.tenxer,
        _ => return false,
    }
    true
}

pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let animations = g.look().animations;
    let d = &mut g.c3;
    d.clock = t;
    if d.coll.viewport().h > 0.0
        && let Some(y) = d.scroll_y.take()
    {
        tx.scroll_to_instant(
            strata_components::collection::element::viewport_path(SCOPE),
            0.0,
            y,
        );
    }
    let driven = d.auto.drive(t, animations, tx);
    let landed = d.deliver_details(t);
    if d.thumbs_on {
        d.work(t);
    }
    let busy = !d.jobs.is_empty() || !d.due.is_empty();
    let settled = d.busy && !busy;
    d.busy = busy;
    if settled {
        d.refresh_filled();
    }
    let grid = grid();
    let ticked = grid.collection(g).tick(g, tx);
    grid.poll(g, tx);
    landed || settled || ticked || driven
}
