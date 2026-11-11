//! The gallery page for C4 ListTable: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c4-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-3-views/C4-list-table.md`.
//!
//! The page is a stand-in List pane over four fake folders: Documents (12 entries, one with a
//! long name), Mixed (600 entries in six file types), Archive (100,000) and Empty. Its header
//! holds the Columns button and labels showing what the host heard: the sort (`c4-sort`), the
//! column layout and how many changes it heard (`c4-layout`), the widths (`c4-widths`) and the
//! last intent (`c4-intent`). The page stands in for Strata's policy: it sorts on a sort
//! intent (folders first), groups by its fake file types (Folder first, then A–Z, Other last),
//! and keeps the layout and widths it hears. C24's autoscroll runs on the rows and the
//! sideways scroll (the shield mounts in the root layer), and `c4-bars` shows both of the
//! table's scroll bars in a pane narrower than its columns.
//!
//! | Key | Does |
//! | --- | --- |
//! | Ctrl+G | Group by file type, or stop |
//! | Ctrl+E | Sort by Extension, which no column shows |
//! | Ctrl+K | "Customize columns…": the columns menu on the headings |
//! | Ctrl+1 | Switch the view away and back (the table leaves the view, then returns) |
//! | F5 | Refresh |
//!
//! `STRATA_C4_BUDGET_WARMUP=1` makes the page's second poll wait 1.1 s of real time once, then
//! print [`WARMED`] on stderr: c1's workaround for Rune's commit-budget line (Rune draft
//! U-C1e-5), for `tests/c4.rs`'s budget run only.

use std::cmp::Ordering;
use std::sync::Arc;

use rune::{Align, Cx, El, Key, KeyEvent, Tx, col, row, text};
use strata_components::collection::element::{Collection, CollectionState, Controllers, Intent};
use strata_components::collection::selection::{CursorGone, Extend, SyncRule};
use strata_components::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use strata_components::list_table::{
    Cell, Column, ColumnId, ColumnLayout, ColumnWidths, DropSite, LayoutChange, ListParts,
    ListTable, ListTableState, MenuAt, SortDir, SortIntent, SortShown, groups_from_runs,
};
use strata_components::scroll::{
    Autoscroll, AutoscrollPointer, AutoscrollSettings, BarMode, Shield,
};

use crate::{Gallery, Scene};

/// The variable that turns on the budget warm-up (see the module docs).
pub const WARMUP: &str = "STRATA_C4_BUDGET_WARMUP";

/// What the page prints on stderr once the warm-up is over.
pub const WARMED: &str = "c4: budget warm-up done";

/// The collection's and the table's key.
pub const SCOPE: &str = "c4-list";

/// The fake folders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Folder {
    #[default]
    Documents,
    Mixed,
    Archive,
    Empty,
}

impl Folder {
    fn title(self) -> &'static str {
        match self {
            Folder::Documents => "Documents",
            Folder::Mixed => "Mixed",
            Folder::Archive => "Archive",
            Folder::Empty => "Empty",
        }
    }

    fn listing(self) -> Vec<ListEntry> {
        let entry = |id: u64, name: String, kind: EntryKind, icon: &str| {
            ListEntry::new(EntryId::new(id), name, kind, icon)
        };
        match self {
            Folder::Documents => {
                let folders = ["Projects", "Receipts", "Music"];
                let files = [
                    ("Notes.md", "file_text"),
                    ("budget.ods", "file"),
                    ("main.rs", "file_code"),
                    ("holiday.jpg", "file_image"),
                    ("backup.tar.gz", "archive"),
                    ("letter.odt", "file_text"),
                    ("scan.pdf", "file"),
                    ("README", "file_text"),
                    (
                        "Quarterly report for the infrastructure committee, final revision with all appendices.pdf",
                        "file",
                    ),
                ];
                folders
                    .iter()
                    .map(|n| (n.to_string(), EntryKind::Folder, "folder"))
                    .chain(
                        files
                            .iter()
                            .map(|(n, i)| (n.to_string(), EntryKind::File, *i)),
                    )
                    .enumerate()
                    .map(|(i, (n, k, icon))| entry(1 + i as u64, n, k, icon))
                    .collect()
            }
            Folder::Mixed => {
                let groups: [(&str, &str, &str, usize); 6] = [
                    ("folder", "Folder ", "", 8),
                    ("file_image", "IMG_", ".jpg", 166),
                    ("file_text", "report-", ".odt", 120),
                    ("file", "budget-", ".ods", 60),
                    ("archive", "backup-", ".tar.gz", 30),
                    ("file", "data-", ".bin", 216),
                ];
                let mut out = Vec::new();
                for (g, (icon, stem, ext, count)) in groups.into_iter().enumerate() {
                    for k in 0..count {
                        let id = 500_000 + out.len() as u64;
                        let kind = if g == 0 {
                            EntryKind::Folder
                        } else {
                            EntryKind::File
                        };
                        out.push(entry(id, format!("{stem}{:04}{ext}", k + 1), kind, icon));
                    }
                }
                out
            }
            Folder::Archive => (0..100_000u64)
                .map(|i| {
                    let folder = i % 25 == 0;
                    entry(
                        1_000_000 + i,
                        format!("entry-{:06}.dat", i + 1),
                        if folder {
                            EntryKind::Folder
                        } else {
                            EntryKind::File
                        },
                        if folder { "folder" } else { "file" },
                    )
                })
                .collect(),
            Folder::Empty => Vec::new(),
        }
    }
}

