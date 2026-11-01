// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use super::{SearchState, record_interaction, reveal_result, show_coverage};
use crate::services::{SearchCoverage, SearchItem};

type RowsByPath = HashMap<PathBuf, (SearchItem, gtk::ListBoxRow)>;
type ThumbnailChange = (
    crate::ui::thumbnail::ThumbnailSlot,
    Option<PathBuf>,
    &'static str,
);

/// What the list showed before a render, to restore selection, focus and scroll.
struct RenderedResults {
    query: String,
    query_changed: bool,
    items: Vec<SearchItem>,
    selected_index: Option<i32>,
    selected_path: Option<PathBuf>,
    focused: Option<gtk::Widget>,
    focused_path: Option<PathBuf>,
    scroll_position: f64,
}

impl RenderedResults {
    fn capture(state: &SearchState) -> Self {
        let query = state.field.text().trim().to_owned();
        let query_changed = state.rendered_query.borrow().as_str() != query;
        let items = state.visible_results.borrow().clone();
        let selected_index = state.list.selected_row().map(|row| row.index());
        let selected_path = selected_index
            .and_then(|position| usize::try_from(position).ok())
            .and_then(|position| items.get(position))
            .map(|item| item.path.clone());
        let focused = state.list.root().and_then(|root| root.focus());
        let focused_path = focused_result_path(state, &items, focused.as_ref());
        Self {
            query,
            query_changed,
            items,
            selected_index,
            selected_path,
            focused,
            focused_path,
            scroll_position: state.scroller.vadjustment().value(),
        }
    }

    /// A focused result row was re-rendered away from the window.
    fn lost_result_focus(&self) -> bool {
        self.focused_path.is_some()
            && self
                .focused
                .as_ref()
                .is_some_and(|focused| focused.root().is_none())
    }

    /// The previously selected item's new position, else its old index clamped.
    fn restored_position(&self, items: &[SearchItem]) -> usize {
        self.selected_path
            .as_ref()
            .and_then(|path| items.iter().position(|item| &item.path == path))
            .or_else(|| {
                self.selected_index.map(|position| {
                    usize::try_from(position)
                        .unwrap_or_default()
                        .min(items.len() - 1)
                })
            })
            .unwrap_or(0)
    }
}

fn focused_result_path(
    state: &SearchState,
    items: &[SearchItem],
    focused: Option<&gtk::Widget>,
) -> Option<PathBuf> {
    items.iter().enumerate().find_map(|(position, item)| {
        let row = state.list.row_at_index(position as i32)?;
        focused
            .filter(|focused| **focused == row || focused.is_ancestor(&row))
            .map(|_| item.path.clone())
    })
}

pub(super) fn render_results(
    state: &Rc<SearchState>,
    results: Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let mut before = RenderedResults::capture(state);
    let results_changed = before.items != results;
    if results_changed {
        reconcile_rows(state, &before.items, results);
    }

    let query_empty = before.query.is_empty();
    state
        .rendered_query
        .replace(std::mem::take(&mut before.query));
    let has_results = !state.visible_results.borrow().is_empty();
    show_coverage(&state.truncated_hint, coverage);
    state
        .results
        .set_visible_child_name(if has_results { "results" } else { "status" });
    if !has_results {
        let message = empty_results_message(state, query_empty, indexing);
        state.status.set_text(&crate::i18n::tr(message));
    } else if results_changed || before.query_changed {
        restore_selection(state, &before);
    }

    if results_changed && before.lost_result_focus() {
        restore_result_focus(state, before.focused_path.take());
    }
    if results_changed {
        settle_after_reorder(state, before.scroll_position);
    }
}

/// Keeps rows whose item is unchanged and rebuilds or removes the rest.
fn reconcile_rows(state: &Rc<SearchState>, old_items: &[SearchItem], results: Vec<SearchItem>) {
    state.reconciling_results.set(true);
    let mut rows = old_items
        .iter()
        .enumerate()
        .filter_map(|(position, item)| {
            state
                .list
                .row_at_index(position as i32)
                .map(|row| (item.path.clone(), (item.clone(), row)))
        })
        .collect::<RowsByPath>();
    let mut requested = state.requested_thumbnails.borrow_mut();
    let ordered_rows = results
        .iter()
        .map(|item| reuse_or_build_row(state, &mut rows, &mut requested, item))
        .collect::<Vec<_>>();
    for (path, (_, row)) in rows {
        discard_row(state, &mut requested, &path, &row);
    }
    drop(requested);

    state.positions.replace(
        ordered_rows
            .into_iter()
            .enumerate()
            .map(|(position, row)| (row, position))
            .collect(),
    );
    state.visible_results.replace(results);
    state.list.invalidate_sort();
    state.reconciling_results.set(false);
}

