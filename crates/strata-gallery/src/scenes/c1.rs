// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant};
use strata_components::collection::element::{
    Collection, CollectionState, Controllers, DropOver, Group, Groups, Intent, RowMarks, Slot,
    viewport_path,
};
use strata_components::collection::selection::{
    AnchorRule, CursorGone, Extend, Listing, RestoreMemory, SyncRule,
};
use strata_components::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};

use crate::{Gallery, Scene};

/// The variable that turns on the budget warm-up (see the module docs).
pub const WARMUP: &str = "STRATA_C1_BUDGET_WARMUP";

pub const WARMED: &str = "c1: budget warm-up done";

/// The collection's key: its rows' keys and scroll requests start here.
pub const SCOPE: &str = "c1-list";

/// The fake folders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Folder {
    /// Ten entries, folders first.
    #[default]
    Documents,
    /// 2,000 images: enough to scroll.
    Photos,
    /// 100,000 entries: the performance bar.
    Archive,
    /// Nothing.
    Empty,
}

impl Folder {
    fn title(self) -> &'static str {
        match self {
            Folder::Documents => "Documents",
            Folder::Photos => "Photos",
            Folder::Archive => "Archive",
            Folder::Empty => "Empty",
        }
    }

    /// The folder's entries on "disk".
    fn listing(self) -> Vec<ListEntry> {
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
                ];
                let mut out: Vec<ListEntry> = folders
                    .iter()
                    .map(|n| (*n, EntryKind::Folder, "folder"))
                    .chain(files.iter().map(|(n, i)| (*n, EntryKind::File, *i)))
                    .enumerate()
                    .map(|(i, (n, k, icon))| ListEntry::new(EntryId::new(1 + i as u64), n, k, icon))
                    .collect();
                out.truncate(10);
                out
            }
            Folder::Photos => (0..2_000u64)
                .map(|i| {
                    ListEntry::new(
                        EntryId::new(10_000 + i),
                        format!("IMG_{:04}.jpg", i + 1),
                        EntryKind::File,
                        "file_image",
                    )
                })
                .collect(),
            Folder::Archive => (0..100_000u64)
                .map(|i| {
                    let kind = if i % 25 == 0 {
                        EntryKind::Folder
                    } else {
                        EntryKind::File
                    };
                    let icon = if kind == EntryKind::Folder {
                        "folder"
                    } else {
                        "file"
                    };
                    ListEntry::new(
                        EntryId::new(1_000_000 + i),
                        format!("entry-{:06}.dat", i + 1),
                        kind,
                        icon,
                    )
                })
                .collect(),
            Folder::Empty => Vec::new(),
        }
    }
}

pub struct Demo {
    pub folder: Folder,
    pub listing: ListSource,
    /// Each folder's entries as the "disk" has them, once visited.
    pub disk: HashMap<Folder, Vec<ListEntry>>,
    pub coll: CollectionState,
    pub memory: RestoreMemory<Folder>,
    /// The listing version the element needs: bumped on every change at the same length.
    pub version: u64,
    /// The clock, from the page's poll.
    pub clock: f64,
    /// The preview: the cursor's entry, as `on_cursor` reports it.
    pub preview: String,
    /// The last drop the host heard, or what happened on disk.
    pub last: String,
    /// The last intent the controllers sent, as the page words it ("Clicked Notes.md").
    pub intent: String,
    /// The entry whose rename editor is open, and its text.
    pub renaming: Option<EntryId>,
    pub rename_edit: TextEdit,
    /// Host marks per entry.
    pub marks: HashMap<EntryId, RowMarks>,
    /// The layout key Ctrl+2 switches.
    pub layout: u64,
    /// When set, entering a folder lists it only this long after (a slow load).
    pub slow_load: Option<f64>,
    /// A load in progress: when it completes.
    pub loading: Option<f64>,
    /// New ids for live updates.
    pub next_id: u64,
    /// Polls run since the page opened, for the budget warm-up.
    pub polls: u32,
    pub grouped: Option<Grouped>,
}

