//! The gallery page for C2 ColumnsView: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c2-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-3-views/C2-columns-view.md`.
//!
//! The page is a stand-in Columns pane over a fake tree (see [`folder`]): Home, holding
//! Documents, Pictures, Music, Projects, Empty, Slow (which takes 2 s to list), Big (100,000
//! entries) and a chain `deep/level-1/…/level-20`. Entry ids are fixed per folder, `number ×
//! 1,000,000 + position`, so scripts name rows by them: Home's entries are `row-1000001` on.
//! Its header shows the columns open (`c2-path`) and the last thing the view told the host
//! (`c2-intent`).
//!
//! The page stands in for Strata's policy, as C4's sorts for it:
//!
//! - opening a folder (a click, Return, or Right on a folder whose column isn't open) opens its
//!   column after the one it is in, as a sibling where a deeper column was open;
//! - a plain click on a file in a column before the last closes the deeper columns;
//! - Alt+Up returns to the parent of the active column's folder, selecting the folder it came
//!   from (Back to an ancestor);
//! - the close button, Escape with nothing selected and a click on a column's empty space close
//!   columns as the view asks.
//!
//! It mirrors nothing (Up and Down open no child column): mirroring is Strata's.
//!
//! Rows show size badges (C2b): a folder's item count (Slow's is unknown until it has been
//! listed, so it shows none till then) and
//! a file's size made up from its name. `Demo::show_sizes` stands in for Strata's "Show sizes
//! in Columns view". A rename (a slow second click) draws a stand-in editor, the row's name in
//! an outline, until the selection or cursor moves.

use std::borrow::Cow;

use rune::{Cx, El, Key, KeyEvent, Tx, col, row, text};
use strata_components::collection::element::{Intent, LABEL, Slot};
use strata_components::collection::keyboard::{KeySettings, Letters};
use strata_components::collection::pointer::{ClickCount, ClickModes, PointerSettings};
use strata_components::collection::selection::{CursorGone, Listing, SyncRule};
use strata_components::columns::{
    ColumnSources, ColumnsIntent, ColumnsParts, ColumnsPartsMut, ColumnsState, ColumnsView, Focus,
    HeaderCx, Opening,
};
use strata_components::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource, Point};
use strata_components::preview_drawer::{Arrangement, DrawerRules, DrawerState, PreviewDrawer};
use strata_components::scroll::{Autoscroll, BarMode, Shield};

use crate::{Gallery, Scene};

/// The view's key: the strip's, and the start of every column's.
pub const KEY: &str = "c2";

/// How long Slow takes to list.
const SLOW: f64 = 2.0;

/// One open column's listing.
#[derive(Debug)]
pub struct Listed {
    pub name: String,
    pub source: ListSource,
    pub version: u64,
    /// Still listing, until `due` (the page's clock).
    pub due: Option<f64>,
    /// The entry in the column before whose folder this is.
    pub from: Option<EntryId>,
}

/// The open columns' listings, root first, and whether Slow's item count is known yet (once
/// it has been listed: a count that fills in late).
#[derive(Debug, Default)]
pub struct Listings(pub Vec<Listed>, pub bool);

impl ColumnSources for Listings {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn source(&self, depth: usize) -> &dyn EntrySource {
        &self.0[depth].source
    }
    fn version(&self, depth: usize) -> u64 {
        self.0[depth].version
    }
    fn title(&self, depth: usize) -> &str {
        &self.0[depth].name
    }
    fn loading(&self, depth: usize) -> bool {
        self.0[depth].due.is_some()
    }
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        self.0.get(depth + 1).and_then(|c| c.from)
    }
    fn badge(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let source = &self.0.get(depth)?.source;
        if index >= source.len() {
            return None;
        }
        let entry = source.entry(index);
        if entry.kind.is_folder_like() {
            count_of(entry.name, self.1).map(|n| Cow::Owned(items(n)))
        } else {
            Some(Cow::Owned(size_text(size_of(entry.name))))
        }
    }
}

/// How many entries folder `name` lists, when the page knows: Slow's only once it has been
/// listed (`slow_known`).
fn count_of(name: &str, slow_known: bool) -> Option<usize> {
    match name {
        "Slow" if !slow_known => None,
        "Big" => Some(100_000),
        _ => Some(folder(name).1.len()),
    }
}

