// SPDX-License-Identifier: MIT

//! The sort's buttons in the headers: the direction button and "Choose sort field", GTK's
//! `column_sort_direction_toggle` and `sort_menu_button`, and List's "Sort: …" label
//! (`sync_list_sort_menu`).

use rune::{El, Tx, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::header;
use strata_core::{
    i18n::tr,
    model::{SortDirection, SortKey, ViewPreferences},
};
use strata_desktop::settings::ListColumnId;
use strata_kit::icons::IconName;

use super::{Opener, open_at, toggle};
use crate::{App, buttons, shell};

/// A Columns column's direction button and Sort by, after Refresh, no focus stops (C2's slot).
pub(crate) fn column(app: &App, depth: usize) -> [El; 2] {
    let direction = direction_button(app, depth).key(format!("columns-sort-direction-{depth}"));
    let anchor = format!("columns-sort-{depth}");
    let menu = menu_button(app, depth, &anchor, false, true).key(anchor);
    [direction.focusable(false), menu.focusable(false)]
}

/// Icons' direction button and Sort by, after the thumbnail size, over the view keyed `view`.
pub(crate) fn direction_and_menu(app: &App, view: &str) -> Vec<El> {
    let Some(depth) = app.browser.active_depth() else {
        return Vec::new();
    };
    let anchor = format!("{view}-sort");
    vec![
        buttons::on_return(
            format!("{view}-sort-direction"),
            direction_button(app, depth),
        ),
        buttons::on_return(anchor.clone(), menu_button(app, depth, &anchor, false, false)),
    ]
}

/// List's Sort by, first among its header's actions: its icon, or the "Sort: …" label while
/// the sort has no visible heading of its own.
pub(crate) fn list_menu(app: &App, view: &str) -> Option<El> {
    let depth = app.browser.active_depth()?;
    let anchor = format!("{view}-sort");
    Some(buttons::on_return(
        anchor.clone(),
        menu_button(app, depth, &anchor, true, false),
    ))
}

/// List's direction button, shown only in the Recent root, as GTK's `append_sort_controls`.
pub(crate) fn recent_direction(app: &App, view: &str) -> Option<El> {
    let depth = app.browser.active_depth()?;
    let recent = app
        .browser
        .location_at(depth)
        .is_some_and(|location| location.is_recent_root());
    recent.then(|| {
        buttons::on_return(
            format!("{view}-sort-direction"),
            direction_button(app, depth),
        )
    })
}

/// "Choose sort field", a toggle button pressed while its menu is open; in List, the label
/// in place of the icon while [`labelled`] says so.
fn menu_button(app: &App, depth: usize, anchor: &str, list: bool, in_columns: bool) -> El {
    let look = &app.look;
    let pressed = open_at(app, anchor);
    let owned = anchor.to_owned();
    let press = move |app: &mut App, tx: &mut Tx| {
        let opener = Opener {
            depth,
            anchor: owned.clone(),
            list,
            in_columns,
        };
        toggle(app, opener, tx);
    };
    let label = list.then(|| labelled(app, depth)).flatten();
    match label {
        Some(label) => Button::new(label.clone())
            .variant(ButtonVariant::Ghost)
            .label_with(move |font, ink| text(label).font(font).color(ink))
            .on_press_tx(press)
            .build(&look.kit)
            .checked(pressed),
        None => header::action(look, IconName::Settings, &app.sort.labels.choose)
            .on_press_tx(press)
            .build(&look.kit)
            .checked(pressed),
    }
}

/// The direction button: it reverses the column's sort, insensitive while unsorted or in
/// device order and named for its state (`sync_sort_direction_toggle`).
fn direction_button(app: &App, depth: usize) -> El {
    let look = &app.look;
    let prefs = app.browser.column_preferences(depth).unwrap_or_default();
    let descending = prefs.sort_direction == SortDirection::Descending;
    let icon = if descending {
        IconName::SortDescending
    } else {
        IconName::SortAscending
    };
    let fixed = matches!(prefs.sort_key, SortKey::DeviceOrder | SortKey::Unsorted);
    let name = tr(if prefs.sort_key == SortKey::Unsorted {
        "Unsorted"
    } else if fixed {
        "Device order follows discovery; choose a sort field to reverse its direction"
    } else if descending {
        "Descending — click to reverse"
    } else {
        "Ascending — click to reverse"
    });
    header::action(look, icon, &name)
        .disabled(fixed)
        .on_press_tx(move |app: &mut App, _: &mut Tx| reverse(app, depth))
        .build(&look.kit)
}

fn reverse(app: &mut App, depth: usize) {
    let browser = app.browser.clone();
    let prefs = browser.column_preferences(depth).unwrap_or_default();
    if prefs.sort_key == SortKey::DeviceOrder {
        return;
    }
    let direction = match prefs.sort_direction {
        SortDirection::Ascending => SortDirection::Descending,
        SortDirection::Descending => SortDirection::Ascending,
    };
    browser.set_sort_direction(depth, direction);
}

/// List's label, GTK's `sync_list_sort_menu`: "Sort: Extension ↑", with Name's special name
/// order spelled out; none while the sorted heading shows the sort itself.
fn labelled(app: &App, depth: usize) -> Option<String> {
    let prefs = app.browser.column_preferences(depth).unwrap_or_default();
    let hidden = &shell::preferences(app).list_hidden_columns;
    if !needs_label(prefs, hidden) {
        return None;
    }
    let mut field = app.sort.labels.field(prefs.sort_key).to_owned();
    if special_names(prefs) {
        let order = if prefs.name_comparison.natural {
            &app.sort.labels.natural
        } else {
            &app.sort.labels.alphabetical
        };
        field = rust_i18n::t!("%{field} · %{order}", field = field, order = order).to_string();
        if prefs.name_comparison.case_sensitive {
            field = rust_i18n::t!("%{field} · case-sensitive", field = field).to_string();
        }
    }
    let direction = if prefs.sort_direction == SortDirection::Ascending {
        "↑"
    } else {
        "↓"
    };
    Some(rust_i18n::t!("Sort: %{field} %{direction}", field = field, direction = direction).to_string())
}

fn special_names(prefs: ViewPreferences) -> bool {
    prefs.sort_key == SortKey::Name
        && (!prefs.name_comparison.natural || prefs.name_comparison.case_sensitive)
}

/// GTK's `needs_sort_label`: a sort with no visible heading of its own, or Name in a special
/// name order.
fn needs_label(prefs: ViewPreferences, hidden: &[ListColumnId]) -> bool {
    let heading = match prefs.sort_key {
        SortKey::Name => Some(ListColumnId::Name),
        SortKey::Size => Some(ListColumnId::Size),
        SortKey::Type => Some(ListColumnId::Type),
        SortKey::Modified => Some(ListColumnId::Modified),
        _ => None,
    };
    !matches!(prefs.sort_key, SortKey::Unsorted | SortKey::DeviceOrder)
        && (heading.is_none_or(|column| hidden.contains(&column)) || special_names(prefs))
}