impl Default for Demo {
    fn default() -> Self {
        let folder = Folder::Documents;
        let entries = folder.listing();
        let mut disk = HashMap::new();
        disk.insert(folder, entries.clone());
        let mut coll = CollectionState::new();
        install(&mut coll.controllers);
        Self {
            folder,
            listing: ListSource::new(entries),
            disk,
            coll,
            memory: RestoreMemory::new(),
            version: 1,
            clock: 0.0,
            preview: String::new(),
            last: String::new(),
            renaming: None,
            intent: String::new(),
            rename_edit: TextEdit::new(""),
            marks: HashMap::new(),
            layout: 1,
            slow_load: None,
            loading: None,
            next_id: 900_000,
            polls: 0,
            grouped: None,
        }
    }
}

fn install(controllers: &mut Controllers) {
    controllers.pointer = Some(Box::new(
        strata_components::collection::pointer::PointerIntent::new(),
    ));

    controllers.keyboard =
        Some(Box::<strata_components::collection::keyboard::Keyboard>::default());

    controllers.marquee = Some(Box::new(
        strata_components::collection::marquee::Marquee::new(),
    ));
}

impl Demo {
    /// The demo open on `folder`, with nothing remembered: as a scene sets it up.
    fn at(folder: Folder) -> Self {
        let mut d = Demo {
            folder,
            ..Demo::default()
        };
        let entries = d
            .disk
            .entry(folder)
            .or_insert_with(|| folder.listing())
            .clone();
        d.listing = ListSource::new(entries);
        d.coll.set_layout_key(d.layout);
        d
    }

    fn name_of(&self, id: Option<EntryId>) -> String {
        id.and_then(|id| self.listing.index_of(id))
            .map(|i| self.listing.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    fn refresh_preview(&mut self) {
        self.preview = self.name_of(self.coll.selection.cursor());
    }

    /// Re-lists the folder from its disk, as F5 does.
    fn refresh(&mut self, tx: &mut Tx) {
        let entries = self.disk.get(&self.folder).cloned().unwrap_or_default();
        self.listing.replace(entries);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
        self.refresh_preview();
    }

    /// Three new entries at the top, as a directory monitor reports them.
    fn live_insert(&mut self, tx: &mut Tx) {
        let mut added = Vec::new();
        for _ in 0..3 {
            self.next_id += 1;
            added.push(ListEntry::new(
                EntryId::new(self.next_id),
                format!("new-{}.txt", self.next_id),
                EntryKind::File,
                "file_text",
            ));
        }
        let disk = self.disk.entry(self.folder).or_default();
        disk.splice(0..0, added.iter().cloned());
        let mut entries = self.listing.entries().to_vec();
        entries.splice(0..0, added);
        self.listing.replace(entries);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
    }

    /// Deletes the cursor's entry, selecting its neighbour (the default map's delete).
    fn delete_cursor(&mut self, tx: &mut Tx) {
        let Some(id) = self.coll.selection.cursor() else {
            return;
        };
        self.coll.before_removal(&self.listing, |e| e == id);
        if let Some(disk) = self.disk.get_mut(&self.folder) {
            disk.retain(|e| e.id != id);
        }
        self.listing.retain(|e| e.id != id);
        self.version += 1;
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::NeighborSelected),
            tx,
        );
        self.refresh_preview();
    }

    /// Deletes the last selected entry on disk only: the listing shows it until a refresh.
    fn delete_on_disk(&mut self) {
        let Some(id) = self
            .coll
            .selection
            .selected_ids(&self.listing)
            .last()
            .copied()
        else {
            return;
        };
        if let Some(disk) = self.disk.get_mut(&self.folder) {
            disk.retain(|e| e.id != id);
        }
        self.last = format!("{} deleted on disk", self.name_of(Some(id)));
    }

    /// Leaves the folder shown for `to`, remembering where it was.
    fn go(&mut self, to: Folder, tx: &mut Tx) {
        if to == self.folder {
            return;
        }
        self.memory
            .remember(self.folder, self.coll.position(&self.listing));
        self.folder = to;
        let entries = self.disk.entry(to).or_insert_with(|| to.listing()).clone();
        let remembered = self.memory.recall(&to).cloned();
        self.version += 1;
        match self.slow_load {
            Some(delay) => {
                self.listing.replace(Vec::new());
                self.loading = Some(self.clock + delay);
                self.coll.enter(
                    SCOPE,
                    &self.listing,
                    remembered.as_ref(),
                    Listing::Loading,
                    tx,
                );
            }
            None => {
                self.listing.replace(entries);
                self.coll.enter(
                    SCOPE,
                    &self.listing,
                    remembered.as_ref(),
                    Listing::Complete,
                    tx,
                );
                if remembered.is_none() && !self.listing.is_empty() {
                    // A first visit picks the first entry: the host's policy.
                    self.coll.selection.auto_select(&self.listing, 0);
                }
            }
        }
        self.refresh_preview();
    }

