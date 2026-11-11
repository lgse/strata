//! The gallery page for C22 Folder peek: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c22-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C22-folder-peek.md`.
//!
//! A Home folder in three views: List (C4's ListTable), Icons (a C1e grid stand-in until C3's
//! IconsGrid lands) and Columns (C2's ColumnsView, which never peeks). Resting the pointer on a
//! folder in List or Icons peeks it; the page stands in for Strata's lister, answering on the
//! virtual clock (Slow after 2 s, Locked with an error, Archive capped at 64 entries). The
//! header line `c22-heard` says what the host last heard.
//!
//! | Key | Does |
//! | --- | --- |
//! | Ctrl+1, Ctrl+2, Ctrl+3 | List, Icons, Columns |
//! | Ctrl+P | Folder peeking on or off |
//! | Ctrl+M | A stand-in menu, its scrim over the window |
//! | `i` | The keyboard peek at the cursor's folder (Strata's 10xer `i`) |
//! | Escape | Closes an open peek first |

use std::ops::Range;
use std::time::Duration;

use rune::{Cx, El, Key, KeyEvent, Tx, col, row, text};
use strata_components::collection::element::{
    Collection, CollectionState, Controllers, Intent, RowMarks,
};
use strata_components::collection::keyboard::Keyboard;
use strata_components::collection::marquee::Marquee;
use strata_components::collection::pointer::PointerIntent;
use strata_components::collection::selection::Listing;
use strata_components::columns::{
    ColumnSources, ColumnsParts, ColumnsPartsMut, ColumnsState, ColumnsView, Focus, Opening,
};
use strata_components::contract::{
    EntryId, EntryKind, EntrySource, ListEntry, ListSource, Rect, ThumbKey,
};
use strata_components::folder_peek::{
    Change, FolderPeek, More, Origin, PeekContents, PeekState, PeekStatus,
};
use strata_components::icons_grid::{IconsGrid, IconsGridState, IconsParts};
use strata_components::list_table::{
    ColumnLayout, ColumnWidths, ListParts, ListTable, ListTableState,
};
use strata_components::scroll::Autoscroll;
use strata_components::thumbnails::{Outcome, Settings, Step, ThumbnailStore, ViewerId, Viewport};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The List collection's key.
pub const LIST: &str = "c22-list";

/// The Icons stand-in's key.
pub const ICONS: &str = "c22-icons";

/// The Columns strip's key.
pub const COLUMNS: &str = "c22-columns";

/// The Icons tile size, C3's default.
const TILE: f32 = 64.0;

/// How long the fake lister takes, and Slow's.
const LIST_DELAY: f64 = 0.1;
const SLOW_DELAY: f64 = 2.0;

/// When a scene opens its peek.
const SCENE_OPEN: f64 = 0.25;

/// Strata's peek load cap (`PEEK_MAX_ENTRIES`): Archive stops there.
const CAP: usize = 64;

/// The views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    List,
    Icons,
    Columns,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::List => "List",
            Mode::Icons => "Icons",
            Mode::Columns => "Columns",
        }
    }
}

/// Home's entries in display order, folders first, with their ids.
const HOME: &[(u64, &str, bool)] = &[
    (1, "Projects", true),
    (2, "Music", true),
    (3, "Photos", true),
    (4, "Empty", true),
    (5, "Locked", true),
    (6, "Slow", true),
    (7, "Archive", true),
    (8, "Downloads", true),
    // Cut in Icons: its tile carries the whole name as a tooltip.
    (
        13,
        "Quarterly reports for the infrastructure committee",
        true,
    ),
    (9, "notes.md", false),
    (10, "budget.ods", false),
    (11, "holiday.jpg", false),
    (12, "README", false),
];

fn home() -> Vec<ListEntry> {
    home_with(0)
}

/// Home with `extra` more folders after its entries, for a list long enough to scroll its
/// rows out of the mounted range.
fn home_with(extra: usize) -> Vec<ListEntry> {
    HOME.iter()
        .map(|(id, name, folder)| {
            let (kind, icon) = if *folder {
                (EntryKind::Folder, "folder")
            } else {
                (EntryKind::File, "file")
            };
            ListEntry::new(EntryId::new(*id), *name, kind, icon)
        })
        .chain((0..extra).map(|i| {
            ListEntry::new(
                EntryId::new(1_000 + i as u64),
                format!("Folder {i:03}"),
                EntryKind::Folder,
                "folder",
            )
        }))
        .collect()
}

