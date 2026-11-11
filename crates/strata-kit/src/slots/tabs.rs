//! The `tabs.*` slot tokens (for example `tabs.tab`) that BrowserTabs reads.
//!
//! Work package: C7, `docs/work/phase-1-leaves/C7-browser-tabs.md`.
//!
//! W2 seeded `tabs.tab.height`, the strip's band; C7 keeps its name and meaning (slot names are
//! API) and adds the rest. Every default follows the rest of the [`Look`]: colours from `kit.c`
//! and `fill`, lengths from `frame`, `space` and the label's type. The exception is the widest
//! tab, [`DEFAULT_MAX_WIDTH`], rune-kit's.

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

/// How many characters of its label the narrowest tab shows.
pub const MIN_LABEL_CHARS: f32 = 10.0;

/// The bundled font's advance (JetBrains Mono, K6: 600 units to the em, every character). A
/// proportional interface font the user picks averages narrower, so as many of its characters
/// fit.
pub const LABEL_ADVANCE_EM: f32 = 0.6;

/// What rune-kit's tab draws around its title, at the pinned Rune rev
/// (`rune-kit/src/shell/tab_strip/tab.rs`): 6 points of padding each side and 6 after the
/// icon (`px(6.0).gap(6.0)`, line 82), the 16 point icon box (line 205), and the 24 points an
/// active or hovered title keeps clear of the close button (`mr(24.0)`, line 234).
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

/// The tab strip's band height: `tabs.tab.height`, else a bar's height plus the frame inset
/// above and below, as Strata's tab strip grows with the header.
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

/// The narrowest tab: `tabs.tab.min_width`, else room for [`MIN_LABEL_CHARS`] of its label
/// (`kit.ty.ui_compact`, which rune-kit draws titles in) beside its icon, with its close
/// button showing: 136 points at the default 13 point type. It grows with the text size.
pub fn min_width(look: &Look) -> f32 {
    look.slots.tabs.min_width.unwrap_or_else(|| {
        let ty = look.kit.ty.ui_compact;
        let char = ty.size * LABEL_ADVANCE_EM + ty.tracking;
        (MIN_LABEL_CHARS * char + TAB_CHROME).ceil()
    })
}

/// The widest tab: `tabs.tab.max_width`, else [`DEFAULT_MAX_WIDTH`], and never under
/// [`min_width`].
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

/// The colour under the strip, which a dragged tab and the close slot paint opaque:
/// `tabs.strip.fill`, else `kit.c.bg_deep`, the header's.
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

/// The New tab button under a folder drag: `tabs.new.drop_fill`, else the accent at 15 %
/// (Strata's `.tab-drop-hover`).
pub fn new_drop_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .tabs
        .new_drop_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.15))
}

/// A number badge's ground: `tabs.badge.fill`, else the accent at 15 % (Strata's
/// `.tab-key-hint`).
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
