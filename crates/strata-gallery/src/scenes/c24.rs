//! The gallery page for C24 Scroll extras: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c24-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C24-scroll-extras.md`.
//!
//! Four panes, one at a time:
//!
//! - **List**: a 10,000-entry collection (every tenth a folder) with List's overlay scroll bar
//!   and middle-click autoscroll in front of C1b's pointer intent, with C1c's keys and C1d's
//!   marquee installed. The header shows the last intent (`c24-intent`), the last back or
//!   forward button (`c24-history`), the autoscroll's state (`c24-status`), a stand-in for an
//!   armed 10xer chord (`c24-chord`) and a filter field (`c24-filter`).
//! - **Nested**: three 1,000-entry collections in a sideways strip, as Columns lays them out:
//!   each with a column's bar, the strip with its own, one autoscroll for the window.
//! - **Bars**: a plain viewport that scrolls both ways, with the app-wide bar on each axis and
//!   autoscroll on both.
//! - **Panel**: a stand-in sidebar (`c24-sidebar`, a plain scroll viewport) beside the list, and
//!   a stand-in Sort by panel (`c24-panel`, opened and closed by `c24-panel-button`) with C24's
//!   [`OutsideWheel`] outside it: a wheel tick outside closes it and scrolls the list under the
//!   pointer, and nothing over the sidebar or header.
//!
//! | Key | Does |
//! | --- | --- |
//! | Alt+Left | Go to another folder (List): the autoscroll in the one left stops |
//! | Ctrl+H | Hide or show the list: hiding it stops its autoscroll |
//! | g | Arm the stand-in 10xer chord; Escape (or an Escape that stops autoscroll) cancels it |
//!
//! `STRATA_C24_BUDGET_WARMUP=1` makes the page's second poll wait 1.1 s of real time once,
//! then print [`WARMED`], as C1e's page does for Rune's once-a-second budget lines (Rune draft
//! U-C1e-5). Only `tests/c24.rs`'s budget run sets it.

use rune::{
    Cx, Drag, El, Key, KeyEvent, PointerButton, ScrollPos, TextEdit, Tx, col, row, text, text_input,
};
use strata_components::collection::element::{
    Collection, CollectionState, Controllers, Intent, viewport_path,
};
use strata_components::collection::marquee::MarqueeSettings;
use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource, Point, Rect};
use strata_components::rename::{Initial, RenameEditor, RenameState};
use strata_components::scroll::{
    Autoscroll, AutoscrollPointer, AutoscrollSettings, Bar, BarLook, BarMode, OutsideWheel,
    ScrollFeed, Shield, Start, Stop, Target, overlay,
};

use crate::{Gallery, Scene};

/// The variable that turns on the budget warm-up (see the module docs).
pub const WARMUP: &str = "STRATA_C24_BUDGET_WARMUP";

/// What the page prints on stderr once the warm-up is over.
pub const WARMED: &str = "c24: budget warm-up done";

/// The list's collection key.
pub const SCOPE: &str = "c24-files";

/// The list bar's key.
pub const BAR: &str = "c24-files-bar";

/// The nested pane's strip key, and its columns' collection keys (`c24-col-0` …).
pub const STRIP: &str = "c24-strip";

/// The Bars pane's viewport key.
pub const GRID: &str = "c24-grid";

/// How many columns the nested pane shows.
pub const COLUMNS: usize = 3;

/// A nested column's width: three overflow the gallery's 960.
const COLUMN_W: f32 = 380.0;

/// Which pane the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    #[default]
    List,
    Nested,
    Bars,
    Panel,
}

/// One column of the nested pane.
pub struct Column {
    pub listing: ListSource,
    pub coll: CollectionState,
    pub feed: ScrollFeed,
    pub bar: Bar,
}

