// SPDX-License-Identifier: MIT

//! The context menus: which rows a target gets, in Strata's words, and what a chosen row
//! does. strata-components' C21 draws them and holds no policy.
//!
//! Ported from the GTK views: the rows and their order from `item_options.rs` and
//! `folder_menu.rs` under `src/ui/browser/context_menu/`, their visibility from
//! `ItemMenuOptions::refresh` in `item_menu.rs`, and their hints from `default_hint` in
//! `src/ui/shortcut_reference.rs`. A fix there has to be carried here until the GTK views
//! are deleted. The port covers what this interface opens, a local folder. Left out, with
//! what brings each back:
//!
//! - the Trash and Recent variants (Restore, Remove from Recent, the delete rows' other
//!   forms), the search result's "Open file location", and an administrator location's
//!   rules: the sidebar and the location bar, which reach those places;
//! - "Send to…", custom actions and file-provider rows: devices, the custom-action catalog
//!   and the providers;
//! - the 10xer hints;
//! - each row's icon.
//!
//! A row whose action belongs to a part this interface does not have yet is shown as the
//! GTK menu shows it at rest and does nothing: [`Row::wired`] is `None` for it.

use rune::{El, Keybind};
use strata_components::{
    context_menu::{ContextMenu, ContextMenuState, Item, MenuModel, MenuTarget, Section},
    contract::Point,
};
use strata_core::{
    i18n::{count, tr},
    model::{ArchiveFormat, EntryKind, FileEntry, Location, MetadataValue},
};

use crate::{
    App, columns,
    slots::{Frame, Part, Place, Setup},
};

/// The one menu the window has open, and what it was opened on.
#[derive(Default)]
pub(crate) struct Menu {
    pub(crate) state: ContextMenuState,
    /// The column the menu is for.
    depth: usize,
    /// What assistive technology hears the menu called: its entry or its folder.
    name: String,
}

/// A menu to open: what it is for, and where.
pub(crate) struct Opening {
    /// The column the menu is for.
    pub(crate) depth: usize,
    pub(crate) name: String,
    pub(crate) target: MenuTarget,
    /// In window points.
    pub(crate) at: Point,
}

/// The menu's element, with its surfaces in the layer while it is open.
fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let menu = ContextMenu::new(
        "menu",
        |app: &App| &app.menu.state,
        |app: &mut App| &mut app.menu.state,
    )
    .name(app.menu.name())
    .on_intent(columns::chosen)
    .build(app, &app.look, frame.layer);
    Some(menu)
}

impl Part for Menu {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(_: &Setup<'_>) -> Self {
        Self::default()
    }
}

impl Menu {
    pub(crate) fn show(&mut self, opening: Opening, model: MenuModel) {
        self.depth = opening.depth;
        self.name = opening.name;
        self.state.open(opening.target, opening.at, model);
    }

    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// What a chosen row does here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wired {
    /// Open the entry as a click on it does.
    Open,
    /// Start the inline rename: V9 opens the editor.
    Rename,
    SelectAll,
    Refresh,
    ToggleHidden,
}

/// One row of a menu: its id, its label in the catalogs, and the chords its hint shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    /// The bindings `default_hint` shows for the row, as Rune spells chords.
    hint: &'static [&'static str],
    /// The hint is a plain key, which "Type to search" takes: hidden while that is on.
    typed: bool,
    pub(crate) wired: Option<Wired>,
}

const fn row(id: &'static str, label: &'static str) -> Row {
    Row {
        id,
        label,
        hint: &[],
        typed: false,
        wired: None,
    }
}

impl Row {
    const fn hint(mut self, chords: &'static [&'static str]) -> Self {
        self.hint = chords;
        self
    }

    const fn typed(mut self) -> Self {
        self.typed = true;
        self
    }

    const fn does(mut self, wired: Wired) -> Self {
        self.wired = Some(wired);
        self
    }

    /// Whether the row shows its hint: it has one, and "Type to search" does not take it.
    fn hinted(&self, typing: bool) -> bool {
        let taken = self.typed && typing;
        !self.hint.is_empty() && !taken
    }

    fn item(&self, typing: bool) -> Item {
        let item = Item::new(self.id, tr(self.label));
        if !self.hinted(typing) {
            return item;
        }
        let chords: Vec<String> = self
            .hint
            .iter()
            .map(|chord| Keybind::new(chord).label())
            .collect();
        item.shortcut(chords.join(" / "))
    }
}

