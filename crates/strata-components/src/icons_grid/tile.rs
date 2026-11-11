//! A tile: the thumbnail frame (C1e's icon slot), the name (its label slot) and the card that
//! stacks them with the details line (its content slot). The collection draws no chrome of
//! its own around a tile (`Collection::bare_items`): the frame and the name light up for the
//! tile's state, as Strata's `.file-icons` rules light `.icons-card-icon-frame` and
//! `.icons-card-label`, never the gutters between tiles.
//!
//! A line mounts all its tiles in one frame, and every element costs there (C3 review round
//! 1, B1: a 32 pt grid's scroll frames), so a tile is kept to the fewest: the card, which is
//! the cell itself (`Collection::bare_items`), the frame with the icon and the image in it, the
//! name and the details line, five or six. The gap and the reserved lines are margins, not
//! wrappers, and the name is sized from its measured cut, so layout never measures it.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Align, Color, El, SurfaceFit, col, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::slots::icons as tokens;

use super::Shared;
use super::caption::Cut;
use super::metrics::TileMetrics;
use crate::collection::element::{ContentSlot, ICON, LABEL, RowCx, Slot};
use crate::thumbnails::{Step, THUMB};

/// The key of a tile's details line, under its cell (`row-{id}/details`).
pub const DETAILS: &str = "details";

/// What one build worked out for every tile.
#[derive(Clone, Copy, Debug)]
pub(super) struct Built {
    pub m: TileMetrics,
    /// A cell's width.
    pub cell_w: f32,
    /// The width a name wraps at.
    pub caption_w: f32,
    /// The thumbnail step the grid draws at.
    pub step: Step,
}

impl Built {
    /// `name` as this build's captions show it.
    pub fn cut<S>(&self, shared: &Shared<S>, s: &S, name: &str) -> Cut {
        let parts = (shared.parts)(s);
        parts.grid.captions.get(
            name,
            self.m.name_font,
            self.caption_w,
            self.m.name_lines.max(1),
        )
    }

    /// A hash of everything this build draws every tile with: its sizes, the caption width
    /// and the fonts' epoch (a name's cut follows from them and the name, which the row's own
    /// version holds: a fallback font loading re-cuts names at the next build), and the
    /// thumbnail step.
    pub fn key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        self.m.key().hash(&mut h);
        rune::text::font_epoch().hash(&mut h);
        self.caption_w.to_bits().hash(&mut h);
        self.step.hash(&mut h);
        h.finish()
    }
}

/// The frame's look for a tile: its state's fill and outline, and its hover fill, none in
/// keyboard mode (`RowCx::hover_live` off) until the pointer moves.
pub(super) fn frame_look(rcx: &RowCx<'_>) -> (Option<Color>, Option<(f32, Color)>, Option<Color>) {
    let (look, st, hover) = (rcx.look, rcx.state, rcx.hover_live);
    let k = &look.kit;
    if st.drop_target {
        return (
            Some(tokens::frame_drop_fill(look)),
            Some((1.0, tokens::frame_focus_ring(look))),
            None,
        );
    }
    let ring = if st.cursor_shown {
        Some((2.0, tokens::frame_focus_ring(look)))
    } else if st.marks.enter_target {
        Some((1.0, k.c.text_accent))
    } else if st.selected {
        Some((1.0, tokens::frame_selected_ring(look)))
    } else {
        None
    };
    if st.selected {
        return (
            Some(tokens::frame_selected_fill(look)),
            ring,
            hover.then(|| tokens::frame_selected_hover_fill(look)),
        );
    }
    (None, ring, hover.then(|| tokens::frame_hover_fill(look)))
}

