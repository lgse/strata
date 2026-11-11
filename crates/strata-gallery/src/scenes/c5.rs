//! The gallery page for C5 InlineRenameEditor: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c5-` and end in `-light` or `-dark`.
//!
//! The page is a host, as Strata will be: a C1e collection (C1b, C1c and C1d installed) of
//! a fake folder, with the editor in its rename slot. F2 renames the cursor's entry, as does a
//! slow second click on its name. The host's parts:
//!
//! - **The validator** ([`validate`]): a stand-in for Strata's `validate_basename` and
//!   `TypedPath::parse`, with Strata's messages. **Remote** (Ctrl+T) makes the folder
//!   non-local, where a `/` is refused instead of being a path.
//! - **Tab**: a [`By::Next`] or [`By::Previous`] ending moves the cursor to the neighbour and
//!   opens the editor there; at either end it only commits.
//! - **The log** (`c5-log-0`, newest first, to `c5-log-2`): what the editor asked.
//! - **The view mode** (`c5-mode`, Ctrl+2): a stand-in shortcut the host leaves alone while
//!   the field has the keyboard (`RenameState::has_keyboard`), as Strata's Ctrl+1–3.
//! - **Go to Downloads** (`c5-go`): navigation, which commits an open edit first.
//! - **Search** (`c5-other`): another field, so the keyboard can move to it.
//! - **List** (Ctrl+L): the same editor inside C4's ListTable, in its primary column.
//!
//! Work package: `docs/work/phase-1-leaves/C5-inline-rename.md`.

use rune::{Align, Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant};
use strata_components::collection::element::{Collection, CollectionState, Controllers, Intent};
use strata_components::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use strata_components::list_table::{
    Cell, Column, ColumnLayout, ColumnWidths, ListParts, ListTable, ListTableState,
};
use strata_components::rename::{
    By, Initial, Kept, Polled, RenameEditor, RenameIntent, RenameState,
};

use crate::{Gallery, Scene};

/// The collection's key: its rows' keys and scroll requests start here.
pub const SCOPE: &str = "c5-files";

/// The ListTable's key, for the `c5-list` scenes.
pub const TABLE: &str = "c5-table";

/// C5's demo state, held in `Gallery::c5`.
pub struct Demo {
    pub listing: ListSource,
    pub coll: CollectionState,
    pub rename: RenameState,
    pub version: u64,
    pub clock: f64,
    /// The folder isn't local: a `/` is refused, not a path.
    pub remote: bool,
    /// The folder shown.
    pub folder: &'static str,
    /// What the editor asked, newest first.
    pub log: Vec<String>,
    /// The other field (Search).
    pub other: TextEdit,
    /// The stand-in view mode, which Ctrl+2 toggles.
    pub mode: u8,
    /// The editor in C4's ListTable instead of the plain collection.
    pub list: bool,
    pub table: ListTableState,
    pub layout: ColumnLayout,
    pub widths: ColumnWidths,
    /// The field's text centred, as in Icons (`c5-centered`).
    pub centered: bool,
    /// The pane's width, for the narrow review scene.
    pub width: Option<f32>,
}