/// A folder's count as Strata words it.
fn items(n: usize) -> String {
    match n {
        0 => "No items".to_owned(),
        1 => "1 item".to_owned(),
        n => format!("{n} items"),
    }
}

/// A made-up size for file `name`, fixed by its name.
fn size_of(name: &str) -> u64 {
    let h = name.bytes().fold(1_469_598_103u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    });
    120 + h % 9_000_000
}

/// A size in decimal units, as Strata shows them.
fn size_text(bytes: u64) -> String {
    match bytes {
        b if b < 1_000 => format!("{b} bytes"),
        b if b < 1_000_000 => format!("{:.1} kB", b as f64 / 1e3),
        b => format!("{:.1} MB", b as f64 / 1e6),
    }
}

/// C2's demo state, held in `Gallery::c2`.
pub struct Demo {
    pub listings: Listings,
    pub columns: ColumnsState,
    pub clock: f64,
    /// The last thing the view told the host.
    pub intent: String,
    /// The C9 drawer beside the strip, with its Columns slot reserved, when set.
    pub drawer: Option<DrawerState>,
    /// The strip's width, when narrower than the page.
    pub width: Option<f32>,
    /// A sideways scroll of the strip to make once laid out.
    pub scroll_x: Option<f32>,
    /// A scroll of a column's listing to make once laid out.
    pub scroll_y: Option<(usize, f32)>,
    /// A held file drag at this point of column `.0`, once laid out (the drop wash).
    pub hold: Option<(usize, f32, f32)>,
    /// Folders open on one click (the default in Columns) or two.
    pub folder_clicks: ClickCount,
    /// What plain letters do in a column: passed on (Strata's default), or type-ahead.
    pub letters: Letters,
    /// Strata's "Show sizes in Columns view".
    pub show_sizes: bool,
    /// The row whose stand-in rename editor is open: its column and entry.
    pub renaming: Option<(usize, EntryId)>,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            listings: Listings::default(),
            columns: ColumnsState::new(KEY, Autoscroll::new()),
            clock: 0.0,
            intent: String::new(),
            drawer: None,
            width: None,
            scroll_x: None,
            scroll_y: None,
            hold: None,
            folder_clicks: ClickCount::One,
            letters: Letters::Pass,
            show_sizes: true,
            renaming: None,
        }
    }
}

/// The fake tree: folder `name`'s number and its entries, folders first. Unknown names list
/// three files under a number from their name.
pub fn folder(name: &str) -> (u64, Vec<(String, EntryKind)>) {
    let folders = |names: &[&str]| -> Vec<(String, EntryKind)> {
        names
            .iter()
            .map(|n| (n.to_string(), EntryKind::Folder))
            .collect()
    };
    let files = |names: Vec<String>| -> Vec<(String, EntryKind)> {
        names.into_iter().map(|n| (n, EntryKind::File)).collect()
    };
    let seq = |stem: &str, ext: &str, n: usize| -> Vec<String> {
        (1..=n).map(|i| format!("{stem}{i:04}{ext}")).collect()
    };
    let mut out;
    let number = match name {
        "Home" => {
            out = folders(&[
                "Documents",
                "Pictures",
                "Music",
                "Projects",
                "Empty",
                "Slow",
                "Big",
                "deep",
            ]);
            out.extend(files(
                [
                    "notes.txt",
                    "todo.md",
                    "photo.jpg",
                    "budget.ods",
                    "report-final.pdf",
                ]
                .map(String::from)
                .to_vec(),
            ));
            1
        }
        "Documents" => {
            out = folders(&["Reports", "Letters", "Taxes"]);
            out.extend(files(vec![
                "cv.pdf".into(),
                "budget-2026.ods".into(),
                "Quarterly report for the infrastructure committee, final revision with all appendices.pdf".into(),
                "letter.odt".into(),
            ]));
            out.extend(files(seq("scan-", ".pdf", 20)));
            2
        }
        "Pictures" => {
            out = folders(&["Holidays", "Screenshots"]);
            out.extend(files(seq("IMG_", ".jpg", 60)));
            3
        }
        "Music" => {
            out = files(seq("track-", ".flac", 40));
            4
        }
        "Projects" => {
            out = folders(&["strata", "rune", "website"]);
            out.extend(files(vec!["README.md".into(), "Cargo.toml".into()]));
            5
        }
        "Reports" => {
            out = files(seq("report-q", ".pdf", 8));
            6
        }
        "Letters" => {
            out = files(seq("letter-", ".odt", 6));
            7
        }
        "Taxes" => {
            out = files(seq("return-", ".pdf", 5));
            8
        }
        "Holidays" => {
            out = files(seq("beach-", ".jpg", 12));
            9
        }
        "Screenshots" => {
            out = files(seq("screen-", ".png", 9));
            10
        }
        "Empty" => {
            out = Vec::new();
            11
        }
        "Slow" => {
            out = files(seq("late-", ".txt", 10));
            12
        }
        "Big" => {
            out = (0..100_000)
                .map(|i| {
                    let kind = if i % 25 == 0 {
                        EntryKind::Folder
                    } else {
                        EntryKind::File
                    };
                    (format!("entry-{:06}", i + 1), kind)
                })
                .collect();
            13
        }
        "deep" => {
            out = folders(&["level-1"]);
            out.extend(files(seq("deep-", ".txt", 4)));
            20
        }
        _ if name.starts_with("level-") => {
            let n: u64 = name[6..].parse().unwrap_or(1);
            out = if n < 20 {
                vec![(format!("level-{}", n + 1), EntryKind::Folder)]
            } else {
                Vec::new()
            };
            out.extend(files(seq(&format!("{name}-"), ".txt", 4)));
            20 + n
        }
        _ => {
            out = files(vec![
                format!("{name}.txt"),
                format!("{name}-notes.md"),
                "index.html".into(),
            ]);
            100 + name
                .bytes()
                .fold(0u64, |h, b| (h * 31 + u64::from(b)) % 899)
        }
    };
    (number, out)
}

