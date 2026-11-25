// SPDX-License-Identifier: MIT

//! Responsive breakpoints and dimensions for the Settings dialog.

use strata_desktop::settings::TextSize;

use crate::{App, shell};

pub(super) const DIALOG_WIDTH: f32 = 1250.0;
pub(super) const DIALOG_HEIGHT: f32 = 800.0;
pub(super) const RAIL_WIDTH: f32 = 240.0;
pub(super) const COMPACT_RAIL_WIDTH: f32 = 56.0;

pub(super) const COMPACT_NAV_BREAKPOINT: f32 = 900.0;
pub(super) const COMPACT_CONTENT_BREAKPOINT: f32 = 1000.0;
pub(super) const NARROW_CONTROLS_BREAKPOINT: f32 = 620.0;

const CONTENT_PAD_X: f32 = 48.0;
const CONTENT_PAD_Y: f32 = 24.0;
const CONTENT_BOTTOM_PAD: f32 = 40.0;
const COMPACT_CONTENT_PAD_X: f32 = 20.0;
const COMPACT_CONTENT_PAD_Y: f32 = 18.0;
const COMPACT_CONTENT_BOTTOM_PAD: f32 = 32.0;

/// Responsive layout decisions for the Settings dialog at the current window width and text scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ResponsiveLayout {
    pub(super) dialog_width: f32,
    pub(super) logical_width: f32,
    pub(super) compact_nav: bool,
    pub(super) compact_content: bool,
    pub(super) narrow_controls: bool,
}

impl ResponsiveLayout {
    pub(super) fn for_app(app: &App) -> Self {
        let margin = app.look.space.xxl;
        let text_size = shell::preferences(app).text_size;
        let window_width = app.window_size().map(|(w, _)| w);
        Self::from_dimensions(window_width, margin, text_size)
    }

    pub(super) fn from_dimensions(
        window_width: Option<i32>,
        margin: f32,
        text_size: TextSize,
    ) -> Self {
        let dialog_width = match window_width {
            Some(width) => ((width as f32) - margin * 2.0).clamp(1.0, DIALOG_WIDTH),
            None => DIALOG_WIDTH,
        };
        let scale =
            (text_size.root_font_px() as f32 / TextSize::DEFAULT.root_font_px() as f32).max(0.1);
        let logical_width = dialog_width / scale;
        Self {
            dialog_width,
            logical_width,
            compact_nav: logical_width < COMPACT_NAV_BREAKPOINT,
            compact_content: logical_width < COMPACT_CONTENT_BREAKPOINT,
            narrow_controls: logical_width < NARROW_CONTROLS_BREAKPOINT,
        }
    }

    pub(super) const fn rail_width(self) -> f32 {
        if self.compact_nav {
            COMPACT_RAIL_WIDTH
        } else {
            RAIL_WIDTH
        }
    }

    pub(super) const fn content_padding(self) -> (f32, f32, f32) {
        if self.compact_content {
            (
                COMPACT_CONTENT_PAD_X,
                COMPACT_CONTENT_PAD_Y,
                COMPACT_CONTENT_BOTTOM_PAD,
            )
        } else {
            (CONTENT_PAD_X, CONTENT_PAD_Y, CONTENT_BOTTOM_PAD)
        }
    }

    pub(super) fn content_width(self) -> f32 {
        let (pad_x, _, _) = self.content_padding();
        (self.dialog_width - self.rail_width() - pad_x * 2.0).max(240.0)
    }
}
