// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::FileEntry;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(super) fn animate(
    widget: &impl IsA<gtk::Widget>,
    duration: Duration,
    frame: impl Fn(Duration) + 'static,
    on_done: impl FnOnce() + 'static,
) {
    let started = Instant::now();
    let callback = Rc::new(RefCell::new(Some(on_done)));
    let tick_id = Rc::new(RefCell::new(None));
    let callback_for_tick = callback.clone();
    let id_for_tick = tick_id.clone();
    let id = widget.add_tick_callback(move |_, _| {
        let elapsed = started.elapsed();
        frame(elapsed);
        if elapsed < duration {
            return gtk::glib::ControlFlow::Continue;
        }
        id_for_tick.take();
        if let Some(callback) = callback_for_tick.take() {
            callback();
        }
        gtk::glib::ControlFlow::Break
    });
    tick_id.replace(Some(id));
    // Hidden windows stop ticking; file operations must still complete.
    gtk::glib::timeout_add_local_once(duration + Duration::from_millis(100), move || {
        if let Some(id) = tick_id.take() {
            id.remove();
        }
        if let Some(callback) = callback.take() {
            callback();
        }
    });
}

pub(super) fn sampled_indices(length: usize, limit: usize) -> Vec<usize> {
    let count = length.min(limit);
    if count == 0 {
        return Vec::new();
    }
    (0..count).map(|index| index * length / count).collect()
}

#[derive(Clone)]
pub(super) struct EntryAnimationTarget {
    pub(super) entry: FileEntry,
    pub(super) row: gtk::Widget,
}

pub(super) fn collect_entry_targets(
    source: &gtk::Widget,
    entries: &[FileEntry],
) -> Vec<EntryAnimationTarget> {
    let mut candidates = Vec::new();
    walk_widgets(source, &mut |widget| {
        if widget.is_mapped()
            && is_entry_row(widget)
            && entries
                .iter()
                .any(|entry| contains_entry_name(widget, &entry.display_name))
        {
            candidates.push(widget.clone());
        }
    });

    let mut used = HashSet::new();
    entries
        .iter()
        .filter_map(|entry| {
            let display_path = entry.location.display_path();
            let matching_path = candidates.iter().position(|row| {
                !used.contains(row)
                    && row
                        .tooltip_text()
                        .is_some_and(|tooltip| tooltip == display_path)
            });
            let index = matching_path.or_else(|| {
                candidates.iter().position(|row| {
                    !used.contains(row) && contains_entry_name(row, &entry.display_name)
                })
            })?;
            let row = candidates[index].clone();
            used.insert(row.clone());
            Some(EntryAnimationTarget {
                entry: entry.clone(),
                row,
            })
        })
        .collect()
}

pub(super) fn bounds_in_overlay(
    widget: &gtk::Widget,
    overlay: &gtk::Overlay,
) -> Option<gtk::graphene::Rect> {
    widget.compute_bounds(overlay)
}

pub(super) fn icon_center_in_overlay(
    row: &gtk::Widget,
    overlay: &gtk::Overlay,
) -> Option<(f64, f64)> {
    let icon = find_thumbnail(row)?;
    let bounds = bounds_in_overlay(icon.upcast_ref(), overlay)?;
    Some((
        f64::from(bounds.x() + bounds.width() / 2.0),
        f64::from(bounds.y() + bounds.height() / 2.0),
    ))
}

fn is_entry_row(widget: &gtk::Widget) -> bool {
    ["file-row", "list-row", "icons-card"]
        .iter()
        .any(|class| widget.has_css_class(class))
        && !widget.has_css_class("new-entry-row")
}

fn contains_entry_name(widget: &gtk::Widget, name: &str) -> bool {
    if widget
        .clone()
        .downcast::<gtk::Label>()
        .is_ok_and(|label| label.text() == name)
        || widget
            .clone()
            .downcast::<gtk::Inscription>()
            .is_ok_and(|inscription| inscription.text().as_deref() == Some(name))
    {
        return true;
    }

    let mut child = widget.first_child();
    while let Some(widget) = child {
        if contains_entry_name(&widget, name) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn find_thumbnail(widget: &gtk::Widget) -> Option<crate::ui::thumbnail::ThumbnailSlot> {
    if let Ok(icon) = widget
        .clone()
        .downcast::<crate::ui::thumbnail::ThumbnailSlot>()
    {
        return Some(icon);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(icon) = find_thumbnail(&widget) {
            return Some(icon);
        }
        child = widget.next_sibling();
    }
    None
}

#[cfg(test)]
mod tests;

fn walk_widgets(widget: &gtk::Widget, visit: &mut dyn FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(widget) = child {
        walk_widgets(&widget, visit);
        child = widget.next_sibling();
    }
}
