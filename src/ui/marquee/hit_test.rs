// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, graphene};

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

/// A view may reserve selected rows for outgoing file drags. Content-only views
/// retain the same selection-space hit regions before and after row selection.
pub(in crate::ui) fn item_content_predicate(
    targets: MarqueeTargets,
    content: ItemPredicate,
    drag_selected_rows: bool,
) -> ItemPredicate {
    Rc::new(move |surface, x, y| {
        targets.borrow().iter().any(|target| {
            hits_item(target, surface, (x, y), |position, widget| {
                let selected_drag_source = drag_selected_rows
                    && target.selection.is_selected(position)
                    && super::origin::hosts_drag_source(widget);
                selected_drag_source || content_hit(surface, widget, (x, y), &content)
            })
        })
    })
}

fn hits_item_allocation(target: &MarqueeTarget, surface: &gtk::Widget, point: (f64, f64)) -> bool {
    hits_item(target, surface, point, |_, _| true)
}

pub(super) struct ItemHit {
    position: u32,
    widget: gtk::Widget,
    object: Option<glib::Object>,
}

impl ItemHit {
    /// The same row still shows the same model item.
    pub(super) fn is_same(&self, other: &Self) -> bool {
        self.position == other.position
            && self.widget == other.widget
            && same_model_item(self.object.as_ref(), other.object.as_ref())
    }

    /// Activates like GTK's own double-click on a list or grid item.
    pub(super) fn activate(&self) {
        if let Err(error) = self
            .widget
            .activate_action("list.activate-item", Some(&self.position.to_variant()))
        {
            tracing::warn!(%error, "item activation from selection space failed");
        }
    }
}

fn same_model_item(a: Option<&glib::Object>, b: Option<&glib::Object>) -> bool {
    // Columns' lazy entry model returns a fresh string object on every lookup.
    match (
        a.and_then(|a| a.downcast_ref::<gtk::StringObject>()),
        b.and_then(|b| b.downcast_ref::<gtk::StringObject>()),
    ) {
        (Some(a), Some(b)) => a.string() == b.string(),
        _ => a == b,
    }
}

pub(super) fn item_at(
    targets: &MarqueeTargets,
    surface: &gtk::Widget,
    point: (f64, f64),
) -> Option<ItemHit> {
    targets.borrow().iter().find_map(|target| {
        let mut hit = None;
        (target.visit_items)(&mut |position, widget| {
            if hit.is_none() && contains(surface, widget, point) {
                hit = Some(ItemHit {
                    position,
                    widget: widget.clone(),
                    object: target.selection.item(position),
                });
            }
        });
        hit
    })
}

fn hits_item(
    target: &MarqueeTarget,
    surface: &gtk::Widget,
    point: (f64, f64),
    accepts: impl Fn(u32, &gtk::Widget) -> bool,
) -> bool {
    let mut hit = false;
    (target.visit_items)(&mut |position, widget| {
        if contains(surface, widget, point) {
            hit |= accepts(position, widget);
        }
    });
    hit
}

fn contains(surface: &gtk::Widget, widget: &gtk::Widget, point: (f64, f64)) -> bool {
    widget.is_mapped()
        && widget
            .compute_bounds(surface)
            .is_some_and(|bounds| bounds_contain(&bounds, point))
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
