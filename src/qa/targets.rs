// SPDX-License-Identifier: MIT

//! Finds widgets by AT-SPI role and accessible name and aims at their bounds.

use std::collections::HashMap;

use gtk::{glib, prelude::*};

use super::{input::Injector, protocol::At, state};

mod fields;
mod locate;
mod lookup;
mod naming;
mod states;
mod wanted;

pub(crate) use self::{
    fields::{
        editable_fields, editable_focused, editable_texts, field, focused_field, text_content,
    },
    locate::{locate, presented_target},
    lookup::{
        find_presented, open_dialog, presented, result_rows, search_results, window_titled,
        windows_titled,
    },
    naming::{accessible_description, accessible_name, role_name},
    states::states,
    wanted::{Wanted, has_state},
};

pub(crate) type Memory = HashMap<String, glib::WeakRef<gtk::Widget>>;

pub(crate) type Boxes = HashMap<String, Rect>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl Rect {
    pub(crate) fn centre(&self) -> (f64, f64) {
        (
            self.x + (self.width / 2.0).floor(),
            self.y + (self.height / 2.0).floor(),
        )
    }

    pub(crate) fn point(&self, at: Option<&At>) -> (f64, f64) {
        match at {
            None => self.centre(),
            Some(at) => (
                at.x.resolve(self.x, self.width),
                at.y.resolve(self.y, self.height),
            ),
        }
    }

    /// Whole pixels, as AT-SPI reports extents; sub-pixel drift is not movement.
    pub(crate) fn pixels(&self) -> [i64; 4] {
        [self.x, self.y, self.width, self.height].map(|value| value.floor() as i64)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Located {
    pub(crate) widget: gtk::Widget,
    pub(crate) rect: Rect,
    pub(crate) point: (f64, f64),
}

/// `Missing` is accepted by `absent` and the patient lookups; `Broken` is always an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LookupError {
    Missing(String),
    Broken(String),
}

impl LookupError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Missing(message) | Self::Broken(message) => message,
        }
    }
}

fn missing(message: impl Into<String>) -> LookupError {
    LookupError::Missing(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aim {
    Point,
    DragOrigin,
}

#[derive(Clone, Copy)]
pub(crate) struct Kept<'a> {
    pub(crate) memory: &'a Memory,
    pub(crate) widgets: &'a Memory,
    pub(crate) boxes: &'a Boxes,
}

/// Popovers are their own native surfaces, hence the surface offset and screen origin.
pub(crate) fn screen_rect(widget: &gtk::Widget, injector: &Injector) -> Result<Rect, LookupError> {
    let broken = |message: &str| LookupError::Broken(message.to_owned());
    let bounds =
        state::rendered_bounds(widget).ok_or_else(|| missing("the target is not on screen"))?;
    let native = widget
        .native()
        .ok_or_else(|| broken("the target has no native surface"))?;
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

pub(crate) fn extents(widget: &gtk::Widget, injector: &Injector) -> Result<Rect, LookupError> {
    let broken = |message: &str| LookupError::Broken(message.to_owned());
    let native = widget
        .native()
        .ok_or_else(|| broken("the target has no native surface"))?;
    let bounds = widget
        .compute_bounds(&native)
        .ok_or_else(|| missing("the target has no bounds"))?;
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
