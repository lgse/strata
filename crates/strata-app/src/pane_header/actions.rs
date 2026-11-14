// SPDX-License-Identifier: MIT

//! The headers' own actions, in GTK's order, for both places GTK has them: the Icons and List
//! pane header (`ContentActions` and `icons_controls` in `src/ui/browser_modes/`, `list_pane.rs`)
//! and a Columns column's header (`column_header` in `src/ui/browser/columns/append/header.rs`),
//! which C2's `header_actions` slot asks for the targeted column only.
//!
//! GTK's order differs by view. Each list runs left to right, with who brings each action:
//!
//! - **Icons** (`icons_controls.rs`, and `icons_pane.rs` for New Folder): New Folder, in a
//!   chooser window only (A20's chooser, prepended); the leftovers of an interrupted operation
//!   (whoever ports interrupted operations); Empty Trash, over the Trash's root (A5,
//!   [`super::empty_trash`]); Refresh (A5, [`super::refresh`]); Thumbnail size (A5,
//!   `super::thumbnail_size`); the sort direction and Sort by (A10); the filter (A10).
//! - **List** (`ListActions::new` and `build_list_pane` in `list_pane.rs`): Sort by (A10), first;
//!   the leftovers; Empty Trash; New Folder, in a chooser window only; Refresh; the sort
//!   direction, in the Recent root only (A10); the Columns menu (A2); the filter.
//! - **A Columns column** (`column_header` in `columns/append/header.rs`): the leftovers; Empty
//!   Trash; New Folder, in a chooser window only; Refresh; the sort direction and Sort by; the
//!   filter; Close, after the first column (C2 itself).
//!
//! A new action goes in its place in [`pane`] and [`column`]. In the pane header a button takes
//! `buttons::on_return`; in a column's it is not a focus stop, as C2's slot asks.

use rune::{El, row};
use strata_components::columns::HeaderCx;

use super::{empty_trash, refresh};
use crate::{App, buttons};

/// The actions before the thumbnail size in the header over the single-pane view keyed `view`.
/// In List, GTK's Sort by leads them, and its direction button follows Refresh in the Recent
/// root only.
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
    let mut actions = row().items_center();
    if let Some(button) = empty_trash::built(app, depth) {
        let key = format!("columns-empty-trash-{depth}");
        actions = actions.child(button.key(key).focusable(false));
    }
    let refresh = refresh::built(app, depth).key(format!("columns-refresh-{depth}"));
    actions = actions.child(refresh.focusable(false));
    actions = actions.children(crate::sort::column(app, depth));
    Some(actions.children(crate::filter::column_actions(app, depth)))
}