/// The name box's look for a tile: its fill and text colour when selected or a drop target,
/// and its hover fill otherwise, none in keyboard mode until the pointer moves.
pub(super) fn name_look(rcx: &RowCx<'_>) -> (Option<Color>, Color, Option<Color>) {
    let (look, st) = (rcx.look, rcx.state);
    if st.selected || st.drop_target {
        (
            Some(tokens::caption_selected_fill(look)),
            tokens::caption_selected_text(look),
            None,
        )
    } else {
        (
            None,
            look.kit.c.text_base,
            rcx.hover_live.then(|| tokens::caption_hover_fill(look)),
        )
    }
}

/// The entry's catalog icon, else the Folder or File icon, in the frame's icon colour.
fn placeholder(rcx: &RowCx<'_>, points: f32) -> El {
    let look = rcx.look;
    let fallback = if rcx.entry.kind.is_folder_like() {
        IconName::Folder
    } else {
        IconName::File
    };
    let glyph = look
        .icon_named(rcx.entry.icon)
        .unwrap_or_else(|| look.icon(fallback));
    icon(glyph, points).color(tokens::frame_icon(look))
}

/// The icon slot: the frame (keyed [`ICON`], so C23's flights fly it, selection and all), with
/// the entry's placeholder icon centred in it and, when the entry has a thumbnail key and
/// thumbnails show, its image over the slot. As C25's `picture` draws them: the image is fitted
/// (letterboxed) in the slot, and the icon fades out through the handle's opacity while an image
/// is held, so a thumbnail landing, changing size or going needs no build.
///
/// Versioned by the store's handles for the key and the step, never the key's state.
pub(super) fn icon_slot<S: 'static>(shared: &Rc<Shared<S>>, b: Built) -> Slot<S> {
    let (version_shared, build_shared) = (shared.clone(), shared.clone());
    let key = b.key();
    Slot::new(
        move |s: &S, rcx: &RowCx<'_>| match (rcx.entry.thumbnail, (version_shared.parts)(s).thumbs)
        {
            (Some(k), Some((store, _))) => key ^ store.version(k).rotate_left(29),
            _ => key,
        },
        move |s: &S, rcx: &RowCx<'_>| {
            let m = &b.m;
            let look = rcx.look;
            let glyph = placeholder(rcx, m.slot);
            let (fill, ring, hover) = frame_look(rcx);
            let mut frame = row()
                .key(ICON)
                .w(m.frame)
                .h(m.frame)
                .center()
                .radius(tokens::frame_radius(look))
                .a11y_hidden();
            frame = match (rcx.entry.thumbnail, (build_shared.parts)(s).thumbs) {
                (Some(key), Some((store, _))) if store.enabled() => {
                    let thumb = store.thumb(key);
                    frame.child(glyph.live_opacity(thumb.icon_alpha())).child(
                        el().key(THUMB)
                            .absolute(m.frame_pad, m.frame_pad)
                            .w(m.slot)
                            .h(m.slot)
                            .pixels_fit(thumb.pixels(), SurfaceFit::Fit),
                    )
                }
                _ => frame.child(glyph),
            };
            if let Some(fill) = fill {
                frame = frame.fill(fill);
            }
            if let Some((w, c)) = ring {
                frame = frame.outline(w, -w, c);
            }
            if let Some(c) = hover {
                frame = frame.hover(c);
            }
            frame
        },
    )
}

