// SPDX-License-Identifier: MIT

//! The Sort by menu's rows and what each does, GTK's `SortMenu` (`menu_model`, `sync`,
//! `install_actions`). A row is a frameless button with GTK's role, checked as GTK's check
//! shows; an insensitive row is drawn and skipped by the keys, and Make default and Reset to
//! default sort show only while they would do something (GTK's `hidden-when`).

use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_core::model::{FolderSort, NameComparison, SortDirection, SortKey, ViewPreferences};
use strata_desktop::settings::folder_sort_policy::{self, DefaultSort};
use strata_kit::{icons::IconName, look::Look};

use super::{Open, close};
use crate::{
    App, buttons, folder_views,
    popover::{Kind, Plain},
    shell,
    slots::Frame,
};

const SURFACE: &str = "sort-menu";
const ASCENDING: &str = "sort-ascending";
const DESCENDING: &str = "sort-descending";
const NATURAL: &str = "sort-natural";
const ALPHABETICAL: &str = "sort-alphabetical";
const CASE_SENSITIVE: &str = "sort-case-sensitive";
const FOLDERS_FIRST: &str = "sort-folders-first";
const MAKE_DEFAULT: &str = "sort-make-default";
const RESET: &str = "sort-reset";

/// GTK's fields, in its order, with their row keys. Unsorted has no row: choosing the checked
/// field turns sorting off.
pub(super) const FIELDS: [(&str, SortKey, &str); 8] = [
    ("Name", SortKey::Name, "sort-field-name"),
    ("Size", SortKey::Size, "sort-field-size"),
    ("Modified", SortKey::Modified, "sort-field-modified"),
    ("Type", SortKey::Type, "sort-field-type"),
    ("Extension", SortKey::Extension, "sort-field-extension"),
    ("Created", SortKey::Created, "sort-field-created"),
    ("Device order", SortKey::DeviceOrder, "sort-field-device"),
    ("Recency", SortKey::Recency, "sort-field-recency"),
];

/// What the menu shows for its column: the column's sort, and which rows are live.
struct Shown {
    prefs: ViewPreferences,
    fields: Vec<(&'static str, SortKey, &'static str)>,
    list: bool,
    make_default: bool,
    reset: bool,
}

impl Shown {
    fn of(app: &App, open: &Open) -> Self {
        let browser = &app.browser;
        let prefs = browser.column_preferences(open.depth).unwrap_or_default();
        let location = browser.location_at(open.depth);
        let fields = FIELDS
            .into_iter()
            .filter(|(_, key, _)| match key {
                SortKey::DeviceOrder => location.as_ref().is_some_and(|at| at.is_camera_photo_root()),
                SortKey::Recency => location.as_ref().is_some_and(|at| at.is_recent_root()),
                _ => true,
            })
            .collect();
        let folder_sort = browser.folder_sort_at(open.depth);
        let make_default = folder_sort != FolderSort::Unremembered
            && prefs.sort_key.stored_name().is_some()
            && prefs.sort_options() != browser.preferences().sort_options();
        Self {
            prefs,
            fields,
            list: open.list,
            make_default,
            reset: matches!(folder_sort, FolderSort::Saved(..)),
        }
    }

    /// GTK's `editable`: a name order and its case apply only to a real sort.
    fn names_live(&self) -> bool {
        !matches!(
            self.prefs.sort_key,
            SortKey::Unsorted | SortKey::DeviceOrder | SortKey::Recency
        )
    }

    fn direction_live(&self) -> bool {
        !matches!(self.prefs.sort_key, SortKey::Unsorted | SortKey::DeviceOrder)
    }

    fn folders_live(&self) -> bool {
        self.prefs.sort_key != SortKey::Recency
    }

    /// The rows the keys move through, in order: the sensitive ones that show.
    fn stops(&self) -> Vec<&'static str> {
        let mut stops: Vec<_> = self.fields.iter().map(|(_, _, key)| *key).collect();
        if self.list && self.direction_live() {
            stops.extend([ASCENDING, DESCENDING]);
        }
        if self.names_live() {
            stops.extend([NATURAL, ALPHABETICAL, CASE_SENSITIVE]);
        }
        if self.folders_live() {
            stops.push(FOLDERS_FIRST);
        }
        stops.extend(self.make_default.then_some(MAKE_DEFAULT));
        stops.extend(self.reset.then_some(RESET));
        stops
    }
}

/// The menu's rows the keys move through, in order; none while it is closed.
pub(super) fn stops(app: &App) -> Vec<&'static str> {
    app.sort
        .open
        .as_ref()
        .map_or_else(Vec::new, |open| Shown::of(app, open).stops())
}

