// SPDX-License-Identifier: MIT

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
    /// Frame clock when a disappearing row queued `tx.focus(SURFACE, false)`, so a same-frame
    /// follow-up poll does not misread the still-focused disappearing row as focus leaving.
    focusing_surface_at: Option<f64>,
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

pub(crate) fn toggle(app: &mut App, opener: Opener, tx: &mut Tx) {
    if app
        .sort
        .open
        .as_ref()
        .is_some_and(|open| open.anchor == opener.anchor)
    {
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
        focusing_surface_at: None,
    });
    if let Some(first) = menu::stops(app).first() {
        tx.focus_when_shown(*first, true);
    }
}

fn close(app: &mut App) {
    if let Some(open) = app.sort.open.take() {
        app.sort.refocus = open.returns;
    }
}

/// Whether the menu is open over the column at `depth`, for its button's pressed state.
fn open_at(app: &App, anchor: &str) -> bool {
    app.sort
        .open
        .as_ref()
        .is_some_and(|open| open.anchor == anchor)
}

impl Sort {
    /// Whether the sort menu currently holds focus or is handing it back to the view.
    pub(crate) fn holds_focus(&self) -> bool {
        self.open.is_some() || self.refocus.is_some()
    }
}

pub(super) fn focus_surface(app: &mut App, tx: &mut Tx) {
    if let Some(open) = app.sort.open.as_mut() {
        open.focusing_surface_at = Some(tx.clock());
    }
    tx.focus(menu::SURFACE, false);
}

fn follow(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    if let Some(key) = app.sort.refocus.take() {
        tx.focus_when_shown(key, true);
        return true;
    }
    let Some(open) = app.sort.open.as_ref() else {
        return false;
    };
    // A Columns column's button takes no focus: the view keeps it while the menu opens.
    let anchor = open.returns.unwrap_or(&open.anchor);
    let focusing_surface = open.focusing_surface_at == Some(clock);
    let left =
        !focusing_surface && popover::focus_left(tx, anchor, &menu::stops(app), &[menu::SURFACE]);
    let moved =
        shell::view_focus(app) != open.view || app.browser.location_at(open.depth) != open.over;
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

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        follow(app, clock, tx)
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.sort.open.is_some().then_some(Holds::Popover)
    }
}

#[cfg(test)]
mod tests;
