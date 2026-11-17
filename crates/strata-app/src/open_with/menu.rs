// SPDX-License-Identifier: MIT

//! Context-menu MIME lookup (`src/ui/browser/context_menu/open_with.rs`) and keyboard
//! `open-with` target resolution (`src/ui/browser/file_commands.rs`).

use std::{cell::Cell, rc::Rc};

use rune::Tx;
use strata_components::{context_menu::MenuTarget, contract::EntryId};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::adapters::gio_file_for_location;

use super::{
    ChooserSpec, OpenWithContext,
    apps::{Applications, Choices, choices_for_type, requires_uri_handlers, resolve, uses_portal},
    launch::{self, ADMINISTRATOR_OPEN_REFUSAL, LaunchBatch},
    recent,
};
use crate::{
    App, clipboard,
    commands::{self, Call, On, Outcome},
    error_dialog::{self, Error},
    menus, shell,
};

#[derive(Clone)]
pub(super) struct MenuSelection {
    pub(super) depth: usize,
    pub(super) id: EntryId,
    pub(super) locations: Vec<Location>,
    pub(super) files: Vec<gio::File>,
    pub(super) content_types: Vec<String>,
    pub(super) choices: Choices,
    pub(super) default: Option<gio::AppInfo>,
}

impl MenuSelection {
    fn matches(&self, depth: usize, id: EntryId, entries: &[FileEntry]) -> bool {
        self.depth == depth && self.id == id && entries_match_locations(entries, &self.locations)
    }
}

fn entries_match_locations(entries: &[FileEntry], locations: &[Location]) -> bool {
    entries.len() == locations.len()
        && entries
            .iter()
            .zip(locations)
            .all(|(entry, loc)| clipboard::paths::locations_equal(&entry.location, loc))
}

#[derive(Clone)]
pub(super) struct FolderSelection {
    pub(super) depth: usize,
    pub(super) location: Location,
    pub(super) history: recent::State,
}

fn supports_folder_open_with(location: &Location) -> bool {
    ![
        clipboard::paths::is_trash(location),
        location.is_recent_location(),
        location.is_administrator(),
    ]
    .into_iter()
    .any(|excluded| excluded)
}

/// Starts the asynchronous Open With lookup for a newly opened context menu.
pub(crate) fn prepare_menu(app: &mut App, depth: usize, entry: Option<EntryId>) {
    if !commands::has(app, "open-with") {
        return;
    }
    app.open_with.menu_selection = None;
    app.open_with.folder_selection = None;
    let expected = app.open_with.bump_menu_generation();
    match entry {
        Some(id) => prepare_entry_menu(app, depth, id, expected),
        None => prepare_folder_menu(app, depth, expected),
    }
}

fn prepare_entry_menu(app: &mut App, depth: usize, id: EntryId, expected: u64) {
    let Some((_, entries)) = menus::context_entries(app, depth, id) else {
        return;
    };
    if entries.is_empty() {
        return;
    }
    update_menu_open_with(
        app,
        false,
        Some(tr("Looking for compatible applications…")),
        false,
    );
    let multiple = entries.len() > 1;
    let locations: Vec<Location> = entries.iter().map(|entry| entry.location.clone()).collect();
    let files: Vec<gio::File> = locations.iter().map(gio_file_for_location).collect();
    let generation = app.open_with.menu_generation.clone();
    let later = app.open_with.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let Some(resolved) = resolve(&files, || generation.get() == expected).await else {
            return;
        };
        later.then(move |app, tx| {
            apply_entry_lookup(
                app,
                EntryLookup {
                    depth,
                    id,
                    expected,
                    multiple,
                    locations,
                    files,
                },
                resolved,
            );
            shell::settle(app, tx);
            true
        });
    });
}

struct EntryLookup {
    depth: usize,
    id: EntryId,
    expected: u64,
    multiple: bool,
    locations: Vec<Location>,
    files: Vec<gio::File>,
}

fn apply_entry_lookup(
    app: &mut App,
    lookup: EntryLookup,
    resolved: Result<Applications, &'static str>,
) {
    if !menu_still_targets(
        app,
        lookup.expected,
        lookup.depth,
        MenuTarget::Entry(lookup.id),
    ) {
        return;
    }
    let applications = match resolved {
        Ok(applications) => applications,
        Err(reason) => {
            update_menu_open_with(app, false, Some(tr(reason)), false);
            return;
        }
    };
    let explanation = applications.unavailable_reason().map(tr);
    let add_open = lookup.multiple && (applications.default.is_some() || uses_portal());
    app.open_with.menu_selection = Some(MenuSelection {
        depth: lookup.depth,
        id: lookup.id,
        locations: lookup.locations,
        files: lookup.files,
        content_types: applications.content_types,
        choices: applications.choices,
        default: applications.default,
    });
    update_menu_open_with(app, explanation.is_none(), explanation, add_open);
}