fn reuse_or_build_row(
    state: &Rc<SearchState>,
    rows: &mut RowsByPath,
    requested: &mut HashSet<PathBuf>,
    item: &SearchItem,
) -> gtk::ListBoxRow {
    let Some((previous, row)) = rows.remove(&item.path) else {
        return append_result_row(state, item);
    };
    if previous == *item {
        return row;
    }
    discard_row(state, requested, &item.path, &row);
    append_result_row(state, item)
}

fn discard_row(
    state: &SearchState,
    requested: &mut HashSet<PathBuf>,
    path: &Path,
    row: &gtk::ListBoxRow,
) {
    requested.remove(path);
    crate::ui::thumbnail::cancel_thumbnails_in(row.upcast_ref());
    state.list.remove(row);
}

fn append_result_row(state: &Rc<SearchState>, item: &SearchItem) -> gtk::ListBoxRow {
    let row = result_row(state, item);
    state.list.append(&row);
    row
}

fn restore_selection(state: &SearchState, before: &RenderedResults) {
    let items = state.visible_results.borrow();
    let restored = if before.query_changed {
        0
    } else {
        before.restored_position(&items)
    };
    drop(items);
    state
        .list
        .select_row(state.list.row_at_index(restored as i32).as_ref());
}

fn empty_results_message(state: &SearchState, query_empty: bool, indexing: bool) -> &'static str {
    if state.history.borrow().is_some() {
        if query_empty {
            "No folder history yet"
        } else {
            "No matching folders"
        }
    } else if indexing {
        "Searching…"
    } else {
        "No matching files or folders"
    }
}

fn restore_result_focus(state: &SearchState, focused_path: Option<PathBuf>) {
    let focused_position = focused_path.and_then(|path| {
        state
            .visible_results
            .borrow()
            .iter()
            .position(|item| item.path == path)
    });
    if let Some(row) = focused_position
        .and_then(|position| state.list.row_at_index(position as i32))
        .or_else(|| state.list.selected_row())
    {
        row.grab_focus();
    } else {
        state.field.grab_focus_without_selecting();
    }
}

fn settle_after_reorder(state: &Rc<SearchState>, scroll_position: f64) {
    record_interaction(state);
    let revision = state.interaction_revision.get();
    let weak = Rc::downgrade(state);
    state.list.add_tick_callback(move |_, _| {
        let weak = weak.clone();
        // Reordered rows keep their old allocation until this frame's layout.
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                settle_rows(&state, revision, scroll_position);
            }
        });
        glib::ControlFlow::Break
    });
}

fn settle_rows(state: &SearchState, revision: u64, scroll_position: f64) {
    if state.layer.is_visible() && state.interaction_revision.get() == revision {
        restore_scroll(state, scroll_position);
    }
    refresh_visible_thumbnails(state);
}

fn restore_scroll(state: &SearchState, scroll_position: f64) {
    if !state.navigation_started.get() {
        state.scroller.vadjustment().set_value(scroll_position);
        return;
    }
    if let Some(row) = state.list.selected_row() {
        scroll_row_into_view(state, &row);
    }
}

fn fallback_icon(item: &SearchItem) -> &'static str {
    if item.is_directory {
        crate::assets::icons::FOLDER
    } else {
        crate::ui::browser::icon_for_name(&item.name)
    }
}

fn result_row(state: &Rc<SearchState>, item: &SearchItem) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = crate::ui::thumbnail::ThumbnailSlot::new(19);
    icon.add_css_class("search-result-thumbnail");
    crate::ui::thumbnail::show_customized_icon(&icon, &item.path, fallback_icon(item), 19);
    content.append(&icon);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = result_label(&item.name, "search-result-name");
    let full_path = item.path.to_string_lossy();
    crate::ui::accessibility::set_label(&row, &full_path);
    let path = result_label(&full_path, "search-result-path");
    labels.append(&name);
    labels.append(&path);
    content.append(&labels);
    row.set_child(Some(&content));
    install_result_menu(state, &row, item);
    row
}

fn result_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label
}

