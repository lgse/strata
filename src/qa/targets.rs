// SPDX-License-Identifier: MIT

//! What GTK reports about a widget: its AT-SPI role, accessible name, states and bounds.

use gtk::prelude::*;

use super::input::Injector;
pub(crate) use super::protocol::{LookupError, Rect};

mod fields;
mod lookup;
mod naming;
mod states;

pub(crate) use self::{
    fields::{editable_focused, text_content},
    lookup::presented,
    naming::{accessible_description, accessible_name, role_name},
    states::states,
};

/// The box GTK computes for a widget, whether or not it is mapped: a page just shown has
/// one before its first frame.
pub(crate) fn extents(widget: &gtk::Widget, injector: &Injector) -> Result<Rect, LookupError> {
    let broken = |message: &str| LookupError::Broken(message.to_owned());
    let native = widget
        .native()
        .ok_or_else(|| broken("the target has no native surface"))?;
    let bounds = widget
        .compute_bounds(&native)
        .ok_or_else(|| LookupError::Missing("the target has no bounds".to_owned()))?;
    let surface = native
        .surface()
        .ok_or_else(|| broken("the target's surface is not realized"))?;
    let (offset_x, offset_y) = native.surface_transform();
    let (origin_x, origin_y) = injector
        .surface_origin(&surface)
        .map_err(LookupError::Broken)?;
    Ok(Rect {
        x: f64::from(origin_x) + offset_x + f64::from(bounds.x()),
        y: f64::from(origin_y) + offset_y + f64::from(bounds.y()),
        width: f64::from(bounds.width()),
        height: f64::from(bounds.height()),
    })
}
