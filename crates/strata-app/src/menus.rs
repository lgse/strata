// SPDX-License-Identifier: MIT

//! The context menus: which rows a target gets, in Strata's words, what a chosen row does,
//! and the one way a view opens one ([`open`]). strata-components' C21 draws them and holds
//! no policy.
//!
//! Ported from the GTK views: the rows and their order from `item_options.rs` and
//! `folder_menu.rs` under `src/ui/browser/context_menu/`, their visibility from
//! `ItemMenuOptions::refresh` in `item_menu.rs`, and their hints from `default_hint` in
//! `src/ui/shortcut_reference.rs`: the catalog's keys, read through the window's keymap
//! (`commands::Live`), so a saved keymap changes them as it changes the keys. A fix there has
//! to be carried here until the GTK views are deleted. The port covers what this interface opens, a local folder. Left out, with
//! what brings each back:
//!
//! - the Trash and Recent variants (Restore, Remove from Recent, the delete rows' other
//!   forms), the search result's "Open file location", and an administrator location's
//!   rules: the sidebar and the location bar, which reach those places;
//! - "Send to…", custom actions and file-provider rows: devices, the custom-action catalog
//!   and the providers;
//! - the 10xer hints.
//!
//! A row runs the command registered under the row's id ([`crate::commands`]). A row whose
//! command belongs to a part this interface does not have yet is shown as the GTK menu shows
//! it at rest and does nothing: `commands::has` is false for its id.

use rune::{El, Tx};
use strata_components::{
    context_menu::{
        ContextMenu, ContextMenuState, Item, MenuIntent, MenuModel, MenuTarget, Section,
    },
    contract::{EntryId, Point, Rect},
};
use strata_core::{
    i18n::{count, tr},
    model::{ArchiveFormat, EntryKind, FileEntry, Location},
};
use strata_desktop::keys::{self, Command, Keys, Map, Shown, bindings as key};
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::{self, Action, Holds, On},
    rename, shell,
    slots::{Frame, Part, Place, Setup},
};

/// The one menu the window has open, and what it was opened on.
pub(crate) struct Menu {
    pub(crate) state: ContextMenuState,
    /// The column the menu is for.
    depth: usize,
    /// Index in `MenuModel::sections` where external file-provider sections are spliced.
    provider_slot: usize,
    /// What assistive technology hears the menu called: its entry or its folder.
    name: String,
}

/// A menu to open: what it is for, and what it lists.
struct Opening {
    name: String,
    target: MenuTarget,
    model: MenuModel,
    /// Where the custom actions go; none in a chooser's menus, as in GTK's.
    custom_slot: Option<usize>,
    provider_slot: usize,
    provider_paths: Option<Vec<std::path::PathBuf>>,
    background: bool,
}

/// Entry `id` in column `depth` of the shown view, and what a menu opened on it acts on: the
/// selection when the entry is in it, else the entry alone (`context_entries` in
/// `src/ui/browser/context_menu.rs`). A filter's result and the results selected come first,
/// as GTK asks `selected_search_results` before the browser.
pub(crate) fn context_entries(
    app: &App,
    depth: usize,
    id: EntryId,
) -> Option<(FileEntry, Vec<FileEntry>)> {
    let rows = &shell::shown_view(app)?.rows;
    let (entry, entries) = match crate::filter::targets::at(app, depth, id) {
        Some(entry) => {
            let entries = crate::filter::targets::of_row(app, depth, id).unwrap_or_default();
            (entry, entries)
        }
        None => {
            let position = (rows.position)(app, (depth, id))?;
            let entry = app.browser.entry_at(depth, position)?;
            // The selection when the entry is in it, else the entry alone.
            let selected = app.browser.selected_entries();
            let within = selected
                .iter()
                .any(|other| other.location == entry.location);
            let entries = if within {
                selected
            } else {
                vec![entry.clone()]
            };
            (entry, entries)
        }
    };
    Some((entry, entries))
}

/// Whether column `depth` lists a real folder where `New Folder`, `New File`, and `New Folder
/// with Selection` apply (`can_create_in_pane` in `src/ui/browser/context_menu/item_menu.rs`).
fn can_create_in_pane(app: &App, depth: usize, id: EntryId) -> bool {
    if crate::filter::targets::at(app, depth, id).is_some() {
        return false;
    }
    app.browser.location_at(depth).is_some_and(|location| {
        !crate::clipboard::paths::is_trash(&location) && !location.is_recent_location()
    })
}

