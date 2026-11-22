// SPDX-License-Identifier: MIT

//! Compact per-tab width calculation and graceful shrinking before horizontal scrolling.

use rune::TextStyle;
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;
use unicode_width::UnicodeWidthStr;

use super::data::LABEL_CHARS;

/// Characters of a long tab's label kept before the strip stops shrinking and starts scrolling,
/// matching `strata_kit::slots::tabs::MIN_LABEL_CHARS`.
const MIN_SHRUNK_LABEL_CHARS: f32 = slot::MIN_LABEL_CHARS;

/// Advance per column in `em`, slightly above JetBrains Mono's 0.60 so semibold/proportional
/// labels have breathing room when deciding whether to shrink.
const LABEL_ADVANCE_EM: f32 = 0.62;

/// Extra horizontal margin for glyph bearings and rounding when estimating natural width.
const ESTIMATE_SLACK: f32 = 4.0;

/// Close button hit box width/height inside each tab (matching GTK's 18 px button + padding).
pub(super) const CLOSE_BUTTON_SIZE: f32 = 20.0;

/// The semibold compact label style matching GTK's `.tab-select { font-weight: 600 }`.
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

/// The narrowest cap applied to long tabs before horizontal scrolling begins. Short tabs (such as
/// `~`) stay at their own narrower natural width rather than inflating to this floor.
pub(super) fn min_cap(look: &Look) -> f32 {
    look.slots.tabs.min_width.unwrap_or_else(|| {
        let char_w = char_advance(label_font(look));
        (MIN_SHRUNK_LABEL_CHARS * char_w + tab_chrome(look)).ceil()
    })
}

/// The widest a tab may grow: enough for [`LABEL_CHARS`] characters plus chrome, or the user's
/// `tabs.tab.max_width` override, never below [`min_cap`].
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

/// Water-filling width cap across tabs with natural widths `natural`.
///
/// When all tabs fit in `available` (minus inter-tab gaps), returns `bounds.1` so every tab takes
/// its exact natural content width. When they exceed `available`, shrinks the widest tabs first
/// while leaving compact tabs (such as `~`) alone, clamping at `bounds.0` so excess tabs scroll.
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