pub(crate) const NEW_FOLDER: Row = row("new-folder", "New Folder").hint(&["ctrl+shift+n"]);
pub(crate) const NEW_FILE: Row = row("new-file", "New File");
pub(crate) const GROUP: Row = row("group", "New Folder with Selection");
pub(crate) const OPEN: Row = row("open", "Open").hint(&["enter"]).does(Wired::Open);
pub(crate) const OPEN_WITH: Row = row("open-with", "Open With…");
pub(crate) const PREVIEW: Row = row("preview", "Quick preview").hint(&["space"]);
pub(crate) const RUN: Row = row("run", "Run");
pub(crate) const TERMINAL: Row = row("terminal", "Open in Terminal").hint(&["ctrl+alt+t"]);
pub(crate) const OPEN_IN: Row = row("open-in", "Open in…");
pub(crate) const NEW_TAB: Row = row("open-in-tab", "New Tab").hint(&["ctrl+enter"]);
pub(crate) const NEW_WINDOW: Row = row("open-in-window", "New Window").hint(&["shift+enter"]);
pub(crate) const EXTRACT: Row = row("extract", "Extract here");
pub(crate) const EXTRACT_TO: Row = row("extract-to", "Extract to…");
pub(crate) const CUT: Row = row("cut", "Cut").hint(&["ctrl+x"]);
pub(crate) const COPY: Row = row("copy", "Copy").hint(&["ctrl+c"]);
pub(crate) const DUPLICATE: Row = row("duplicate", "Duplicate").hint(&["ctrl+d"]);
pub(crate) const RENAME: Row = row("rename", "Rename")
    .hint(&["f2", "ctrl+r"])
    .does(Wired::Rename);
pub(crate) const MOVE_TO: Row = row("move-to", "Move to…");
pub(crate) const COPY_TO: Row = row("copy-to", "Copy to…");
pub(crate) const COMPRESS: Row = row("compress", "Compress…");
pub(crate) const PIN: Row = row("pin", "Pin to sidebar").hint(&["p"]).typed();
pub(crate) const CUSTOMIZE: Row = row("customize", "Customize…");
pub(crate) const COPY_PATH: Row = row("copy-path", "Copy path").hint(&["y"]).typed();
pub(crate) const COPY_NAME: Row = row("copy-name", "Copy name");
pub(crate) const COPY_PATHS: Row = row("copy-paths", "Copy paths").hint(&["y"]).typed();
pub(crate) const COPY_NAMES: Row = row("copy-names", "Copy names");
pub(crate) const PROPERTIES: Row = row("properties", "Properties").hint(&["alt+enter"]);
/// The background menu's Properties shows no hint (`ContextHint::None` in `folder_menu.rs`).
pub(crate) const FOLDER_PROPERTIES: Row = row("properties", "Properties");
pub(crate) const TRASH: Row = row("trash", "Move to Trash").hint(&["delete"]);
pub(crate) const DELETE: Row = row("delete", "Permanently delete").hint(&["shift+delete"]);
pub(crate) const PASTE: Row = row("paste", "Paste").hint(&["ctrl+v"]);
pub(crate) const SELECT_ALL: Row = row("select-all", "Select All")
    .hint(&["ctrl+a"])
    .does(Wired::SelectAll);
pub(crate) const REFRESH: Row = row("refresh", "Refresh").hint(&["f5"]).does(Wired::Refresh);
pub(crate) const SHOW_HIDDEN: Row = row("hidden", "Show Hidden Files")
    .hint(&["ctrl+h"])
    .does(Wired::ToggleHidden);
pub(crate) const HIDE_HIDDEN: Row = row("hidden", "Hide Hidden Files")
    .hint(&["ctrl+h"])
    .does(Wired::ToggleHidden);