fn prepare_folder_menu(app: &mut App, depth: usize, expected: u64) {
    let Some(location) = app
        .browser
        .location_at(depth)
        .filter(supports_folder_open_with)
    else {
        return;
    };
    update_menu_open_with(
        app,
        false,
        Some(tr("Looking for compatible applications…")),
        false,
    );
    let generation = app.open_with.menu_generation.clone();
    let later = app.open_with.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let history = recent::read().await;
        if generation.get() != expected {
            return;
        }
        later.then(move |app, tx| {
            if !menu_still_targets(app, expected, depth, MenuTarget::Background) {
                return false;
            }
            app.open_with.folder_selection = Some(FolderSelection {
                depth,
                location,
                history,
            });
            update_menu_open_with(app, true, None, false);
            shell::settle(app, tx);
            true
        });
    });
}

fn menu_still_targets(app: &App, expected: u64, depth: usize, target: MenuTarget) -> bool {
    [
        app.open_with.menu_generation.get() == expected,
        app.menu.state.is_open(),
        app.menu.depth() == depth,
        app.menu.state.target() == Some(&target),
    ]
    .into_iter()
    .all(|holds| holds)
}

fn update_menu_open_with(
    app: &mut App,
    enabled: bool,
    description: Option<String>,
    add_open: bool,
) {
    let Some(mut model) = app.menu.state.model().cloned() else {
        return;
    };
    let open_item = add_open.then(|| app.menu.open_item());
    for section in &mut model.sections {
        let Some(pos) = section
            .items
            .iter()
            .position(|item| &*item.action == "open-with")
        else {
            continue;
        };
        section.items[pos].enabled = enabled;
        section.items[pos].description = description;
        if let Some(open) = open_item
            && !section.items.iter().any(|item| &*item.action == "open")
        {
            section.items.insert(pos, open);
        }
        break;
    }
    app.menu.state.update_model(model);
}

/// Launches the shared default application when `Open` is chosen on a multi-selection context
/// menu (`open_with_default_app` in `src/ui/browser/context_menu/item_menu.rs`).
pub(crate) fn open_selection_default(app: &mut App, depth: usize, id: EntryId) -> bool {
    let Some((_, entries)) = menus::context_entries(app, depth, id) else {
        return false;
    };
    if entries.len() <= 1 {
        return false;
    }
    let Some(selection) = app
        .open_with
        .menu_selection
        .take()
        .filter(|selection| selection.matches(depth, id, &entries))
    else {
        return true;
    };
    launch_selected_default(app, selection);
    true
}

fn launch_selected_default(app: &mut App, selection: MenuSelection) {
    if uses_portal() {
        launch::launch_with_portal(selection.files, app.open_with.later.clone());
        return;
    }
    let Some(default) = selection.default else {
        return;
    };
    let context = launch::launch_context();
    if let Err(error) = launch::launch(
        &default,
        LaunchBatch {
            files: &selection.files,
            content_types: &selection.content_types,
        },
        Some(&context),
    ) {
        error_dialog::show(
            app,
            Error::new(tr("Unable to open file"), error.message().to_owned()),
        );
    }
}

pub(super) fn command(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    match call.on {
        On::Row((depth, id)) => {
            open_row_menu(app, depth, id, tx);
            Outcome::Ran
        }
        On::Column(depth) => {
            open_folder_menu(app, depth);
            Outcome::Ran
        }
        On::Focus => {
            open_targets(app, tx, None);
            Outcome::Ran
        }
        On::Place(_) => Outcome::Declined,
    }
}

fn open_row_menu(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
    let Some((_, entries)) = menus::context_entries(app, depth, id) else {
        return;
    };
    if entries
        .iter()
        .any(|entry| entry.location.is_administrator())
    {
        error_dialog::show(
            app,
            Error::new(tr("Unable to open item"), tr(ADMINISTRATOR_OPEN_REFUSAL)),
        );
        return;
    }
    if let Some(selection) = app
        .open_with
        .menu_selection
        .take()
        .filter(|selection| selection.matches(depth, id, &entries))
    {
        super::show_chooser(
            app,
            ChooserSpec {
                files: selection.files,
                content_types: selection.content_types,
                choices: selection.choices,
                context: OpenWithContext::Explicit,
            },
        );
        return;
    }
    resolve_and_open(app, entries, tx, None);
}

