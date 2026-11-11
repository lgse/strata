//! The player layout the model and route views share, and the parts they put in it: a details
//! header (an eyebrow over a title), a badge row and its badges.
//!
//! It follows Strata's `src/ui/preview/player_layout.rs` and `components.rs` (`upstream/main`
//! 35ee677ab): one vertically centred block, so a taller pane keeps the picture and its controls
//! together. From the top:
//!
//! 1. the content, the largest box at its aspect ratio that fits the room the rows below leave
//!    it, at most [`MAX_CONTENT_WIDTH`] wide;
//! 2. the details header;
//! 3. the badges, if any;
//! 4. a timeline, if any (the route view's elevation profile);
//! 5. the controls.
//!
//! The gaps are Strata's: [`Look::space`]`.xl` around the block, `.lg` under the content, `.xs`
//! between the rows.
//!
//! **For C13.** This is the shared player layout (coordinator ruling): C13's audio and video
//! views reuse [`section`] and [`Areas`] rather than building their own, as Strata's do. Its
//! badges and details parts are here too ([`details_header`], [`badge_row`], [`badge`]).
//!
//! **Badges are not live regions.** rune-kit's `Badge` is a polite live region, so a triangle
//! count that fills in would be announced; Strata's badges are plain labels. These are drawn
//! from Kit tokens, with no live region.

use rune::{El, col, row, text};
use strata_kit::look::Look;

/// The widest the content draws, in points: Strata's `MAX_CONTENT_WIDTH`.
pub const MAX_CONTENT_WIDTH: f32 = 1280.0;

/// The narrowest the rows under the content are, in points: Strata's `PANEL_MIN`.
pub const PANEL_MIN: f32 = 200.0;

/// An item's place among its siblings of the same kind, which the host counts (Strata's
/// `ListingPosition`): which entries count, and in what order, is the host's policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// From 1.
    pub index: usize,
    pub count: usize,
    /// Search results stand in for the listing.
    pub results: bool,
}

impl Position {
    /// Strata's caption: "2 of 4 in folder", or "2 of 4 in results".
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
    col()
        .key(key.into())
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(look.space.xl)
        .gap(look.space.lg)
        .center()
        // The width cap is put as the height it gives at the aspect: at Rune 5d421fd an
        // `aspect_fit` box with a `max_w` takes the cap for the cross room, so a box bound by the
        // height overflows a column narrower than the cap (C13's Rune draft U-C13-4).
        .child(
            areas
                .content
                .aspect_fit(areas.aspect)
                .max_h(MAX_CONTENT_WIDTH / areas.aspect.max(f32::EPSILON))
                .min_h(0.0),
        )
        .child(lower)
}

/// Strata's `DetailsHeader`: a muted eyebrow ("2 of 4 in folder") over the title. Without an
/// eyebrow only the title shows.
pub fn details_header(key: &str, eyebrow: Option<String>, title: &str, look: &Look) -> El {
    let k = &look.kit;
    let mut header = col().key(format!("{key}-details")).gap(look.space.xxs);
    if let Some(eyebrow) = eyebrow {
        header = header.child(
            text(eyebrow)
                .key(format!("{key}-eyebrow"))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .truncate(),
        );
    }
    header.child(
        text(title.to_owned())
            .key(format!("{key}-title"))
            .font(look.ty.title)
            .color(k.c.text_base)
            .truncate_middle(),
    )
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