/// Every row either menu can show: what a chosen id is looked up in. The two hidden-files
/// rows share an id, as the two Properties rows do.
const ROWS: &[Row] = &[
    NEW_FOLDER,
    NEW_FILE,
    GROUP,
    OPEN,
    OPEN_WITH,
    PREVIEW,
    RUN,
    TERMINAL,
    OPEN_IN,
    NEW_TAB,
    NEW_WINDOW,
    EXTRACT,
    EXTRACT_TO,
    CUT,
    COPY,
    DUPLICATE,
    RENAME,
    MOVE_TO,
    COPY_TO,
    COMPRESS,
    PIN,
    CUSTOMIZE,
    COPY_PATH,
    COPY_NAME,
    COPY_PATHS,
    COPY_NAMES,
    PROPERTIES,
    TRASH,
    DELETE,
    PASTE,
    SELECT_ALL,
    REFRESH,
    SHOW_HIDDEN,
    HIDE_HIDDEN,
    FOLDER_PROPERTIES,
];

/// What the row with `id` does here, if anything.
pub(crate) fn wired(id: &str) -> Option<Wired> {
    ROWS.iter().find(|row| row.id == id)?.wired
}

/// What an entry's menu is built from.
pub(crate) struct EntryTarget<'a> {
    /// The entry under the pointer.
    pub(crate) entry: &'a FileEntry,
    /// What the menu acts on: the selection when the entry is in it, else the entry alone
    /// (`context_entries` in `src/ui/browser/context_menu.rs`).
    pub(crate) entries: &'a [FileEntry],
    /// Whether the pane's folder can be trashed from and deleted from, where GIO says.
    pub(crate) can_trash: Option<bool>,
    pub(crate) can_delete: Option<bool>,
    /// "Type to search" takes plain keys, so their hints are hidden.
    pub(crate) typing: bool,
}

