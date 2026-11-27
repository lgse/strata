// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::Tx;
use strata_core::{i18n::tr, model::Location};
use strata_fs::{adapters::bookmarks, services::io_error_message};

use super::{Dialog, dismiss};
use crate::{
    App,
    clipboard::paths::{copy_path_text, is_trash},
    commands::{self, On},
    error_dialog::{self, Error},
    shell,
    sidebar::PinState,
    slots::Later,
};

const ADMINISTRATOR_OPEN_REFUSAL: &str = "Other applications can't open files in a folder opened as administrator. Preview the file here, or copy it to another folder first.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PinAction {
    Pin,
    Unpin,
}

impl PinAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Pin => "Pin",
            Self::Unpin => "Unpin",
        }
    }
}

pub(super) fn pin_action_for(
    location: &Location,
    is_directory: bool,
    state: PinState,
) -> Option<PinAction> {
    if !is_directory || is_trash(location) {
        return None;
    }
    match state {
        PinState::Available => Some(PinAction::Pin),
        PinState::Pinned => Some(PinAction::Unpin),
        PinState::Unavailable => None,
    }
}

pub(super) fn compact_display_path(location: &Location) -> String {
    location
        .native_path()
        .map(compact_native_path)
        .unwrap_or_else(|| location.display_path())
}

fn compact_native_path(path: &Path) -> String {
    let home = glib::home_dir();
    if path == home {
        return "~".to_owned();
    }
    path.strip_prefix(&home)
        .ok()
        .map(|suffix| format!("~/{}", suffix.to_string_lossy()))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(super) fn open_clicked(app: &mut App, tx: &mut Tx) {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let location = single.location.clone();
    dismiss(app);
    if is_trash(&location) {
        error_dialog::show(
            app,
            Error::new(
                tr("Unable to open item"),
                tr("Items in Trash cannot be opened"),
            ),
        );
        return;
    }
    if location.is_administrator() {
        error_dialog::show(
            app,
            Error::new(tr("Unable to open item"), tr(ADMINISTRATOR_OPEN_REFUSAL)),
        );
        return;
    }
    app.browser.open_location(location);
    shell::settle(app, tx);
}

pub(super) fn rename_clicked(app: &mut App, tx: &mut Tx) {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let Some(entry) = single.entry.clone() else {
        return;
    };
    let depth = single
        .rename_depth
        .or_else(|| app.browser.active_depth())
        .unwrap_or(0);
    let id = shell::id_of(app, &entry.location);
    dismiss(app);
    commands::run(app, "rename", On::Row((depth, id)), tx);
}

pub(super) fn pin_clicked(app: &mut App) {
    let later = app.properties.later.clone();
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let location = single.location.clone();
    let name = single.name.clone();
    let standard = app.sidebar.pin_state(&location) == PinState::Unavailable;
    dismiss(app);
    if standard {
        return;
    }
    update_bookmarks(later, move |places| {
        if places.iter().any(|(pinned, _)| *pinned == location) {
            return false;
        }
        places.push((location.clone(), name.clone()));
        true
    });
}

pub(super) fn unpin_clicked(app: &mut App) {
    let later = app.properties.later.clone();
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let location = single.location.clone();
    dismiss(app);
    update_bookmarks(later, move |places| {
        let before = places.len();
        places.retain(|(pinned, _)| *pinned != location);
        places.len() != before
    });
}

fn update_bookmarks(
    later: Later<App>,
    change: impl Fn(&mut Vec<(Location, String)>) -> bool + Send + 'static,
) {
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        change(&mut places).then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            let detail = io_error_message(&error);
            later.then(move |app: &mut App, _: &mut Tx| {
                error_dialog::show(
                    app,
                    Error::new(tr("Unable to update pinned folders"), detail),
                );
                true
            });
        }
    });
}

pub(super) fn copy_path_clicked(app: &mut App) {
    let Some((location, is_directory)) = app.properties.dialog.as_mut().and_then(|dialog| {
        let Dialog::Single(single) = dialog else {
            return None;
        };
        single.copied = true;
        Some((single.location.clone(), single.is_directory))
    }) else {
        return;
    };
    let text = copy_path_text(&location, is_directory);
    app.clipboard.write_text(&text);
}