    fn heard(&mut self, intent: Intent) {
        self.intent = match intent {
            Intent::Clicked { id } => format!("Clicked {}", self.name_of(Some(id))),
            Intent::Activate { id, .. } => format!("Opened {}", self.name_of(Some(id))),
            Intent::OpenInNew { id, window } => {
                let place = if window { "window" } else { "tab" };
                format!("Opened {} in a new {place}", self.name_of(Some(id)))
            }
            Intent::BeginDrag { ids, .. } => format!("Dragging {} items", ids.len()),
            Intent::BeginRename { id } => {
                let name = self.name_of(Some(id));
                self.renaming = Some(id);
                self.rename_edit = TextEdit::new(name.clone());
                self.rename_edit.focused = true;
                format!("Renaming {name}")
            }
            Intent::ContextMenu { target, .. } => match target {
                Some(id) => format!("Menu on {}", self.name_of(Some(id))),
                None => "Menu on the background".to_owned(),
            },
            Intent::Edge(d) => format!("Edge {d:?}"),
            _ => "Other".to_owned(),
        };
    }

    fn commit_rename(&mut self) {
        let Some(id) = self.renaming.take() else {
            return;
        };
        let name = self.rename_edit.text.clone();
        let rename = |entries: &mut Vec<ListEntry>| {
            if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                e.name = name.clone();
            }
        };
        if let Some(disk) = self.disk.get_mut(&self.folder) {
            rename(disk);
        }
        let mut entries = self.listing.entries().to_vec();
        rename(&mut entries);
        self.listing.replace(entries);
        self.version += 1;
        self.intent = format!("Renamed to {name}");
        self.refresh_preview();
    }

    /// Escape in the rename field: the name stays.
    fn cancel_rename(&mut self) {
        self.renaming = None;
        self.intent = "Rename cancelled".to_owned();
    }

    /// A slow load completing.
    fn finish_load(&mut self, tx: &mut Tx) {
        self.loading = None;
        let entries = self.disk.get(&self.folder).cloned().unwrap_or_default();
        self.listing.replace(entries);
        self.version += 1;
        let restoring = self.coll.restore_pending();
        self.coll.sync(
            SCOPE,
            &self.listing,
            SyncRule::complete(CursorGone::Neighbor),
            tx,
        );
        if !restoring && self.coll.selection.cursor().is_none() && !self.listing.is_empty() {
            self.coll.selection.auto_select(&self.listing, 0);
        }
        self.refresh_preview();
    }
}

fn scene(g: &mut Gallery, dark: bool, folder: Folder) {
    g.open("c1", dark);
    g.c1 = Demo::at(folder);
}

fn ten_rows(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
}

fn empty(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Empty);
}

fn hundred_k(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Archive);
}

fn photos(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Photos);
}

fn selection(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
    let d = &mut g.c1;
    d.coll.selection.select_only(&d.listing, 3);
    d.coll.selection.extend_to(&d.listing, 5, Extend::Replace);
    d.coll
        .selection
        .move_cursor(&d.listing, 7, AnchorRule::Keep);
    d.coll.keyboard_moved();
    d.coll.note_selection(&d.listing);
    d.refresh_preview();
}

/// A drag of files hovering the Receipts folder.
fn drop_hover(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
    g.c1.coll
        .set_drop_over(Some(DropOver::Row(EntryId::new(2))));
}

/// The rename editor open on Notes.md.
fn rename(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
    let d = &mut g.c1;
    d.coll.selection.select_only(&d.listing, 3);
    d.renaming = Some(EntryId::new(4));
    d.rename_edit = TextEdit::new("Notes.md");
    d.refresh_preview();
}

