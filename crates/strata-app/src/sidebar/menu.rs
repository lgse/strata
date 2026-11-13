// SPDX-License-Identifier: MIT

//! The sidebar's context menus, as the GTK sidebar attaches them to its rows
//! (`attach_place_context_menu_with` in `src/ui/window/sidebar_context_menu.rs`, and
//! `build_trash_menu` in `src/ui/window/sidebar_trash.rs`), drawn by strata-components' C21.
//!
//! - **Opening.** A right-click on a row opens its menu under the pointer, centred on it, as
//!   GTK's popover points at a 1×1 rectangle at the click (`open_on_secondary_click`); the
//!   Menu key and Shift+F10 on a focused row open it under the row, centred on it, as GTK
//!   pops the row's popover up pointing at the row (`open_sidebar_context_menu`). Rune reports
//!   both as the row's secondary press (`El::on_secondary_press`), the keyboard's at the row's
//!   bottom-left, and `Tx::by_keys` tells them apart; C8 keeps each row's box.
//! - **The rows**, in GTK's order: "Open in New Tab" and a separator, then "Connect to
//!   server…" on Network, "Unpin", "Customize…" on a folder with a local path, and
//!   "Properties"; the Trash's menu has "Unpin" and "Properties" after the separator, and
//!   "Empty Trash…" in a section of its own while the Trash is known to hold something
//!   ([`super::trash`]), which runs the trash part's `empty-trash`.
//! - **The target** is the row and its location as the menu opened, as GTK's menu holds its
//!   row's location: a relist while it is open renumbers the pins, and Unpin still takes the
//!   one it was opened on.
//! - **Unpin** is the sidebar's: it hides a built-in place, saving its `sidebar_show_*`
//!   setting, or takes a pin out of the bookmarks file (`unpin_location`). The other rows'
//!   commands belong to other packages' parts (tabs, the Connect to server dialog, the
//!   Customize dialog, Properties) and are drawn as GTK draws them, doing nothing until those
//!   parts land; each will add the target it needs to `commands::On` when it wires its row.

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

use super::{
    places::Row,
    relist,
    trash::{self, Contents},
};
use crate::{
    App,
    commands::{self, On},
    shell,
};

/// The menu's key: its surface is `sidebar-menu-menu`, its rows `sidebar-menu-item<n>`.
const KEY: &str = "sidebar-menu";

/// One row: its id, its label in the catalogs, and whether it destroys something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MenuRow {
    pub(super) id: &'static str,
    label: &'static str,
    danger: bool,
}

impl MenuRow {
    const fn new(id: &'static str, label: &'static str) -> Self {
        Self {
            id,
            label,
            danger: false,
        }
    }

    fn item(self) -> Item {
        Item::new(self.id, tr(self.label)).danger(self.danger)
    }
}

pub(super) const OPEN_IN_TAB: MenuRow = MenuRow::new("open-in-tab", "Open in New Tab");
pub(super) const CONNECT: MenuRow = MenuRow::new("connect-to-server", "Connect to server…");
pub(super) const UNPIN: MenuRow = MenuRow::new("unpin", "Unpin");
pub(super) const CUSTOMIZE: MenuRow = MenuRow::new("customize", "Customize…");
pub(super) const PROPERTIES: MenuRow = MenuRow::new("properties", "Properties");
/// The trash part's command (`crate::trash::empty`).
pub(super) const EMPTY_TRASH: MenuRow = MenuRow {
    danger: true,
    ..MenuRow::new("empty-trash", "Empty Trash…")
};

fn is_trash(item: &ItemId) -> bool {
    matches!(item, ItemId::Place(id) if id == "trash")
}

/// The rows the menu of `row` shows, by section, with what the Trash holds: Empty Trash only
/// once it is known to hold something (`trash_menu_visibility`).
pub(super) fn rows(row: &Row, trash: Contents) -> Vec<Vec<MenuRow>> {
    if is_trash(&row.item) {
        let mut sections = vec![vec![OPEN_IN_TAB], vec![UNPIN, PROPERTIES]];
        if trash == Contents::NonEmpty {
            sections.push(vec![EMPTY_TRASH]);
        }
        return sections;
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

fn model(row: &Row, trash: Contents) -> MenuModel {
    rows(row, trash)
        .into_iter()
        .fold(MenuModel::new(), |model, section| {
            model.section(Section::new(section.into_iter().map(MenuRow::item)))
        })
}

/// What a menu was opened on: the row, and its location then.
struct Target {
    item: ItemId,
    location: Location,
}

/// A row asked for its menu, at `point` in window points; `by_keys` when the keyboard asked.
pub(super) fn open(app: &mut App, item: &ItemId, point: PointerPress, by_keys: bool) {
    let Some(row) = app.sidebar.location_of(item) else {
        return;
    };
    let model = model(row, app.sidebar.trash.contents);
    let opens_trash = is_trash(item);
    // GTK's popover points at the row from the keyboard, and at the click from the pointer.
    let keyboard = by_keys.then(|| app.sidebar.state.row_box(item)).flatten();
    let pointed = keyboard.unwrap_or([point.x, point.y, 0.0, 0.0]);
    let at = under(pointed, model.width(&app.look));
    let target = MenuTarget::Other(Rc::new(Target {
        item: item.clone(),
        location: row.location.clone(),
    }));
    app.sidebar.menu.open(target, at, model);
    // GTK probes the Trash again as its menu shows.
    if opens_trash {
        trash::refresh(app);
    }
}

/// Where a menu `width` wide opens to sit under `rect`, centred on it, as a GTK popover in its
/// bottom position sits under what it points at: a row, or the point of a click.
fn under([x, y, w, h]: [f32; 4], width: f32) -> Point {
    Point::new(x + (w - width) / 2.0, y + h)
}

/// The probe found the Trash's contents changed: an open Trash menu takes them at once.
pub(super) fn contents_changed(app: &mut App) {
    let Some(MenuTarget::Other(target)) = app.sidebar.menu.target() else {
        return;
    };
    let Some(target) = target
        .downcast_ref::<Target>()
        .filter(|target| is_trash(&target.item))
    else {
        return;
    };
    let Some(row) = app.sidebar.location_of(&target.item) else {
        return;
    };
    let model = model(row, app.sidebar.trash.contents);
    app.sidebar.menu.update_model(model);
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

/// A row of the menu was chosen; the menu has closed. Unpin is the sidebar's, Empty Trash the
/// trash part's command.
fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    let MenuTarget::Other(target) = target else {
        return;
    };
    let Some(target) = target.downcast_ref::<Target>() else {
        return;
    };
    if &*action == UNPIN.id {
        unpin(app, target);
    } else if &*action == EMPTY_TRASH.id {
        commands::run(app, EMPTY_TRASH.id, On::Focus, tx);
    }
}

/// GTK's Unpin: a built-in place is hidden, a pin leaves the bookmarks file.
fn unpin(app: &mut App, target: &Target) {
    match &target.item {
        ItemId::Place(id) => {
            let id = id.clone();
            shell::saved(app).change(|settings| hide_place(settings, &id));
            relist(app);
        }
        ItemId::Pin(_) => unpin_location(target.location.clone()),
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