fn install_result_menu(state: &Rc<SearchState>, row: &gtk::ListBoxRow, item: &SearchItem) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let target = item.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(state) = weak.upgrade()
            && let Some(row) = gesture.widget().and_downcast::<gtk::ListBoxRow>()
        {
            show_result_menu(&state, &row, target.clone(), (x, y));
        }
    });
    row.add_controller(click);
}

fn show_result_menu(
    state: &Rc<SearchState>,
    row: &gtk::ListBoxRow,
    item: SearchItem,
    (x, y): (f64, f64),
) {
    use crate::ui::browser::context_menu::{
        context_menu_option, context_menu_popover, show_context_popover,
    };

    super::close_result_menu(state);
    record_interaction(state);
    state.list.select_row(Some(row));
    let content = crate::ui::accessibility::menu_box();
    content.add_css_class("folder-context-menu");
    let reveal = context_menu_option(
        crate::assets::icons::FOLDER_OPEN,
        "Open containing folder",
        crate::ui::shortcut_reference::ContextHint::ContainingFolder,
    );
    reveal.set_sensitive(item.path.parent().is_some());
    content.append(&reveal);
    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    popover.connect_closed(|popover| popover.unparent());
    let weak = Rc::downgrade(state);
    reveal.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            reveal_result(&state, item.clone());
        }
    });
    show_context_popover(&popover, &scroll, row.upcast_ref(), x, y);
    state.context_menu.replace(Some(popover));
}

pub(super) fn refresh_visible_thumbnails(state: &SearchState) {
    let adjustment = state.scroller.vadjustment();
    let viewport = (adjustment.value(), adjustment.page_size());
    for (image, path, fallback) in thumbnail_changes(state, viewport) {
        if let Some(path) = path {
            crate::ui::thumbnail::set_thumbnail_or_icon_for_path(&image, &path, fallback, 19, 32);
        } else {
            crate::ui::thumbnail::show_fallback_icon(&image, fallback, 19);
        }
    }
}

fn thumbnail_changes(state: &SearchState, viewport: (f64, f64)) -> Vec<ThumbnailChange> {
    let items = state.visible_results.borrow();
    let mut requested = state.requested_thumbnails.borrow_mut();
    let mut changes = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let Some(row) = i32::try_from(position)
            .ok()
            .and_then(|position| state.list.row_at_index(position))
        else {
            continue;
        };
        let Some(visible) = row_visible(state, &row, viewport) else {
            continue;
        };
        if let Some(change) = thumbnail_change(&row, item, visible, &mut requested) {
            changes.push(change);
        }
    }
    changes
}

fn row_visible(state: &SearchState, row: &gtk::ListBoxRow, viewport: (f64, f64)) -> Option<bool> {
    let bounds = row.compute_bounds(&state.list)?;
    let (viewport_top, viewport_height) = viewport;
    Some(intersects_viewport(
        f64::from(bounds.y()),
        f64::from(bounds.height()),
        viewport_top,
        viewport_height,
    ))
}

/// Requests a thumbnail for a row scrolled into view, or releases one scrolled out.
fn thumbnail_change(
    row: &gtk::ListBoxRow,
    item: &SearchItem,
    visible: bool,
    requested: &mut HashSet<PathBuf>,
) -> Option<ThumbnailChange> {
    let image = row
        .child()
        .and_then(|content| content.first_child())
        .and_then(|child| {
            child
                .downcast::<crate::ui::thumbnail::ThumbnailSlot>()
                .ok()
        })?;
    let fallback = fallback_icon(item);
    let changed = if visible {
        requested.insert(item.path.clone())
    } else {
        requested.remove(&item.path)
    };
    changed.then(|| (image, visible.then(|| item.path.clone()), fallback))
}

fn intersects_viewport(
    row_top: f64,
    row_height: f64,
    viewport_top: f64,
    viewport_height: f64,
) -> bool {
    row_top < viewport_top + viewport_height && row_top + row_height > viewport_top
}

pub(super) fn scroll_row_into_view(state: &SearchState, row: &gtk::ListBoxRow) {
    let Some(bounds) = row.compute_bounds(&state.list) else {
        return;
    };
    let adjustment = state.scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_bottom = viewport_top + adjustment.page_size();
    let row_top = f64::from(bounds.y());
    let row_bottom = row_top + f64::from(bounds.height());
    if row_top < viewport_top {
        adjustment.set_value(row_top);
    } else if row_bottom > viewport_bottom {
        adjustment.set_value(row_bottom - adjustment.page_size());
    }
}