/// Every host mark: cut, dragging, the Enter target and the open path.
fn marks(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
    let d = &mut g.c1;
    let mut set = |id: u64, f: fn(&mut RowMarks)| {
        let mut m = RowMarks::default();
        f(&mut m);
        d.marks.insert(EntryId::new(id), m);
    };
    set(4, |m| m.cut = true);
    set(5, |m| m.dragging = true);
    set(6, |m| m.enter_target = true);
    set(1, |m| m.active_path = true);
}

/// The first-visit pick on the Documents folder, drawn as selected.
fn first_visit(g: &mut Gallery, dark: bool) {
    scene(g, dark, Folder::Documents);
    let d = &mut g.c1;
    d.coll.selection.auto_select(&d.listing, 0);
    d.refresh_preview();
}

fn marquee_settings(g: &Gallery) -> strata_components::collection::marquee::MarqueeSettings {
    let mut settings = strata_components::collection::marquee::MarqueeSettings::default();
    settings.animations = g.look().animations;
    settings
}

fn marquee(g: &mut Gallery, dark: bool) {
    use strata_components::contract::{Point, Rect, UniformRows};
    scene(g, dark, Folder::Documents);
    let row = g.look().row.list;
    let d = &mut g.c1;
    d.coll.selection.select_only(&d.listing, 1);
    d.coll.selection.toggle(&d.listing, 2);
    let geometry = UniformRows::new(d.listing.len(), row, 960.0).viewport_height(596.0);
    let visible = Rect::new(0.0, 0.0, 960.0, 596.0);
    let mut cx = strata_components::collection::element::Ctx::new(
        &d.listing,
        &geometry,
        visible,
        0.0,
        &mut d.coll.selection,
    );
    if let Some(m) = d.coll.controllers.marquee.as_mut() {
        let ctrl = rune::Mods {
            ctrl: true,
            ..rune::Mods::default()
        };
        m.begin(Point::new(640.0, 420.0), ctrl, &mut cx);
        m.moved(Point::new(380.0, row * 2.5), &mut cx);
    }
    d.refresh_preview();
}

pub const SCENES: &[Scene] = &[
    ("c1-empty-folder-light", |g| empty(g, false)),
    ("c1-empty-folder-dark", |g| empty(g, true)),
    ("c1-ten-rows-light", |g| ten_rows(g, false)),
    ("c1-ten-rows-dark", |g| ten_rows(g, true)),
    ("c1-hundred-k-light", |g| hundred_k(g, false)),
    ("c1-hundred-k-dark", |g| hundred_k(g, true)),
    ("c1-selection-light", |g| selection(g, false)),
    ("c1-selection-dark", |g| selection(g, true)),
    ("c1-drop-hover-light", |g| drop_hover(g, false)),
    ("c1-drop-hover-dark", |g| drop_hover(g, true)),
    ("c1-rename-light", |g| rename(g, false)),
    ("c1-rename-dark", |g| rename(g, true)),
    ("c1-marquee-light", |g| marquee(g, false)),
    ("c1-marquee-dark", |g| marquee(g, true)),
    ("c1-groups-light", |g| groups(g, false)),
    ("c1-groups-dark", |g| groups(g, true)),
    ("c1-groups-stuck-light", |g| groups_stuck(g, false)),
    ("c1-groups-stuck-dark", |g| groups_stuck(g, true)),
];

pub const REVIEW: &[Scene] = &[
    ("c1-photos-light", |g| photos(g, false)),
    ("c1-photos-dark", |g| photos(g, true)),
    ("c1-marks-light", |g| marks(g, false)),
    ("c1-marks-dark", |g| marks(g, true)),
    ("c1-first-visit-light", |g| first_visit(g, false)),
    ("c1-first-visit-dark", |g| first_visit(g, true)),
];

