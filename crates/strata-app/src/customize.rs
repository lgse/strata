// SPDX-License-Identifier: MIT

mod body;
mod custom;
mod emoji;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use rune::{El, Tx};
use strata_components::dialog::{Focus, StrataDialog};
use strata_core::{
    i18n::tr,
    model::{FileEntry, FolderColorValue, Location},
};

use crate::{
    App,
    commands::{Action, Call, Holds, On, Outcome},
    shell,
    slots::{At, Frame, Part, Place, Setup},
};

/// The item a dialog customizes.
struct Item {
    path: PathBuf,
    name: String,
    folder: bool,
    /// Told apart from the item before, so each dialog takes its own first focus.
    generation: u64,
    return_focus: Option<String>,
}

pub(crate) struct Customize {
    /// The item, kept after the dialog closes for its closing motion.
    item: Option<Item>,
    open: bool,
    opened: u64,
    custom: custom::Custom,
    emoji: emoji::Emoji,
}

impl Part for Customize {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            item: None,
            open: false,
            opened: 0,
            custom: custom::Custom::new(),
            emoji: emoji::Emoji::new(),
        }
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        let custom = custom::poll(app, tx);
        let emoji = emoji::poll(app, tx);
        custom || emoji
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let customize = &app.customize;
        (customize.open || customize.custom.is_open() || customize.emoji.is_open()).then_some(
            Holds::Modal {
                field: customize.custom.field_focused() || customize.emoji.field_focused(),
            },
        )
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("customize", customize)];
}

fn customize(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let target = match &call.on {
        On::Row(at) => {
            row_entry(app, *at).map(|entry| (entry.location.clone(), entry.is_directory()))
        }
        On::Column(depth) => app.browser.location_at(*depth).map(|folder| (folder, true)),
        On::Place(location) => Some((location.clone(), true)),
        On::Focus => None,
    };
    let Some((location, folder)) = target else {
        return Outcome::Declined;
    };
    let Some(path) = location.native_path().map(PathBuf::from) else {
        return Outcome::Declined;
    };
    // Work around upstream Rune #559/#1492 regression (`a3d7bff`): when a modal context menu
    // closes and this modal dialog opens in the same commit, Rune's `ModalEffect::Return` checks
    // `can_focus` against the already-mounted new modal and drops the opener's focus.
    let return_focus = match call.on {
        On::Row(_) | On::Column(_) => shell::view_focus(app).map(str::to_owned),
        _ => None,
    };
    open(app, path, name_of(&location), folder, return_focus);
    Outcome::Ran
}

fn row_entry(app: &App, (depth, id): At) -> Option<FileEntry> {
    if let Some(result) = crate::filter::targets::at(app, depth, id) {
        return Some(result);
    }
    shell::shown_view(app)
        .and_then(|view| (view.rows.position)(app, (depth, id)))
        .and_then(|position| app.browser.entry_at(depth, position))
}

fn name_of(location: &Location) -> String {
    let path = location.native_path();
    path.and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| path.map(|path| path.to_string_lossy().into_owned()))
        .unwrap_or_else(|| location.display_name())
}

fn open(app: &mut App, path: PathBuf, name: String, folder: bool, return_focus: Option<String>) {
    let icon = shell::preferences(app).item_icon(&path).map(str::to_owned);
    let customize = &mut app.customize;
    customize.custom.close();
    customize.emoji.close();
    customize.emoji.sync(icon.as_deref());
    customize.opened += 1;
    customize.item = Some(Item {
        path,
        name,
        folder,
        generation: customize.opened,
        return_focus,
    });
    customize.open = true;
}

/// The item an open dialog customizes.
#[cfg(test)]
pub(crate) fn customizing(app: &App) -> Option<&std::path::Path> {
    let customize = &app.customize;
    customize
        .item
        .as_ref()
        .filter(|_| customize.open)
        .map(|item| item.path.as_path())
}

/// Done, Escape, the close button and a click outside.
fn close(app: &mut App) {
    app.customize.open = false;
    app.customize.emoji.close();
}

/// The item's saved colour and icon, as the dialog shows them.
fn saved(app: &App) -> Option<(Option<FolderColorValue>, Option<String>)> {
    let item = app.customize.item.as_ref()?;
    let preferences = shell::preferences(app);
    Some((
        preferences.item_color(&item.path),
        preferences.item_icon(&item.path).map(str::to_owned),
    ))
}

/// Saves `color` for the item, or forgets its colour with `None`.
fn choose_color(app: &mut App, color: Option<FolderColorValue>) {
    let Some(path) = app.customize.item.as_ref().map(|item| item.path.clone()) else {
        return;
    };
    shell::change_customization(app, |settings| {
        settings.set_item_colors(&[path], color.as_ref())
    });
}

/// Saves `icon` (`strata-*` or `emoji:*`) for the item.
fn save_icon(app: &mut App, icon: &str) {
    let Some(path) = app.customize.item.as_ref().map(|item| item.path.clone()) else {
        return;
    };
    shell::change_customization(app, |settings| settings.set_item_icon(&path, Some(icon)));
}

/// Saves the bundled icon `name` for the item.
fn choose_icon(app: &mut App, name: &'static str) {
    save_icon(app, name);
    app.customize.emoji.sync(None);
}

/// Clear: the item's colour and icon are forgotten.
fn clear(app: &mut App) {
    let Some(path) = app.customize.item.as_ref().map(|item| item.path.clone()) else {
        return;
    };
    shell::change_customization(app, |settings| settings.clear_item_customization(&path));
    app.customize.emoji.sync(None);
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let customize = &app.customize;
    let item = customize.item.as_ref()?;
    let (color, icon) = saved(app)?;
    let title = tr(if item.folder {
        "Customize Folder"
    } else {
        "Customize File"
    });
    let shown = body::Shown {
        item,
        color: color.as_ref(),
        icon: icon.as_deref(),
    };
    let mut over = Vec::new();
    let children = body::children(app, &shown, &mut over);
    let mut dialog = StrataDialog::new(format!("customize-{}", item.generation), title, close)
        .open(customize.open)
        .subtitle(item.name.clone())
        .children(children)
        .secondary(tr("Clear"), clear)
        .primary(tr("Done"), close)
        .initial_focus(Focus::Primary)
        .close_label(tr("Close dialog"));
    if let Some(target) = &item.return_focus {
        dialog = dialog.return_focus(target.clone());
    }
    dialog.build(&app.look, frame.layer);
    frame.layer.extend(over);
    custom::build(app, item, frame);
    None
}