/// The label slot: the name, cut to its lines, in a padded box that lights up for the tile's
/// state, with the space its lines leave below it as margin, so the details line sits where
/// the name's reserved lines end. Rename's editor takes its place while the tile renames
/// (C1e's rename slot).
pub(super) fn label_slot<S: 'static>(shared: &Rc<Shared<S>>, b: Built) -> Slot<S> {
    let build_shared = shared.clone();
    let key = b.key();
    Slot::new(
        move |_: &S, _: &RowCx<'_>| key,
        move |s: &S, rcx: &RowCx<'_>| {
            let m = &b.m;
            let look = rcx.look;
            let cut = b.cut(&build_shared, s, rcx.entry.name);
            let (fill, color, hover) = name_look(rcx);
            let below = (m.caption_height() - (cut.size.1 + m.caption_pad)).max(0.0);
            let mut name = text(cut.text.to_string())
                .key(LABEL)
                .a11y_hidden()
                .font(m.name_font)
                .color(color)
                .align(Align::Center)
                .wrap_anywhere()
                .max_lines(m.name_lines.max(1))
                .px(m.caption_pad)
                .py(m.caption_pad / 2.0)
                .mb(below)
                .radius(tokens::caption_radius(look))
                // Its measured size, so layout doesn't measure the text again for its min- and
                // max-content widths.
                .w(cut.size.0 + 2.0 * m.caption_pad)
                .h(cut.size.1 + m.caption_pad);
            if let Some(fill) = fill {
                name = name.fill(fill);
            }
            if let Some(hover) = hover {
                name = name.hover(hover);
            }
            name
        },
    )
}

/// The details line's text for a tile, from the host.
pub(super) fn details_of<S>(shared: &Shared<S>, s: &S, rcx: &RowCx<'_>) -> Option<String> {
    let (_, f) = shared.details.as_ref()?;
    f(s, rcx).map(|d| d.to_string())
}

/// The content slot: the card, its frame, name and details stacked and centred, inside the
/// tile's margin and the card's padding. A tile whose name is cut carries the whole name as a
/// tooltip over its frame, its name and its details, none while it renames or a folder peek is
/// open from it (C22).
pub(super) fn content_slot<S: 'static>(shared: &Rc<Shared<S>>, b: Built) -> ContentSlot<S> {
    let (version_shared, build_shared) = (shared.clone(), shared.clone());
    let key = b.key();
    ContentSlot::new(
        move |s: &S, rcx: &RowCx<'_>| match &version_shared.details {
            Some((version, _)) => key ^ version(s, rcx).rotate_left(17),
            None => key,
        },
        move |s: &S, rcx: &RowCx<'_>, frame: El, label: El| {
            let m = &b.m;
            let look = rcx.look;
            // A tile a folder peek is open from (C22) shows no tooltip: the card beside it holds
            // the folder's name, and a tooltip would cover one or the other.
            let help = (b.cut(&build_shared, s, rcx.entry.name).cut
                && !rcx.state.renaming
                && !rcx.state.marks.peek_anchor)
                .then(|| rcx.entry.name.to_owned());
            let (frame, label) = match &help {
                Some(name) => (frame.help(name.clone()), label.help(name.clone())),
                None => (frame, label),
            };
            // The card takes no padding of its own: it is the cell, and a padded cell would be
            // wider than the empty cells that fill a short last row, so the cells of that row
            // wouldn't share its width equally as the geometry says. Its children keep inside
            // the card's inner width instead.
            let inner = (b.cell_w - 2.0 * (m.margin + m.card_x)).max(0.0);
            let mut card = col()
                .w_full()
                .h_full()
                .items_center()
                .child(frame.mt(m.margin + m.card_y));
            if m.name_lines > 0 {
                // The rename editor (C5's) keeps the name's reserved lines, so the details line
                // stays put.
                let label = if rcx.state.renaming {
                    col()
                        .w(inner)
                        .h(m.caption_height())
                        .items_center()
                        .child(label)
                } else {
                    label
                };
                card = card.child(label.mt(m.caption_gap));
            }
            if m.details_line > 0.0
                && let Some(details) = details_of(&build_shared, s, rcx).filter(|d| !d.is_empty())
            {
                let mut line = text(details)
                    .key(DETAILS)
                    .a11y_hidden()
                    .font(m.details_font)
                    .color(look.kit.c.text_muted)
                    .align(Align::Center)
                    .truncate()
                    .h(m.details_line)
                    .px(m.caption_pad)
                    .max_w(inner)
                    .min_w(0.0)
                    .shrink();
                if let Some(name) = help {
                    line = line.help(name);
                }
                card = card.child(line);
            }
            card
        },
    )
}