impl Default for Demo {
    fn default() -> Self {
        let mut coll = CollectionState::new();
        install(&mut coll.controllers);
        Self {
            listing: ListSource::new(documents()),
            coll,
            rename: RenameState::new(),
            version: 1,
            clock: 0.0,
            remote: false,
            folder: "Documents",
            log: Vec::new(),
            other: TextEdit::new(""),
            mode: 1,
            list: false,
            table: ListTableState::new(),
            layout: ColumnLayout::new(columns().iter().map(|c| c.id().clone())),
            widths: ColumnWidths::new(),
            centered: false,
            width: None,
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

/// The fake Documents folder: folders first, then files with every kind of stem.
fn documents() -> Vec<ListEntry> {
    let entries: [(&str, EntryKind, &str); 10] = [
        ("Projects", EntryKind::Folder, "folder"),
        ("photos.2024", EntryKind::Folder, "folder"),
        ("Notes.md", EntryKind::File, "text-markdown"),
        ("archive.tar.gz", EntryKind::File, "package-x-generic"),
        (".bashrc", EntryKind::File, "text-x-script"),
        ("README", EntryKind::File, "text-x-generic"),
        ("photo.final.JPG", EntryKind::File, "image-x-generic"),
        ("todo.txt", EntryKind::File, "text-x-generic"),
        ("report.pdf", EntryKind::File, "application-pdf"),
        (
            "a very long file name that runs past the column's edge.txt",
            EntryKind::File,
            "text-x-generic",
        ),
    ];
    entries
        .into_iter()
        .enumerate()
        .map(|(i, (name, kind, icon))| ListEntry::new(EntryId::new(i as u64 + 1), name, kind, icon))
        .collect()
}

fn downloads() -> Vec<ListEntry> {
    ["setup.sh", "invoice.pdf"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            ListEntry::new(
                EntryId::new(100 + i as u64),
                name,
                EntryKind::File,
                "text-x-generic",
            )
        })
        .collect()
}

/// The host's rules for a name: a stand-in for Strata's `validate_basename` and
/// `TypedPath::parse`, with Strata's messages. Policy, not the editor's.
pub fn validate(remote: bool, kind: EntryKind, name: &str) -> Option<String> {
    let says = |m: &str| Some(m.to_owned());
    if name.trim().is_empty() {
        return says("Enter a name");
    }
    if name == "." || name == ".." {
        return says("That name is reserved");
    }
    if !name.contains('/') {
        return None;
    }
    if remote {
        return says("Names cannot contain /");
    }
    // A typed path (`operations/rename/typed-paths`).
    if name.starts_with('/') || name.contains("//") {
        return says("Enter a name");
    }
    let parts: Vec<&str> = name.trim_end_matches('/').split('/').collect();
    if parts.iter().any(|p| p.trim().is_empty()) {
        return says("Enter a name");
    }
    if matches!(parts.last(), Some(&".") | Some(&"..")) {
        return says("That name is reserved");
    }
    if name.ends_with('/') && !kind.is_folder_like() {
        return says("An existing file can’t become a folder");
    }
    None
}

impl Demo {
    fn name_of(&self, id: EntryId) -> String {
        self.listing
            .index_of(id)
            .map(|i| self.listing.entry(i).name.to_owned())
            .unwrap_or_default()
    }

    fn kind_of(&self, id: EntryId) -> EntryKind {
        self.listing
            .index_of(id)
            .map_or(EntryKind::File, |i| self.listing.entry(i).kind)
    }

    fn scope(&self) -> &'static str {
        if self.list { TABLE } else { SCOPE }
    }

    fn note(&mut self, line: String) {
        self.log.insert(0, line);
        self.log.truncate(3);
    }

    /// Opens the editor on entry `id` and reveals its row.
    fn begin(&mut self, id: EntryId, tx: &mut Tx) {
        let Some(index) = self.listing.index_of(id) else {
            return;
        };
        let name = self.name_of(id);
        let kind = self.kind_of(id);
        let polled = self
            .rename
            .begin(self.scope(), id, &name, kind, Initial::Stem);
        self.heard_all(polled, tx);
        self.coll.reveal(self.scope(), &self.listing, index, tx);
    }

    /// F2: renames the cursor's entry, unless the field has the keyboard (it took F2 then,
    /// as Strata's does).
    fn rename_cursor(&mut self, tx: &mut Tx) {
        if self.rename.has_keyboard() {
            return;
        }
        if let Some(id) = self.coll.selection.cursor() {
            self.begin(id, tx);
        }
    }

    fn heard_all(&mut self, polled: Polled, tx: &mut Tx) -> bool {
        for intent in polled.intents {
            self.heard(intent, tx);
        }
        polled.changed
    }

    /// What the editor asked.
    fn heard(&mut self, intent: RenameIntent, tx: &mut Tx) {
        let id = intent.id();
        let old = self.name_of(id);
        let line = match &intent {
            RenameIntent::Rename { name, by, .. } => {
                self.apply(id, name);
                format!("Renamed {old} to {name} ({})", by_word(*by))
            }
            RenameIntent::Move { path, by, .. } => {
                format!("Move {old} to {path} ({})", by_word(*by))
            }
            RenameIntent::Keep { why, by, .. } => {
                format!("Kept {old} ({}, {})", kept_word(*why), by_word(*by))
            }
            RenameIntent::FieldMenu { .. } => format!("Menu on the field of {old}"),
            _ => "Other".to_owned(),
        };
        self.note(line);
        if let Some(step) = intent.by().and_then(By::step) {
            self.advance(id, step, tx);
        }
    }

    /// The entry takes its new name and keeps its id, as this host promises.
    fn apply(&mut self, id: EntryId, name: &str) {
        let mut entries = self.listing.entries().to_vec();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            e.name = name.to_owned();
        }
        self.listing.replace(entries);
        self.version += 1;
    }