fn chooser_entry_menu(app: &App, depth: usize, id: EntryId) -> Option<Opening> {
    let (entry, entries) = context_entries(app, depth, id)?;
    let single = entries.len() == 1;
    let typing = Hints::of(shell::preferences(app));
    let mut items = vec![RENAME.item(typing).enabled(single)];
    if single && crate::preview::entry_supports_quick_preview(&entry) {
        items.push(PREVIEW.item(typing));
    }
    items.push(PROPERTIES.item(typing));
    if entry.location.native_path().is_some() {
        items.push(COMPRESS.item(typing));
    }
    let in_trash = app
        .browser
        .location_at(depth)
        .as_ref()
        .is_some_and(crate::clipboard::paths::is_trash);
    let can_trash = app.browser.can_trash_at(depth) != Some(false);
    let can_delete = app.browser.can_delete_at(depth) != Some(false);
    if !in_trash && can_trash {
        items.push(TRASH.item(typing));
    }
    if can_delete {
        items.push(DELETE.item(typing).danger(true));
    }
    Some(Opening {
        name: entry.display_name,
        target: MenuTarget::Entry(id),
        model: MenuModel::new().section(Section::new(items)),
        custom_slot: None,
        provider_slot: 0,
        provider_paths: None,
        background: false,
    })
}

fn chooser_folder_menu(app: &App, depth: usize) -> Option<Opening> {
    let location = app.browser.location_at(depth)?;
    let item = NEW_FOLDER
        .item(Hints::of(shell::preferences(app)))
        .enabled(app.chooser.allows_create());
    Some(Opening {
        name: location.display_name(),
        target: MenuTarget::Background,
        model: MenuModel::new().section(Section::new([item])),
        custom_slot: None,
        provider_slot: 0,
        provider_paths: None,
        background: true,
    })
}

