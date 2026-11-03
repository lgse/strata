// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::graphene;
use gtk::prelude::*;

use super::geometry::bounds_contain;
use super::{ItemPredicate, MarqueeTarget, MarqueeTargets};

/// An item-origin policy that treats the whole allocated row as item space.
pub(in crate::ui) fn item_bounds_predicate(targets: MarqueeTargets) -> ItemPredicate {
    Rc::new(move |surface, x, y| {
        targets
            .borrow()
            .iter()
            .any(|target| hits_item_allocation(target, surface, (x, y)))
    })
}

/// Selected rows claim their whole allocation for file drag; unselected rows
/// claim only rendered content, leaving their blank space for marquee selection.
pub(in crate::ui) fn item_content_predicate(
    targets: MarqueeTargets,
    content: ItemPredicate,
) -> ItemPredicate {
    Rc::new(move |surface, x, y| hits_item_bounds(surface, (x, y), &targets, &content))
}

fn hits_item_allocation(target: &MarqueeTarget, surface: &gtk::Widget, point: (f64, f64)) -> bool {
    let mut hit = false;
    (target.visit_items)(&mut |_, widget| {
        if widget.is_mapped()
            && let Some(bounds) = widget.compute_bounds(surface)
        {
            hit |= bounds_contain(&bounds, point);
        }
    });
    hit
}

fn hits_item_bounds(
    surface: &gtk::Widget,
    point: (f64, f64),
    targets: &MarqueeTargets,
    content: &ItemPredicate,
) -> bool {
    targets
        .borrow()
        .iter()
        .any(|target| hits_item_content(target, surface, point, content))
}

fn hits_item_content(
    target: &MarqueeTarget,
    surface: &gtk::Widget,
    point: (f64, f64),
    content: &ItemPredicate,
) -> bool {
    let mut hit = false;
    (target.visit_items)(&mut |position, widget| {
        if !widget.is_mapped() {
            return;
        }
        if let Some(bounds) = widget.compute_bounds(surface)
            && bounds_contain(&bounds, point)
        {
            hit |= target.selection.is_selected(position)
                || content_hit(surface, widget, point, content);
        }
    });
    hit
}

fn content_hit(
    surface: &gtk::Widget,
    widget: &gtk::Widget,
    (x, y): (f64, f64),
    content: &ItemPredicate,
) -> bool {
    surface
        .compute_point(widget, &graphene::Point::new(x as f32, y as f32))
        .is_some_and(|point| content(widget, f64::from(point.x()), f64::from(point.y())))
}