/// Folder `name`'s listing, with its fixed ids.
fn listing(name: &str) -> ListSource {
    let (number, entries) = folder(name);
    ListSource::new(
        entries
            .into_iter()
            .enumerate()
            .map(|(i, (n, kind))| {
                let icon = if kind.is_folder_like() {
                    "folder"
                } else {
                    "file"
                };
                ListEntry::new(
                    EntryId::new(number * 1_000_000 + i as u64 + 1),
                    n,
                    kind,
                    icon,
                )
            })
            .collect(),
    )
}

/// Folder `name`'s listing, for tests that open it themselves.
pub fn listing_of(name: &str) -> ListSource {
    listing(name)
}

/// The id of the entry named `name` in folder `parent`.
pub fn id_in(parent: &str, name: &str) -> EntryId {
    let (number, entries) = folder(parent);
    let i = entries
        .iter()
        .position(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no {name} in {parent}"));
    EntryId::new(number * 1_000_000 + i as u64 + 1)
}

impl Demo {
    fn name_of(&self, depth: usize, id: EntryId) -> String {
        let source = &self.listings.0[depth].source;
        source
            .index_of(id)
            .map(|i| source.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    /// The listing for folder `name`, entered from `from`, as the page lists it: Slow takes a
    /// while.
    fn listed(&self, name: &str, from: Option<EntryId>) -> Listed {
        let slow = name == "Slow";
        Listed {
            name: name.to_owned(),
            source: if slow {
                ListSource::new(Vec::new())
            } else {
                listing(name)
            },
            version: 1,
            due: slow.then_some(self.clock + SLOW),
            from,
        }
    }

    /// Opens folder `id` of column `depth` as the next column: a sibling where a deeper column
    /// was open.
    fn open(
        &mut self,
        look: &strata_kit::look::Look,
        depth: usize,
        id: EntryId,
        focus: Focus,
        tx: &mut Tx,
    ) {
        let name = self.name_of(depth, id);
        let listed = self.listed(&name, Some(id));
        let listing = if listed.due.is_some() {
            Listing::Loading
        } else {
            Listing::Complete
        };
        let child = depth + 1;
        self.listings.0.truncate(child);
        self.listings.0.push(listed);
        let source = &self.listings.0[child].source;
        if child < self.columns.len() {
            self.columns
                .swap_from(child, Opening::new(source, listing), focus, look, tx);
        } else {
            self.columns
                .push(Opening::new(source, listing), focus, look, tx);
        }
    }

    fn close_from(&mut self, look: &strata_kit::look::Look, depth: usize, tx: &mut Tx) {
        let depth = depth.max(1);
        self.listings.0.truncate(depth);
        self.columns.truncate(depth, look, tx);
    }

    fn path(&self) -> String {
        self.listings
            .0
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// What the page does with what the view tells it: Strata's policy, stood in for.
fn heard(g: &mut Gallery, intent: ColumnsIntent, tx: &mut Tx) {
    let look = g.look.clone();
    let d = &mut g.c2;
    d.intent = match intent {
        ColumnsIntent::Collection { depth, intent } => match intent {
            Intent::Activate { id, .. } => {
                let name = d.name_of(depth, id);
                let source = &d.listings.0[depth].source;
                let folder = source
                    .index_of(id)
                    .is_some_and(|i| source.entry(i).kind.is_folder_like());
                if folder {
                    d.open(&look, depth, id, Focus::Column, tx);
                    format!("Opened {name}")
                } else {
                    format!("Launched {name}")
                }
            }
            Intent::Clicked { id } => {
                let name = d.name_of(depth, id);
                let source = &d.listings.0[depth].source;
                let file = source
                    .index_of(id)
                    .is_some_and(|i| !source.entry(i).kind.is_folder_like());
                if file && depth + 1 < d.listings.0.len() {
                    d.close_from(&look, depth + 1, tx);
                }
                format!("Clicked {name}")
            }
            Intent::BeginDrag { ids, .. } => format!("Dragging {} from column {depth}", ids.len()),
            Intent::BeginRename { id } => {
                d.renaming = Some((depth, id));
                format!("Renaming {}", d.name_of(depth, id))
            }
            Intent::ContextMenu { target, .. } => match target {
                Some(id) => format!("Menu on {}", d.name_of(depth, id)),
                None => format!("Menu on column {depth}"),
            },
            Intent::OpenInNew { id, .. } => format!("New tab for {}", d.name_of(depth, id)),
            Intent::Edge(dir) => format!("Edge {dir:?} in column {depth}"),
            _ => format!("Other in column {depth}"),
        },
        ColumnsIntent::Cursor { .. } | ColumnsIntent::Selection { .. } => {
            // The stand-in editor closes when the selection or cursor moves.
            d.renaming = None;
            return;
        }
        ColumnsIntent::EnterFolder { depth, id } => {
            let name = d.name_of(depth, id);
            d.open(&look, depth, id, Focus::FirstEntry, tx);
            format!("Entered {name}")
        }
        ColumnsIntent::LeftOfRoot => "Left of the root".to_owned(),
        ColumnsIntent::CloseFrom(depth) => {
            d.close_from(&look, depth, tx);
            format!("Closed from column {depth}")
        }
        ColumnsIntent::HeaderMenu { depth, .. } => {
            format!("Folder menu for {}", d.listings.0[depth].name)
        }
        ColumnsIntent::Width(w) => {
            d.columns.set_default_width(w);
            format!("Width {w:.0}")
        }
        ColumnsIntent::Dropped {
            depth,
            target,
            dropped,
        } => {
            let into = match target {
                Some(id) => d.name_of(depth, id),
                None => d.listings.0[depth].name.clone(),
            };
            format!("Dropped {} into {into}", dropped.data.file_paths().len())
        }
        _ => "Other".to_owned(),
    };
}

/// The page's view: the listings and the strip's state from `Gallery::c2`.
pub fn columns_view() -> ColumnsView<Gallery> {
    ColumnsView::new(
        |g: &Gallery| ColumnsParts {
            sources: &g.c2.listings,
            state: &g.c2.columns,
            look: &g.look,
        },
        |g: &mut Gallery| ColumnsPartsMut {
            sources: &g.c2.listings,
            state: &mut g.c2.columns,
            look: &g.look,
        },
    )
    .collection(|g: &Gallery, depth, c| {
        c.settings(KeySettings::default().letters(g.c2.letters))
            .rename(
                move |g: &Gallery| g.c2.renaming.filter(|(d, _)| *d == depth).map(|(_, id)| id),
                // A stand-in for C5's editor: the name, outlined.
                Slot::pure(|_: &Gallery, rcx| {
                    let kit = &rcx.look.kit;
                    text(rcx.entry.name.to_owned())
                        .key(LABEL)
                        .a11y_hidden()
                        .font(kit.ty.ui_compact)
                        .color(kit.c.text_base)
                        .border(1.0, kit.c.border_focus)
                        .truncate()
                        .shrink()
                        .min_w(0.0)
                }),
            )
    })
    .show_sizes(|g: &Gallery| g.c2.show_sizes)
    .pointer_settings(|g: &Gallery| {
        PointerSettings::default().modes(ClickModes::new(ClickCount::Two, g.c2.folder_clicks))
    })
    .header_actions(|g: &Gallery, cx: HeaderCx| {
        // A stand-in for Refresh and Sort by: one button, to show where the actions go.
        let kit = g.kit();
        Some(
            rune_kit::IconButton::new(g.look.icon(strata_kit::icons::IconName::Refresh))
                .variant(rune_kit::IconButtonVariant::GhostMuted)
                .control_size(rune_kit::ControlSize::Small)
                .label("Refresh")
                .on_press_tx(move |g: &mut Gallery, _tx: &mut Tx| {
                    g.c2.intent = format!("Refreshed column {}", cx.depth);
                })
                .build(kit)
                .focusable(false)
                .key(format!("c2-refresh-{}", cx.depth)),
        )
    })
    .on_intent(heard)
}

// Scenes.

fn start(g: &mut Gallery, dark: bool) {
    g.open("c2", dark);
    g.c2 = Demo::default();
    open_home(g, &mut Tx::default());
}

/// Opens Home as the only column, focused: what every scene starts from, and what the page
/// shows when the index opens it.
fn open_home(g: &mut Gallery, tx: &mut Tx) {
    let look = g.look.clone();
    let d = &mut g.c2;
    d.listings.0.push(d.listed("Home", None));
    d.columns.push(
        Opening::new(&d.listings.0[0].source, Listing::Complete),
        Focus::Column,
        &look,
        tx,
    );
}

/// Opens `path` under Home, each folder as the next column, the last one active.
fn open_path(g: &mut Gallery, path: &[&str]) {
    let look = g.look.clone();
    let d = &mut g.c2;
    let mut tx = Tx::default();
    let mut parent = "Home".to_owned();
    for (depth, name) in path.iter().enumerate() {
        let id = id_in(&parent, name);
        let listing = d.listed(name, Some(id));
        d.listings.0.push(listing);
        let state = if d.listings.0[depth + 1].due.is_some() {
            Listing::Loading
        } else {
            Listing::Complete
        };
        d.columns.push(
            Opening::new(&d.listings.0[depth + 1].source, state),
            Focus::Column,
            &look,
            &mut tx,
        );
        // The parent selects the folder it opened, as a click there does.
        let source = &d.listings.0[depth].source;
        let i = source.index_of(id).expect("listed");
        let c = &mut d.columns.column_mut(depth).expect("open").collection;
        let _ = c.selection.select_only(source, i);
        c.note_selection(source);
        parent = (*name).to_owned();
    }
}

/// Puts column `depth`'s cursor on entry `index`, selected, in keyboard mode.
fn cursor(g: &mut Gallery, depth: usize, index: usize) {
    let d = &mut g.c2;
    let source = &d.listings.0[depth].source;
    let c = &mut d.columns.column_mut(depth).expect("open").collection;
    let _ = c.selection.select_only(source, index);
    c.keyboard_moved();
    c.note_selection(source);
}

fn three_columns(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Documents", "Reports"]);
    let look = g.look.clone();
    g.c2.columns.set_active(1, &look, &mut Tx::default());
}

fn keyboard(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Pictures"]);
    // A multi-selection kept in Pictures, while the keyboard works in Home.
    {
        let d = &mut g.c2;
        let source = &d.listings.0[1].source;
        let c = &mut d.columns.column_mut(1).expect("open").collection;
        let _ = c.selection.select_only(source, 4);
        let _ = c.selection.extend_to(
            source,
            7,
            strata_components::collection::selection::Extend::Replace,
        );
        c.note_selection(source);
    }
    let look = g.look.clone();
    g.c2.columns.set_active(0, &look, &mut Tx::default());
    cursor(g, 0, 1);
}

fn overflow(g: &mut Gallery, dark: bool) {
    start(g, dark);
    let mut path = vec!["deep"];
    let names: Vec<String> = (1..=5).map(|n| format!("level-{n}")).collect();
    path.extend(names.iter().map(String::as_str));
    open_path(g, &path);
    // Shown at rest: the strip's bar (no square at its leading end).
    g.c2.columns.set_bar_mode(BarMode::Always);
}

fn loading_empty(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Empty"]);
    // Slow listed from Home, beside the empty column: still loading at 1 s.
    let d = &mut g.c2;
    d.listings.0[1].due = None;
    let look = g.look.clone();
    let d = &mut g.c2;
    let id = id_in("Home", "Slow");
    let mut listed = d.listed("Slow", Some(id));
    listed.due = Some(1_000.0);
    d.listings.0.push(listed);
    d.columns.push(
        Opening::new(&d.listings.0[2].source, Listing::Loading),
        Focus::No,
        &look,
        &mut Tx::default(),
    );
}

fn drop_column(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Documents", "Reports"]);
    g.c2.hold = Some((1, 0.5, 0.5));
}

fn preview_slot(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Projects", "strata"]);
    let look = g.look.clone();
    g.c2.columns.set_active(1, &look, &mut Tx::default());
    let mut drawer = DrawerState::new();
    drawer.reserve(true);
    g.c2.drawer = Some(drawer);
}

fn hundred_k(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Big"]);
    let h = g.look.row.columns;
    g.c2.scroll_y = Some((1, 50_000.0 * h));
}

fn wide_column(g: &mut Gallery, dark: bool) {
    start(g, dark);
    g.c2.columns.set_default_width(1_100.0);
    open_path(g, &["Music"]);
}

/// Sizes in the badges: Documents on the open path in Home, two selected rows in Documents,
/// the keyboard cursor on one of them.
fn badges(g: &mut Gallery, dark: bool) {
    start(g, dark);
    open_path(g, &["Documents", "Reports"]);
    let look = g.look.clone();
    g.c2.columns.set_active(1, &look, &mut Tx::default());
    let d = &mut g.c2;
    let source = &d.listings.0[1].source;
    let c = &mut d.columns.column_mut(1).expect("open").collection;
    let taxes = source
        .index_of(id_in("Documents", "Taxes"))
        .expect("listed");
    let cv = source
        .index_of(id_in("Documents", "cv.pdf"))
        .expect("listed");
    let _ = c.selection.select_only(source, taxes);
    let _ = c.selection.toggle(source, cv);
    c.keyboard_moved();
    c.note_selection(source);
}

fn sizes_off(g: &mut Gallery, dark: bool) {
    badges(g, dark);
    g.c2.show_sizes = false;
}

pub const SCENES: &[Scene] = &[
    ("c2-three-columns-light", |g| three_columns(g, false)),
    ("c2-three-columns-dark", |g| three_columns(g, true)),
    ("c2-keyboard-light", |g| keyboard(g, false)),
    ("c2-keyboard-dark", |g| keyboard(g, true)),
    ("c2-overflow-light", |g| overflow(g, false)),
    ("c2-overflow-dark", |g| overflow(g, true)),
    ("c2-loading-empty-light", |g| loading_empty(g, false)),
    ("c2-loading-empty-dark", |g| loading_empty(g, true)),
    ("c2-drop-column-light", |g| drop_column(g, false)),
    ("c2-drop-column-dark", |g| drop_column(g, true)),
    ("c2-preview-slot-light", |g| preview_slot(g, false)),
    ("c2-preview-slot-dark", |g| preview_slot(g, true)),
    ("c2-hundred-k-light", |g| hundred_k(g, false)),
    ("c2-hundred-k-dark", |g| hundred_k(g, true)),
    ("c2-wide-column-light", |g| wide_column(g, false)),
    ("c2-wide-column-dark", |g| wide_column(g, true)),
    ("c2-badges-light", |g| badges(g, false)),
    ("c2-badges-dark", |g| badges(g, true)),
    ("c2-sizes-off-light", |g| sizes_off(g, false)),
    ("c2-sizes-off-dark", |g| sizes_off(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c2-home-light", |g| start(g, false)),
    ("c2-home-dark", |g| start(g, true)),
];

fn label(key: &str, t: String, g: &Gallery) -> El {
    let kit = g.kit();
    text(t).key(key).font(kit.ty.ui).color(kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c2;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let v = columns_view();
    let strip = v.build(g);
    let strip = match d.width {
        Some(w) => col().w(w).h_full().min_h(0.0).child(strip),
        None => col().w_full().h_full().min_h(0.0).child(strip),
    };
    let body = match &d.drawer {
        Some(drawer) => PreviewDrawer::new(drawer, |g: &mut Gallery| {
            g.c2.drawer.get_or_insert_with(DrawerState::new)
        })
        .arrangement(Arrangement::Columns(v.fit(g, &DrawerRules::STRATA)))
        .placeholder("No preview")
        .build(strip, look, cx),
        None => strip,
    };
    let header = row()
        .w_full()
        .px(look.space.lg)
        .py(look.space.xs)
        .gap(look.space.lg)
        .center_y()
        .child(
            text(d.path())
                .key("c2-path")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(label("c2-intent", d.intent.clone(), g));
    let mut page = col()
        .key("c2-page")
        .w_full()
        .h_full()
        .child(header)
        .child(col().w_full().grow(1.0).min_h(0.0).child(body));
    if let Some(shield) = Shield::new(
        |g: &Gallery| g.c2.columns.autoscroll(),
        |g: &Gallery| g.look(),
    )
    .build(g)
    {
        page = page.child(shield);
    }
    page
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index, as
/// it does once the strip has nothing left to clear or close.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let look = g.look.clone();
    let d = &mut g.c2;
    if key.key == Key::Up && key.mods.alt {
        // Back to the parent of the active column's folder, selecting the folder it came from.
        let active = d.columns.active();
        if active == 0 {
            return true;
        }
        let parent = active - 1;
        let came_from = d.listings.open_child(parent);
        d.listings.0.truncate(parent + 1);
        d.columns
            .return_to(parent, &d.listings, came_from, &look, tx);
        d.intent = format!("Back to {}", d.listings.0[parent].name);
        return true;
    }
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Deliver fake async results here; a worker wakes the loop with `Gallery::waker`. True when
/// state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    // Opened from the index, the page has no scene to set it up.
    if g.c2.listings.0.is_empty() {
        open_home(g, tx);
        changed = true;
    }
    let look = g.look.clone();
    {
        let d = &mut g.c2;
        d.clock = t;
        // Slow arrives.
        for depth in 0..d.listings.0.len() {
            if d.listings.0[depth].due.is_some_and(|due| t >= due) {
                let c = &mut d.listings.0[depth];
                c.due = None;
                c.source = listing(&c.name);
                c.version += 1;
                if c.name == "Slow" {
                    d.listings.1 = true;
                }
                d.columns.sync(
                    depth,
                    &d.listings.0[depth].source,
                    SyncRule::complete(CursorGone::Neighbor),
                    tx,
                );
                changed = true;
            }
        }
        let strip = d.columns.strip_rect();
        if strip.w > 0.0 {
            if let Some(x) = d.scroll_x.take() {
                tx.scroll_to_x(strata_components::columns::keys::strip(KEY), x);
            }
            if let Some((depth, y)) = d.scroll_y.take()
                && let Some(key) = d.columns.column_key(depth)
            {
                tx.scroll_to_instant(
                    strata_components::collection::element::viewport_path(&key),
                    0.0,
                    y,
                );
            }
            if let Some((depth, fx, fy)) = d.hold.take()
                && let Some(c) = d.columns.column(depth)
            {
                let v = c.collection.viewport();
                if v.w > 0.0 {
                    d.columns
                        .drag_over(Some(Point::new(v.x + v.w * fx, v.y + v.h * fy)));
                    changed = true;
                } else {
                    d.hold = Some((depth, fx, fy));
                }
            }
        }
        // The edge of the column that meets the preview lights the drawer's divider line.
        let lit = d.columns.last_edge_lit();
        if let Some(drawer) = d.drawer.as_mut()
            && drawer.line_lit() != lit
        {
            drawer.light_line(lit);
            changed = true;
        }
        let animations = look.animations;
        changed |= d.columns.autoscroll().drive(t, animations, tx);
    }
    changed | columns_view().tick(g, t, tx)
}
