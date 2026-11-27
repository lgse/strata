// SPDX-License-Identifier: MIT

//! Compact per-tab width calculation and graceful shrinking before horizontal scrolling.

use rune::TextStyle;
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;
use unicode_width::UnicodeWidthStr;

use super::data::LABEL_CHARS;

const MIN_SHRUNK_LABEL_CHARS: f32 = slot::MIN_LABEL_CHARS;

const LABEL_ADVANCE_EM: f32 = 0.62;

/// Extra horizontal margin for glyph bearings and rounding when estimating natural width.
const ESTIMATE_SLACK: f32 = 4.0;

pub(super) const CLOSE_BUTTON_SIZE: f32 = 20.0;

pub(super) fn label_font(look: &Look) -> TextStyle {
    TextStyle {
        weight: 600.0,
        tracking: 0.0,
        ..look.kit.ty.ui_compact
    }
}

/// Horizontal padding, icon, gaps, and close button reserved around a tab's label.
pub(super) fn tab_chrome(look: &Look) -> f32 {
    look.space.lg
        + look.icon_size.md
        + look.space.sm
        + look.space.sm
        + CLOSE_BUTTON_SIZE
        + look.space.xs
}

fn char_advance(font: TextStyle) -> f32 {
    font.size * LABEL_ADVANCE_EM + font.tracking
}

pub(super) fn min_cap(look: &Look) -> f32 {
    look.slots.tabs.min_width.unwrap_or_else(|| {
        let char_w = char_advance(label_font(look));
        (MIN_SHRUNK_LABEL_CHARS * char_w + tab_chrome(look)).ceil()
    })
}

pub(super) fn max_cap(look: &Look) -> f32 {
    let min_w = min_cap(look);
    look.slots
        .tabs
        .max_width
        .unwrap_or_else(|| {
            let char_w = char_advance(label_font(look));
            (LABEL_CHARS as f32 * char_w + tab_chrome(look) + ESTIMATE_SLACK)
                .ceil()
                .max(slot::DEFAULT_MAX_WIDTH)
        })
        .max(min_w)
}

/// Estimated natural width of a tab showing `label`, clamped to `max_w`.
pub(super) fn estimate_tab_width(label: &str, look: &Look, max_w: f32) -> f32 {
    let cols = label.width().max(1) as f32;
    let text_w = cols * char_advance(label_font(look)) + ESTIMATE_SLACK;
    (tab_chrome(look) + text_w).ceil().min(max_w)
}

pub(super) fn width_cap(
    natural: &[f32],
    available: Option<f32>,
    gap: f32,
    bounds: (f32, f32),
) -> f32 {
    let (min_w, max_w) = bounds;
    let (Some(available), false) = (available, natural.is_empty()) else {
        return max_w;
    };
    let gaps = natural.len().saturating_sub(1) as f32 * gap;
    let budget = (available - gaps).max(0.0);
    let mut widths: Vec<f32> = natural.iter().map(|&w| w.min(max_w)).collect();
    let total: f32 = widths.iter().sum();
    if total <= budget {
        return max_w;
    }
    widths.sort_by(f32::total_cmp);
    let mut used = 0.0;
    for (i, w) in widths.iter().copied().enumerate() {
        let remaining = (widths.len() - i) as f32;
        let share = (budget - used) / remaining;
        if share <= w {
            return share.clamp(min_w, max_w);
        }
        used += w;
    }
    max_w
}