/// What the page sorts by: the four fields with columns, and one without.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Size,
    Type,
    Modified,
    Extension,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Size => "Size",
            Field::Type => "Type",
            Field::Modified => "Modified",
            Field::Extension => "Extension",
        }
    }

    fn column(self) -> Option<&'static str> {
        match self {
            Field::Name => Some("name"),
            Field::Size => Some("size"),
            Field::Type => Some("type"),
            Field::Modified => Some("modified"),
            Field::Extension => None,
        }
    }

    fn of(column: &ColumnId) -> Option<Field> {
        Some(match column.as_str() {
            "name" => Field::Name,
            "size" => Field::Size,
            "type" => Field::Type,
            "modified" => Field::Modified,
            _ => return None,
        })
    }
}

/// The fake metadata: derived from the entry, so 100,000 entries need no table of it.
fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(i) => &name[i + 1..],
    }
}

fn type_of(kind: EntryKind, name: &str) -> &'static str {
    if kind.is_folder_like() {
        return "Folder";
    }
    match extension(name) {
        "jpg" => "JPEG image",
        "md" => "Markdown document",
        "odt" => "Text document",
        "ods" => "Spreadsheet",
        "gz" => "Archive (gzip)",
        "rs" => "Rust source code",
        "pdf" => "PDF document",
        _ => "Other",
    }
}

fn size_of(id: EntryId) -> u64 {
    id.get().wrapping_mul(7919) % 5_000_000
}

fn size_text(kind: EntryKind, id: EntryId) -> String {
    if kind.is_folder_like() {
        return match id.get() % 7 {
            0 => "No items".to_owned(),
            1 => "1 item".to_owned(),
            n => format!("{n} items"),
        };
    }
    let b = size_of(id);
    if b >= 1_000_000 {
        format!("{:.1} MB", b as f64 / 1e6)
    } else if b >= 1_000 {
        format!("{:.1} kB", b as f64 / 1e3)
    } else {
        format!("{b} bytes")
    }
}

fn modified_of(id: EntryId) -> String {
    let n = id.get().wrapping_mul(2_654_435_761) % 1_000_000;
    format!(
        "2026-{:02}-{:02} {:02}:{:02}",
        1 + n % 12,
        1 + n / 12 % 28,
        n / 400 % 24,
        n / 9 % 60
    )
}

fn mode_of(kind: EntryKind) -> &'static str {
    if kind.is_folder_like() {
        "drwxr-xr-x  755"
    } else {
        "-rw-r--r--  644"
    }
}

/// The page's columns: Strata's five, with its default widths and minimums
/// (`src/ui/browser_modes/list_columns.rs`).
pub fn columns() -> Vec<Column<Gallery>> {
    vec![
        Column::primary("name", "Name")
            .width(160.0)
            .min(160.0)
            .sortable(),
        Column::new("mode", "Mode", |_: &Gallery, rcx| {
            Cell::text(mode_of(rcx.entry.kind)).mono().muted()
        })
        .width(160.0)
        .min(80.0),
        Column::new("size", "Size", |_: &Gallery, rcx| {
            Cell::text(size_text(rcx.entry.kind, rcx.entry.id)).muted()
        })
        .width(90.0)
        .min(70.0)
        .align(Align::End)
        .sortable(),
        Column::new("type", "Type", |_: &Gallery, rcx| {
            Cell::text(type_of(rcx.entry.kind, rcx.entry.name)).muted()
        })
        .width(120.0)
        .min(80.0)
        .sortable(),
        Column::new("modified", "Modified", |_: &Gallery, rcx| {
            Cell::text(modified_of(rcx.entry.id)).muted()
        })
        .width(150.0)
        .min(110.0)
        .sortable(),
    ]
}