/// The menu of entry `id` in column `depth` of the shown view, or of the selection it is
/// part of.
fn entry_menu(app: &App, depth: usize, id: EntryId) -> Option<Opening> {
    if app.browser.is_chooser_mode() {
        return chooser_entry_menu(app, depth, id);
    }
    let (entry, entries) = context_entries(app, depth, id)?;
    let can_create = can_create_in_pane(app, depth, id);
    let model = self::entry(&EntryTarget {
        entry: &entry,
        entries: &entries,
        can_create,
        can_trash: app.browser.can_trash_at(depth),
        can_delete: app.browser.can_delete_at(depth),
        typing: Hints::of(shell::preferences(app)),
    });
    let has_native_parent = app
        .browser
        .location_at(depth)
        .is_some_and(|loc| loc.native_path().is_some());
    let provider_paths = has_native_parent
        .then(|| {
            entries
                .iter()
                .map(|e| e.location.native_path().map(std::path::Path::to_path_buf))
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    let custom_slot = if can_create { 3 } else { 1 };
    Some(Opening {
        name: entry.display_name,
        target: MenuTarget::Entry(id),
        model,
        custom_slot: Some(custom_slot),
        provider_slot: custom_slot + 1,
        provider_paths,
        background: false,
    })
}

/// The menu of column `depth`'s folder: a press on its empty space or its header, or the
/// Menu key with no selected cursor.
fn folder_menu(app: &App, depth: usize) -> Option<Opening> {
    if app.browser.is_chooser_mode() {
        return chooser_folder_menu(app, depth);
    }
    // The name the pane has: `src/ui/browser/columns/append.rs`.
    let location = app.browser.location_at(depth)?;
    let name = location.display_name();
    let counts = app.browser.column_entry_counts(depth);
    let provider_paths = location.native_path().map(|p| vec![p.to_path_buf()]);
    let model = folder(&FolderTarget {
        has_entries: counts.is_some_and(|counts| counts.total > 0),
        can_paste: crate::clipboard::may_paste(app, &location),
        can_open_terminal: crate::terminal::can_open(&location),
        hidden_shown: app.browser.preferences().show_hidden,
        typing: Hints::of(shell::preferences(app)),
    });
    Some(Opening {
        name,
        target: MenuTarget::Background,
        model,
        custom_slot: Some(2),
        provider_slot: 3,
        provider_paths,
        background: true,
    })
}

/// A menu asked for, in any view: by a secondary press, or by the Menu key.
pub(crate) struct Request {
    /// The browser's column the menu is for.
    pub(crate) depth: usize,
    /// The row's entry, or `None` for the column's folder.
    pub(crate) entry: Option<EntryId>,
    /// Where it opens, in window points.
    pub(crate) anchor: Rect,
}

/// Opens the context menu of a row of the shown view, or of a column's folder. A view's
/// handler calls it for the collection's `Intent::ContextMenu`, and for whatever else of its
/// own opens a folder's menu.
///
/// A press with any button outside the field commits an open rename in the GTK view. The
/// press that opens a menu leaves Rune's field the keyboard, so the edit is ended here.
pub(crate) fn open(app: &mut App, request: Request, tx: &mut Tx) {
    let Request {
        depth,
        entry,
        anchor,
    } = request;
    rename::commit(app, tx);
    // The menu acts on the browser's selection, which takes what the view changed first.
    shell::publish(app);
    if app.browser.location_at(depth).is_some() && app.browser.active_depth() != Some(depth) {
        app.browser.set_active_column(depth);
    }
    // `focus_context_entry` / `focus_context_column` (`src/ui/browser/context_menu/focus.rs`):
    // give the shown view the keyboard before the menu mounts so dismissing the menu — or a
    // dialog opened from it — restores focus to the menu's column rather than leaving it in
    // the sidebar or header, except on a folder/background menu while `depth` shows filter
    // results, where a focused filter field keeps focus across dismissal.
    let search_active = entry.is_none()
        && crate::filter::targets::selected_with_depth(app)
            .is_some_and(|(_, showing)| showing == depth);
    if !search_active && let Some(view) = shell::view_focus(app) {
        tx.focus(view, tx.by_keys());
    }
    let opening = match entry {
        Some(id) => entry_menu(app, depth, id),
        None => folder_menu(app, depth),
    };
    if let Some(mut opening) = opening {
        fill_custom_slot(app, &mut opening, depth, entry);
        let sections = crate::file_providers::begin_menu(
            app,
            opening.provider_paths.take(),
            opening.background,
            tx,
        );
        crate::file_providers::menus::splice_provider_sections(
            &mut opening.model,
            opening.provider_slot,
            sections,
        );
        app.menu
            .show(depth, opening, Point::new(anchor.x, anchor.y));
        crate::open_with::prepare_menu(app, depth, entry);
    }
}

/// Puts the custom actions that match into the menu's slot for them, when it has one.
fn fill_custom_slot(app: &mut App, opening: &mut Opening, depth: usize, entry: Option<EntryId>) {
    if let Some(slot) = opening.custom_slot {
        opening.model.sections[slot] = crate::custom_actions::begin_menu(app, depth, entry);
    }
}

/// A row of the open menu was chosen. The menu has closed, and the keyboard is back in the
/// view. The row runs the command registered under its id (`commands::run`), on the row's
/// entry or on the column's folder: what the command does is decided once, where a key and
/// a button ask for it too. A row no part has a command for does nothing.
fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    if crate::custom_actions::is_menu_action(&action) {
        crate::custom_actions::activate_menu_action(app, &action);
        return;
    }
    if crate::file_providers::menus::is_provider_action(&action) {
        crate::file_providers::activate_menu_action(app, &action, tx);
        return;
    }
    let depth = app.menu.depth();
    let on = match target.entry() {
        Some(id) => On::Row((depth, id)),
        None => On::Column(depth),
    };
    commands::run(app, &action, on, tx);
}

/// Matches GTK's root context-menu width floor (`style_menu_scroll` in
/// `src/ui/browser/context_menu/actions/presentation.rs`): keeps entry and folder menus at a
/// consistent width across targets and async lookups.
const MENU_MIN_WIDTH: f32 = 310.0;

fn min_width(look: &strata_kit::look::Look) -> f32 {
    let scale = strata_kit::typography::factor(look.kit.ty.ui_compact.size);
    (MENU_MIN_WIDTH * scale).round()
}

/// The menu's element, with its surfaces in the layer while it is open.
fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let mut menu = ContextMenu::new(
        "menu",
        |app: &App| &app.menu.state,
        |app: &mut App| &mut app.menu.state,
    )
    .name(app.menu.name())
    .on_intent(chosen);
    if !app.browser.is_chooser_mode() {
        menu = menu.min_width(min_width(&app.look));
    }
    Some(menu.build(app, &app.look, frame.layer))
}

impl Part for Menu {
    const PLACE: Place<App> = Place::Overlays(build);

    const COMMANDS: &'static [Action<App>] = crate::window_keys::COMMANDS;

    fn keyboard(app: &App) -> Option<Holds> {
        app.menu.state.is_open().then_some(Holds::Menu)
    }

