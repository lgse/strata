// SPDX-License-Identifier: MIT

//! A pane's directory model and the pages, counts, and marks that follow it.

use gtk::{gio, prelude::*};

use super::list_pane::list_row_parts;
use crate::{
    app::{Browser, BrowserColumnSnapshot},
    ui::{
        browser::{ClipboardMark, ClipboardMarks, mark_in},
        loading_skeleton,
    },
};

use super::pane::{Pane, STATUS_PAGE};

pub(super) fn set_mode_mark_style(widget: &impl IsA<gtk::Widget>, mark: ClipboardMark) {
    if mark == ClipboardMark::Cut {
        widget.add_css_class("cut");
    } else {
        widget.remove_css_class("cut");
    }
    if let Some((icon, _)) = crate::ui::icons_cell::parts(widget) {
        icon.set_mark(mark);
    } else if let Some((icon, ..)) = widget
        .upcast_ref()
        .downcast_ref::<gtk::Box>()
        .and_then(list_row_parts)
    {
        icon.set_mark(mark);
    }
}

pub(super) fn refresh_mark_pane(pane: &Pane, browser: &Browser, marks: &ClipboardMarks) {
    pane.search.refresh_mark_rows();
    for section in pane.item_sections() {
        section.bound_items.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let mark = pane
                .bound_entry(browser, &item)
                .map_or(ClipboardMark::None, |entry| mark_in(marks, &entry.location));
            set_mode_mark_style(&widget, mark);
            true
        });
    }
}

pub(super) fn replace_entries(pane: &Pane, browser: &Browser, count: usize) {
    let values = browser
        .with_entries(pane.depth, 0..count, |entries| {
            entries
                .iter()
                .map(crate::ui::browser::entry_model_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pane.splice_values(0, pane.model.n_items(), &values);
}

pub(super) fn detach_pane_models(pane: &Pane) {
    pane.detached.set(true);
    pane.search.detach();
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
        crate::ui::browser::detach_collection_view(&section.view);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
}

pub(super) fn deactivate_pane_models(pane: &Pane) {
    pane.detached.set(true);
    for section in pane.all_sections() {
        section.syncing.set(true);
        section.selection.set_model(None::<&gio::ListModel>);
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(None::<&gio::ListModel>);
    }
    // Keep the pane's controls, not a second directory snapshot and its identity map.
    pane.model.splice(0, pane.model.n_items(), &[]);
}

pub(super) fn reconnect_pane_model(pane: &Pane) {
    if !pane.detached.replace(false) {
        return;
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(Some(&pane.model));
    }
    for section in pane.all_sections() {
        section.selection.set_model(Some(&section.view_model));
    }
}

pub(super) fn show_count(pane: &Pane) {
    show_count_controls(pane);
    if pane.model.n_items() == 0 {
        pane.status.remove_css_class("error");
        pane.status
            .set_label(&crate::i18n::tr("This directory is empty"));
        pane.retry.set_visible(false);
        pane.administrator.set_visible(false);
        pane.loading.show(STATUS_PAGE);
    } else {
        pane.loading.show(loading_skeleton::CONTENT_PAGE);
    }
}

pub(super) fn show_count_controls(pane: &Pane) {
    let count = pane.model.n_items();
    if let Some(entry) = &pane.filter_entry {
        entry.set_placeholder_text(Some(&crate::ui::browser::filter_placeholder(
            count as usize,
        )));
    }
    if let Some(button) = &pane.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

pub(super) fn apply_snapshot(pane: &Pane, snapshot: &BrowserColumnSnapshot, browser: &Browser) {
    replace_entries(pane, browser, snapshot.count);
    reconnect_pane_model(pane);
    let restarts_loading =
        snapshot.loading && (snapshot.count == 0 || !snapshot.location.is_camera_photo_root());
    // A load still in progress is not an empty directory; showing that page first
    // would hand the surface focus before the grace period decides.
    if restarts_loading {
        show_count_controls(pane);
    } else {
        show_count(pane);
    }
    super::selection::set_selections(pane, &snapshot.selected_positions);
    if let Some(&focused) = snapshot.selected_positions.last() {
        super::selection::scroll_pane_to_source(pane, focused);
    }
    pane.truncated_hint.set_visible(snapshot.truncated);
    pane.leftovers_button
        .set_visible(!browser.column_leftovers(pane.depth).is_empty());
    pane.not_responding.set_visible(snapshot.not_responding);
    show_snapshot_progress(pane, snapshot, restarts_loading);
}

fn show_snapshot_progress(pane: &Pane, snapshot: &BrowserColumnSnapshot, restarts_loading: bool) {
    if snapshot.loading {
        pane.spinner.start();
        if restarts_loading {
            pane.loading.start();
        }
        return;
    }
    pane.spinner.stop();
    if let Some(message) = snapshot.error.as_deref() {
        show_load_error(pane, message, snapshot.administrator.is_some());
    }
}

pub(super) fn show_load_error(pane: &Pane, message: &str, offers_administrator: bool) {
    pane.status.set_label(&rust_i18n::t!(
        "Unable to read this directory\n%{message}",
        message = message
    ));
    pane.status.add_css_class("error");
    pane.retry.set_visible(true);
    pane.administrator.set_visible(offers_administrator);
    pane.loading.show(STATUS_PAGE);
}

pub(super) fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
