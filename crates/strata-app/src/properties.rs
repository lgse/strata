// SPDX-License-Identifier: MIT

mod actions;
mod measure;
mod media;
mod permissions;
#[cfg(test)]
mod tests;
mod view;

use std::path::Path;

use gio::prelude::*;
use rune::Tx;
use strata_core::{
    i18n::{count, file_size, tr},
    model::{ArchiveFormat, FileEntry, Location},
    util::{DateDisplay, modified_date_for_seconds},
};
use strata_desktop::keys::{Modifiers, bindings::PROPERTIES};
use strata_fs::{
    adapters::{
        CONTENT_TYPE_ATTRIBUTES, content_type_of, directory_summary::DirectorySummary,
        gio_file_for_location, info_is_hidden,
    },
    services::refine_reported_content_type,
};
use strata_kit::icons::IconName;

use crate::{
    App,
    clipboard::paths::is_trash_root,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    filter, menus,
    slots::{Later, Part, Place, Setup},
};

#[derive(Default)]
struct Tasks {
    info: Option<glib::JoinHandle<()>>,
    measure: Option<glib::JoinHandle<()>>,
    media: media::ActiveLoad,
}

impl Tasks {
    fn abort_all(&mut self) {
        if let Some(handle) = self.info.take() {
            handle.abort();
        }
        if let Some(handle) = self.measure.take() {
            handle.abort();
        }
        self.media.abort();
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        self.abort_all();
    }
}

struct Single {
    generation: u64,
    location: Location,
    entry: Option<FileEntry>,
    rename_depth: Option<usize>,
    is_directory: bool,
    name: String,
    subtitle: String,
    icon: IconName,
    size: String,
    spinning: bool,
    contains: Option<String>,
    warning: Option<String>,
    modified: String,
    opens_with: String,
    hidden: String,
    copied: bool,
    media: media::Section,
    permissions: permissions::Editor,
}

struct Selection {
    generation: u64,
    title: String,
    size: String,
    spinning: bool,
    contains: String,
    warning: Option<String>,
}

enum Dialog {
    Single(Box<Single>),
    Selection(Selection),
}

struct Notice {
    title: String,
    message: String,
    generation: u64,
}

pub(crate) struct Properties {
    later: Later<App>,
    dialog: Option<Dialog>,
    open: bool,
    opened: u64,
    notice: Option<Notice>,
    notice_open: bool,
    notices: u64,
    tasks: Tasks,
}

impl Part for Properties {
    const PLACE: Place<App> = Place::Dialogs(view::build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            dialog: None,
            open: false,
            opened: 0,
            notice: None,
            notice_open: false,
            notices: 0,
            tasks: Tasks::default(),
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        (app.properties.open || app.properties.notice_open).then_some(Holds::Modal { field: false })
    }

    fn tab_hidden(app: &mut App, _tx: &mut Tx) {
        dismiss(app);
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("properties", command)
        .command(PROPERTIES)
        .keys(&[Keys::at(
            Stage::PropertiesCommand,
            &[PROPERTIES.ignoring(Modifiers::SUPER_MASK)],
        )
        .when(|_, at| !at.text())])];
}

fn dismiss(app: &mut App) {
    app.properties.open = false;
    app.properties.notice_open = false;
    app.properties.tasks.abort_all();
}

fn close_notice(app: &mut App) {
    app.properties.notice_open = false;
}

fn show_notice(app: &mut App, title: String, message: String) {
    let state = &mut app.properties;
    state.notices += 1;
    state.notice = Some(Notice {
        title,
        message,
        generation: state.notices,
    });
    state.notice_open = true;
}