/// The menu's surface, in the layer while it is open.
pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) {
    let Some(open) = app.sort.open.as_ref() else {
        return;
    };
    let shown = Shown::of(app, open);
    let look = &app.look;
    let labels = &app.sort.labels;
    let prefs = shown.prefs;
    let mut body = col()
        .w(300.0)
        .padding(look.space.sm)
        .gap(look.space.xxs)
        .child(heading(look, &labels.sort_by))
        .children(shown.fields.iter().map(|(_, key, row_key)| {
            let row = field_row(app, *key, row_key);
            option(look, Row { checked: prefs.sort_key == *key, ..row })
        }));
    if shown.list {
        body = body.child(separator(look)).children(direction_rows(app, &shown));
    }
    body = body
        .child(separator(look))
        .child(heading(look, &labels.name_order))
        .children(name_rows(app, &shown))
        .child(separator(look))
        .child(option(look, Row {
            key: FOLDERS_FIRST,
            role: "menuitemcheckbox",
            name: labels.folders_first.clone(),
            description: None,
            checked: prefs.folders_first,
            sensitive: shown.folders_live(),
            press: folders_first,
        }));
    if shown.make_default || shown.reset {
        body = body.child(separator(look)).children(default_rows(app, &shown));
    }
    let plain = Plain {
        key: SURFACE,
        anchor: open.anchor.clone(),
        kind: Kind::Menu,
        label: labels.choose.clone(),
        stops: shown.stops(),
        own_arrows: &[],
        listing: shell::view_focus(app),
        close,
        parent: None,
    };
    plain.build(look, body, frame);
}

fn field_row(app: &App, key: SortKey, row_key: &'static str) -> Row {
    let press: fn(&mut App, &mut Tx) = match key {
        SortKey::Name => |app, _| field(app, SortKey::Name),
        SortKey::Size => |app, _| field(app, SortKey::Size),
        SortKey::Modified => |app, _| field(app, SortKey::Modified),
        SortKey::Type => |app, _| field(app, SortKey::Type),
        SortKey::Extension => |app, _| field(app, SortKey::Extension),
        SortKey::Created => |app, _| field(app, SortKey::Created),
        SortKey::DeviceOrder => |app, _| field(app, SortKey::DeviceOrder),
        SortKey::Recency => |app, _| field(app, SortKey::Recency),
        SortKey::Unsorted => |_, _| {},
    };
    let description = (key == SortKey::DeviceOrder)
        .then(|| app.sort.labels.device_order_description.clone());
    Row {
        key: row_key,
        role: "menuitemradio",
        name: app.sort.labels.field(key).to_owned(),
        description,
        checked: false,
        sensitive: true,
        press,
    }
}

fn direction_rows(app: &App, shown: &Shown) -> Vec<El> {
    let labels = &app.sort.labels;
    let ascending = shown.prefs.sort_direction == SortDirection::Ascending;
    let row = |key, name, checked, press| Row {
        key,
        role: "menuitemradio",
        name,
        description: None,
        checked,
        sensitive: shown.direction_live(),
        press,
    };
    vec![
        option(&app.look, row(ASCENDING, labels.ascending.clone(), ascending, |app, _| {
            direction(app, SortDirection::Ascending);
        })),
        option(&app.look, row(DESCENDING, labels.descending.clone(), !ascending, |app, _| {
            direction(app, SortDirection::Descending);
        })),
    ]
}

fn name_rows(app: &App, shown: &Shown) -> Vec<El> {
    let labels = &app.sort.labels;
    let comparison = shown.prefs.name_comparison;
    let live = shown.names_live();
    let row = |key, role, name, checked, press| Row {
        key,
        role,
        name,
        description: None,
        checked,
        sensitive: live,
        press,
    };
    vec![
        option(&app.look, row(NATURAL, "menuitemradio", labels.natural.clone(), comparison.natural, |app, _| {
            names(app, |comparison| comparison.natural = true);
        })),
        option(&app.look, row(ALPHABETICAL, "menuitemradio", labels.alphabetical.clone(), !comparison.natural, |app, _| {
            names(app, |comparison| comparison.natural = false);
        })),
        option(&app.look, row(CASE_SENSITIVE, "menuitemcheckbox", labels.case_sensitive.clone(), comparison.case_sensitive, |app, _| {
            names(app, |comparison| comparison.case_sensitive = !comparison.case_sensitive);
        })),
    ]
}

fn default_rows(app: &App, shown: &Shown) -> Vec<El> {
    let labels = &app.sort.labels;
    let row = |key, name, press| Row {
        key,
        role: "menuitem",
        name,
        description: None,
        checked: false,
        sensitive: true,
        press,
    };
    let mut rows = Vec::new();
    if shown.make_default {
        rows.push(option(&app.look, row(MAKE_DEFAULT, labels.make_default.clone(), make_default)));
    }
    if shown.reset {
        rows.push(option(&app.look, row(RESET, labels.reset.clone(), reset)));
    }
    rows
}