/// `compact_native_path` in `src/ui/browser/paths.rs`: the home folder as `~`.
fn compact_path(location: &Location) -> String {
    let Some(path) = location.native_path() else {
        return location.display_path();
    };
    let home = glib::home_dir();
    if path == home {
        return "~".to_owned();
    }
    match path.strip_prefix(&home) {
        Ok(below) => format!("~/{}", below.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// `selected_items_summary` in `src/ui/browser/context_menu.rs`: the first three names,
/// cut at thirty characters.
fn summary(entries: &[FileEntry]) -> String {
    const MAX_CHARS: usize = 30;
    let first: Vec<&str> = entries
        .iter()
        .take(3)
        .map(|entry| entry.display_name.as_str())
        .collect();
    let mut names = first.join(", ");
    if entries.len() > 3 {
        names.push_str(", …");
    }
    if names.chars().count() > MAX_CHARS {
        names = names.chars().take(MAX_CHARS - 1).collect();
        names.push('…');
    }
    names
}

fn is_file(entry: &FileEntry) -> bool {
    matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink)
}

/// `entry_is_regular_executable` in `src/ui/browser/desktop.rs`.
fn runs(entry: &FileEntry) -> bool {
    is_file(entry) && matches!(entry.mode, MetadataValue::Known(mode) if mode & 0o111 != 0)
}

fn section(rows: &[(Row, bool)], typing: bool) -> Section {
    let shown = rows.iter().filter(|(_, shown)| *shown);
    Section::new(shown.map(|(row, _)| row.item(typing)))
}

fn open_in(typing: bool) -> Item {
    let children = Section::new([NEW_TAB.item(typing), NEW_WINDOW.item(typing)]);
    Item::submenu(OPEN_IN.id, tr(OPEN_IN.label), [children])
}

fn delete_rows(target: &EntryTarget<'_>) -> Section {
    // `move_to_trash_is_visible` and `permanently_delete_is_visible`: shown unless GIO says
    // the folder supports neither.
    let mut rows = Vec::new();
    if target.can_trash.unwrap_or(true) {
        rows.push(TRASH.item(target.typing));
    }
    if target.can_delete.unwrap_or(true) {
        rows.push(DELETE.item(target.typing).danger(true));
    }
    Section::new(rows)
}

fn creation(typing: bool) -> Section {
    Section::new([NEW_FOLDER.item(typing), NEW_FILE.item(typing)])
}

/// One entry's menu: `SingleOpenOptions` and `SingleEditOptions`.
fn single(target: &EntryTarget<'_>) -> MenuModel {
    let (entry, typing) = (target.entry, target.typing);
    let folder = entry.is_directory();
    let archive = ArchiveFormat::for_entry(entry).is_some();
    let mut open = section(
        &[
            (OPEN, true),
            (OPEN_WITH, true),
            // `entry_supports_quick_preview` in `src/ui/preview.rs`, for a local file.
            (PREVIEW, is_file(entry)),
            (RUN, runs(entry)),
            (TERMINAL, folder),
        ],
        typing,
    );
    if folder {
        open.items.push(open_in(typing));
    }
    open.items
        .extend(section(&[(EXTRACT, archive), (EXTRACT_TO, archive)], typing).items);
    MenuModel::new()
        .header_line(entry.display_name.clone())
        .header_line(compact_path(&entry.location))
        .section(creation(typing))
        .section(section(&[(GROUP, true)], typing))
        .section(open)
        .section(section(
            &[(CUT, true), (COPY, true), (DUPLICATE, true), (RENAME, true)],
            typing,
        ))
        .section(section(&[(MOVE_TO, true), (COPY_TO, true)], typing))
        .section(section(&[(COMPRESS, true)], typing))
        .section(section(
            &[
                (PIN, folder),
                (CUSTOMIZE, true),
                (COPY_PATH, true),
                (COPY_NAME, true),
                (PROPERTIES, true),
            ],
            typing,
        ))
        .section(delete_rows(target))
}

/// Several entries' menu: `MultipleOpenOptions` and `MultipleEditOptions`.
fn several(target: &EntryTarget<'_>) -> MenuModel {
    let typing = target.typing;
    let heading = rust_i18n::t!(
        "%{items} selected",
        items = count("items", target.entries.len())
    );
    let mut open = section(&[(OPEN, true), (OPEN_WITH, true)], typing);
    if target.entries.iter().any(FileEntry::is_directory) {
        open.items.push(open_in(typing));
    }
    MenuModel::new()
        .header_line(heading)
        .header_line(summary(target.entries))
        .section(creation(typing))
        .section(section(&[(GROUP, true)], typing))
        .section(open)
        .section(section(
            &[(CUT, true), (COPY, true), (DUPLICATE, true)],
            typing,
        ))
        .section(section(&[(MOVE_TO, true), (COPY_TO, true)], typing))
        .section(section(&[(COMPRESS, true)], typing))
        .section(section(
            &[(COPY_PATHS, true), (COPY_NAMES, true), (PROPERTIES, true)],
            typing,
        ))
        .section(delete_rows(target))
}

/// The menu of the entry under the pointer, or of the selection it is part of.
pub(crate) fn entry(target: &EntryTarget<'_>) -> MenuModel {
    if target.entries.len() > 1 {
        several(target)
    } else {
        single(target)
    }
}

/// What a pane's background menu is built from.
pub(crate) struct FolderTarget {
    /// The folder lists something: Select All has entries to select.
    pub(crate) has_entries: bool,
    pub(crate) hidden_shown: bool,
    pub(crate) typing: bool,
}

/// A pane's background menu: `FolderMenuOptions`. Paste is there and insensitive: this
/// interface has no clipboard yet, and the GTK menu greys it while the clipboard is empty.
pub(crate) fn folder(target: &FolderTarget) -> MenuModel {
    let typing = target.typing;
    let hidden = if target.hidden_shown {
        HIDE_HIDDEN
    } else {
        SHOW_HIDDEN
    };
    MenuModel::new()
        .section(Section::new([
            NEW_FOLDER.item(typing),
            NEW_FILE.item(typing),
            PASTE.item(typing).enabled(false),
        ]))
        .section(section(&[(OPEN_WITH, true), (TERMINAL, true)], typing))
        .section(Section::new([
            SELECT_ALL.item(typing).enabled(target.has_entries),
            REFRESH.item(typing),
            hidden.item(typing),
        ]))
        .section(section(
            &[(CUSTOMIZE, true), (FOLDER_PROPERTIES, true)],
            typing,
        ))
}

#[cfg(test)]
mod tests;