    /// After a Tab ending: the editor goes on to the neighbour, unless `id` was at that end.
    fn advance(&mut self, id: EntryId, step: isize, tx: &mut Tx) {
        let Some(index) = self.listing.index_of(id) else {
            return;
        };
        let Some(next) = index
            .checked_add_signed(step)
            .filter(|&i| i < self.listing.len())
        else {
            return;
        };
        let _ = self.coll.selection.select_only(&self.listing, next);
        let next_id = self.listing.id_at(next);
        self.begin(next_id, tx);
    }

    /// Another folder: an open edit commits first, as Strata's click-away does (the blur
    /// would come a frame after the listing changed).
    fn go(&mut self, tx: &mut Tx) {
        let polled = self.rename.commit(By::Host);
        self.heard_all(polled, tx);
        let (folder, entries) = if self.folder == "Documents" {
            ("Downloads", downloads())
        } else {
            ("Documents", documents())
        };
        self.folder = folder;
        self.listing = ListSource::new(entries);
        self.coll.selection.clear();
        self.version += 1;
    }
}

fn by_word(by: By) -> &'static str {
    match by {
        By::Enter => "Enter",
        By::Escape => "Escape",
        By::Blur => "focus left",
        By::Next => "Tab",
        By::Previous => "Shift+Tab",
        By::Host => "host",
        By::Listing => "listing changed",
        By::Scrolled => "scrolled out",
        _ => "other",
    }
}

fn kept_word(why: Kept) -> &'static str {
    match why {
        Kept::Cancelled => "cancelled",
        Kept::Unchanged => "unchanged",
        Kept::Empty => "empty",
        Kept::Invalid => "invalid",
        _ => "other",
    }
}

/// The editor, as every view hosts it.
fn editor() -> RenameEditor<Gallery> {
    RenameEditor::new(
        |g: &Gallery| &g.c5.rename,
        |g: &mut Gallery| &mut g.c5.rename,
    )
    .validate(|g: &Gallery, id, name: &str| validate(g.c5.remote, g.c5.kind_of(id), name))
    .on_intent(|g: &mut Gallery, intent, tx| g.c5.heard(intent, tx))
}