    fn new(_: &Setup<'_>) -> Self {
        Self {
            state: ContextMenuState::default(),
            depth: 0,
            provider_slot: 0,
            name: String::new(),
        }
    }
}

impl Menu {
    fn show(&mut self, depth: usize, opening: Opening, at: Point) {
        self.depth = depth;
        self.provider_slot = opening.provider_slot;
        self.name = opening.name;
        self.state.open(opening.target, at, opening.model);
    }

    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    pub(crate) fn provider_slot(&self) -> usize {
        self.provider_slot
    }

    /// The Open row, with the hint `app`'s keymap shows.
    pub(crate) fn open_item(app: &App) -> Item {
        OPEN.item(Hints::of(shell::preferences(app)))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Which map's hints a menu shows, as the keymap in force when it opens says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hints {
    /// "Type to search" takes plain keys, so a row hides a hint that is one
    /// (`type_to_search_active` in `src/ui/preferences/input_settings.rs`).
    typing: bool,
    /// 10xer mode's hints (`tenxer_hint` in `src/ui/shortcut_reference.rs`).
    tenxer: bool,
}

impl Hints {
    pub(crate) fn of(settings: &strata_desktop::settings::Preferences) -> Self {
        Self {
            typing: settings.type_to_search && !settings.tenxer_mode,
            tenxer: settings.tenxer_mode,
        }
    }

    /// The default map's hints, with "Type to search" on or off.
    #[cfg(test)]
    pub(crate) const fn default_map(typing: bool) -> Self {
        Self {
            typing,
            tenxer: false,
        }
    }

    #[cfg(test)]
    pub(crate) const TENXER: Self = Self {
        typing: false,
        tenxer: true,
    };
}

/// What a row hints in 10xer mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TenxerHint {
    /// The default map's hint.
    Same,
    /// Another command's keys: 10xer's verb for the row.
    Keys(Shown),
    /// None: the row's command has no key of its own there.
    Hidden,
}

/// One row of a menu: its id, its label in the catalogs, its leading icon, and the catalog's
/// keys its hint shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) icon: IconName,
    /// What `default_hint` shows for the row, labelled from the keymap in force when the menu
    /// opens.
    hint: Option<Shown>,
    /// A command whose shipped keys may all type: "Type to search" then takes them, and the
    /// hint is hidden while it is on (`typed_only` in `default_hint`).
    typed: Option<Command>,
    tenxer: TenxerHint,
}

const fn row(id: &'static str, label: &'static str, icon: IconName) -> Row {
    Row {
        id,
        label,
        icon,
        hint: None,
        typed: None,
        tenxer: TenxerHint::Same,
    }
}

/// Whether every key the default map binds to `command` types.
fn types(command: Command) -> bool {
    let bindings = command.bindings_in(Map::Default);
    bindings.iter().all(|binding| binding.is_typed())
}

impl Row {
    const fn hint(mut self, keys: Shown) -> Self {
        self.hint = Some(keys);
        self
    }

    /// The hint of `command`, whose keys "Type to search" may take. 10xer mode shows none.
    const fn typed_hint(mut self, command: Command) -> Self {
        self.hint = Some(command.shown());
        self.typed = Some(command);
        self.tenxer = TenxerHint::Hidden;
        self
    }

    /// 10xer mode's hint for the row: `keys`.
    const fn tenxer_hint(mut self, keys: Shown) -> Self {
        self.tenxer = TenxerHint::Keys(keys);
        self
    }

    /// No hint in 10xer mode.
    const fn no_tenxer_hint(mut self) -> Self {
        self.tenxer = TenxerHint::Hidden;
        self
    }

    /// The keys the row hints with `hints`: 10xer's, or the default map's unless "Type to
    /// search" takes them.
    fn hinted(&self, hints: Hints) -> Option<Shown> {
        if !hints.tenxer {
            let taken = hints.typing && self.typed.is_some_and(types);
            return self.hint.filter(|_| !taken);
        }
        match self.tenxer {
            TenxerHint::Same => self.hint,
            TenxerHint::Keys(keys) => Some(keys),
            TenxerHint::Hidden => None,
        }
    }

    /// The row's hint as the window's keymap labels it: none when it has none, when "Type to
    /// search" takes its keys, or when nothing binds them.
    fn hint_label(&self, typing: Hints) -> Option<String> {
        let shown = self.hinted(typing)?;
        let label = keys::label(&Keys::One(shown));
        (!label.is_empty()).then_some(label)
    }