fn open_folder_menu(app: &mut App, depth: usize) {
    let Some(location) = app
        .browser
        .location_at(depth)
        .filter(supports_folder_open_with)
    else {
        return;
    };
    let Some(selection) = app.open_with.folder_selection.take().filter(|sel| {
        sel.depth == depth && clipboard::paths::locations_equal(&sel.location, &location)
    }) else {
        return;
    };
    let file = gio_file_for_location(&location);
    let requires_uris = requires_uri_handlers(std::slice::from_ref(&file));
    let choices = choices_for_type(&selection.history, "inode/directory", requires_uris);
    super::show_chooser(
        app,
        ChooserSpec {
            files: vec![file],
            content_types: vec!["inode/directory".to_owned()],
            choices,
            context: OpenWithContext::Explicit,
        },
    );
}

pub(crate) type RefusalReport = Rc<dyn Fn(&mut App, &str, &mut Tx)>;

/// Resolves the current focus targets (`command_targets`) and opens the Open With chooser
/// (`BrowserView::open_targets_with`). `report` receives untranslated refusal reasons when a
/// caller such as 10xer displays shortcut feedback.
pub(crate) fn open_targets(app: &mut App, tx: &mut Tx, report: Option<RefusalReport>) -> bool {
    let entries = focus_entries(app);
    if entries.is_empty() {
        if let Some(report) = report {
            report(app, "Nothing to open", tx);
        }
        return false;
    }
    resolve_and_open(app, entries, tx, report);
    true
}

fn resolve_and_open(
    app: &mut App,
    entries: Vec<FileEntry>,
    tx: &mut Tx,
    report: Option<RefusalReport>,
) {
    let locations: Vec<Location> = entries.iter().map(|entry| entry.location.clone()).collect();
    if locations.iter().any(Location::is_administrator) {
        if let Some(report) = report {
            report(app, ADMINISTRATOR_OPEN_REFUSAL, tx);
        }
        return;
    }
    let files: Vec<gio::File> = locations.iter().map(gio_file_for_location).collect();
    let folder = app.browser.active_location();
    let focus = tx.focused().map(str::to_owned);
    let expected = app.open_with.bump_target_generation();
    let generation: Rc<Cell<u64>> = app.open_with.target_generation.clone();
    let later = app.open_with.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let Some(resolved) = resolve(&files, || generation.get() == expected).await else {
            return;
        };
        later.then(move |app, tx| {
            finish_target_lookup(
                app,
                tx,
                TargetGuard {
                    expected,
                    folder,
                    focus,
                    locations,
                    files,
                    report,
                },
                resolved,
            )
        });
    });
}

struct TargetGuard {
    expected: u64,
    folder: Option<Location>,
    focus: Option<String>,
    locations: Vec<Location>,
    files: Vec<gio::File>,
    report: Option<RefusalReport>,
}

fn target_guard_holds(app: &mut App, tx: &Tx, guard: &TargetGuard) -> bool {
    let intact = [
        app.open_with.target_generation.get() == guard.expected,
        !super::foreign_modal_open(app),
        tx.focused() == guard.focus.as_deref(),
        app.browser.active_location() == guard.folder,
    ]
    .into_iter()
    .all(|holds| holds);
    intact && entries_match_locations(&focus_entries(app), &guard.locations)
}

fn report_refusal(
    app: &mut App,
    report: Option<&RefusalReport>,
    reason: &str,
    tx: &mut Tx,
) -> bool {
    let Some(report) = report else {
        return false;
    };
    report(app, reason, tx);
    true
}

fn finish_target_lookup(
    app: &mut App,
    tx: &mut Tx,
    guard: TargetGuard,
    resolved: Result<Applications, &'static str>,
) -> bool {
    if !target_guard_holds(app, tx, &guard) {
        return false;
    }
    let applications = match resolved {
        Ok(applications) => applications,
        Err(reason) => return report_refusal(app, guard.report.as_ref(), reason, tx),
    };
    if let Some(reason) = applications.unavailable_reason() {
        return report_refusal(app, guard.report.as_ref(), reason, tx);
    }
    super::show_chooser(
        app,
        ChooserSpec {
            files: guard.files,
            content_types: applications.content_types,
            choices: applications.choices,
            context: OpenWithContext::Explicit,
        },
    );
    shell::settle(app, tx);
    true
}

pub(crate) fn cancel_target_lookup(app: &mut App) {
    app.open_with.bump_target_generation();
}

fn focus_entries(app: &mut App) -> Vec<FileEntry> {
    crate::tenxer::command_targets(app)
}
