// SPDX-License-Identifier: MIT

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

/// How many characters of its label the narrowest tab shows.
pub const MIN_LABEL_CHARS: f32 = 10.0;

pub const LABEL_ADVANCE_EM: f32 = 0.6;

pub const TAB_CHROME: f32 = 6.0 + 16.0 + 6.0 + 24.0 + 6.0;

/// The widest a tab grows to: rune-kit's horizontal default.
pub const DEFAULT_MAX_WIDTH: f32 = 224.0;

/// BrowserTabs' slot tokens. `None` inherits.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Tabs {
    pub tab_height: Option<f32>,
    pub pill_height: Option<f32>,
    pub tab_gap: Option<f32>,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub fade_width: Option<f32>,
    pub strip_fill: Option<Color>,
    pub underline_color: Option<Color>,
    pub underline_height: Option<f32>,
    pub drop_fill: Option<Color>,
    pub new_drop_fill: Option<Color>,
    pub badge_fill: Option<Color>,
    pub badge_text: Option<Color>,
}

pub fn tab_height(look: &Look) -> f32 {
    look.slots
        .tabs
        .tab_height
        .unwrap_or(look.frame.bar + 2.0 * look.frame.inset)
}

/// A tab's own height, its pill: `tabs.pill.height`, else `frame.control`.
pub fn pill_height(look: &Look) -> f32 {
    look.slots.tabs.pill_height.unwrap_or(look.frame.control)
}

/// The space between two tabs: `tabs.tab.gap`, else `space.sm`.
pub fn tab_gap(look: &Look) -> f32 {
    look.slots.tabs.tab_gap.unwrap_or(look.space.sm)
}

pub fn min_width(look: &Look) -> f32 {
    look.slots.tabs.min_width.unwrap_or_else(|| {
        let ty = look.kit.ty.ui_compact;
        let char = ty.size * LABEL_ADVANCE_EM + ty.tracking;
        (MIN_LABEL_CHARS * char + TAB_CHROME).ceil()
    })
}

pub fn max_width(look: &Look) -> f32 {
    look.slots
        .tabs
        .max_width
        .unwrap_or(DEFAULT_MAX_WIDTH)
        .max(min_width(look))
}

/// The fade over an overflowing strip's edges: `tabs.fade.width`, else `space.xxl`.
pub fn fade_width(look: &Look) -> f32 {
    look.slots.tabs.fade_width.unwrap_or(look.space.xxl)
}

pub fn strip_fill(look: &Look) -> Color {
    look.slots.tabs.strip_fill.unwrap_or(look.kit.c.bg_deep)
}

/// The active tab's underline: `tabs.underline.color`, else `kit.c.bg_accent`.
pub fn underline_color(look: &Look) -> Color {
    look.slots
        .tabs
        .underline_color
        .unwrap_or(look.kit.c.bg_accent)
}

/// The underline's thickness: `tabs.underline.height`, else 3 points (Strata's indicator).
pub fn underline_height(look: &Look) -> f32 {
    look.slots.tabs.underline_height.unwrap_or(3.0)
}

/// A tab under a drag it takes: `tabs.drop.fill`, else `fill.highlight`.
pub fn drop_fill(look: &Look) -> Color {
    look.slots.tabs.drop_fill.unwrap_or(look.fill.highlight)
}

pub fn new_drop_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .tabs
        .new_drop_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.15))
}

pub fn badge_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .tabs
        .badge_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.15))
}

/// A number badge's digit: `tabs.badge.text`, else `kit.c.text_accent`.
pub fn badge_text(look: &Look) -> Color {
    look.slots.tabs.badge_text.unwrap_or(look.kit.c.text_accent)
}

pub const TOKENS: &[Token] = &[
    Token::length("tabs.tab.height", tab_height, |l, v| {
        l.slots.tabs.tab_height = Some(v)
    }),
    Token::length("tabs.pill.height", pill_height, |l, v| {
        l.slots.tabs.pill_height = Some(v)
    }),
    Token::length("tabs.tab.gap", tab_gap, |l, v| {
        l.slots.tabs.tab_gap = Some(v)
    }),
    Token::length("tabs.tab.min_width", min_width, |l, v| {
        l.slots.tabs.min_width = Some(v)
    }),
    Token::length("tabs.tab.max_width", max_width, |l, v| {
        l.slots.tabs.max_width = Some(v)
    }),
    Token::length("tabs.fade.width", fade_width, |l, v| {
        l.slots.tabs.fade_width = Some(v)
    }),
    Token::color("tabs.strip.fill", strip_fill, |l, v| {
        l.slots.tabs.strip_fill = Some(v)
    }),
    Token::color("tabs.underline.color", underline_color, |l, v| {
        l.slots.tabs.underline_color = Some(v)
    }),
    Token::length("tabs.underline.height", underline_height, |l, v| {
        l.slots.tabs.underline_height = Some(v)
    }),
    Token::color("tabs.drop.fill", drop_fill, |l, v| {
        l.slots.tabs.drop_fill = Some(v)
    }),
    Token::color("tabs.new.drop_fill", new_drop_fill, |l, v| {
        l.slots.tabs.new_drop_fill = Some(v)
    }),
    Token::color("tabs.badge.fill", badge_fill, |l, v| {
        l.slots.tabs.badge_fill = Some(v)
    }),
    Token::color("tabs.badge.text", badge_text, |l, v| {
        l.slots.tabs.badge_text = Some(v)
    }),
];