/// C24's demo state, held in `Gallery::c24`.
pub struct Demo {
    pub pane: Pane,
    pub listing: ListSource,
    pub coll: CollectionState,
    pub version: u64,
    pub feed: ScrollFeed,
    pub bar: Bar,
    /// The window's one autoscroll, shared by every collection's decorator and the grid.
    pub auto: Autoscroll,
    pub columns: Vec<Column>,
    pub strip_feed: ScrollFeed,
    pub strip_bar: Bar,
    pub grid_feed: ScrollFeed,
    pub grid_bars: [Bar; 2],
    pub grid_rect: Rect,
    pub intent: String,
    pub history: String,
    pub stopped: String,
    pub chord: bool,
    pub filter: TextEdit,
    /// C5's rename editor (the `c24-rename` scene): a text field inside the listing.
    pub rename: RenameState,
    pub clock: f64,
    pub hidden: bool,
    /// Which folder the list shows: 0 the first, 1 after Alt+Left.
    pub folder: u8,
    /// A scroll the poll asks for once the list has laid out (a scene's).
    pub scroll_to: Option<f32>,
    /// Start an autoscroll at the list's centre once it has laid out (a scene's).
    pub demo_start: bool,
    pub polls: u32,
    /// The stand-in Sort by panel is open (the Panel pane).
    pub panel: bool,
    /// Where the stand-in sidebar stands.
    pub side_feed: ScrollFeed,
}

fn entries(n: u64, first_id: u64, prefix: &str) -> Vec<ListEntry> {
    (0..n)
        .map(|i| {
            let folder = i % 10 == 0;
            let (kind, icon, name) = if folder {
                (
                    EntryKind::Folder,
                    "folder",
                    format!("{prefix} folder {i:05}"),
                )
            } else {
                (
                    EntryKind::File,
                    "file_text",
                    format!("{prefix} file {i:05}.txt"),
                )
            };
            ListEntry::new(EntryId::new(first_id + i), name, kind, icon)
        })
        .collect()
}

/// C1b's pointer intent behind C24's autoscroll, C1c's keys and C1d's marquee.
fn install(c: &mut Controllers, auto: &Autoscroll, scope: &str, feed: &ScrollFeed) {
    c.pointer = Some(Box::new(AutoscrollPointer::new(
        Some(Box::new(
            strata_components::collection::pointer::PointerIntent::new(),
        )),
        auto.clone(),
        scope,
        feed.clone(),
    )));
    c.keyboard = Some(Box::<strata_components::collection::keyboard::Keyboard>::default());
    c.marquee = Some(Box::new(
        strata_components::collection::marquee::Marquee::new(),
    ));
}

impl Default for Demo {
    fn default() -> Self {
        let auto = Autoscroll::new();
        let feed = ScrollFeed::new();
        let mut coll = CollectionState::new();
        install(&mut coll.controllers, &auto, SCOPE, &feed);
        let columns = (0..COLUMNS)
            .map(|i| {
                let feed = ScrollFeed::new();
                let mut coll = CollectionState::new();
                install(&mut coll.controllers, &auto, &column_key(i), &feed);
                Column {
                    listing: ListSource::new(entries(1_000, 100_000 * (i as u64 + 1), "col")),
                    coll,
                    feed,
                    bar: Bar::new(),
                }
            })
            .collect();
        Self {
            pane: Pane::List,
            listing: ListSource::new(entries(10_000, 1, "item")),
            coll,
            version: 0,
            feed,
            bar: Bar::new(),
            auto,
            columns,
            strip_feed: ScrollFeed::new(),
            strip_bar: Bar::new(),
            grid_feed: ScrollFeed::new(),
            grid_bars: [Bar::new(), Bar::new()],
            grid_rect: Rect::default(),
            intent: "Nothing yet".to_owned(),
            history: "No history button".to_owned(),
            stopped: String::new(),
            chord: false,
            filter: TextEdit::new(""),
            rename: RenameState::new(),
            clock: 0.0,
            hidden: false,
            folder: 0,
            scroll_to: None,
            demo_start: false,
            polls: 0,
            panel: false,
            side_feed: ScrollFeed::new(),
        }
    }
}

