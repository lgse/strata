// SPDX-License-Identifier: MIT

/// A column's span along the strip's content, and the width of the columns after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnSpan {
    pub left: f32,
    pub right: f32,
    /// The columns after it (a child column): shown too where there is room.
    pub trailing: f32,
}

impl ColumnSpan {
    pub fn width(self) -> f32 {
        self.right - self.left
    }

    pub fn reveal_target(self, current: f32, page: f32, lower: f32, max: f32) -> f32 {
        let max = max.max(lower);
        // A column wider than the viewport is as visible as it gets while it fills it.
        let visible = if self.width() > page {
            self.left <= current && current + page <= self.right
        } else {
            self.left >= current && self.right <= current + page
        };
        if visible {
            return current.clamp(lower, max);
        }
        (self.right + self.trailing - page)
            .min(self.left)
            .clamp(lower, max)
    }
}
