// SPDX-License-Identifier: MIT

//! The sidebar's context menus, as the GTK sidebar attaches them to its rows
//! (`attach_place_context_menu_with` in `src/ui/window/sidebar_context_menu.rs`,
//! `build_trash_menu` in `src/ui/window/sidebar_trash.rs`, `attach_device_menu` in
//! `src/ui/window/device_menu.rs`, and `append_network_share` in
//! `src/ui/window/sidebar_devices.rs`), drawn by strata-components' C21.
//!
//! - **Opening.** A right-click on a row opens its menu under the pointer, centred on it, as
//!   GTK's popover points at a 1×1 rectangle at the click (`open_on_secondary_click`); the
//!   Menu key and Shift+F10 on a focused row open it under the row, centred on it, as GTK
//!   pops the row's popover up pointing at the row (`open_sidebar_context_menu`). Rune reports
//!   both as the row's secondary press (`El::on_secondary_press`), the keyboard's at the row's
//!   bottom-left, and `Tx::by_keys` tells them apart; C8 keeps each row's box.
//! - **Places & pins:** "Open in New Tab" and a separator, then "Connect to server…" on Network,
//!   "Unpin", "Customize…" on a folder with a local path, and "Properties"; the Trash's menu has
//!   "Unpin" and "Properties" after the separator, and "Empty Trash…" in a section of its own
//!   while the Trash is known to hold something ([`super::trash`]).
//! - **Devices:** "Open in New Tab" (when mounted outside chooser mode), "Unlock" / "Lock" on
//!   encrypted volumes, "Eject" / "Unmount" when releasable, "Mount" on unmounted non-locked
//!   volumes, "Properties" on mounted devices, and "Disconnect" (danger) on network shares.

use std::rc::Rc;

use rune::{El, PointerPress, Tx};
use strata_components::{
    context_menu::{ContextMenu, Item, MenuIntent, MenuModel, MenuTarget, Section},
    contract::Point,
    sidebar::{ItemId, LockState},
};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::settings::Preferences;
use strata_fs::adapters::bookmarks;
use strata_kit::icons::IconName;

use super::{
    devices::{self, DeviceMenuInfo, MediaRelease},
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

/// Matches GTK's sidebar context-menu width floor (`.folder-context-menu` in `src/style.css`).
const MIN_WIDTH: f32 = 230.0;

fn min_width(look: &strata_kit::look::Look) -> f32 {
    let scale = strata_kit::typography::factor(look.kit.ty.ui_compact.size);
    (MIN_WIDTH * scale).round()
}

/// One row: its id, its label in the catalogs, its leading icon, and whether it destroys
/// something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MenuRow {
    pub(super) id: &'static str,
    label: &'static str,
    icon: IconName,
    danger: bool,
}

impl MenuRow {
    const fn new(id: &'static str, label: &'static str, icon: IconName) -> Self {
        Self {
            id,
            label,
            icon,
            danger: false,
        }
    }

    fn item(self) -> Item {
        Item::new(self.id, tr(self.label))
            .icon(self.icon)
            .danger(self.danger)
    }
}

pub(super) const OPEN_IN_TAB: MenuRow =
    MenuRow::new("open-in-tab", "Open in New Tab", IconName::Plus);
pub(super) const CONNECT: MenuRow =
    MenuRow::new("connect-to-server", "Connect to server…", IconName::Plus);
pub(super) const UNPIN: MenuRow = MenuRow::new("unpin", "Unpin", IconName::Pin);
pub(super) const CUSTOMIZE: MenuRow = MenuRow::new("customize", "Customize…", IconName::Palette);
pub(super) const PROPERTIES: MenuRow =
    MenuRow::new("properties", "Properties", IconName::Properties);