/// A nested column's collection key.
pub fn column_key(i: usize) -> String {
    format!("c24-col-{i}")
}

impl Demo {
    fn name_of(&self, id: EntryId) -> String {
        use strata_components::contract::EntrySource;
        let listing: &dyn EntrySource = match self.pane {
            Pane::Nested => self
                .columns
                .iter()
                .find(|c| c.listing.index_of(id).is_some())
                .map_or(&self.listing, |c| &c.listing),
            _ => &self.listing,
        };
        listing
            .index_of(id)
            .map(|i| listing.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    fn heard(&mut self, intent: Intent) {
        self.intent = match intent {
            Intent::OpenInNew { id, window } => {
                let place = if window { "window" } else { "tab" };
                format!("Opened {} in a new {place}", self.name_of(id))
            }
            Intent::Activate { id, .. } => format!("Opened {}", self.name_of(id)),
            Intent::Clicked { id } => format!("Clicked {}", self.name_of(id)),
            _ => "Other".to_owned(),
        };
    }

    fn stopped_by(&mut self, stop: Stop) {
        self.stopped = match stop {
            Stop::Escape => {
                // Strata's Escape that stops autoscroll also cancels an armed chord.
                self.chord = false;
                "Stopped by Escape".to_owned()
            }
            Stop::Press(b) => format!("Stopped by a {b:?} press"),
            _ => "Stopped".to_owned(),
        };
    }

    /// Alt+Left: another folder. `enter` resets the controllers, which stops the list's
    /// autoscroll.
    fn go(&mut self, tx: &mut Tx) {
        use strata_components::collection::selection::Listing;
        self.folder = 1 - self.folder;
        let prefix = if self.folder == 0 { "item" } else { "other" };
        self.listing = ListSource::new(entries(10_000, 1 + 50_000 * self.folder as u64, prefix));
        self.version += 1;
        self.coll
            .enter(SCOPE, &self.listing, None, Listing::Complete, tx);
    }
}

fn scene(g: &mut Gallery, dark: bool, pane: Pane) {
    g.open("c24", dark);
    g.c24 = Demo {
        pane,
        ..Demo::default()
    };
}

/// The list at rest: the overlay bar is hidden.
fn ten_k(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
}

/// The sidebar and the list with the stand-in Sort by panel open over them.
fn panel(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::Panel);
    g.c24.panel = true;
}

/// The list scrolled into its middle with the bar shown (`BarMode::Always`).
fn ten_k_bar(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
    g.c24.bar.set_mode(BarMode::Always);
    g.c24.scroll_to = Some(140_000.0);
}

/// An autoscroll running, the pointer still on its anchor in the list's middle.
fn autoscroll(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
    g.c24.demo_start = true;
}

/// Ten entries: the list fits, so no bar and no autoscroll.
fn fits(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
    g.c24.listing = ListSource::new(entries(10, 1, "item"));
}

/// The strip of three columns, each bar and the strip's shown.
fn nested(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::Nested);
    g.c24.strip_bar.set_mode(BarMode::Always);
    for c in &g.c24.columns {
        c.bar.set_mode(BarMode::Always);
    }
}

/// The plain viewport, both bars shown.
fn bars(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::Bars);
    for b in &g.c24.grid_bars {
        b.set_mode(BarMode::Always);
    }
}

/// C5's rename editor open on a folder's row (`item folder 00010`, `row-11`): a text field
/// inside the listing, where Strata's `is_text_target` keeps a middle click from starting
/// autoscroll, and where the row under it would open in a new tab.
fn rename(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
    let _ = g.c24.rename.begin(
        SCOPE,
        EntryId::new(11),
        "item folder 00010",
        EntryKind::Folder,
        Initial::Stem,
    );
}

