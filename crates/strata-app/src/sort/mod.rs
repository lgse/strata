// SPDX-License-Identifier: MIT

//! The sort controls (A10): the "Choose sort field" button and its menu, the sort direction
//! button, and the List view's "Sort: …" label, GTK's `src/ui/browser/pane_header/sort_menu.rs`
//! and `column_sort_direction_toggle` in `src/ui/browser/pane_header.rs`. Driven on GTK first
//! (2026-11-14).
//!
//! - **Where they are** ([`controls`]): in a Columns column's header, the direction and Sort by
//!   after Refresh and before the filter's toggle; in Icons, after the thumbnail size; in List,
//!   Sort by leads the header's actions and the direction shows only in the Recent root, as
//!   GTK places them (`pane_header::actions` has the orders).
//! - **The menu** ([`menu`]) is a plain popover of the menu kind ([`popover::Kind::Menu`]),
//!   GTK's `gtk::PopoverMenu`: SORT BY with Name, Size, Modified, Type, Extension and Created
//!   (Device order in a camera's photos, Recency in Recent), as radio rows; in List, Ascending
//!   and Descending; NAME ORDER with Natural and Alphabetical, and Case-sensitive names; Folders
//!   first; then Make default and Reset to default sort, each shown only while it would do
//!   something. A row runs at once and the menu stays open, with its checks updated. Choosing
//!   the checked field turns sorting off: Unsorted has no row.
//! - **The keyboard**: the menu opens on its first row. While it is open it takes every key
//!   ([`Holds::Popover`]); Escape or a press outside closes it and gives the button the
//!   keyboard back. A Columns column's buttons are no focus stops (C2's slot), so there the
//!   view gets it back instead: GTK leaves it on the header's button.
//! - **The direction button** reverses the column's sort, named "Ascending — click to reverse"
//!   or "Descending — click to reverse", and is insensitive while the column is unsorted or in
//!   device order, named for why.
//! - **List's label**: while the sort has no visible column of its own, or Name sorts in a
//!   special name order, the button shows "Sort: Extension ↑" in place of its icon, and is
//!   named so (`sync_list_sort_menu`).
//!
//! - **List's headings** ([`headings`]): Name, Size, Type and Modified sort, the sorted one
//!   showing its direction; a click reverses the sorted heading and sorts another ascending.
//!
//! The menu closes when another folder or view shows, or the keyboard leaves it, as GTK's
//! goes with its pane.

mod controls;
pub(crate) mod headings;
mod menu;

use rune::{El, Tx};
use strata_core::{
    i18n::tr,
    model::{Location, SortKey},
};

use crate::{
    App,
    commands::Holds,
    popover, shell,
    slots::{Frame, Part, Place, Setup},
};

pub(crate) use controls::{column, direction_and_menu, list_menu, recent_direction};

/// The sort controls' state: the open menu, if any, and the words.
pub(crate) struct Sort {
    open: Option<Open>,
    /// Where the keyboard goes once the menu closed, when not its button: a Columns column's
    /// buttons take no focus.
    refocus: Option<&'static str>,
    labels: Labels,
}

/// The open menu: the column it sorts, the button it hangs from, and what it opened over.
struct Open {
    depth: usize,
    anchor: String,
    /// List's menu adds the two directions.
    list: bool,
    over: Option<Location>,
    /// The shown view's listing as it opened.
    view: Option<&'static str>,
    /// In Columns, where the keyboard goes when it closes.
    returns: Option<&'static str>,
}

/// The words, translated once: the language changes only on restart.
pub(crate) struct Labels {
    choose: String,
    sort_by: String,
    name_order: String,
    ascending: String,
    descending: String,
    natural: String,
    alphabetical: String,
    case_sensitive: String,
    folders_first: String,
    make_default: String,
    reset: String,
    device_order_description: String,
    /// The fields' names, in [`menu::FIELDS`]' order.
    fields: Vec<String>,
}

impl Labels {
    fn translated() -> Self {
        Self {
            choose: tr("Choose sort field"),
            sort_by: tr("SORT BY"),
            name_order: tr("NAME ORDER"),
            ascending: tr("Ascending"),
            descending: tr("Descending"),
            natural: tr("Natural"),
            alphabetical: tr("Alphabetical"),
            case_sensitive: tr("Case-sensitive names"),
            folders_first: tr("Folders first"),
            make_default: tr("Make default"),
            reset: tr("Reset to default sort"),
            device_order_description: tr(
                "Append photos as the device lists them; not necessarily chronological. Selecting this reloads the library.",
            ),
            fields: menu::FIELDS.iter().map(|(label, ..)| tr(label)).collect(),
        }
    }

    /// A field's name, GTK's `field_label`: Name's for Unsorted, which has no row.
    fn field(&self, key: SortKey) -> &str {
        menu::FIELDS
            .iter()
            .position(|(_, field, _)| *field == key)
            .map_or(&self.fields[0], |at| &self.fields[at])
    }
}

/// What opens the menu: the column, its button's key, and whether it is List's.
pub(crate) struct Opener {
    pub(crate) depth: usize,
    pub(crate) anchor: String,
    pub(crate) list: bool,
    /// A Columns column's button, which takes no focus.
    pub(crate) in_columns: bool,
}

/// Opens the menu under its button with the keyboard on its first row, or closes it when it
/// is open there.
pub(crate) fn toggle(app: &mut App, opener: Opener, tx: &mut Tx) {
    if app.sort.open.as_ref().is_some_and(|open| open.anchor == opener.anchor) {
        close(app);
        return;
    }
    let view = shell::view_focus(app);
    let over = app.browser.location_at(opener.depth);
    app.sort.open = Some(Open {
        depth: opener.depth,
        anchor: opener.anchor,
        list: opener.list,
        over,
        view,
        returns: opener.in_columns.then_some(view).flatten(),
    });
    if let Some(first) = menu::stops(app).first() {
        tx.focus(*first, true);
    }
}

fn close(app: &mut App) {
    if let Some(open) = app.sort.open.take() {
        app.sort.refocus = open.returns;
    }
}

/// Whether the menu is open over the column at `depth`, for its button's pressed state.
fn open_at(app: &App, anchor: &str) -> bool {
    app.sort.open.as_ref().is_some_and(|open| open.anchor == anchor)
}

/// The menu closes when the keyboard leaves it, or when another folder or view shows; the
/// keyboard goes back to the Columns view after it closed there. True when anything changed.
fn follow(app: &mut App, tx: &mut Tx) -> bool {
    if let Some(key) = app.sort.refocus.take() {
        tx.focus(key, true);
        return true;
    }
    let Some(open) = app.sort.open.as_ref() else {
        return false;
    };
    // A Columns column's button takes no focus: the view keeps it while the menu opens.
    let anchor = open.returns.unwrap_or(&open.anchor);
    let left = popover::focus_left(tx, anchor, &menu::stops(app), &[]);
    let moved = shell::view_focus(app) != open.view
        || app.browser.location_at(open.depth) != open.over;
    if left || moved {
        close(app);
        return true;
    }
    false
}

/// The menu's surface, in the layer while it is open.
fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    menu::build(app, frame);
    None
}

impl Part for Sort {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            open: None,
            refocus: None,
            labels: Labels::translated(),
        }
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        follow(app, tx)
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.sort.open.is_some().then_some(Holds::Popover)
    }
}

#[cfg(test)]
mod tests;