/// Makes Home `extra` folders longer (the page's tests scroll it).
pub fn lengthen(d: &mut Demo, extra: usize) {
    d.home = ListSource::new(home_with(extra));
}

/// What the fake lister answers for folder `name`: the rows, the status and the more count.
fn listing(name: &str) -> (Vec<ListEntry>, PeekStatus, Option<More>) {
    let entry = |i: usize, name: String, folder: bool| {
        let (kind, icon) = if folder {
            (EntryKind::Folder, "folder")
        } else {
            (EntryKind::File, "file_text")
        };
        ListEntry::new(EntryId::new(10_000 + i as u64), name, kind, icon)
    };
    let names = |list: &[(&str, bool)]| -> Vec<ListEntry> {
        list.iter()
            .enumerate()
            .map(|(i, (n, f))| entry(i, n.to_string(), *f))
            .collect()
    };
    match name {
        "Projects" => {
            let mut rows = names(&[("src", true), ("docs", true), ("assets", true)]);
            rows.extend((rows.len()..20).map(|i| entry(i, format!("file-{i:02}.txt"), false)));
            (rows, PeekStatus::Ready, None)
        }
        "Music" => (
            names(&[
                ("Albums", true),
                ("playlist.m3u", false),
                (
                    "A very long recording name from the summer festival, remastered.flac",
                    false,
                ),
            ]),
            PeekStatus::Ready,
            None,
        ),
        "Photos" => {
            let rows = (0..4)
                .map(|i| {
                    let mut e = entry(i, format!("IMG_{:04}.jpg", 101 + i), false);
                    e.icon = "file_image".into();
                    e.thumbnail = Some(ThumbKey::new(500 + i as u64));
                    e
                })
                .collect();
            (rows, PeekStatus::Ready, None)
        }
        "Locked" => (
            Vec::new(),
            PeekStatus::Error("Permission denied".into()),
            None,
        ),
        "Slow" => (
            names(&[("a.txt", false), ("b.txt", false)]),
            PeekStatus::Ready,
            None,
        ),
        "Archive" => {
            // Strata stops at 64; the folder holds more, so the count is a floor.
            let rows = (0..CAP)
                .map(|i| entry(i, format!("backup-{i:03}.tar"), false))
                .collect();
            (rows, PeekStatus::Ready, Some(More::AtLeast(CAP - 8)))
        }
        // A host that knows there are none past the rows says so.
        "Downloads" => (
            names(&[("setup.iso", false)]),
            PeekStatus::Ready,
            Some(More::Exactly(0)),
        ),
        _ => (Vec::new(), PeekStatus::Ready, None),
    }
}

/// The host's listing of the peeked folder.
#[derive(Debug, Default)]
pub struct Peeked {
    pub name: String,
    pub status: PeekStatus,
    pub rows: ListSource,
    pub more: Option<More>,
    /// The peek it lists ([`PeekTarget::serial`]).
    ///
    /// [`PeekTarget::serial`]: strata_components::folder_peek::PeekTarget::serial
    pub serial: u64,
}

/// A listing the fake lister is working on: for the peek `serial`, of folder `name`, due on
/// the page's clock.
#[derive(Debug)]
pub struct Job {
    pub serial: u64,
    pub name: String,
    pub due: f64,
}

/// One column: Home.
#[derive(Debug, Default)]
pub struct OneColumn {
    pub source: ListSource,
}

impl ColumnSources for OneColumn {
    fn len(&self) -> usize {
        1
    }
    fn source(&self, _: usize) -> &dyn EntrySource {
        &self.source
    }
    fn version(&self, _: usize) -> u64 {
        1
    }
    fn title(&self, _: usize) -> &str {
        "Home"
    }
    fn open_child(&self, _: usize) -> Option<EntryId> {
        None
    }
}