/// The collection as the page builds it, for its view and its poll.
pub fn collection() -> Collection<Gallery> {
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c1.listing, &g.c1.coll),
        |g: &mut Gallery| (&g.c1.listing, &mut g.c1.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c1.version,
    )
    .on_cursor(|g: &mut Gallery, _, _| g.c1.refresh_preview())
    .on_intent(|g: &mut Gallery, intent, _| g.c1.heard(intent))
    .on_drop(|g: &mut Gallery, target, dropped, _| {
        let n = dropped.data.uris().len();
        let to = match target {
            Some(id) => g.c1.name_of(Some(id)),
            None => g.c1.folder.title().to_owned(),
        };
        g.c1.last = format!("Dropped {n} on {to}");
    })
    .marks(|g: &Gallery, id| g.c1.marks.get(&id).copied().unwrap_or_default())
    .rename(
        |g: &Gallery| g.c1.renaming,
        Slot::pure(|g: &Gallery, rcx| {
            let kit = g.kit();
            row()
                .grow(1.0)
                .h_full()
                .px(rcx.look.space.xs)
                .center_y()
                .radius(kit.radius.sm)
                .fill(kit.c.bg_base)
                .outline(1.0, -1.0, kit.c.border_focus)
                .child(
                    text_input(&g.c1.rename_edit, kit.ty.ui_compact)
                        .color(kit.c.text_base)
                        .key("c1-rename")
                        .label("Rename")
                        .on_edit(|g: &mut Gallery| &mut g.c1.rename_edit)
                        .on_submit(|g: &mut Gallery, _| g.c1.commit_rename())
                        .on_input_key(|g: &mut Gallery, key: &KeyEvent, _| {
                            if key.key != Key::Escape {
                                return false;
                            }
                            g.c1.cancel_rename();
                            true
                        })
                        .grow(1.0),
                )
        }),
    )
    .empty(|g: &Gallery| {
        let kit = g.kit();
        Some(
            col()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(
                    text("Folder is empty")
                        .key("c1-empty-note")
                        .font(kit.ty.ui)
                        .color(kit.c.text_muted),
                ),
        )
    })
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    if g.c1.grouped.is_some() {
        return grouped_view(g, cx);
    }
    let d = &g.c1;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let count = d.listing.len();
    let title = if d.loading.is_some() {
        format!("{} · loading…", d.folder.title())
    } else {
        format!("{} · {count} items", d.folder.title())
    };
    let header = row()
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .gap(look.space.xl)
        .center_y()
        .child(
            text(title)
                .key("c1-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(
            text(format!("Preview: {}", d.preview))
                .key("c1-preview")
                .font(kit.ty.ui)
                .color(kit.c.text_muted),
        )
        .child(
            Button::new("Refresh")
                .variant(ButtonVariant::Ghost)
                .on_press_tx(|g: &mut Gallery, tx: &mut Tx| g.c1.refresh(tx))
                .build(kit)
                .key("c1-refresh"),
        )
        .child(
            text(d.intent.clone())
                .key("c1-intent")
                .font(kit.ty.ui)
                .color(kit.c.text_muted),
        )
        .child(
            text(d.last.clone())
                .key("c1-last")
                .font(kit.ty.ui)
                .color(kit.c.text_muted),
        );
    let list = collection()
        .name(d.folder.title())
        .settings(marquee_settings(g))
        .build(g);
    col().key("c1-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(list),
    )
}

pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c1;
    let mods = key.mods;
    match &key.key {
        Key::Escape if d.renaming.is_some() => d.cancel_rename(),
        Key::Function(5) => d.refresh(tx),
        Key::Delete if mods.ctrl || mods.cmd => d.delete_on_disk(),
        Key::Delete => d.delete_cursor(tx),
        Key::Left if mods.alt => d.go(Folder::Documents, tx),
        Key::Right if mods.alt => d.go(Folder::Photos, tx),
        _ if key.is_command('i') => d.live_insert(tx),
        _ if key.is_command('2') => {
            d.layout = if d.layout == 1 { 2 } else { 1 };
            d.coll.set_layout_key(d.layout);
        }
        _ => return false,
    }
    true
}

pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    if g.c1.grouped.is_some() {
        return grouped_poll(g, t, tx);
    }
    let d = &mut g.c1;
    d.clock = t;
    let mut changed = false;
    d.polls = d.polls.saturating_add(1);
    if d.polls == 2 && std::env::var_os(WARMUP).is_some_and(|v| !v.is_empty()) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        eprintln!("{WARMED}");
    }
    if d.loading.is_some_and(|due| t >= due) {
        d.finish_load(tx);
        changed = true;
    }
    changed | collection().settings(marquee_settings(g)).tick(g, tx)
}

