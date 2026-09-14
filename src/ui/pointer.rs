// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

pub(super) fn exceeds_drag_threshold(press: (f64, f64), point: (f64, f64), threshold: i32) -> bool {
    (point.0 - press.0).abs() > f64::from(threshold)
        || (point.1 - press.1).abs() > f64::from(threshold)
}

/// Labels expand to fill rows; only their rendered text, not that extra allocation,
/// is item content. Thumbnail slots and Icons' two-line caption are explicit hit regions.
pub(super) fn hits_item_content(surface: &gtk::Widget, x: f64, y: f64) -> bool {
    let picked = surface.pick(x, y, gtk::PickFlags::DEFAULT);
    let mut current = picked.clone();
    let mut in_item = false;
    while let Some(widget) = current {
        if widget.is::<gtk::Editable>()
            || widget.is::<gtk::Button>()
            || widget.is::<gtk::Range>()
            || widget.is::<gtk::Scrollbar>()
        {
            return true;
        }
        in_item |= widget.has_css_class("file-row")
            || widget.has_css_class("list-row")
            || widget.has_css_class("icons-card");
        if widget == *surface {
            break;
        }
        current = widget.parent();
    }
    if !in_item {
        return false;
    }
    let mut current = picked;
    while let Some(widget) = current {
        if widget.is::<super::thumbnail::ThumbnailSlot>()
            || widget.is::<gtk::Image>()
            || widget.is::<gtk::Inscription>()
        {
            return true;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            let Some(point) =
                surface.compute_point(label, &gtk::graphene::Point::new(x as f32, y as f32))
            else {
                return false;
            };
            let (offset, _) = label.layout_offsets();
            let (_, logical) = label.layout().pixel_extents();
            let left = offset + logical.x();
            return point.x() >= left as f32 && point.x() < (left + logical.width()) as f32;
        }
        if widget == *surface {
            break;
        }
        current = widget.parent();
    }
    false
}

/// The full Name column, including row padding, uses content-only hit testing.
pub(super) fn hits_list_item_content(row: &gtk::Widget, x: f64, y: f64) -> bool {
    let in_name = row
        .first_child()
        .filter(|cell| cell.has_css_class("list-name-cell"))
        .and_then(|cell| cell.compute_bounds(row))
        .is_some_and(|bounds| {
            x >= f64::from(bounds.x()) && x < f64::from(bounds.x() + bounds.width())
        });
    !in_name || hits_item_content(row, x, y)
}

pub(super) fn is_background(surface: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut current = surface.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = current {
        if widget.has_css_class("file-row")
            || widget.has_css_class("list-row")
            || widget.has_css_class("icons-card")
            || widget.is::<gtk::Editable>()
            || widget.is::<gtk::Button>()
            || widget.is::<gtk::Range>()
            || widget.is::<gtk::Scrollbar>()
        {
            return false;
        }
        if widget == *surface {
            return true;
        }
        current = widget.parent();
    }
    false
}

struct PendingClick {
    item: glib::Object,
    position: u32,
    press: (f64, f64),
    moved: bool,
}

/// A recycled row or a gesture that crossed the drag threshold cannot activate on
/// release, even if the pointer returned to its original position or DnD was cancelled.
pub(super) fn connect_click_release(
    click: &gtk::GestureClick,
    item: &gtk::ListItem,
    released: impl Fn(&gtk::GestureClick, i32) + 'static,
) {
    let pending = Rc::new(RefCell::new(None::<PendingClick>));
    let item_for_press = item.downgrade();
    let pending_for_press = pending.clone();
    click.connect_pressed(move |_, _, x, y| {
        pending_for_press.take();
        if let Some(item) = item_for_press.upgrade()
            && item.position() != gtk::INVALID_LIST_POSITION
            && let Some(value) = item.item()
        {
            pending_for_press.replace(Some(PendingClick {
                item: value,
                position: item.position(),
                press: (x, y),
                moved: false,
            }));
        }
    });
    let pending_for_update = pending.clone();
    click.connect_update(move |gesture, sequence| {
        if let (Some(pending), Some(point), Some(widget)) = (
            pending_for_update.borrow_mut().as_mut(),
            gesture.point(sequence),
            gesture.widget(),
        ) {
            pending.moved |= exceeds_drag_threshold(
                pending.press,
                point,
                widget.settings().gtk_dnd_drag_threshold(),
            );
        }
    });
    let pending_for_cancel = pending.clone();
    click.connect_cancel(move |_, _| {
        pending_for_cancel.take();
    });
    let item = item.downgrade();
    click.connect_released(move |gesture, count, x, y| {
        let (Some(pending), Some(item), Some(widget)) =
            (pending.take(), item.upgrade(), gesture.widget())
        else {
            return;
        };
        if !pending.moved
            && !exceeds_drag_threshold(
                pending.press,
                (x, y),
                widget.settings().gtk_dnd_drag_threshold(),
            )
            && item.position() == pending.position
            && item.item().as_ref() == Some(&pending.item)
        {
            released(gesture, count);
        }
    });
}

#[cfg(test)]
mod tests;
