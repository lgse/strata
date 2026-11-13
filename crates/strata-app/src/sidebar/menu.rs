// SPDX-License-Identifier: MIT

//! The sidebar's context menus, as the GTK sidebar attaches them to its rows
//! (`attach_place_context_menu_with` in `src/ui/window/sidebar_context_menu.rs`, and
//! `build_trash_menu` in `src/ui/window/sidebar_trash.rs`), drawn by strata-components' C21.
//!
//! - **Opening.** A right-click on a row opens its menu at the pointer; the Menu key and
//!   Shift+F10 on a focused row open it at the row, which Rune reports as the same secondary
//!   press (`El::on_secondary_press`), as GTK pops the row's popover up on it
//!   (`open_sidebar_context_menu`).
//! - **The rows**, in GTK's order: "Open in New Tab" and a separator, "Connect to server…" on
//!   Network, "Unpin", "Customize…" on a folder with a local path, and "Properties"; the
//!   Trash's menu has "Unpin" and "Properties".
//! - **Unpin** is the sidebar's: it hides a built-in place, saving its `sidebar_show_*`
//!   setting, or takes a pin out of the bookmarks file (`unpin_location`). The other rows'
//!   commands belong to other packages' parts (tabs, the Connect to server dialog, the
//!   Customize dialog, Properties) and are drawn as GTK draws them, doing nothing until those
//!   parts land; each will add the target it needs to `commands::On` when it wires its row.
//!
//! Not here yet: the Trash's "Empty Trash…", which shows only while the Trash holds
//! something.

use std::rc::Rc;

use rune::{El, PointerPress, Tx};
use strata_components::{
    context_menu::{ContextMenu, Item, MenuIntent, MenuModel, MenuTarget, Section},
    contract::Point,
    sidebar::ItemId,
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::settings::Preferences;
use strata_fs::adapters::bookmarks;

use super::{places::Row, relist};
use crate::{App, shell};

/// The menu's key: its surface is `sidebar-menu-menu`, its rows `sidebar-menu-item<n>`.
const KEY: &str = "sidebar-menu";

/// One row: its id, and its label in the catalogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MenuRow {
    pub(super) id: &'static str,
    label: &'static str,
}

impl MenuRow {
    fn item(self) -> Item {
        Item::new(self.id, tr(self.label))
    }
}

pub(super) const OPEN_IN_TAB: MenuRow = MenuRow {
    id: "open-in-tab",
    label: "Open in New Tab",
};
pub(super) const CONNECT: MenuRow = MenuRow {
    id: "connect-to-server",
    label: "Connect to server…",
};
pub(super) const UNPIN: MenuRow = MenuRow {
    id: "unpin",
    label: "Unpin",
};
pub(super) const CUSTOMIZE: MenuRow = MenuRow {
    id: "customize",
    label: "Customize…",
};
pub(super) const PROPERTIES: MenuRow = MenuRow {
    id: "properties",
    label: "Properties",
};

/// The rows the menu of `row` shows, by section.
pub(super) fn rows(row: &Row) -> Vec<Vec<MenuRow>> {
    if matches!(&row.item, ItemId::Place(id) if id == "trash") {
        return vec![vec![UNPIN, PROPERTIES]];
    }
    let mut actions = Vec::new();
    if matches!(&row.item, ItemId::Place(id) if id == "network") {
        actions.push(CONNECT);
    }
    actions.push(UNPIN);
    if row.location.native_path().is_some() {
        actions.push(CUSTOMIZE);
    }
    actions.push(PROPERTIES);
    vec![vec![OPEN_IN_TAB], actions]
}

fn model(row: &Row) -> MenuModel {
    rows(row)
        .into_iter()
        .fold(MenuModel::new(), |model, section| {
            model.section(Section::new(section.into_iter().map(MenuRow::item)))
        })
}

/// A row asked for its menu, at `point` in window points.
pub(super) fn open(app: &mut App, item: &ItemId, point: PointerPress) {
    let Some(row) = app.sidebar.location_of(item) else {
        return;
    };
    let model = model(row);
    let target = MenuTarget::Other(Rc::new(item.clone()));
    app.sidebar
        .menu
        .open(target, Point::new(point.x, point.y), model);
}

/// The menu's anchor, with its surfaces in `layer` while it is open.
pub(super) fn element(app: &App, layer: &mut Vec<El>) -> El {
    ContextMenu::new(
        KEY,
        |app: &App| &app.sidebar.menu,
        |app: &mut App| &mut app.sidebar.menu,
    )
    // GTK's menu box has no name of its own.
    .name(String::new())
    .on_intent(chosen)
    .build(app, &app.look, layer)
}

/// A row of the menu was chosen; the menu has closed.
fn chosen(app: &mut App, intent: MenuIntent, _: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    let MenuTarget::Other(target) = target else {
        return;
    };
    let Some(item) = target.downcast_ref::<ItemId>() else {
        return;
    };
    if &*action == UNPIN.id {
        unpin(app, item);
    }
}

/// GTK's Unpin: a built-in place is hidden, a pin leaves the bookmarks file.
fn unpin(app: &mut App, item: &ItemId) {
    match item {
        ItemId::Place(id) => {
            let id = id.clone();
            shell::saved(app).change(|settings| hide_place(settings, &id));
            relist(app);
        }
        ItemId::Pin(_) => {
            if let Some(row) = app.sidebar.location_of(item) {
                unpin_location(row.location.clone());
            }
        }
        ItemId::Device(_) => {}
    }
}

/// The setting that shows the built-in place `id` (`set_sidebar_show_*`).
fn hide_place(settings: &mut Preferences, id: &str) {
    let shown = match id {
        "home" => &mut settings.sidebar_show_home,
        "trash" => &mut settings.sidebar_show_trash,
        "network" => &mut settings.sidebar_show_network,
        "recent" => &mut settings.sidebar_show_recent,
        "desktop" => &mut settings.sidebar_show_desktop,
        "documents" => &mut settings.sidebar_show_documents,
        "downloads" => &mut settings.sidebar_show_downloads,
        "music" => &mut settings.sidebar_show_music,
        "pictures" => &mut settings.sidebar_show_pictures,
        "videos" => &mut settings.sidebar_show_videos,
        _ => return,
    };
    *shown = false;
}

/// Takes every line for `location` out of the bookmarks file (`remove_pinned_place`); the
/// sidebar hears the change from the store.
fn unpin_location(location: Location) {
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        let before = places.len();
        places.retain(|(pinned, _)| *pinned != location);
        (places.len() != before).then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            tracing::warn!(%error, "the pinned folders could not be saved");
        }
    });
}