/// A row: a frameless button with its role, its name, and a check at its right while on.
struct Row {
    key: &'static str,
    role: &'static str,
    name: String,
    description: Option<String>,
    checked: bool,
    sensitive: bool,
    press: fn(&mut App, &mut Tx),
}

fn option(look: &Look, row_of: Row) -> El {
    let kit = &look.kit;
    let name = row_of.name;
    let checked = row_of.checked;
    let check = look.icon(IconName::Check);
    let press = row_of.press;
    let built = Button::new(name.clone())
        .variant(ButtonVariant::Ghost)
        .disabled(!row_of.sensitive)
        .label_with(move |font, ink| {
            row()
                .grow(1.0)
                .gap(8.0)
                .child(text(name).font(font).color(ink).grow(1.0))
                .children(checked.then(|| icon(check, 16.0).color(ink)))
        })
        .on_press_tx(move |app: &mut App, tx: &mut Tx| press(app, tx))
        .build(kit)
        .w_full()
        .role(row_of.role);
    // A plain menu item carries no check state, as GTK's Make default does not.
    let built = if row_of.role == "menuitem" {
        built
    } else {
        built.checked(checked)
    };
    let built = match row_of.description {
        Some(description) => built.description(description),
        None => built,
    };
    buttons::on_return(row_of.key, built)
}

fn heading(look: &Look, words: &str) -> El {
    text(words.to_owned())
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
        .px(look.space.sm)
        .py(look.space.xs)
}

fn separator(look: &Look) -> El {
    el().role("separator")
        .w_full()
        .h(1.0)
        .my(look.space.xxs)
        .fill(look.kit.c.border_base)
}

/// The depth of the column the open menu sorts.
fn depth(app: &App) -> Option<usize> {
    app.sort.open.as_ref().map(|open| open.depth)
}

/// GTK's `field` action: the checked field turns sorting off, another sorts by it.
fn field(app: &mut App, key: SortKey) {
    let Some(depth) = depth(app) else {
        return;
    };
    let browser = app.browser.clone();
    let active = browser
        .column_preferences(depth)
        .is_some_and(|prefs| prefs.sort_key == key);
    browser.set_sort_key(depth, if active { SortKey::Unsorted } else { key });
}

fn direction(app: &mut App, direction: SortDirection) {
    if let Some(depth) = depth(app) {
        app.browser.clone().set_sort_direction(depth, direction);
    }
}

/// GTK's `names` and `sensitive` actions: the column's requested comparison, changed.
fn names(app: &mut App, change: fn(&mut NameComparison)) {
    let Some(depth) = depth(app) else {
        return;
    };
    let browser = app.browser.clone();
    let mut comparison = browser
        .requested_column_preferences(depth)
        .unwrap_or_default()
        .name_comparison;
    change(&mut comparison);
    browser.set_name_comparison(depth, comparison);
}

fn folders_first(app: &mut App, _: &mut Tx) {
    if let Some(depth) = depth(app) {
        app.browser.clone().toggle_folders_first(depth);
    }
}

/// GTK's `default` action: the column's sort becomes the default, saved, and the folders'
/// own sorts that now equal it go (`set_default_sort_options`).
fn make_default(app: &mut App, tx: &mut Tx) {
    let Some(depth) = depth(app) else {
        return;
    };
    let browser = app.browser.clone();
    let Some(options) = browser
        .requested_column_preferences(depth)
        .map(|prefs| prefs.sort_options())
    else {
        return;
    };
    let mut set = DefaultSort::Unstorable;
    shell::saved(app).change(|settings| {
        set = folder_sort_policy::set_default_sort_options(settings, options);
    });
    let DefaultSort::Stored(prune) = set else {
        return;
    };
    let mut defaults = browser.preferences();
    defaults.set_sort_options(options);
    browser.apply_default_preferences(defaults);
    if let Some(prune) = prune {
        folder_views::change_sorts(app, |views| prune.apply(views), tx);
    }
}

/// GTK's `reset` action: the folder's own sort goes, and its column takes the default.
fn reset(app: &mut App, tx: &mut Tx) {
    let Some(depth) = depth(app) else {
        return;
    };
    let browser = app.browser.clone();
    let Some(location) = browser.location_at(depth) else {
        return;
    };
    let settings = shell::preferences(app);
    if let Some(forgotten) = folder_sort_policy::forget_folder_sort(settings, &location) {
        folder_views::change_sorts(app, |views| forgotten.apply(views), tx);
    }
    browser.restore_default_sort(depth, browser.preferences().sort_options());
}
