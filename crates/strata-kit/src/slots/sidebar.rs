//! The `sidebar.*` slot tokens (for example `sidebar.row`) that PlacesSidebar reads.
//!
//! Work package: C8, `docs/work/phase-1-leaves/C8-places-sidebar.md`.
//!
//! W2 seeded `sidebar.row.height` so K4 has a slot to override before C8 lands. C8 owns this
//! file. Slot names are API: renaming one breaks users' override files (plan K4).
//!
//! Every default follows the rest of the [`Look`]: colours from `kit.c`, lengths from `row`,
//! `space`, `frame` and `icon_size`. The one exception is `sidebar.pane.width`, Strata's own
//! default sidebar width, which no other token says.

use rune::Color;

use crate::look::Look;
use crate::tokens::Token;

/// Strata's default full sidebar width (`SIDEBAR_WIDTH` in `src/ui/window/sidebar_width.rs`,
/// lgse/strata `upstream/main` 35ee677ab).
pub const DEFAULT_PANE_WIDTH: f32 = 201.0;

/// The inset rune-kit's `SidebarRow` puts before its icon (its private `INSET`, at the pinned
/// Rune rev). The rail's own row uses it too, so a step moves no icon.
pub const ROW_INSET: f32 = 8.0;

/// The box rune-kit's `SidebarRow` draws its icon in (its 16 point avatar box; C8's U22 asks for
/// it to be settable). The rail's icon box matches it at any text size, so a step moves no
/// icon even when K2 scales the icons past it.
pub const ICON_BOX: f32 = 16.0;

/// PlacesSidebar's slot tokens. `None` inherits.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Sidebar {
    pub row_height: Option<f32>,
    pub row_gap: Option<f32>,
    pub selected_fill: Option<Color>,
    pub selected_icon: Option<Color>,
    pub hover_fill: Option<Color>,
    pub divider_color: Option<Color>,
    pub divider_inset: Option<f32>,
    pub gap_open: Option<f32>,
    pub gap_reach: Option<f32>,
    pub heading_height: Option<f32>,
    pub rail_width: Option<f32>,
    pub pane_width: Option<f32>,
}

/// A sidebar row's height: `sidebar.row.height`, else `row.sidebar`.
pub fn row_height(look: &Look) -> f32 {
    look.slots.sidebar.row_height.unwrap_or(look.row.sidebar)
}

/// The space between two rows, which a pin gap fills: `sidebar.row.gap`, else `space.xxs`.
pub fn row_gap(look: &Look) -> f32 {
    look.slots.sidebar.row_gap.unwrap_or(look.space.xxs)
}

/// The current row's fill: `sidebar.selected.fill`, else the accent at 18 %.
pub fn selected_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .sidebar
        .selected_fill
        .unwrap_or_else(|| k.alpha(k.c.bg_accent, 0.18))
}

/// The current row's icon colour: `sidebar.selected.icon`, else `kit.c.icon_accent`.
pub fn selected_icon(look: &Look) -> Color {
    look.slots
        .sidebar
        .selected_icon
        .unwrap_or(look.kit.c.icon_accent)
}

/// A hovered row's or heading's fill: `sidebar.hover.fill`, else the text colour at 6 %.
pub fn hover_fill(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .sidebar
        .hover_fill
        .unwrap_or_else(|| k.alpha(k.c.text_base, 0.06))
}

/// Drive dividers, heading rules and the rail's section separators: `sidebar.divider.color`,
/// else the base border at 24 %.
pub fn divider_color(look: &Look) -> Color {
    let k = &look.kit;
    look.slots
        .sidebar
        .divider_color
        .unwrap_or_else(|| k.alpha(k.c.border_base, 0.24))
}

/// How far a drive divider stays in from each side of the full sidebar:
/// `sidebar.divider.inset`, else `space.md + space.xxs` (Strata's 10 px).
pub fn divider_inset(look: &Look) -> f32 {
    look.slots
        .sidebar
        .divider_inset
        .unwrap_or(look.space.md + look.space.xxs)
}

/// A pin gap's height while it takes a drop: `sidebar.gap.open`, else half of `row.sidebar`.
/// Slot tokens inherit only from core tokens, so it doesn't follow `sidebar.row.height`.
pub fn gap_open(look: &Look) -> f32 {
    look.slots
        .sidebar
        .gap_open
        .unwrap_or(look.row.sidebar / 2.0)
}

/// How far a pin gap's drop area reaches over each neighbouring row's edge:
/// `sidebar.gap.reach`, else `space.sm`.
pub fn gap_reach(look: &Look) -> f32 {
    look.slots.sidebar.gap_reach.unwrap_or(look.space.sm)
}

/// A PINNED or DEVICES heading's height: `sidebar.heading.height`, else `frame.control`.
pub fn heading_height(look: &Look) -> f32 {
    look.slots
        .sidebar
        .heading_height
        .unwrap_or(look.frame.control)
}

/// The icon rail's width: `sidebar.rail.width`, else the row icon's box with the row's and
/// the list's insets on each side (`ICON_BOX + 2 × (ROW_INSET + space.xs)`).
pub fn rail_width(look: &Look) -> f32 {
    look.slots
        .sidebar
        .rail_width
        .unwrap_or(ICON_BOX + 2.0 * (ROW_INSET + look.space.xs))
}

/// The full sidebar's starting width, before the user resizes it: `sidebar.pane.width`, else
/// [`DEFAULT_PANE_WIDTH`].
pub fn pane_width(look: &Look) -> f32 {
    look.slots.sidebar.pane_width.unwrap_or(DEFAULT_PANE_WIDTH)
}

pub const TOKENS: &[Token] = &[
    Token::length("sidebar.row.height", row_height, |l, v| {
        l.slots.sidebar.row_height = Some(v)
    }),
    Token::length("sidebar.row.gap", row_gap, |l, v| {
        l.slots.sidebar.row_gap = Some(v)
    }),
    Token::color("sidebar.selected.fill", selected_fill, |l, v| {
        l.slots.sidebar.selected_fill = Some(v)
    }),
    Token::color("sidebar.selected.icon", selected_icon, |l, v| {
        l.slots.sidebar.selected_icon = Some(v)
    }),
    Token::color("sidebar.hover.fill", hover_fill, |l, v| {
        l.slots.sidebar.hover_fill = Some(v)
    }),
    Token::color("sidebar.divider.color", divider_color, |l, v| {
        l.slots.sidebar.divider_color = Some(v)
    }),
    Token::length("sidebar.divider.inset", divider_inset, |l, v| {
        l.slots.sidebar.divider_inset = Some(v)
    }),
    Token::length("sidebar.gap.open", gap_open, |l, v| {
        l.slots.sidebar.gap_open = Some(v)
    }),
    Token::length("sidebar.gap.reach", gap_reach, |l, v| {
        l.slots.sidebar.gap_reach = Some(v)
    }),
    Token::length("sidebar.heading.height", heading_height, |l, v| {
        l.slots.sidebar.heading_height = Some(v)
    }),
    Token::length("sidebar.rail.width", rail_width, |l, v| {
        l.slots.sidebar.rail_width = Some(v)
    }),
    Token::length("sidebar.pane.width", pane_width, |l, v| {
        l.slots.sidebar.pane_width = Some(v)
    }),
];