/// The plain collection.
fn collection(key: &'static str, centered: bool) -> Collection<Gallery> {
    let mut editor = editor();
    if centered {
        editor = editor.align(Align::Center);
    }
    Collection::new(
        key,
        |g: &Gallery| (&g.c5.listing, &g.c5.coll),
        |g: &mut Gallery| (&g.c5.listing, &mut g.c5.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c5.version,
    )
    .on_intent(|g: &mut Gallery, intent, tx| {
        if let Intent::BeginRename { id } = intent {
            g.c5.begin(id, tx);
        }
    })
    .rename(
        move |g: &Gallery| g.c5.rename.renaming_in(key),
        editor.slot(),
    )
}

fn columns() -> Vec<Column<Gallery>> {
    vec![
        Column::primary("name", "Name").width(260.0).min(120.0),
        Column::new("kind", "Kind", |_: &Gallery, rcx| {
            Cell::text(if rcx.entry.kind.is_folder_like() {
                "Folder"
            } else {
                "File"
            })
            .muted()
        })
        .width(100.0)
        .min(60.0),
    ]
}

fn table() -> ListTable<Gallery> {
    ListTable::new(
        TABLE,
        columns(),
        |g: &Gallery| ListParts {
            source: &g.c5.listing,
            collection: &g.c5.coll,
            table: &g.c5.table,
            layout: &g.c5.layout,
            widths: &g.c5.widths,
            sort: None,
            look: g.look(),
        },
        |g: &mut Gallery| &mut g.c5.table,
    )
}

/// Opens the page with the editor on entry `id`, as F2 would.
fn editing(g: &mut Gallery, dark: bool, id: u64, setup: impl FnOnce(&mut Demo)) {
    g.open("c5", dark);
    let d = &mut g.c5;
    setup(d);
    let id = EntryId::new(id);
    if let Some(index) = d.listing.index_of(id) {
        let _ = d.coll.selection.select_only(&d.listing, index);
    }
    let name = d.name_of(id);
    let kind = d.kind_of(id);
    let _ = d.rename.begin(d.scope(), id, &name, kind, Initial::Stem);
}

fn file(g: &mut Gallery, dark: bool) {
    editing(g, dark, 3, |_| {});
}

fn folder(g: &mut Gallery, dark: bool) {
    editing(g, dark, 1, |_| {});
}

fn dotfile(g: &mut Gallery, dark: bool) {
    editing(g, dark, 5, |_| {});
}

fn multidot(g: &mut Gallery, dark: bool) {
    editing(g, dark, 4, |_| {});
}

/// A refused typed path, with its message.
fn invalid(g: &mut Gallery, dark: bool) {
    editing(g, dark, 3, |_| {});
    let d = &mut g.c5;
    d.rename.edit.replace_text("a//b");
    d.rename.edit.set_caret(4);
    let _ = d
        .rename
        .validated(validate(d.remote, EntryKind::File, "a//b"));
}

fn list(g: &mut Gallery, dark: bool) {
    editing(g, dark, 3, |d| d.list = true);
}

fn centered(g: &mut Gallery, dark: bool) {
    editing(g, dark, 3, |d| {
        d.centered = true;
        d.width = Some(220.0);
    });
}

fn long(g: &mut Gallery, dark: bool) {
    editing(g, dark, 10, |d| d.width = Some(240.0));
    g.c5.rename.edit.set_caret(g.c5.rename.edit.text.len());
}

/// The folder with the cursor on Notes.md and no edit open.
fn browse(g: &mut Gallery, dark: bool) {
    g.open("c5", dark);
    let d = &mut g.c5;
    let _ = d.coll.selection.select_only(&d.listing, 2);
}

/// Notes.md being renamed in a folder that isn't local, where a `/` is refused.
fn remote(g: &mut Gallery, dark: bool) {
    editing(g, dark, 3, |d| d.remote = true);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c5", dark)`, then fills in `g.c5`.
pub const SCENES: &[Scene] = &[
    ("c5-file-light", |g| file(g, false)),
    ("c5-file-dark", |g| file(g, true)),
    ("c5-folder-light", |g| folder(g, false)),
    ("c5-folder-dark", |g| folder(g, true)),
    ("c5-dotfile-light", |g| dotfile(g, false)),
    ("c5-dotfile-dark", |g| dotfile(g, true)),
    ("c5-multidot-light", |g| multidot(g, false)),
    ("c5-multidot-dark", |g| multidot(g, true)),
    ("c5-invalid-light", |g| invalid(g, false)),
    ("c5-invalid-dark", |g| invalid(g, true)),
    ("c5-list-light", |g| list(g, false)),
    ("c5-list-dark", |g| list(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c5-centered-light", |g| centered(g, false)),
    ("c5-centered-dark", |g| centered(g, true)),
    ("c5-long-light", |g| long(g, false)),
    ("c5-long-dark", |g| long(g, true)),
    ("c5-browse-light", |g| browse(g, false)),
    ("c5-browse-dark", |g| browse(g, true)),
    ("c5-remote-light", |g| remote(g, false)),
    ("c5-remote-dark", |g| remote(g, true)),
];

fn label(g: &Gallery, key: &str, t: String) -> El {
    let kit = g.kit();
    text(t).key(key).font(kit.ty.ui).color(kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c5;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let kit = g.kit();
    let mut layer = Vec::new();
    let place = if d.remote {
        format!("{} (remote)", d.folder)
    } else {
        d.folder.to_owned()
    };
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
                    text(format!("{place} · {} items", d.listing.len()))
                        .key("c5-title")
                        .font(kit.ty.ui_medium)
                        .color(kit.c.text_base),
                )
                .child(label(g, "c5-mode", format!("View {}", d.mode)))
                .child(row().grow(1.0))
                .child(
                    text_input(&d.other, kit.ty.ui_compact)
                        .single_line()
                        .key("c5-other")
                        .label("Search")
                        .placeholder("Search", kit.c.text_faint)
                        .color(kit.c.text_base)
                        .w(140.0)
                        .px(look.space.xs)
                        .fill(kit.c.bg_base)
                        .radius(kit.radius.sm)
                        .outline(1.0, -1.0, kit.c.border_base)
                        .on_edit(|g: &mut Gallery| &mut g.c5.other),
                )
                .child(
                    Button::new(if d.folder == "Documents" {
                        "Go to Downloads"
                    } else {
                        "Go to Documents"
                    })
                    .variant(ButtonVariant::Ghost)
                    .on_press_tx(|g: &mut Gallery, tx: &mut Tx| g.c5.go(tx))
                    .build(kit)
                    .key("c5-go"),
                ),
        )
        .child(
            col().w_full().gap(look.space.xxs).children(
                d.log
                    .iter()
                    .enumerate()
                    .map(|(i, line)| label(g, &format!("c5-log-{i}"), line.clone())),
            ),
        );
    layer.extend(editor().message(g, look));
    let body = if d.list {
        table().build(g, collection(TABLE, false).name(d.folder), &mut layer)
    } else {
        collection(SCOPE, d.centered).name(d.folder).build(g)
    };
    let pane = match d.width {
        Some(w) => col().w(w).h_full().min_h(0.0).child(body),
        None => col().w_full().h_full().min_h(0.0).child(body),
    };
    let page = col().key("c5-page").w_full().h_full().child(header).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(look.content.pad)
            .child(pane),
    );
    rune_kit::root(page, layer)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c5;
    // The field's keys are its own: the host's shortcuts wait while it has the keyboard.
    if d.rename.has_keyboard() {
        return false;
    }
    match &key.key {
        Key::Function(2) if !(key.mods.ctrl || key.mods.alt || key.mods.shift || key.mods.logo) => {
            d.rename_cursor(tx);
        }
        _ if key.is_command('2') => d.mode = if d.mode == 1 { 2 } else { 1 },
        _ if key.is_command('t') => d.remote = !d.remote,
        _ if key.is_command('l') => d.list = !d.list,
        _ => return false,
    }
    true
}

/// Runs at the start of every frame while this page is open, with the clock in seconds:
/// the editor's tick, which hears a blur, a row scrolled out and the listing changing.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c5;
    d.clock = t;
    let polled = d.rename.tick(d.scope(), &d.listing, &d.coll);
    d.heard_all(polled, tx)
}