/// The trash part's command (`crate::trash::empty`).
pub(super) const EMPTY_TRASH: MenuRow = MenuRow {
    danger: true,
    ..MenuRow::new("empty-trash", "Empty Trash…", IconName::Trash)
};
pub(super) const UNLOCK: MenuRow = MenuRow::new("device-unlock", "Unlock", IconName::Lock);
pub(super) const LOCK: MenuRow = MenuRow::new("device-lock", "Lock", IconName::Unlock);
pub(super) const MOUNT: MenuRow = MenuRow::new("device-mount", "Mount", IconName::Drive);
pub(super) const EJECT: MenuRow = MenuRow::new("device-eject", "Eject", IconName::Eject);
pub(super) const UNMOUNT: MenuRow = MenuRow::new("device-unmount", "Unmount", IconName::Eject);
pub(super) const DISCONNECT: MenuRow = MenuRow {
    danger: true,
    ..MenuRow::new("device-disconnect", "Disconnect", IconName::Eject)
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

/// The rows a device row's context menu shows, by section (`device_menu.rs` and
/// `append_network_share` in `sidebar_devices.rs`).
pub(super) fn device_rows(info: &DeviceMenuInfo, chooser: bool) -> Vec<Vec<MenuRow>> {
    if info.pending {
        return Vec::new();
    }
    if info.network {
        let mut sections = Vec::new();
        if !chooser && info.root.is_some() {
            sections.push(vec![OPEN_IN_TAB]);
        }
        if info.root.is_some() {
            sections.push(vec![PROPERTIES]);
        }
        sections.push(vec![DISCONNECT]);
        return sections;
    }
    let mut sections = Vec::new();
    if !chooser && info.root.is_some() {
        sections.push(vec![OPEN_IN_TAB]);
    }
    let mut actions = Vec::new();
    if let Some(lock) = info.lock {
        actions.push(match lock {
            LockState::Locked => UNLOCK,
            LockState::Unlocked => LOCK,
        });
    }
    if let Some(release) = info.release {
        actions.push(match release {
            MediaRelease::EjectVolume | MediaRelease::EjectMount => EJECT,
            MediaRelease::UnmountMount => UNMOUNT,
        });
    }
    if info.can_mount && info.root.is_none() && info.lock != Some(LockState::Locked) {
        actions.push(MOUNT);
    }
    if info.root.is_some() {
        actions.push(PROPERTIES);
    }
    if !actions.is_empty() {
        sections.push(actions);
    }
    sections
}

fn sections_to_model(sections: Vec<Vec<MenuRow>>) -> MenuModel {
    sections
        .into_iter()
        .fold(MenuModel::new(), |model, section| {
            model.section(Section::new(section.into_iter().map(MenuRow::item)))
        })
}

pub(super) fn model(row: &Row, trash: Contents) -> MenuModel {
    sections_to_model(rows(row, trash))
}

/// What a menu was opened on: a place/pin row, or a device row.
enum Target {
    Place { item: ItemId, location: Location },
    Device { id: String, root: Option<Location> },
}

/// A row asked for its menu, at `point` in window points. Focus moves to the row before the
/// menu opens so closing the menu — or cancelling a modal dialog opened from one of its rows —
/// returns focus to that sidebar row, as GTK's `popover.set_parent(row)` does.
pub(super) fn open(app: &mut App, item: &ItemId, point: PointerPress, tx: &mut Tx) {
    let (model, target, opens_trash) = match item {
        ItemId::Device(id) => {
            let Some(info) = app.sidebar.devices.menu_info(id) else {
                return;
            };
            let sections = device_rows(&info, app.browser.is_chooser_mode());
            if sections.is_empty() {
                return;
            }
            (
                sections_to_model(sections),
                Target::Device {
                    id: info.id,
                    root: info.root,
                },
                false,
            )
        }
        ItemId::Place(_) | ItemId::Pin(_) => {
            let Some(row) = app.sidebar.location_of(item) else {
                return;
            };
            (
                model(row, app.sidebar.trash.contents),
                Target::Place {
                    item: item.clone(),
                    location: row.location.clone(),
                },
                is_trash(item),
            )
        }
    };
    let by_keys = tx.by_keys();
    // GTK's popover points at the row from the keyboard, and at the click from the pointer.
    let keyboard = by_keys.then(|| app.sidebar.state.row_box(item)).flatten();
    let pointed = keyboard.unwrap_or([point.x, point.y, 0.0, 0.0]);
    let at = under(
        pointed,
        model.width_with_min(&app.look, min_width(&app.look)),
    );
    tx.focus(item.key(super::KEY), by_keys);
    app.sidebar
        .menu
        .open(MenuTarget::Other(Rc::new(target)), at, model);
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
    let Some(Target::Place { item, .. }) = target.downcast_ref::<Target>() else {
        return;
    };
    if !is_trash(item) {
        return;
    }
    let Some(row) = app.sidebar.location_of(item) else {
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
    .min_width(min_width(&app.look))
    .on_intent(chosen)
    .build(app, &app.look, layer)
}

/// A row of the menu was chosen; the menu has closed.
fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    let MenuTarget::Other(target) = target else {
        return;
    };
    let Some(target) = target.downcast_ref::<Target>() else {
        return;
    };
    match target {
        Target::Place { item, location } => {
            if &*action == UNPIN.id {
                unpin(app, item, location, tx);
            } else {
                commands::run(app, &action, On::Place(location.clone()), tx);
            }
        }
        Target::Device { id, root } => chosen_device(app, (id, root.as_ref()), &action, tx),
    }
}

fn chosen_device(app: &mut App, (id, root): (&str, Option<&Location>), action: &str, tx: &mut Tx) {
    match action {
        action if action == OPEN_IN_TAB.id => {
            if let Some(root) = root {
                commands::run(app, OPEN_IN_TAB.id, On::Place(root.clone()), tx);
            }
        }
        action if action == PROPERTIES.id => {
            if let Some(root) = root {
                commands::run(app, PROPERTIES.id, On::Place(root.clone()), tx);
            }
        }
        action if action == UNLOCK.id => devices::unlock(app, id),
        action if action == LOCK.id => devices::lock(app, id),
        action if action == MOUNT.id => devices::mount(app, id),
        action if action == EJECT.id || action == UNMOUNT.id => devices::release(app, id, tx),
        action if action == DISCONNECT.id => devices::disconnect(app, id, tx),
        _ => {}
    }
}

/// GTK's Unpin: a built-in place is hidden, a pin leaves the bookmarks file. The menu gives the
/// keyboard back to the row it was opened on ([`open`]), which a hidden place takes away: the
/// sidebar's first row takes it, as GTK's rebuilt list draws its ring there, whichever row went
/// (driven on GTK in F4's round 3, by a right-click and by the Menu key; finding
/// lgse/strata#2406's Unpin case).
fn unpin(app: &mut App, item: &ItemId, location: &Location, tx: &mut Tx) {
    match item {
        ItemId::Place(id) => {
            let id = id.clone();
            shell::saved(app).change(|settings| hide_place(settings, &id));
            relist(app);
            if let Some(key) = app.sidebar.first_key() {
                tx.focus(&key, tx.by_keys());
            }
        }
        ItemId::Pin(_) => unpin_location(location.clone()),
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
