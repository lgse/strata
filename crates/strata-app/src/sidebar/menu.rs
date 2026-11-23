// SPDX-License-Identifier: MIT

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

const MIN_WIDTH: f32 = 230.0;

fn min_width(look: &strata_kit::look::Look) -> f32 {
    let scale = strata_kit::typography::factor(look.kit.ty.ui_compact.size);
    (MIN_WIDTH * scale).round()
}

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
    if opens_trash {
        trash::refresh(app);
    }
}

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