/// The grouped folder's collection key.
pub const GROUPS_SCOPE: &str = "c1-groups";

/// The grouped folder's groups, in display order: (label, icon, name pattern, count).
const GROUPS: [(&str, &str, &str, usize); 6] = [
    ("Folders", "folder", "Folder", 8),
    ("Images", "file_image", "IMG_", 166),
    ("Documents", "file_text", "report-", 120),
    ("Spreadsheets", "file", "budget-", 60),
    ("Archives", "archive", "backup-", 30),
    ("Other", "file", "data-", 216),
];

pub struct Grouped {
    pub listing: ListSource,
    pub coll: CollectionState,
    /// Where to scroll once the viewport is laid out, in content points.
    pub scroll_to: Option<f32>,
    pub clock: f64,
}

impl Grouped {
    fn new() -> Self {
        let mut entries = Vec::new();
        let mut groups = Vec::new();
        for (g, (label, icon, stem, count)) in GROUPS.into_iter().enumerate() {
            for k in 0..count {
                let id = EntryId::new(500_000 + entries.len() as u64);
                let (name, kind) = match g {
                    0 => (format!("{stem} {:02}", k + 1), EntryKind::Folder),
                    1 => (format!("{stem}{:04}.jpg", k + 1), EntryKind::File),
                    2 => (format!("{stem}{:03}.odt", k + 1), EntryKind::File),
                    3 => (format!("{stem}{:02}.ods", k + 1), EntryKind::File),
                    4 => (format!("{stem}{:02}.tar.gz", k + 1), EntryKind::File),
                    _ => (format!("{stem}{:03}.bin", k + 1), EntryKind::File),
                };
                entries.push(ListEntry::new(id, name, kind, icon));
            }
            groups.push(Group::new(g as u64, label, count));
        }
        let mut coll = CollectionState::new();
        install(&mut coll.controllers);
        coll.set_groups(Groups::new(groups));
        Self {
            listing: ListSource::new(entries),
            coll,
            scroll_to: None,
            clock: 0.0,
        }
    }
}

fn grouped_scene(g: &mut Gallery, dark: bool) -> &mut Grouped {
    g.open("c1", dark);
    g.c1.grouped.insert(Grouped::new())
}

/// At the top: a selection in Images, the cursor moved on by keyboard.
fn groups(g: &mut Gallery, dark: bool) {
    let d = grouped_scene(g, dark);
    d.coll.selection.select_only(&d.listing, 10);
    d.coll.selection.extend_to(&d.listing, 12, Extend::Replace);
    d.coll.keyboard_moved();
    d.coll.note_selection(&d.listing);
}

fn groups_stuck(g: &mut Gallery, dark: bool) {
    let h = g.look().row.list;
    let d = grouped_scene(g, dark);
    d.scroll_to = Some(175.0 * h + 14.0);
}

fn grouped_collection() -> Collection<Gallery> {
    fn d(g: &Gallery) -> &Grouped {
        g.c1.grouped.as_ref().expect("the grouped scene")
    }
    Collection::new(
        GROUPS_SCOPE,
        |g: &Gallery| (&d(g).listing, &d(g).coll),
        |g: &mut Gallery| {
            let d = g.c1.grouped.as_mut().expect("the grouped scene");
            (&d.listing, &mut d.coll)
        },
        |g: &Gallery| g.look(),
        |_: &Gallery| 1,
    )
    .name("Grouped")
}

fn grouped_view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let kit = g.kit();
    let d = g.c1.grouped.as_ref().expect("the grouped scene");
    let header = row()
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .center_y()
        .child(
            text(format!("Grouped · {} items", d.listing.len()))
                .key("c1-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        );
    col().key("c1-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(grouped_collection().build(g)),
    )
}

fn grouped_poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = g.c1.grouped.as_mut().expect("the grouped scene");
    d.clock = t;
    // Once laid out: a scroll asked for before the viewport exists has nothing to move.
    if d.coll.viewport().h > 0.0
        && let Some(y) = d.scroll_to.take()
    {
        tx.scroll_to_instant(viewport_path(GROUPS_SCOPE), 0.0, y);
    }
    grouped_collection().tick(g, tx)
}