/// C4's demo state, held in `Gallery::c4`.
pub struct Demo {
    pub folder: Folder,
    pub listing: ListSource,
    pub coll: CollectionState,
    pub table: ListTableState,
    pub layout: ColumnLayout,
    pub widths: ColumnWidths,
    pub field: Field,
    pub dir: SortDir,
    pub grouped: bool,
    pub version: u64,
    pub clock: f64,
    /// The pane's width, when narrower than the page.
    pub pane: Option<f32>,
    /// A sideways scroll to make once laid out.
    pub scroll_x: Option<f32>,
    /// A vertical scroll to make once laid out.
    pub scroll_y: Option<f32>,
    /// The table is in the view (Ctrl+1 switches it away).
    pub shown: bool,
    pub layout_changes: u32,
    pub last_layout: String,
    pub intent: String,
    pub polls: u32,
    /// The window's autoscroll (C24), on the table's rows and its sideways scroll.
    pub auto: Autoscroll,
}

/// The page as the index opens it: Documents, as the default scene shows.
impl Default for Demo {
    fn default() -> Self {
        Demo::at(Folder::Documents)
    }
}

impl Demo {
    /// Nothing listed yet: what [`Demo::at`] fills in.
    fn blank() -> Self {
        let mut coll = CollectionState::new();
        let table = ListTableState::new();
        let auto = Autoscroll::new();
        install(&mut coll.controllers);
        Self {
            folder: Folder::Documents,
            listing: ListSource::new(Vec::new()),
            coll,
            table,
            auto,
            layout: ColumnLayout::new(columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            field: Field::Name,
            dir: SortDir::Ascending,
            grouped: false,
            version: 1,
            clock: 0.0,
            pane: None,
            scroll_x: None,
            scroll_y: None,
            shown: true,
            layout_changes: 0,
            last_layout: String::new(),
            intent: String::new(),
            polls: 0,
        }
    }
}

/// C1c's keys and C1d's marquee. The pointer comes with the page's first poll
/// ([`install_pointer`]).
fn install(controllers: &mut Controllers) {
    controllers.keyboard =
        Some(Box::<strata_components::collection::keyboard::Keyboard>::default());
    controllers.marquee = Some(Box::new(
        strata_components::collection::marquee::Marquee::new(),
    ));
}

/// C1b's pointer intent behind C24's autoscroll, which moves the rows and the table's sideways
/// scroll (`ListTable::sideways_target`). The target reads the table's state through the
/// page's accessor, so it is made once the gallery exists: on the page's first poll, which
/// Rune runs before the frame's input.
fn install_pointer(g: &mut Gallery) {
    let sideways = table().sideways_target(g);
    let d = &mut g.c4;
    d.coll.controllers.pointer = Some(Box::new(
        AutoscrollPointer::new(
            Some(Box::new(
                strata_components::collection::pointer::PointerIntent::new(),
            )),
            d.auto.clone(),
            SCOPE,
            d.table.feed().clone(),
        )
        .sideways(sideways),
    ));
}

/// Where a group sits in Strata's grouped order: Folder first, then A–Z, Other last.
fn group_rank(t: &str) -> (u8, &str) {
    match t {
        "Folder" => (0, ""),
        "Other" => (2, ""),
        t => (1, t),
    }
}

impl Demo {
    fn at(folder: Folder) -> Self {
        let mut d = Demo {
            folder,
            ..Demo::blank()
        };
        d.listing = ListSource::new(folder.listing());
        d.resort();
        d
    }