    fn item(&self, typing: Hints) -> Item {
        let item = Item::new(self.id, tr(self.label)).icon(self.icon);
        match self.hint_label(typing) {
            Some(label) => item.shortcut(label),
            None => item,
        }
    }
}

pub(crate) const NEW_FOLDER: Row =
    row("new-folder", "New Folder", IconName::FolderPlus).hint(key::NEW_FOLDER.shown());
pub(crate) const NEW_FILE: Row = row("new-file", "New File", IconName::FilePlus);
pub(crate) const GROUP: Row = row("group", "New Folder with Selection", IconName::FolderPlus);
pub(crate) const OPEN: Row = row("open", "Open", IconName::Open).hint(key::ENTER.shown());
pub(crate) const OPEN_WITH: Row = row("open-with", "Open With…", IconName::AppWindow);
pub(crate) const PREVIEW: Row =
    row("preview", "Quick preview", IconName::Eye).hint(key::PREVIEW.shown());
pub(crate) const RUN: Row = row("run", "Run", IconName::Play);
pub(crate) const TERMINAL: Row = row("terminal", "Open in Terminal", IconName::Terminal)
    .hint(key::TERMINAL.shown())
    .no_tenxer_hint();
pub(crate) const OPEN_IN: Row = row("open-in", "Open in…", IconName::Open);
pub(crate) const NEW_TAB: Row =
    row("open-in-tab", "New Tab", IconName::Plus).hint(key::OPEN_IN_NEW_TAB.shown());
pub(crate) const NEW_WINDOW: Row =
    row("open-in-window", "New Window", IconName::AppWindow).hint(key::OPEN_IN_NEW_WINDOW.shown());
pub(crate) const EXTRACT: Row = row("extract", "Extract here", IconName::Extract);
pub(crate) const EXTRACT_TO: Row = row("extract-to", "Extract to…", IconName::FolderArchive);
pub(crate) const CUT: Row = row("cut", "Cut", IconName::Cut)
    .hint(key::CUT.shown())
    .tenxer_hint(key::YANK_CUT.shown());
pub(crate) const COPY: Row = row("copy", "Copy", IconName::Copy)
    .hint(key::COPY.shown())
    .tenxer_hint(key::YANK.shown());
pub(crate) const DUPLICATE: Row = row("duplicate", "Duplicate", IconName::CopyPlus)
    .hint(key::DUPLICATE.shown())
    .no_tenxer_hint();
pub(crate) const RENAME: Row = row("rename", "Rename", IconName::Rename)
    .hint(key::RENAME.shown())
    .tenxer_hint(key::RENAME.alternate(0).shown());
pub(crate) const MOVE_TO: Row =
    row("move-to", "Move to…", IconName::FolderInput).tenxer_hint(key::MOVE_TO.shown());
pub(crate) const COPY_TO: Row =
    row("copy-to", "Copy to…", IconName::FolderOutput).tenxer_hint(key::COPY_TO.shown());
pub(crate) const COMPRESS: Row = row("compress", "Compress…", IconName::PackagePlus);
pub(crate) const PIN: Row = row("pin", "Pin to sidebar", IconName::Pin).typed_hint(key::PIN);
pub(crate) const CUSTOMIZE: Row = row("customize", "Customize…", IconName::Palette);
pub(crate) const COPY_PATH: Row =
    row("copy-path", "Copy path", IconName::Route).typed_hint(key::COPY_PATH);
pub(crate) const COPY_NAME: Row = row("copy-name", "Copy name", IconName::FileType);
pub(crate) const COPY_PATHS: Row =
    row("copy-paths", "Copy paths", IconName::Route).typed_hint(key::COPY_PATH);
pub(crate) const COPY_NAMES: Row = row("copy-names", "Copy names", IconName::FileType);
pub(crate) const PROPERTIES: Row =
    row("properties", "Properties", IconName::Properties).hint(key::PROPERTIES.shown());
/// The background menu's Properties shows no hint (`ContextHint::None` in `folder_menu.rs`).
pub(crate) const FOLDER_PROPERTIES: Row = row("properties", "Properties", IconName::Properties);
pub(crate) const TRASH: Row = row("trash", "Move to Trash", IconName::Trash)
    .hint(key::MOVE_TO_TRASH.shown())
    .tenxer_hint(key::MOVE_TO_TRASH.alternate(0).shown());
