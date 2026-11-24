// SPDX-License-Identifier: MIT

//! In-window text-field context menu fed by Rune's `App::on_field_menu` fallback on Linux.

use rune::{El, FieldMenuRequest, PopupEntry, PopupItem, Tx};
use strata_components::{
    context_menu::{
        ContextMenu, ContextMenuState, Item, MenuIntent, MenuModel, MenuTarget, Section,
    },
    contract::Point,
};
use strata_core::i18n::tr;
use strata_desktop::keys::{self, Binding, Key as DesktopKey, Keys, Shown};
use strata_kit::icons::IconName;

use crate::{App, slots::Frame};

/// State for an open in-window text-field context menu.
#[derive(Default)]
pub(super) struct FieldMenu {
    state: ContextMenuState,
    field_path: Option<String>,
    /// Deferred until the commit that unmounts the modal menu, because Rune's
    /// `field_menu_pick` checks `self.live(&path)` which rejects fields behind a modal.
    pending_pick: Option<String>,
}

impl FieldMenu {
    pub(super) fn is_open(&self) -> bool {
        self.state.is_open()
    }
}

/// Opens Strata's styled, localized field menu for a `FieldMenuRequest` delivered by Rune.
pub(crate) fn open(app: &mut App, request: FieldMenuRequest, tx: &mut Tx) {
    let model = map_menu(request.menu().entries());
    let point = request.point();
    app.menu.field.field_path = Some(request.field_path().to_owned());
    app.menu.field.pending_pick = None;
    app.menu
        .field
        .state
        .open(MenuTarget::Background, Point::new(point.x, point.y), model);
    tx.focus("field-menu-menu", false);
}

/// Delivers a picked field-menu action once the modal menu has unmounted.
pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(action) = app.menu.field.pending_pick.take() else {
        return false;
    };
    tx.resolve_field_menu(Some(&action));
    true
}

/// Renders the field menu above dialogs so it stays interactive over modal forms.
pub(crate) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    if !app.menu.field.is_open() {
        return None;
    }
    let menu = ContextMenu::new(
        "field-menu",
        |app: &App| &app.menu.field.state,
        |app: &mut App| &mut app.menu.field.state,
    )
    .name(tr("Edit"))
    .on_intent(chosen)
    .on_dismiss_tx(dismissed);
    Some(menu.build(app, &app.look, frame.layer))
}

fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { action, .. } = intent;
    app.menu.field.field_path = None;
    app.menu.field.pending_pick = Some((*action).to_owned());
    tx.wake_after(std::time::Duration::ZERO);
}

fn dismissed(app: &mut App, _: MenuTarget, tx: &mut Tx) {
    tx.resolve_field_menu(None);
    if let Some(path) = app.menu.field.field_path.take() {
        tx.focus(path, false);
    }
}

fn map_menu(entries: &[PopupEntry]) -> MenuModel {
    let mut model = MenuModel::new();
    for section in map_sections(entries) {
        model = model.section(section);
    }
    model
}

fn map_sections(entries: &[PopupEntry]) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current = Vec::new();
    for entry in entries {
        match entry {
            PopupEntry::Separator => {
                if !current.is_empty() {
                    sections.push(Section::new(std::mem::take(&mut current)));
                }
            }
            PopupEntry::Item(item) => current.push(map_item(item)),
            PopupEntry::Submenu {
                title,
                menu,
                enabled,
                ..
            } => {
                let sub = Item::submenu(title.clone(), title.clone(), map_sections(menu.entries()))
                    .enabled(*enabled);
                current.push(sub);
            }
            _ => {}
        }
    }
    if !current.is_empty() {
        sections.push(Section::new(current));
    }
    sections
}

fn map_item(item: &PopupItem) -> Item {
    let (label, icon, shortcut) = match item.id() {
        "cut" => (
            tr("Cut"),
            Some(IconName::Cut),
            Some(ctrl_hint(DesktopKey::x)),
        ),
        "copy" => (
            tr("Copy"),
            Some(IconName::Copy),
            Some(ctrl_hint(DesktopKey::c)),
        ),
        "paste" => (
            tr("Paste"),
            Some(IconName::Paste),
            Some(ctrl_hint(DesktopKey::v)),
        ),
        "select-all" => (
            tr("Select All"),
            Some(IconName::ListChecks),
            Some(ctrl_hint(DesktopKey::a)),
        ),
        _ => (item.title().to_owned(), None, None),
    };
    let mut built = Item::new(item.id().to_owned(), label).enabled(item.is_enabled());
    if let Some(icon) = icon {
        built = built.icon(icon);
    }
    if let Some(shortcut) = shortcut {
        built = built.shortcut(shortcut);
    }
    if item.is_checked() {
        built = built.checked(true);
    }
    built
}

fn ctrl_hint(key: DesktopKey) -> String {
    keys::label(&Keys::One(Shown::Binding(Binding::ctrl(key))))
}