    fn name_of(&self, id: EntryId) -> String {
        self.listing
            .index_of(id)
            .map(|i| self.listing.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    /// Sorts the listing by the page's field and direction, folders first (and by group when
    /// grouped), names breaking ties ascending; then groups it.
    fn resort(&mut self) {
        let (field, dir, grouped) = (self.field, self.dir, self.grouped);
        let by_name =
            |a: &ListEntry, b: &ListEntry| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        self.listing.sort_by(|a, b| {
            let folders = b.kind.is_folder_like().cmp(&a.kind.is_folder_like());
            let group = if grouped {
                group_rank(type_of(a.kind, &a.name)).cmp(&group_rank(type_of(b.kind, &b.name)))
            } else {
                Ordering::Equal
            };
            let key = match field {
                Field::Name => by_name(a, b),
                Field::Size => size_of(a.id).cmp(&size_of(b.id)),
                Field::Type => type_of(a.kind, &a.name).cmp(type_of(b.kind, &b.name)),
                Field::Modified => modified_of(a.id).cmp(&modified_of(b.id)),
                Field::Extension => extension(&a.name).cmp(extension(&b.name)),
            };
            let key = match dir {
                SortDir::Ascending => key,
                SortDir::Descending => key.reverse(),
            };
            folders.then(group).then(key).then_with(|| by_name(a, b))
        });
        self.regroup();
    }

    fn regroup(&mut self) {
        let groups = if self.grouped {
            let entries = self.listing.entries();
            groups_from_runs(entries.len(), |i| {
                let t = type_of(entries[i].kind, &entries[i].name);
                let key = t
                    .bytes()
                    .fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
                (key, Arc::from(t))
            })
        } else {
            Default::default()
        };
        self.coll.set_groups(groups);
    }

    fn sorted(&mut self, tx: &mut Tx) {
        self.resort();
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
    }

    fn sort_shown(&self) -> Option<SortShown> {
        self.field.column().map(|c| SortShown {
            column: ColumnId::new(c),
            dir: self.dir,
        })
    }

    fn heard_sort(&mut self, intent: SortIntent, tx: &mut Tx) {
        if let Some(field) = Field::of(&intent.column) {
            self.field = field;
            self.dir = intent.dir;
            self.sorted(tx);
        }
    }

    fn heard_layout(&mut self, change: LayoutChange) {
        self.layout = change.layout;
        self.layout_changes += 1;
        self.last_layout = format!("{:?}", change.why);
    }

    fn heard(&mut self, intent: Intent) {
        self.intent = match intent {
            Intent::Clicked { id } => format!("Clicked {}", self.name_of(id)),
            Intent::Activate { id, .. } => format!("Opened {}", self.name_of(id)),
            Intent::BeginDrag { ids, .. } => format!("Dragging {} items", ids.len()),
            Intent::BeginRename { id } => format!("Renaming {}", self.name_of(id)),
            Intent::ContextMenu { target, .. } => match target {
                Some(id) => format!("Menu on {}", self.name_of(id)),
                None => "Menu on the background".to_owned(),
            },
            Intent::OpenInNew { id, window } => format!(
                "Opened {} in a new {}",
                self.name_of(id),
                if window { "window" } else { "tab" }
            ),
            Intent::Edge(d) => format!("Edge {d:?}"),
            _ => "Other".to_owned(),
        };
    }

    fn sort_label(&self) -> String {
        let arrow = match self.dir {
            SortDir::Ascending => "↑",
            SortDir::Descending => "↓",
        };
        format!("Sorted by {} {arrow}", self.field.label())
    }

    fn layout_label(&self) -> String {
        let order: Vec<&str> = self.layout.order().iter().map(ColumnId::as_str).collect();
        let hidden: Vec<&str> = self.layout.hidden().map(ColumnId::as_str).collect();
        let last = if self.last_layout.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.last_layout)
        };
        format!(
            "Order: {}; hidden: {}; changes: {}{last}",
            order.join(", "),
            if hidden.is_empty() {
                "none".to_owned()
            } else {
                hidden.join(", ")
            },
            self.layout_changes
        )
    }

    fn widths_label(&self) -> String {
        let set: Vec<String> = self
            .widths
            .iter()
            .map(|(id, w)| format!("{id} {w}"))
            .collect();
        if set.is_empty() {
            "Widths: default".to_owned()
        } else {
            format!("Widths: {}", set.join(", "))
        }
    }
}

fn scene(g: &mut Gallery, dark: bool, folder: Folder) -> &mut Demo {
    g.open("c4", dark);
    g.c4 = Demo::at(folder);
    &mut g.c4
}

fn default_scene(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
}

fn sorted_selection(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Documents);
    d.field = Field::Size;
    d.dir = SortDir::Descending;
    d.resort();
    d.coll.selection.select_only(&d.listing, 3);
    d.coll.selection.extend_to(&d.listing, 5, Extend::Replace);
    d.coll.keyboard_moved();
    d.coll.note_selection(&d.listing);
}

