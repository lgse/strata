// SPDX-License-Identifier: MIT

//! A result's menu (`show_result_menu` in `src/ui/search/results.rs`): a right-click on a
//! result opens it at the pointer, with one row, "Open containing folder", described by its
//! key and insensitive for a path with no folder. It reveals the result as Alt+Return does.
//!
//! It is a part of its own, in the dialogs' slot after the palette, so that it draws over the
//! palette and holds the keyboard for a menu ([`Holds::Menu`]) while the palette holds it for
//! its layer: the menus' part sits in the overlays, under every dialog. Escape or a press
//! outside closes it, and the keyboard goes back to the palette's field with the result still
//! highlighted. GTK leaves it on its layer instead, where the next character typed is lost
//! (finding lgse/strata#FINDING, `oracle`).

use std::{path::Path, rc::Rc};

use rune::{El, Tx};
use strata_components::{
    context_menu::{ContextMenu, Item, MenuIntent, MenuModel, MenuTarget, Section},
    contract::Point,
};
use strata_core::i18n::tr;
use strata_desktop::keys::{self, Keys, bindings::OPEN_CONTAINING_FOLDER};
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::Holds,
    slots::{Frame, Part, Place, Setup},
};

/// The menu's key: its surface is `palette-menu-menu`, its row `palette-menu-item0`.
const KEY: &str = "palette-menu";

/// The row's action.
const REVEAL: &str = "open-containing-folder";

/// The part: the menu's state is the palette's (`app.palette.menu`).
pub(crate) struct PaletteMenu;

impl Part for PaletteMenu {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(_: &Setup<'_>) -> Self {
        Self
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.palette.menu.is_open().then_some(Holds::Menu)
    }
}

/// A right-click on the result `id`, now highlighted, at `at`.
pub(super) fn open(app: &mut App, id: &str, at: Point) {
    let has_folder = Path::new(id)
        .parent()
        .is_some_and(|folder| !folder.as_os_str().is_empty());
    let row = Item::new(REVEAL, tr("Open containing folder"))
        .icon(IconName::FolderOpen)
        .shortcut(keys::label(&Keys::One(OPEN_CONTAINING_FOLDER.shown())))
        .enabled(has_folder);
    let model = MenuModel::new().section(Section::new([row]));
    let target = MenuTarget::Other(Rc::new(id.to_owned()));
    app.palette.menu.open(target, at, model);
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let menu = ContextMenu::new(
        KEY,
        |app: &App| &app.palette.menu,
        |app: &mut App| &mut app.palette.menu,
    )
    // GTK's menu box has no name of its own.
    .name(String::new())
    .on_intent(chosen)
    .build(app, &app.look, frame.layer);
    Some(menu)
}

/// The row was chosen; the menu has closed. The palette reveals the result and closes.
fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    let MenuTarget::Other(target) = target else {
        return;
    };
    let Some(id) = target.downcast_ref::<String>() else {
        return;
    };
    if &*action != REVEAL {
        return;
    }
    let polled = app.palette.state.reveal(id, tx);
    super::route(app, polled, tx);
}