fn command(app: &mut App, call: &Call<'_>, _tx: &mut Tx) -> Outcome {
    match &call.on {
        On::Focus => show_focused(app),
        On::Row((depth, id)) => {
            let Some((entry, entries)) = menus::context_entries(app, *depth, *id) else {
                return Outcome::Declined;
            };
            if entries.len() > 1 {
                show_selection(app, entries);
            } else {
                show_entry(app, entry, Some(*depth));
            }
            Outcome::Ran
        }
        On::Column(depth) => {
            let Some(location) = app.browser.location_at(*depth) else {
                return Outcome::Declined;
            };
            show_location(app, location, Some(*depth));
            Outcome::Ran
        }
        On::Place(location) => {
            show_location(app, location.clone(), None);
            Outcome::Ran
        }
    }
}

fn show_focused(app: &mut App) -> Outcome {
    if let Some((selected, depth)) = filter::targets::selected_with_depth(app) {
        if selected.is_empty() {
            return Outcome::Declined;
        }
        if selected.len() == 1 {
            show_entry(app, selected.into_iter().next().expect("one"), Some(depth));
        } else {
            show_selection(app, selected);
        }
        return Outcome::Ran;
    }
    let selected = app.browser.selected_entries();
    if selected.len() > 1 {
        show_selection(app, selected);
        return Outcome::Ran;
    }
    let Some(entry) = app.browser.focused_entry() else {
        return Outcome::Declined;
    };
    let depth = app.browser.focused_item().map(|(depth, _, _)| depth);
    show_entry(app, entry, depth);
    Outcome::Ran
}

/// The location whose Properties are open, for a test of who opens them.
#[cfg(test)]
pub(crate) fn shown_location(app: &App) -> Option<&Location> {
    match (&app.properties.dialog, app.properties.open) {
        (Some(Dialog::Single(single)), true) => Some(&single.location),
        _ => None,
    }
}

/// Opens Properties for `location` with no row entry (`show_folder_properties`).
pub(crate) fn show_location(app: &mut App, location: Location, rename_depth: Option<usize>) {
    if location.is_recent_location() {
        return;
    }
    open_single(app, location, None, rename_depth);
}

/// Opens Properties for a single `entry` (`show_entry_properties` / `show_entry_properties_at`).
pub(crate) fn show_entry(app: &mut App, entry: FileEntry, rename_depth: Option<usize>) {
    let location = entry.location.clone();
    open_single(app, location, Some(entry), rename_depth);
}

/// Opens Selection Properties for two or more `entries`, or Single Properties when one is passed.
pub(crate) fn show_selection(app: &mut App, entries: Vec<FileEntry>) {
    if entries.len() < 2 {
        if let Some(entry) = entries.into_iter().next() {
            show_entry(app, entry, None);
        }
        return;
    }
    let state = &mut app.properties;
    state.tasks.abort_all();
    state.notice_open = false;
    state.opened += 1;
    let generation = state.opened;
    let title =
        rust_i18n::t!("%{items} selected", items = count("items", entries.len())).into_owned();
    state.dialog = Some(Dialog::Selection(Selection {
        generation,
        title,
        size: file_size(0),
        spinning: true,
        contains: tr("Calculating…"),
        warning: None,
    }));
    state.open = true;
    state.tasks.measure = Some(measure::spawn_selection(
        state.later.clone(),
        generation,
        entries,
    ));
}

fn open_single(
    app: &mut App,
    location: Location,
    entry: Option<FileEntry>,
    rename_depth: Option<usize>,
) {
    let is_directory = entry.as_ref().is_none_or(FileEntry::is_directory);
    let measuring = is_directory || is_trash_root(&location);
    let name = entry.as_ref().map_or_else(
        || location.display_name(),
        |entry| entry.display_name.clone(),
    );
    let initial_size = initial_size_text(entry.as_ref(), measuring);
    let modified = initial_modified_text(entry.as_ref());
    let icon = entry_icon(entry.as_ref());
    let state = &mut app.properties;
    state.tasks.abort_all();
    state.notice_open = false;
    state.opened += 1;
    let generation = state.opened;
    let (media_section, media_load) = media::start(
        entry.as_ref(),
        is_directory,
        generation,
        state.later.clone(),
    );
    state.dialog = Some(Dialog::Single(Box::new(Single {
        generation,
        location: location.clone(),
        entry,
        rename_depth,
        is_directory,
        subtitle: tr(if is_directory { "Folder" } else { "File" }),
        icon,
        size: initial_size,
        spinning: measuring,
        contains: measuring.then(|| measure::directory_counts_label(&DirectorySummary::default())),
        warning: None,
        modified,
        opens_with: "—".to_owned(),
        hidden: tr(if name.starts_with('.') { "Yes" } else { "No" }),
        name,
        copied: false,
        media: media_section,
        permissions: permissions::Editor::default(),
    })));
    state.open = true;
    state.tasks.media = media_load;
    if measuring {
        state.tasks.measure = Some(measure::spawn_folder(
            state.later.clone(),
            generation,
            location.clone(),
        ));
    }
    state.tasks.info = Some(spawn_info_query(state.later.clone(), generation, location));
}