/// C22's demo state, held in `Gallery::c22`.
pub struct Demo {
    pub mode: Mode,
    pub home: ListSource,
    pub list: CollectionState,
    pub icons: CollectionState,
    pub grid: IconsGridState,
    pub table: ListTableState,
    pub layout: ColumnLayout,
    pub widths: ColumnWidths,
    pub columns: ColumnsState,
    pub column: OneColumn,
    pub peek: PeekState,
    pub peeked: Peeked,
    pub store: ThumbnailStore,
    pub viewer: ViewerId,
    pub clock: f64,
    /// The pane's width, when narrower than the page.
    pub pane: Option<f32>,
    /// What the host last heard.
    pub heard: String,
    /// A peek a scene opens once its view is laid out.
    pub open_at: Option<(&'static str, Origin)>,
    /// The fake lister's jobs, in the order asked.
    pub jobs: Vec<Job>,
    /// Results that came for a peek no longer open, and were dropped.
    pub stale: usize,
    /// What the host heard of the peek, in order.
    pub events: Vec<String>,
    /// A sideways scroll of the List to make once laid out.
    pub scroll_x: Option<f32>,
    /// A stand-in menu is open, its scrim over the window (Ctrl+M).
    pub menu: bool,
    /// Where the pane shows, in window points: the peek's bounds.
    pub pane_rect: Option<Rect>,
}

impl Default for Demo {
    fn default() -> Self {
        Self::new(&Look::fallback(false))
    }
}

fn install(c: &mut Controllers) {
    c.pointer = Some(Box::new(PointerIntent::new()));
    c.keyboard = Some(Box::<Keyboard>::default());
    c.marquee = Some(Box::new(Marquee::new()));
}

impl Demo {
    fn new(look: &Look) -> Self {
        let home = ListSource::new(home());
        let (mut list, mut icons) = (CollectionState::new(), CollectionState::new());
        install(&mut list.controllers);
        install(&mut icons.controllers);
        let column = OneColumn {
            source: ListSource::new(self::home()),
        };
        let mut columns = ColumnsState::new(COLUMNS, Autoscroll::new());
        columns.push(
            Opening::new(&column.source, Listing::Complete),
            Focus::Column,
            look,
            &mut Tx::default(),
        );
        let mut store = ThumbnailStore::new(Settings::default());
        let viewer = store.viewer();
        Self {
            mode: Mode::List,
            home,
            list,
            icons,
            grid: IconsGridState::new(),
            table: ListTableState::new(),
            layout: ColumnLayout::new(super::c4::columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            columns,
            column,
            peek: PeekState::new(),
            peeked: Peeked::default(),
            store,
            viewer,
            clock: 0.0,
            pane: None,
            heard: String::new(),
            open_at: None,
            jobs: Vec::new(),
            stale: 0,
            events: Vec::new(),
            scroll_x: None,
            menu: false,
            pane_rect: None,
        }
    }

    fn name_of(&self, id: EntryId) -> String {
        self.home
            .index_of(id)
            .map(|i| self.home.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    fn id_of(&self, name: &str) -> Option<EntryId> {
        (0..self.home.len())
            .find(|&i| self.home.entry(i).name == name)
            .map(|i| self.home.id_at(i))
    }

    /// The host's side of a peek opening or closing: start or drop the listing.
    fn relist(&mut self, change: Change, tx: &mut Tx) {
        match change {
            Change::None => {}
            Change::Opened => {
                let Some(target) = self.peek.target() else {
                    return;
                };
                let name = self.name_of(target.folder);
                let delay = if name == "Slow" {
                    SLOW_DELAY
                } else {
                    LIST_DELAY
                };
                let serial = target.serial;
                self.heard = format!("Peek {name}");
                self.events.push(format!("open {name} #{serial}"));
                self.jobs.push(Job {
                    serial,
                    name: name.clone(),
                    due: self.clock + delay,
                });
                self.peeked = Peeked {
                    name,
                    serial,
                    ..Peeked::default()
                };
                tx.wake_after(Duration::from_secs_f64(delay));
            }
            Change::Closed => {
                self.heard = "Peek closed".into();
                self.events.push("closed".into());
                self.peeked = Peeked::default();
            }
        }
    }

    fn heard(&mut self, intent: Intent) {
        if let Intent::BeginDrag { ids, .. } = intent {
            // The host's part of "starting a drag closes the peek".
            let closed = self.peek.close();
            if closed {
                self.peeked = Peeked::default();
                self.events.push("closed by the drag".into());
            }
            self.heard = format!(
                "Dragging {} items{}",
                ids.len(),
                if closed { ", peek closed" } else { "" }
            );
        }
    }

    /// Opens or closes the stand-in menu: the host's part of "rows under an overlay don't
    /// peek".
    fn show_menu(&mut self, on: bool, tx: &mut Tx) {
        self.menu = on;
        let change = self.peek.set_covered(on);
        self.relist(change, tx);
    }

    fn coll(&self) -> &CollectionState {
        match self.mode {
            Mode::Icons => &self.icons,
            _ => &self.list,
        }
    }
}

/// The page's peek.
pub fn peek() -> FolderPeek<Gallery> {
    FolderPeek::new(
        |g: &Gallery| &g.c22.peek,
        |g: &mut Gallery| &mut g.c22.peek,
        |g: &Gallery| {
            let p = &g.c22.peeked;
            PeekContents::new(&p.name, &p.status, &p.rows).more(p.more)
        },
        |g: &Gallery| g.look(),
    )
    .thumbnails(|g: &Gallery| &g.c22.store)
    // The pane, which C4's sideways scroll is narrower than when its columns don't fit.
    .bounds(|g: &Gallery| g.c22.pane_rect)
    .on_change(|g: &mut Gallery, change, tx| g.c22.relist(change, tx))
    .on_open(|g: &mut Gallery, target, _| {
        g.c22.heard = format!("Open {}", g.c22.name_of(target.folder));
    })
}

fn marks(scope: &'static str) -> impl Fn(&Gallery, EntryId) -> RowMarks {
    move |g: &Gallery, id| {
        let mut m = RowMarks::default();
        m.peek_anchor = g.c22.peek.is_anchor(scope, id);
        m
    }
}

/// The List collection, before the table adds its parts.
fn list_base() -> Collection<Gallery> {
    Collection::new(
        LIST,
        |g: &Gallery| (&g.c22.home, &g.c22.list),
        |g: &mut Gallery| (&g.c22.home, &mut g.c22.list),
        |g: &Gallery| g.look(),
        |_: &Gallery| 1,
    )
    .name("Home")
    .marks(marks(LIST))
    .on_intent(|g: &mut Gallery, intent, _| g.c22.heard(intent))
}

fn table() -> ListTable<Gallery> {
    ListTable::new(
        LIST,
        super::c4::columns(),
        |g: &Gallery| ListParts {
            source: &g.c22.home,
            collection: &g.c22.list,
            table: &g.c22.table,
            layout: &g.c22.layout,
            widths: &g.c22.widths,
            sort: None,
            look: g.look(),
        },
        |g: &mut Gallery| &mut g.c22.table,
    )
}

/// The List collection as the view builds it: the table's parts and the peek's rows.
fn list_collection(g: &Gallery) -> Collection<Gallery> {
    table().collection(g, peek().rows(list_base()))
}

/// The Icons collection, before the grid adds its parts: the peek's rows.
fn icons_base(_: &Gallery) -> Collection<Gallery> {
    let base = Collection::new(
        ICONS,
        |g: &Gallery| (&g.c22.home, &g.c22.icons),
        |g: &mut Gallery| (&g.c22.home, &mut g.c22.icons),
        |g: &Gallery| g.look(),
        |_: &Gallery| 1,
    )
    .name("Home")
    .marks(marks(ICONS))
    .on_intent(|g: &mut Gallery, intent, _| g.c22.heard(intent));
    peek().rows(base)
}

/// C3's IconsGrid over Home.
fn grid() -> IconsGrid<Gallery> {
    IconsGrid::new(
        ICONS,
        icons_base,
        |g: &Gallery| IconsParts {
            source: &g.c22.home,
            collection: &g.c22.icons,
            grid: &g.c22.grid,
            size: TILE,
            listing: 1,
            look: g.look(),
            thumbs: None,
        },
        |g: &mut Gallery| &mut g.c22.grid,
    )
}

/// The Icons collection as the view builds it: the grid's parts and the peek's rows.
fn icons_collection(g: &Gallery) -> Collection<Gallery> {
    grid().collection(g)
}

fn columns_view() -> ColumnsView<Gallery> {
    ColumnsView::new(
        |g: &Gallery| ColumnsParts {
            sources: &g.c22.column,
            state: &g.c22.columns,
            look: &g.look,
        },
        |g: &mut Gallery| ColumnsPartsMut {
            sources: &g.c22.column,
            state: &mut g.c22.columns,
            look: &g.look,
        },
    )
}

// Scenes.

fn start(g: &mut Gallery, dark: bool) -> &mut Demo {
    g.open("c22", dark);
    let look = g.look.clone();
    g.c22 = Demo::new(&look);
    g.c22.peek.set_enabled(true);
    &mut g.c22
}

fn list(g: &mut Gallery, dark: bool) {
    start(g, dark);
}

fn opened(g: &mut Gallery, dark: bool, mode: Mode, folder: &'static str) {
    let d = start(g, dark);
    d.mode = mode;
    d.open_at = Some((folder, Origin::Hover));
}

fn off(g: &mut Gallery, dark: bool) {
    g.open("c22", dark);
    let look = g.look.clone();
    g.c22 = Demo::new(&look);
}

fn narrow(g: &mut Gallery, dark: bool) {
    start(g, dark).pane = Some(240.0);
}

fn columns(g: &mut Gallery, dark: bool) {
    start(g, dark).mode = Mode::Columns;
}

fn long(g: &mut Gallery, dark: bool) {
    lengthen(start(g, dark), 400);
}

fn icons(g: &mut Gallery, dark: bool) {
    start(g, dark).mode = Mode::Icons;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c22", dark)`, then fills in `g.c22`.
pub const SCENES: &[Scene] = &[
    ("c22-list-light", |g| list(g, false)),
    ("c22-list-dark", |g| list(g, true)),
    ("c22-list-open-light", |g| {
        opened(g, false, Mode::List, "Projects")
    }),
    ("c22-list-open-dark", |g| {
        opened(g, true, Mode::List, "Projects")
    }),
    ("c22-icons-right-light", |g| {
        opened(g, false, Mode::Icons, "Music")
    }),
    ("c22-icons-right-dark", |g| {
        opened(g, true, Mode::Icons, "Music")
    }),
    ("c22-icons-left-light", |g| {
        opened(g, false, Mode::Icons, "Archive")
    }),
    ("c22-icons-left-dark", |g| {
        opened(g, true, Mode::Icons, "Archive")
    }),
    ("c22-thumbnails-light", |g| {
        opened(g, false, Mode::Icons, "Photos")
    }),
    ("c22-thumbnails-dark", |g| {
        opened(g, true, Mode::Icons, "Photos")
    }),
    ("c22-loading-light", |g| {
        opened(g, false, Mode::List, "Slow")
    }),
    ("c22-loading-dark", |g| opened(g, true, Mode::List, "Slow")),
    ("c22-empty-light", |g| opened(g, false, Mode::List, "Empty")),
    ("c22-empty-dark", |g| opened(g, true, Mode::List, "Empty")),
    ("c22-error-light", |g| {
        opened(g, false, Mode::List, "Locked")
    }),
    ("c22-error-dark", |g| opened(g, true, Mode::List, "Locked")),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c22-off-light", |g| off(g, false)),
    ("c22-off-dark", |g| off(g, true)),
    ("c22-icons-light", |g| icons(g, false)),
    ("c22-icons-dark", |g| icons(g, true)),
    ("c22-narrow-light", |g| narrow(g, false)),
    ("c22-narrow-dark", |g| narrow(g, true)),
    ("c22-columns-light", |g| columns(g, false)),
    ("c22-columns-dark", |g| columns(g, true)),
    ("c22-long-light", |g| long(g, false)),
    ("c22-long-dark", |g| long(g, true)),
];

/// Something to drag from outside the collections (a sidebar row, a tab, another pane), for
/// the host's part of "ending a drag cancels a pending peek".
fn drag_source(g: &Gallery) -> El {
    let kit = g.kit();
    row()
        .key("c22-drag-source")
        .px(g.look().space.sm)
        .radius(kit.radius.sm)
        .border(1.0, kit.c.std_border)
        .child(
            text("Drag me")
                .font(kit.ty.ui_compact)
                .color(kit.c.text_muted),
        )
        .on_drag(|g: &mut Gallery, d: &mut rune::Drag, _| {
            if d.phase == rune::Phase::Up {
                g.c22.heard = "Dropped".into();
                // The host's part of "ending a drag cancels a pending peek".
                g.c22.list.hold_rest();
                g.c22.icons.hold_rest();
            }
        })
}

fn label(key: &str, t: String, g: &Gallery) -> El {
    let kit = g.kit();
    text(t).key(key).font(kit.ty.ui).color(kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c22;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let mut layer = Vec::new();
    let header = row()
        .w_full()
        .px(look.space.lg)
        .py(look.space.xs)
        .gap(look.space.lg)
        .center_y()
        .child(
            text(format!("Home · {}", d.mode.label()))
                .key("c22-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(label(
            "c22-peeking",
            format!(
                "Folder peeking {}",
                if d.peek.enabled() { "on" } else { "off" }
            ),
            g,
        ))
        .child(label("c22-heard", d.heard.clone(), g))
        .child(drag_source(g));
    let body = match d.mode {
        // The peek follows the table's sideways scroll: moving it closes the peek.
        Mode::List => table()
            .follow_sideways(|g: &Gallery| peek().follow_sideways(g))
            .build(g, peek().rows(list_base()), &mut layer),
        Mode::Icons => grid().build(g),
        Mode::Columns => columns_view().build(g),
    };
    if d.mode != Mode::Columns {
        peek().card(g, &mut layer);
    }
    if d.menu {
        // A menu's outside-press scrim over the window, and the menu over the rows.
        layer.push(
            rune_kit::scrim("c22-scrim", |_: &mut Gallery| {})
                .on_press_tx(|g: &mut Gallery, tx| g.c22.show_menu(false, tx)),
        );
        layer.push(
            col()
                .key("c22-menu")
                .role("menu")
                .label("Menu")
                .pin(Some(20.0), Some(70.0), None, None)
                .w(400.0)
                .h(200.0)
                .radius(kit.radius.md)
                .fill(kit.c.surface_raised)
                .border(1.0, kit.c.std_border)
                .on_press(|_: &mut Gallery| {}),
        );
    }
    let pane = match d.pane {
        Some(w) => col().w(w),
        None => col().w_full(),
    }
    .h_full()
    .min_h(0.0)
    .on_layout_tx(|g: &mut Gallery, r, tx| {
        let r = Rect::new(r[0], r[1], r[2], r[3]);
        if g.c22.pane_rect == Some(r) {
            tx.live_only();
        }
        g.c22.pane_rect = Some(r);
    })
    .child(body);
    let page = col().key("c22-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(pane),
    );
    rune_kit::root(page, layer)
}

/// The window rect of folder `id`'s row in the current view, and its viewport.
fn rects(g: &Gallery, id: EntryId) -> Option<(&'static str, Rect, Rect)> {
    let d = &g.c22;
    let index = d.home.index_of(id)?;
    let (scope, c) = match d.mode {
        Mode::List => (LIST, list_collection(g)),
        Mode::Icons => (ICONS, icons_collection(g)),
        Mode::Columns => return None,
    };
    let viewport = peek().viewport(g, d.coll().viewport());
    (viewport.w > 0.0).then(|| (scope, c.window_rect_of(g, index), viewport))
}

pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let mode = |k: &KeyEvent| {
        [('1', Mode::List), ('2', Mode::Icons), ('3', Mode::Columns)]
            .into_iter()
            .find(|(c, _)| k.is_command(*c))
            .map(|(_, m)| m)
    };
    if let Some(m) = mode(key) {
        let d = &mut g.c22;
        d.mode = m;
        // The host's part of "switching views closes the peek": a keyboard peek has no
        // pointer leave to close it.
        if d.peek.close() {
            d.relist(Change::Closed, tx);
        }
        return true;
    }
    if key.is_command('m') {
        let on = !g.c22.menu;
        g.c22.show_menu(on, tx);
        return true;
    }
    if key.is_command('p') {
        let d = &mut g.c22;
        let on = !d.peek.enabled();
        if d.peek.set_enabled(on) {
            d.relist(Change::Closed, tx);
        }
        // The host's part of "turning it off cancels a pending peek, and on peeks nothing
        // until the pointer arrives": a rest under way when peeking changed doesn't count.
        d.list.hold_rest();
        d.icons.hold_rest();
        return true;
    }
    match &key.key {
        Key::Escape => {
            let d = &mut g.c22;
            if d.peek.close() {
                d.relist(Change::Closed, tx);
                return true;
            }
            false
        }
        Key::Text(t) if t == "i" && key.mods == rune::Mods::default() => {
            let d = &g.c22;
            let Some(id) = d.coll().selection.cursor() else {
                return true;
            };
            let folder = d
                .home
                .index_of(id)
                .is_some_and(|i| d.home.entry(i).kind.is_folder_like());
            if let (true, Some((scope, item, viewport))) = (folder, rects(g, id)) {
                let change = g.c22.peek.toggle(scope, id, item, viewport);
                g.c22.relist(change, tx);
            }
            true
        }
        _ => false,
    }
}

/// A fake thumbnail: a gradient, its hue from the key.
fn picture(px: u32, seed: u64) -> Vec<u8> {
    let hue = (seed % 7) as f32 / 7.0 * std::f32::consts::TAU;
    let base = [hue.cos(), (hue + 2.1).cos(), (hue + 4.2).cos()];
    let mut out = Vec::with_capacity((px * px * 4) as usize);
    for y in 0..px {
        let light = 0.45 + 0.45 * (1.0 - y as f32 / px as f32);
        for _ in 0..px {
            for c in base {
                out.push(((0.5 + 0.45 * c) * light * 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    g.c22.clock = t;
    // After the table's first layouts settle (its first width is the columns' alone): the
    // placement is worked out once, as the peek opens.
    if let Some((name, origin)) = g.c22.open_at
        && t >= SCENE_OPEN
        && let Some(id) = g.c22.id_of(name)
        && let Some((scope, item, viewport)) = rects(g, id)
    {
        let d = &mut g.c22;
        d.open_at = None;
        let change = d.peek.open(scope, id, origin, item, viewport);
        d.relist(change, tx);
        changed = true;
    }
    let look = g.look.clone();
    let d = &mut g.c22;
    if d.list.viewport().w > 0.0
        && let Some(x) = d.scroll_x.take()
    {
        tx.scroll_to_x(format!("{LIST}-scroll"), x);
    }
    // Results come back in the order asked; one for a peek no longer open is dropped by its
    // serial, as Strata's request ids drop a closed peek's batches.
    let due: Vec<Job> = {
        let (due, rest) = std::mem::take(&mut d.jobs)
            .into_iter()
            .partition(|j| t >= j.due);
        d.jobs = rest;
        due
    };
    for job in due {
        if d.peek.target().map(|t| t.serial) != Some(job.serial) {
            d.stale += 1;
            continue;
        }
        let (rows, status, more) = listing(&job.name);
        d.peeked.rows = ListSource::new(rows);
        d.peeked.status = status;
        d.peeked.more = more;
        let step = Step::for_tile(&look, look.icon_size.md, 2.0);
        let shown: Range<usize> = 0..d.peeked.rows.len().min(8);
        d.store
            .sync(d.viewer, &d.peeked.rows, Viewport::new(shown, step));
        changed = true;
    }
    while let Some(r) = d.store.pop(t) {
        let px = r.step.px();
        let rgba = picture(px, r.key.get());
        d.store.complete(
            r.ticket,
            Outcome::Ready {
                width: px,
                height: px,
                rgba,
            },
            t,
        );
        changed = true;
    }
    changed
        | match g.c22.mode {
            Mode::List => list_collection(g).tick(g, tx),
            Mode::Icons => {
                let ticked = icons_collection(g).tick(g, tx);
                grid().poll(g, tx) | ticked
            }
            Mode::Columns => columns_view().tick(g, t, tx),
        }
}
