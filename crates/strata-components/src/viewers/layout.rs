// SPDX-License-Identifier: MIT

use rune::{El, col, row, text};
use strata_kit::look::Look;

/// The widest the content draws, in points: Strata's `MAX_CONTENT_WIDTH`.
pub const MAX_CONTENT_WIDTH: f32 = 1280.0;

/// The narrowest the rows under the content are, in points: Strata's `PANEL_MIN`.
pub const PANEL_MIN: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// From 1.
    pub index: usize,
    pub count: usize,
    /// Search results stand in for the listing.
    pub results: bool,
}

impl Position {
    pub fn caption(self) -> String {
        let place = if self.results { "results" } else { "folder" };
        format!("{} of {} in {place}", self.index, self.count)
    }
}

/// What goes in the layout. Each part is built by its view and keyed by it.
pub struct Areas {
    /// The picture.
    pub content: El,
    /// Width over height for the content's box.
    pub aspect: f32,
    pub details: El,
    pub badges: Option<El>,
    pub timeline: Option<El>,
    pub controls: El,
}

/// The block, filling its parent. Key it with `key`.
pub fn section(key: impl Into<String>, areas: Areas, look: &Look) -> El {
    section_with_inset(key, areas, 0.0, look)
}

pub fn section_with_inset(key: impl Into<String>, areas: Areas, inset: f32, look: &Look) -> El {
    let rows_w = |e: El| e.w_full().max_w(MAX_CONTENT_WIDTH).min_w(PANEL_MIN);
    let mut lower = col()
        .w_full()
        .max_w(MAX_CONTENT_WIDTH)
        .gap(look.space.xs)
        .child(rows_w(areas.details));
    if let Some(badges) = areas.badges {
        lower = lower.child(rows_w(badges));
    }
    if let Some(timeline) = areas.timeline {
        lower = lower.child(rows_w(timeline));
    }
    lower = lower.child(rows_w(areas.controls));
    let mut content = areas
        .content
        .aspect_fit(areas.aspect)
        .max_h(MAX_CONTENT_WIDTH / areas.aspect.max(f32::EPSILON))
        .min_h(0.0);
    if inset > 0.0 {
        content = content.mx(inset).my(inset);
    }
    col()
        .key(key.into())
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(look.space.xl)
        .gap(look.space.lg)
        .center()
        .child(content)
        .child(lower)
}

pub fn details_header(key: &str, eyebrow: Option<String>, title: &str, look: &Look) -> El {
    details_header_beside(key, (eyebrow, None), title, look)
}

pub fn details_header_beside(
    key: &str,
    (eyebrow, beside): (Option<String>, Option<El>),
    title: &str,
    look: &Look,
) -> El {
    let k = &look.kit;
    let mut header = col().key(format!("{key}-details")).gap(look.space.xxs);
    let eyebrow = eyebrow.map(|eyebrow| {
        text(eyebrow)
            .key(format!("{key}-eyebrow"))
            .font(k.ty.small)
            .color(k.c.text_muted)
            .truncate()
    });
    header = match (eyebrow, beside) {
        (eyebrow, Some(beside)) => header.child(eyebrow_row(eyebrow, beside, look)),
        (Some(eyebrow), None) => header.child(eyebrow),
        (None, None) => header,
    };
    header.child(
        text(title.to_owned())
            .key(format!("{key}-title"))
            .font(look.ty.title)
            .color(k.c.text_base)
            .truncate_middle(),
    )
}

/// The eyebrow, or the space it holds, with `beside` at the row's end.
fn eyebrow_row(eyebrow: Option<El>, beside: El, look: &Look) -> El {
    let eyebrow = eyebrow.map_or_else(|| col().min_w(0.0), |eyebrow| eyebrow.min_w(0.0));
    row()
        .w_full()
        .gap(look.space.sm)
        .child(eyebrow.grow(1.0))
        .child(beside)
}

/// A row of badges, keyed `<key>-badges`.
pub fn badge_row(key: &str, badges: impl IntoIterator<Item = El>, look: &Look) -> El {
    row()
        .key(format!("{key}-badges"))
        .gap(look.space.sm)
        .children(badges)
}

/// One badge: a small pill with a label.
pub fn badge(key: impl Into<String>, label: impl Into<String>, look: &Look) -> El {
    let k = &look.kit;
    el_pill(look).child(
        text(label.into())
            .key(key.into())
            .font(k.ty.small_medium)
            .color(k.c.text_muted),
    )
}

fn el_pill(look: &Look) -> El {
    let k = &look.kit;
    row()
        .px(look.space.sm)
        .py(look.space.xxs)
        .radius(look.round(k.radius.sm))
        .fill(k.c.layer_02)
        .center_y()
}