fn initial_size_text(entry: Option<&FileEntry>, measuring: bool) -> String {
    if measuring {
        return file_size(0);
    }
    match entry.and_then(|entry| crate::listing::known(&entry.size)) {
        Some(bytes) => file_size(bytes),
        None => "—".to_owned(),
    }
}

fn initial_modified_text(entry: Option<&FileEntry>) -> String {
    match entry.and_then(|entry| crate::listing::known(&entry.modified_unix_seconds)) {
        Some(seconds) => modified_date_for_seconds(seconds, DateDisplay::Full),
        None => "—".to_owned(),
    }
}

fn entry_icon(entry: Option<&FileEntry>) -> IconName {
    let Some(entry) = entry else {
        return IconName::Folder;
    };
    if entry.is_locked_folder() {
        IconName::Lock
    } else if entry.is_directory() {
        IconName::Folder
    } else if ArchiveFormat::for_entry(entry).is_some() {
        IconName::Archive
    } else {
        IconName::File
    }
}

fn spawn_info_query(
    later: Later<App>,
    generation: u64,
    location: Location,
) -> glib::JoinHandle<()> {
    let file = gio_file_for_location(&location);
    let attrs = format!(
        "{CONTENT_TYPE_ATTRIBUTES},standard::is-hidden,standard::size,time::modified,unix::mode,owner::user,owner::group"
    );
    glib::MainContext::default().spawn_local(async move {
        let Ok(info) = file
            .query_info_future(
                &attrs,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
        else {
            return;
        };
        later.then(move |app: &mut App, _: &mut Tx| apply_info(app, generation, &info));
    })
}

fn apply_info(app: &mut App, generation: u64, info: &gio::FileInfo) -> bool {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_mut() else {
        return false;
    };
    if single.generation != generation {
        return false;
    }
    if !single.is_directory {
        single.size = file_size(info.size().max(0) as u64);
    }
    if let Some(time) = info.modification_date_time() {
        single.modified = modified_date_for_seconds(time.to_unix(), DateDisplay::Full);
    }
    single.hidden = tr(if info_is_hidden(info) { "Yes" } else { "No" });
    apply_content_type(single, info);
    apply_permissions_info(single, info);
    true
}

fn apply_content_type(single: &mut Single, info: &gio::FileInfo) {
    let Some(reported) = content_type_of(info) else {
        return;
    };
    let content_type = refine_reported_content_type(&reported, Path::new(&single.name));
    single.subtitle = gio::content_type_get_description(&content_type).into();
    if let Some(default_app) = gio::AppInfo::default_for_type(&content_type, false) {
        single.opens_with = default_app.display_name().into();
    }
}

fn apply_permissions_info(single: &mut Single, info: &gio::FileInfo) {
    let mode = info.attribute_uint32("unix::mode");
    if mode != 0 {
        single.permissions.set_mode(mode);
    }
    single.permissions.owner = info
        .attribute_string("owner::user")
        .map_or_else(|| "—".to_owned(), Into::into);
    single.permissions.group = info
        .attribute_string("owner::group")
        .map_or_else(|| "—".to_owned(), Into::into);
}
