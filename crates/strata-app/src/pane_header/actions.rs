// SPDX-License-Identifier: MIT

use rune::{El, Tx, row};
use strata_components::{columns::HeaderCx, header};
use strata_core::i18n::tr;
use strata_desktop::keys::{Keys, bindings::NEW_FOLDER};
use strata_kit::icons::IconName;

use super::{empty_trash, refresh};
use crate::{
    App, buttons,
    commands::{self, On},
    location::labels::with_keys,
    shell,
};

fn chooser_new_folder(app: &App, depth: usize) -> Option<El> {
    if !app.browser.is_chooser_mode() || !app.chooser.allows_create() {
        return None;
    }
    let look = &app.look;
    let tooltip = with_keys(
        "New Folder (%{keys})",
        Some("New Folder"),
        Keys::One(NEW_FOLDER.shown()),
    );
    let button = header::action(look, IconName::FolderPlus, &tooltip)
        .label(tr("New Folder"))
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            commands::run(app, "new-folder", On::Column(depth), tx);
        })
        .build(&look.accent_icon_kit());
    Some(button)
}

fn insert_chooser_new_folder(
    app: &App,
    (view, depth): (&str, usize),
    list: bool,
    actions: &mut Vec<El>,
) {
    let Some(button) = chooser_new_folder(app, depth) else {
        return;
    };
    let at = if list { actions.len() } else { 0 };
    actions.insert(at, buttons::on_return(format!("{view}-new-folder"), button));
}

pub(super) fn pane(app: &App, view: &str) -> Vec<El> {
    let Some(depth) = app.browser.active_depth() else {
        return Vec::new();
    };
    let list = view == crate::list::VIEW;
    let mut actions = Vec::new();
    if list {
        actions.extend(crate::sort::list_menu(app, view));
    }
    if let Some(button) = empty_trash::built(app, depth) {
        actions.push(buttons::on_return(format!("{view}-empty-trash"), button));
    }
    insert_chooser_new_folder(app, (view, depth), list, &mut actions);
    actions.push(buttons::on_return(
        format!("{view}-refresh"),
        refresh::built(app, depth),
    ));
    if list {
        actions.extend(crate::sort::recent_direction(app, view));
    }
    actions
}

/// The actions in the header of the Columns column `cx` names.
pub(crate) fn column(app: &App, cx: &HeaderCx) -> Option<El> {
    let depth = cx.depth;
    let tenxer = shell::preferences(app).tenxer_mode;
    let mut actions = row().items_center();
    let mut any = false;
    if let Some(button) = empty_trash::built(app, depth) {
        let key = format!("columns-empty-trash-{depth}");
        actions = actions.child(button.key(key).focusable(false).no_focus_ring());
        any = true;
    }
    if !tenxer && let Some(button) = chooser_new_folder(app, depth) {
        let key = format!("columns-new-folder-{depth}");
        actions = actions.child(button.key(key).focusable(false).no_focus_ring());
        any = true;
    }
    if tenxer {
        return any.then_some(actions);
    }
    let refresh = refresh::built(app, depth).key(format!("columns-refresh-{depth}"));
    actions = actions.child(refresh.focusable(false).no_focus_ring());
    actions = actions.children(crate::sort::column(app, depth));
    Some(actions.children(crate::filter::column_actions(app, depth)))
}