/// C5's editor on the list's rows.
fn rename_editor() -> RenameEditor<Gallery> {
    RenameEditor::new(
        |g: &Gallery| &g.c24.rename,
        |g: &mut Gallery| &mut g.c24.rename,
    )
}

/// The list with animations off: autoscroll steps, the bar shows and hides at once.
fn reduced(g: &mut Gallery, dark: bool) {
    scene(g, dark, Pane::List);
    g.look.animations = false;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c24", dark)`, then fills in `g.c24`.
pub const SCENES: &[Scene] = &[
    ("c24-ten-k-light", |g| ten_k(g, false)),
    ("c24-ten-k-dark", |g| ten_k(g, true)),
    ("c24-ten-k-bar-light", |g| ten_k_bar(g, false)),
    ("c24-ten-k-bar-dark", |g| ten_k_bar(g, true)),
    ("c24-autoscroll-light", |g| autoscroll(g, false)),
    ("c24-autoscroll-dark", |g| autoscroll(g, true)),
    ("c24-fits-light", |g| fits(g, false)),
    ("c24-fits-dark", |g| fits(g, true)),
    ("c24-nested-light", |g| nested(g, false)),
    ("c24-nested-dark", |g| nested(g, true)),
    ("c24-bars-light", |g| bars(g, false)),
    ("c24-bars-dark", |g| bars(g, true)),
    ("c24-panel-light", |g| panel(g, false)),
    ("c24-panel-dark", |g| panel(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c24-reduced-light", |g| reduced(g, false)),
    ("c24-reduced-dark", |g| reduced(g, true)),
    ("c24-rename-light", |g| rename(g, false)),
    ("c24-rename-dark", |g| rename(g, true)),
];

/// The controllers' settings, from the look: the page passes them each time it builds or ticks
/// a collection.
pub type Settings = (MarqueeSettings, AutoscrollSettings);

pub fn settings(g: &Gallery) -> Settings {
    let mut marquee = MarqueeSettings::default();
    marquee.animations = g.look().animations;
    let mut auto = AutoscrollSettings::default();
    auto.margin = strata_kit::slots::collection::row_inset(g.look());
    (marquee, auto)
}

/// The list's collection, as the page builds it for its view and its poll.
pub fn collection((marquee, auto): Settings) -> Collection<Gallery> {
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c24.listing, &g.c24.coll),
        |g: &mut Gallery| (&g.c24.listing, &mut g.c24.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c24.version,
    )
    .settings(marquee)
    .settings(auto)
    .on_intent(|g: &mut Gallery, intent, _| g.c24.heard(intent))
    .rename(
        |g: &Gallery| g.c24.rename.renaming_in(SCOPE),
        rename_editor().slot(),
    )
}

/// Nested column `i`'s collection.
pub fn column((marquee, auto): Settings, i: usize) -> Collection<Gallery> {
    Collection::new(
        column_key(i),
        move |g: &Gallery| (&g.c24.columns[i].listing, &g.c24.columns[i].coll),
        move |g: &mut Gallery| {
            let c = &mut g.c24.columns[i];
            (&c.listing, &mut c.coll)
        },
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c24.version,
    )
    .row_height(|_: &Gallery, look| look.row.columns)
    .settings(marquee)
    .settings(auto)
    .on_intent(|g: &mut Gallery, intent, _| g.c24.heard(intent))
}

/// Feeds `feed` and `bar` from a viewport's scroll position.
fn follow(feed: &ScrollFeed, bar: &Bar) -> impl Fn(ScrollPos) + 'static {
    let (feed, bar) = (feed.clone(), bar.clone());
    move |p| {
        feed.set(p);
        bar.follow(p);
    }
}

fn list_view(g: &Gallery) -> El {
    let d = &g.c24;
    if d.hidden {
        return col()
            .w_full()
            .grow(1.0)
            .center()
            .child(text("The list is hidden (Ctrl+H)").key("c24-hidden"));
    }
    let list = collection(settings(g))
        .name("Files")
        .on_scroll(follow(&d.feed, &d.bar))
        .edge(|g: &Gallery| {
            Some(
                g.c24
                    .bar
                    .build::<Gallery>(BAR, &viewport_path(SCOPE), BarLook::list(g.look())),
            )
        })
        .build(g);
    d.bar.area(list)
}

fn nested_view(g: &Gallery) -> El {
    let d = &g.c24;
    let mut strip = row().h_full();
    for i in 0..COLUMNS {
        let c = &d.columns[i];
        let list = column(settings(g), i)
            .name(format!("Column {}", i + 1))
            .on_scroll(follow(&c.feed, &c.bar))
            .edge(move |g: &Gallery| {
                Some(g.c24.columns[i].bar.build::<Gallery>(
                    &format!("{}-bar", column_key(i)),
                    &viewport_path(&column_key(i)),
                    BarLook::column(g.look()),
                ))
            })
            .build(g);
        strip = strip.child(col().w(COLUMN_W).h_full().child(c.bar.area(list)));
    }
    let viewport = col()
        .key(STRIP)
        .w_full()
        .h_full()
        .scroll_x()
        .on_scroll_offset(follow(&d.strip_feed, &d.strip_bar))
        .child(strip);
    let bar = d
        .strip_bar
        .build::<Gallery>("c24-strip-bar", STRIP, BarLook::strip(g.look()));
    d.strip_bar.area(overlay(viewport, [bar]))
}

fn grid_view(g: &Gallery) -> El {
    let d = &g.c24;
    let kit = g.kit();
    let mut content = col().w(2_400.0);
    for r in 0..80 {
        let mut line = row().h(28.0).gap(16.0).px(12.0).center_y();
        for c in 0..12 {
            line = line.child(
                text(format!("cell {r}:{c}"))
                    .w(180.0)
                    .font(kit.ty.ui)
                    .color(kit.c.text_muted),
            );
        }
        content = content.child(line);
    }
    let (feed, bars) = (d.grid_feed.clone(), d.grid_bars.clone());
    let viewport = col()
        .key(GRID)
        .w_full()
        .h_full()
        .scroll_xy()
        .on_scroll_offset(move |p| {
            feed.set(p);
            for b in &bars {
                b.follow(p);
            }
        })
        .on_layout_tx(|g: &mut Gallery, r, tx: &mut Tx| {
            g.c24.grid_rect = Rect::from(r);
            tx.live_only();
        })
        .pointer_buttons([PointerButton::Middle])
        .on_drag(|g: &mut Gallery, d: &mut Drag, tx: &mut Tx| {
            // A plain viewport's own middle press: everything here is background.
            if d.phase != rune::Phase::Down || tx.button() != PointerButton::Middle {
                tx.live_only();
                return;
            }
            let [x, y] = d.window_point();
            let start = Start {
                owner: GRID.to_owned(),
                anchor: Point::new(x, y),
                viewport: g.c24.grid_rect,
                targets: vec![Target::both(GRID, g.c24.grid_feed.clone())],
            };
            if !g.c24.auto.start(start, AutoscrollSettings::default()) {
                tx.live_only();
            }
        })
        .child(content);
    let app = BarLook::app(g.look());
    let bars = [
        d.grid_bars[0].build::<Gallery>("c24-grid-vbar", GRID, app),
        d.grid_bars[1].build::<Gallery>("c24-grid-hbar", GRID, app.horizontal()),
    ];
    let wrapped = overlay(viewport, bars);
    // One area, revealing both.
    let inner = d.grid_bars[1].area(wrapped);
    d.grid_bars[0].area(inner)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c24;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let label =
        |key: &'static str, s: String| text(s).key(key).font(kit.ty.ui).color(kit.c.text_muted);
    let status = if d.auto.active() {
        "Autoscroll: running".to_owned()
    } else if d.stopped.is_empty() {
        "Autoscroll: off".to_owned()
    } else {
        d.stopped.clone()
    };
    let chord = if d.chord { "Chord armed" } else { "No chord" };
    let header = row()
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .gap(look.space.lg)
        .center_y()
        .child(
            text("Scroll extras")
                .key("c24-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(label("c24-intent", d.intent.clone()))
        .child(label("c24-history", d.history.clone()))
        .child(label("c24-status", status))
        .child(label("c24-chord", chord.to_owned()))
        .child(
            row()
                .w(150.0)
                .h(look.frame.control)
                .px(look.space.xs)
                .center_y()
                .radius(kit.radius.sm)
                .fill(kit.c.bg_base)
                .outline(1.0, -1.0, kit.c.border_base)
                .child(
                    text_input(&d.filter, kit.ty.ui_compact)
                        .color(kit.c.text_base)
                        .key("c24-filter")
                        .label("Filter")
                        .on_edit(|g: &mut Gallery| &mut g.c24.filter)
                        .grow(1.0),
                ),
        );
    let body = match d.pane {
        Pane::List => list_view(g),
        Pane::Nested => nested_view(g),
        Pane::Bars => grid_view(g),
        Pane::Panel => panel_view(g),
    };
    let mut page = col()
        .key("c24-page")
        .w_full()
        .h_full()
        // The page's main region, not a button: Rune reads a plain group with a press or drag
        // handler as one, and this one hears only the back and forward buttons.
        .role("main")
        .pointer_buttons([PointerButton::Back, PointerButton::Forward])
        .on_drag(
            |g: &mut Gallery, d: &mut Drag, tx: &mut Tx| match (d.phase, tx.button()) {
                (rune::Phase::Down, PointerButton::Back) => g.c24.history = "Back".to_owned(),
                (rune::Phase::Down, PointerButton::Forward) => g.c24.history = "Forward".to_owned(),
                _ => tx.live_only(),
            },
        )
        .child(header)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .py(look.content.pad)
                .child(body),
        );
    if d.pane == Pane::Panel && d.panel {
        // Over the window, under the panel, in place of a popover's own outside scrim.
        page = page
            .child(outside_wheel().scrim(PANEL_SCRIM))
            .child(sort_panel(g));
    }
    if let Some(shield) = Shield::new(|g: &Gallery| &g.c24.auto, |g: &Gallery| g.look())
        .on_stop(|g: &mut Gallery, stop| g.c24.stopped_by(stop))
        .build(g)
    {
        page = page.child(shield);
    }
    page
}

/// The stand-in panel's key.
pub const PANEL: &str = "c24-panel";

/// The layer outside the panel.
pub const PANEL_SCRIM: &str = "c24-panel-scrim";

/// The stand-in sidebar's key.
pub const SIDEBAR: &str = "c24-sidebar";

/// The stand-in panel's outside: a tick over the list scrolls the list; anywhere else (the
/// sidebar, the header) it only closes the panel.
fn outside_wheel() -> OutsideWheel<Gallery> {
    OutsideWheel::new(|g: &mut Gallery, _| g.c24.panel = false).listings(|g: &Gallery, at| {
        if g.c24.coll.viewport().contains(at) {
            vec![Target::vertical(viewport_path(SCOPE), g.c24.feed.clone())]
        } else {
            Vec::new()
        }
    })
}

/// The stand-in Sort by panel, pinned over the list's top-right.
fn sort_panel(g: &Gallery) -> El {
    let kit = g.kit();
    let look = g.look();
    let mut rows = col().gap(look.space.xxs).padding(look.space.sm);
    for name in ["Name", "Size", "Type", "Modified"] {
        rows = rows.child(
            text(name.to_owned())
                .font(kit.ty.ui)
                .color(kit.c.text_base)
                .px(look.space.sm)
                .py(look.space.xxs),
        );
    }
    let panel = col()
        .key(PANEL)
        .role("dialog")
        .label("Sort by")
        .absolute(560.0, 64.0)
        .w(220.0)
        .radius(kit.radius.md)
        .fill(kit.c.surface_raised)
        .border(1.0, kit.c.std_border)
        // A press inside is the panel's, not the scrim's.
        .on_press(|_: &mut Gallery| {})
        .child(rows);
    OutsideWheel::<Gallery>::surface(panel)
}

/// The Panel pane: the stand-in sidebar and the list.
fn panel_view(g: &Gallery) -> El {
    let kit = g.kit();
    let look = g.look();
    let mut items = col().w_full().gap(look.space.xxs).padding(look.space.sm);
    for i in 0..100 {
        items = items.child(
            text(format!("Place {i:02}"))
                .font(kit.ty.ui)
                .color(kit.c.text_muted)
                .py(look.space.xxs),
        );
    }
    let sidebar = col()
        .key(SIDEBAR)
        .w(180.0)
        .h_full()
        .min_h(0.0)
        .fill(kit.c.layer_01)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(g.c24.side_feed.watch())
        .child(items);
    let button = text(
        if g.c24.panel {
            "Close Sort by"
        } else {
            "Sort by"
        }
        .to_owned(),
    )
    .key("c24-panel-button")
    .role("button")
    .font(kit.ty.ui)
    .color(kit.c.text_base)
    .px(look.space.sm)
    .on_press(|g: &mut Gallery| g.c24.panel = !g.c24.panel);
    row().w_full().h_full().min_h(0.0).child(sidebar).child(
        col()
            .grow(1.0)
            .h_full()
            .min_h(0.0)
            .child(button)
            .child(col().w_full().grow(1.0).min_h(0.0).child(list_view(g))),
    )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c24;
    match &key.key {
        Key::Left if key.mods.alt && d.pane == Pane::List => d.go(tx),
        _ if key.is_command('h') => {
            d.hidden = !d.hidden;
            if d.hidden {
                // The host stops the autoscroll of a listing it hides.
                d.auto.stop_gone(Some(SCOPE));
            }
        }
        Key::Text(t) if t == "g" && !key.mods.ctrl && !key.mods.cmd => d.chord = true,
        Key::Escape if d.chord => d.chord = false,
        _ => return false,
    }
    true
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let animations = g.look().animations;
    let d = &mut g.c24;
    d.clock = t;
    d.polls = d.polls.saturating_add(1);
    if d.polls == 2 && std::env::var_os(WARMUP).is_some_and(|v| !v.is_empty()) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        eprintln!("{WARMED}");
    }
    let mut changed = false;
    // C5's poll: the edit ends on a blur, or its row leaving the view or the listing.
    if d.rename.is_open() {
        changed |= d.rename.tick(SCOPE, &d.listing, &d.coll).changed;
    }
    if d.pane == Pane::List && d.coll.viewport().h > 0.0 {
        if let Some(y) = d.scroll_to.take() {
            tx.scroll_to_instant(viewport_path(SCOPE), 0.0, y);
        }
        if d.demo_start && d.feed.get().max[1] > 0.0 {
            d.demo_start = false;
            let v = d.coll.viewport();
            let start = Start {
                owner: SCOPE.to_owned(),
                anchor: Point::new(v.x + v.w / 2.0, v.y + v.h / 2.0),
                viewport: v,
                targets: vec![Target::vertical(viewport_path(SCOPE), d.feed.clone())],
            };
            changed |= d.auto.start(start, AutoscrollSettings::default());
        }
    }
    changed |= d.auto.drive(t, animations, tx);
    match g.c24.pane {
        Pane::List | Pane::Panel if !g.c24.hidden => changed |= collection(settings(g)).tick(g, tx),
        Pane::Nested => {
            for i in 0..COLUMNS {
                changed |= column(settings(g), i).tick(g, tx);
            }
        }
        _ => {}
    }
    changed
}
