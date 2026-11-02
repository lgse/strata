// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::super::{MAX_WIDTH, MIN_WIDTH};
use super::MIN_SPLIT_PREVIEW_WIDTH;
use crate::ui::browser::COLUMN_WIDTH;

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub(super) available: i32,
    pub(super) occupied: i32,
    pub(super) trailing: i32,
    pub(super) start_minimum: i32,
    pub(super) show_minimum: i32,
    pub(super) separator: i32,
    pub(super) columns: bool,
}

impl Geometry {
    pub(super) fn can_show_preview(self) -> bool {
        self.available - self.separator - self.show_minimum >= MIN_SPLIT_PREVIEW_WIDTH
    }

    pub(super) fn maximum_width(self) -> i32 {
        (self.available - self.separator - self.start_minimum).max(1)
    }

    pub(super) fn minimum_width(self, manual: bool) -> i32 {
        let minimum = if manual || self.columns {
            COLUMN_WIDTH
        } else {
            MIN_WIDTH
        };
        minimum.min(self.maximum_width())
    }

    fn desired_width(self, manual: Option<i32>) -> i32 {
        let free = (self.available - self.separator - self.occupied).max(0);
        let desired = if self.columns {
            // A dragged width is the session's minimum; the preview still fills
            // the free space so no gap opens beside the focused column.
            manual.map_or(free, |manual| free.max(manual))
        } else {
            manual.unwrap_or_else(|| free.saturating_mul(9).saturating_div(10).min(MAX_WIDTH))
        };
        desired.clamp(self.minimum_width(manual.is_some()), self.maximum_width())
    }

    pub(super) fn preview_width(self, manual: Option<i32>) -> i32 {
        self.desired_width(manual).max(MIN_SPLIT_PREVIEW_WIDTH)
    }

    pub(super) fn position(self, manual: Option<i32>) -> i32 {
        self.available - self.separator - self.preview_width(manual)
    }

    pub(super) fn empty_slot_minimum(self, manual: bool) -> i32 {
        (self.minimum_width(manual) - self.trailing).max(0)
    }

    fn empty_slot_width(self, manual: Option<i32>) -> i32 {
        (self.desired_width(manual) - self.trailing).max(0)
    }

    pub(super) fn empty_slot_position(self, manual: Option<i32>) -> i32 {
        self.available - self.separator - self.empty_slot_width(manual)
    }
}

pub(in crate::ui) fn separator(split: &gtk::Paned) -> Option<gtk::Widget> {
    let mut child = split.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "separator" {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

pub(in crate::ui) fn separator_width(split: &gtk::Paned) -> i32 {
    separator(split).map_or(0, |handle| {
        handle.measure(gtk::Orientation::Horizontal, -1).0
    })
}

pub(super) fn sidebar_width(content: &gtk::Paned) -> i32 {
    if content
        .start_child()
        .is_some_and(|child| child.get_visible())
    {
        content.position() + separator_width(content)
    } else {
        0
    }
}