fn hidden(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Documents);
    d.layout = ColumnLayout::normalize(
        d.layout.order(),
        Some(&ColumnId::new("name")),
        ["size", "modified", "mode", "type", "name"],
        ["mode", "type"],
    );
}

fn columns_menu(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Documents);
    d.table.open_columns_menu(MenuAt::Button);
}

fn reorder(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Documents);
    d.table.show_reorder(
        ColumnId::new("mode"),
        Some(DropSite {
            target: ColumnId::new("type"),
            after: false,
        }),
    );
}

fn grouped(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Mixed);
    d.grouped = true;
    d.resort();
}

fn grouped_stuck(g: &mut Gallery, dark: bool) {
    let h = strata_kit::slots::list::row_height(g.look());
    let d = scene(g, dark, Folder::Mixed);
    d.grouped = true;
    d.resort();
    // Folder, then Archive (gzip), then JPEG image from line 40: 6 pt into its header line,
    // the rows below it, and its header stuck at the top.
    d.scroll_y = Some(40.0 * h + 6.0);
}

fn narrow(g: &mut Gallery, dark: bool) {
    let d = scene(g, dark, Folder::Documents);
    d.pane = Some(480.0);
    d.scroll_x = Some(160.0);
}

fn hundred_k(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Archive);
}

/// A narrow pane over 600 rows, scrolled both ways, with both bars shown (`BarMode::Always`):
/// the rows' bar at the pane's visible right edge, not the columns'.
fn bars(g: &mut Gallery, dark: bool) {
    let h = strata_kit::slots::list::row_height(g.look());
    let d = scene(g, dark, Folder::Mixed);
    d.pane = Some(480.0);
    d.scroll_x = Some(160.0);
    d.scroll_y = Some(100.0 * h);
    d.table.set_bar_mode(BarMode::Always);
}

fn empty(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Empty);
}

