// SPDX-License-Identifier: MIT

//! The headers' own actions, in GTK's order, for both places GTK has them: the Icons and List
//! pane header (`ContentActions` and `icons_controls` in `src/ui/browser_modes/`, `list_pane.rs`)
//! and a Columns column's header (`column_header` in `src/ui/browser/columns/append/header.rs`),
//! which C2's `header_actions` slot asks for the targeted column only.
//!
//! GTK's order, and who brings each:
//!
//! | Action | Pane header | Column header | Owner |
//! | --- | --- | --- | --- |
//! | Leftovers of an interrupted operation | yes | yes | whoever ports interrupted operations |
//! | Empty Trash, over the Trash's root | yes | yes | A5, [`super::empty_trash`] |
//! | New Folder, in a chooser window only | no | yes | A20's chooser |
//! | Refresh | yes | yes | A5, [`super::refresh`] |
//! | Thumbnail size, Icons only | yes | no | A5, `super::thumbnail_size` |
//! | Sort direction, Sort by | yes | yes | A10 |
//! | The filter | yes | yes | A10 |
//! | Close, after the first column | no | yes | C2 itself |
//!
//! A new action goes in its place in [`pane`] and [`column`]. In the pane header a button takes
//! `buttons::on_return`; in a column's it is not a focus stop, as C2's slot asks.

use rune::{El, row};
use strata_components::columns::HeaderCx;

use super::{empty_trash, refresh};
use crate::{App, buttons};

/// The actions before the thumbnail size in the header over the single-pane view keyed `view`.
pub(super) fn pane(app: &App, view: &str) -> Vec<El> {
    let Some(depth) = app.browser.active_depth() else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    if let Some(button) = empty_trash::built(app, depth) {
        actions.push(buttons::on_return(format!("{view}-empty-trash"), button));
    }
    actions.push(buttons::on_return(
        format!("{view}-refresh"),
        refresh::built(app, depth),
    ));
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
    Some(actions.child(refresh.focusable(false)))
}