pub(crate) const DELETE: Row = row("delete", "Permanently delete", IconName::CircleX)
    .hint(key::DELETE_PERMANENTLY.shown())
    .tenxer_hint(key::DELETE_PERMANENTLY.alternate(0).shown());
pub(crate) const PASTE: Row = row("paste", "Paste", IconName::Paste)
    .hint(key::PASTE.shown())
    .tenxer_hint(key::PASTE_KEEP_BOTH.shown());
pub(crate) const SELECT_ALL: Row =
    row("select-all", "Select All", IconName::ListChecks).hint(key::SELECT_ALL.shown());
pub(crate) const REFRESH: Row =
    row("refresh", "Refresh", IconName::Refresh).hint(key::REFRESH.shown());
/// The hidden-files rows show the primary key alone: `HIDDEN_FILES.alternate(0)`.
pub(crate) const SHOW_HIDDEN: Row = row("hidden", "Show Hidden Files", IconName::EyeOff)
    .hint(key::HIDDEN_FILES.alternate(0).shown());
pub(crate) const HIDE_HIDDEN: Row =
    row("hidden", "Hide Hidden Files", IconName::Eye).hint(key::HIDDEN_FILES.alternate(0).shown());

/// Every row either menu can show, for the tests of their words and of which have a
/// command. The two hidden-files rows share an id, as the two Properties rows do.
#[cfg(test)]
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

/// What an entry's menu is built from.
pub(crate) struct EntryTarget<'a> {
    /// The entry under the pointer.
    pub(crate) entry: &'a FileEntry,
    /// What the menu acts on: the selection when the entry is in it, else the entry alone
    /// (`context_entries` in `src/ui/browser/context_menu.rs`).
    pub(crate) entries: &'a [FileEntry],
    /// Whether plain creation and grouping apply in the pane (`can_create_in_pane`).
    pub(crate) can_create: bool,
    /// Whether the pane's folder can be trashed from and deleted from, where GIO says.
    pub(crate) can_trash: Option<bool>,
    pub(crate) can_delete: Option<bool>,
    /// "Type to search" takes plain keys, so their hints are hidden; 10xer mode has its own.
    pub(crate) typing: Hints,
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

fn section(rows: &[(Row, bool)], typing: Hints) -> Section {
    let shown = rows.iter().filter(|(_, shown)| *shown);
    Section::new(shown.map(|(row, _)| row.item(typing)))
}

fn open_in(typing: Hints) -> Item {
    let children = Section::new([NEW_TAB.item(typing), NEW_WINDOW.item(typing)]);
    Item::submenu(OPEN_IN.id, tr(OPEN_IN.label), [children]).icon(OPEN_IN.icon)
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

fn creation(typing: Hints) -> Section {
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
            (RUN, crate::run::runs(entry)),
            (
                TERMINAL,
                folder && crate::terminal::can_open(&entry.location),
            ),
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
        .section(Section::default())
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
    // GTK shows Open here only once its open-with lookup finds a default application the
    // entries share (`prepare_open_with`); this interface has no lookup yet, and shows what
    // GTK shows when it finds none (finding lgse/strata#2358).
    let mut open = section(&[(OPEN_WITH, true)], typing);
    if target.entries.iter().any(FileEntry::is_directory) {
        open.items.push(open_in(typing));
    }
    MenuModel::new()
        .header_line(heading)
        .header_line(summary(target.entries))
        .section(creation(typing))
        .section(section(&[(GROUP, true)], typing))
        .section(open)
        .section(Section::default())
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
    let mut model = if target.entries.len() > 1 {
        several(target)
    } else {
        single(target)
    };
    if !target.can_create {
        model.sections.retain(|section| {
            !section
                .items
                .iter()
                .any(|item| matches!(&*item.action, "new-folder" | "new-file" | "group"))
        });
    }
    model
}

/// What a pane's background menu is built from.
pub(crate) struct FolderTarget {
    /// The folder lists something: Select All has entries to select.
    pub(crate) has_entries: bool,
    pub(crate) can_paste: bool,
    pub(crate) can_open_terminal: bool,
    pub(crate) hidden_shown: bool,
    pub(crate) typing: Hints,
}

/// A pane's background menu: `FolderMenuOptions`. Paste follows whether the clipboard offers
/// files and the folder accepts pasted items; Open in Terminal follows `can_open_terminal`.
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
            PASTE.item(typing).enabled(target.can_paste),
        ]))
        .section(Section::new([
            OPEN_WITH.item(typing),
            TERMINAL.item(typing).enabled(target.can_open_terminal),
        ]))
        .section(Section::default())
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