pub const SCENES: &[Scene] = &[
    ("c4-default-light", |g| default_scene(g, false)),
    ("c4-default-dark", |g| default_scene(g, true)),
    ("c4-sorted-selection-light", |g| sorted_selection(g, false)),
    ("c4-sorted-selection-dark", |g| sorted_selection(g, true)),
    ("c4-hidden-light", |g| hidden(g, false)),
    ("c4-hidden-dark", |g| hidden(g, true)),
    ("c4-columns-menu-light", |g| columns_menu(g, false)),
    ("c4-columns-menu-dark", |g| columns_menu(g, true)),
    ("c4-reorder-light", |g| reorder(g, false)),
    ("c4-reorder-dark", |g| reorder(g, true)),
    ("c4-grouped-light", |g| grouped(g, false)),
    ("c4-grouped-dark", |g| grouped(g, true)),
    ("c4-grouped-stuck-light", |g| grouped_stuck(g, false)),
    ("c4-grouped-stuck-dark", |g| grouped_stuck(g, true)),
    ("c4-narrow-light", |g| narrow(g, false)),
    ("c4-narrow-dark", |g| narrow(g, true)),
    ("c4-hundred-k-light", |g| hundred_k(g, false)),
    ("c4-hundred-k-dark", |g| hundred_k(g, true)),
    ("c4-empty-light", |g| empty(g, false)),
    ("c4-empty-dark", |g| empty(g, true)),
    ("c4-bars-light", |g| bars(g, false)),
    ("c4-bars-dark", |g| bars(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[];

/// The page's table.
pub fn table() -> ListTable<Gallery> {
    ListTable::new(
        SCOPE,
        columns(),
        |g: &Gallery| ListParts {
            source: &g.c4.listing,
            collection: &g.c4.coll,
            table: &g.c4.table,
            layout: &g.c4.layout,
            widths: &g.c4.widths,
            sort: g.c4.sort_shown(),
            look: g.look(),
        },
        |g: &mut Gallery| &mut g.c4.table,
    )
    .on_sort(|g: &mut Gallery, intent, tx| g.c4.heard_sort(intent, tx))
    .on_layout(|g: &mut Gallery, change, _| g.c4.heard_layout(change))
    .on_widths(|g: &mut Gallery, widths, _| g.c4.widths = widths)
}

/// The page's collection, before the table adds its parts.
pub fn collection() -> Collection<Gallery> {
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c4.listing, &g.c4.coll),
        |g: &mut Gallery| (&g.c4.listing, &mut g.c4.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c4.version,
    )
    .on_intent(|g: &mut Gallery, intent, _| g.c4.heard(intent))
    .settings(marquee_settings())
    .empty(|g: &Gallery| {
        let kit = g.kit();
        Some(
            col()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(
                    text("Folder is empty")
                        .key("c4-empty-note")
                        .font(kit.ty.ui)
                        .color(kit.c.text_muted),
                ),
        )
    })
}

fn marquee_settings() -> strata_components::collection::marquee::MarqueeSettings {
    strata_components::collection::marquee::MarqueeSettings::default()
}

/// The page's collection with autoscroll's settings, whose row margin is the look's.
pub fn base(g: &Gallery) -> Collection<Gallery> {
    let mut auto = AutoscrollSettings::default();
    auto.margin = strata_kit::slots::collection::row_inset(g.look());
    collection().settings(auto)
}

fn label(key: &str, t: String, g: &Gallery) -> El {
    let kit = g.kit();
    text(t).key(key).font(kit.ty.ui).color(kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c4;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let mut layer = Vec::new();
    let table = table();
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
                        .key("c4-title")
                        .font(kit.ty.ui_medium)
                        .color(kit.c.text_base),
                )
                .child(label("c4-sort", d.sort_label(), g))
                .child(label("c4-intent", d.intent.clone(), g))
                .child(row().grow(1.0))
                .child(table.columns_button(g, &mut layer)),
        )
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .child(label("c4-layout", d.layout_label(), g))
                .child(label("c4-widths", d.widths_label(), g)),
        );
    let body = if d.shown {
        table.build(g, base(g).name(d.folder.title()), &mut layer)
    } else {
        text("Icons view")
            .key("c4-elsewhere")
            .color(kit.c.text_muted)
    };
    let pane = match d.pane {
        Some(w) => col().w(w).h_full().min_h(0.0).child(body),
        None => col().w_full().h_full().min_h(0.0).child(body),
    };
    let page = col().key("c4-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(pane),
    );
    if let Some(shield) = Shield::new(|g: &Gallery| &g.c4.auto, |g: &Gallery| g.look()).build(g) {
        layer.push(shield);
    }
    rune_kit::root(page, layer)
}

pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c4;
    match &key.key {
        Key::Function(5) => d.sorted(tx),
        _ if key.is_command('g') => {
            d.grouped = !d.grouped;
            d.sorted(tx);
        }
        _ if key.is_command('e') => {
            d.field = Field::Extension;
            d.dir = SortDir::Ascending;
            d.sorted(tx);
        }
        _ if key.is_command('k') => d.table.open_columns_menu(MenuAt::Headings(None)),
        _ if key.is_command('1') => {
            d.shown = !d.shown;
            if !d.shown {
                d.table.reset();
                // The host stops the autoscroll of a listing it hides.
                d.auto.stop_gone(Some(SCOPE));
            }
        }
        _ => return false,
    }
    true
}

pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    if g.c4.coll.controllers.pointer.is_none() {
        install_pointer(g);
    }
    let d = &mut g.c4;
    d.clock = t;
    d.polls = d.polls.saturating_add(1);
    if d.polls == 2 && std::env::var_os(WARMUP).is_some_and(|v| !v.is_empty()) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        eprintln!("{WARMED}");
    }
    if d.coll.viewport().h > 0.0 {
        if let Some(x) = d.scroll_x.take() {
            tx.scroll_to_x(format!("{SCOPE}-scroll"), x);
        }
        if let Some(y) = d.scroll_y.take() {
            tx.scroll_to_instant(
                strata_components::collection::element::viewport_path(SCOPE),
                0.0,
                y,
            );
        }
    }
    let animations = g.look().animations;
    let driven = g.c4.auto.drive(t, animations, tx);
    if !g.c4.shown {
        return driven;
    }
    let base = base(g);
    table().collection(g, base).tick(g, tx) | driven
}
