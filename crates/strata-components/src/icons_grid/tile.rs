// SPDX-License-Identifier: MIT

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Align, Color, El, SurfaceFit, col, el, row, text};
use strata_kit::slots::icons as tokens;

use super::Shared;
use super::caption::Cut;
use super::metrics::TileMetrics;
use crate::collection::element::{ContentSlot, ICON, LABEL, RowCx, Slot};
use crate::thumbnails::{Step, THUMB, entry_glyph, icon_opacity};

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

    pub fn key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        self.m.key().hash(&mut h);
        rune::text::font_epoch().hash(&mut h);
        self.caption_w.to_bits().hash(&mut h);
        self.step.hash(&mut h);
        h.finish()
    }
}

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

fn placeholder(rcx: &RowCx<'_>, points: f32) -> El {
    let look = rcx.look;
    let default_color = tokens::frame_icon(look);
    let glyph = match crate::thumbnails::mark_icon(rcx.state.marks) {
        Some(mark) => rune::icon(look.icon(mark), points)
            .key(crate::thumbnails::GLYPH)
            .color(default_color)
            .a11y_hidden(),
        None => entry_glyph(look, rcx.entry, points, default_color),
    };
    match icon_opacity(rcx, true) {
        opacity if opacity < 1.0 => glyph.opacity(opacity),
        _ => glyph,
    }
}

fn tile_icon_version<S>(shared: &Shared<S>, key: u64, s: &S, rcx: &RowCx<'_>) -> u64 {
    let mark = crate::thumbnails::mark_icon(rcx.state.marks);
    let thumb = rcx.entry.thumbnail.filter(|_| rcx.entry.glyph.is_none());
    let mut h = std::hash::DefaultHasher::new();
    key.hash(&mut h);
    mark.hash(&mut h);
    let parts = (shared.parts)(s);
    if mark.is_none()
        && let (Some(k), Some((store, _))) = (thumb, parts.thumbs)
    {
        let ver = if parts.grid.thumbs_deferred() {
            store.existing_version(k)
        } else {
            store.version(k)
        };
        ver.hash(&mut h);
    }
    shared
        .badge
        .as_ref()
        .map_or(0, |(ver, _)| ver(s, rcx.entry.id))
        .hash(&mut h);
    h.finish()
}

pub(super) fn icon_slot<S: 'static>(shared: &Rc<Shared<S>>, b: Built) -> Slot<S> {
    let (version_shared, build_shared) = (shared.clone(), shared.clone());
    let key = b.key();
    Slot::new(
        move |s: &S, rcx: &RowCx<'_>| tile_icon_version(&version_shared, key, s, rcx),
        move |s: &S, rcx: &RowCx<'_>| {
            let m = &b.m;
            let look = rcx.look;
            let marked = crate::thumbnails::mark_icon(rcx.state.marks).is_some();
            let glyph = placeholder(rcx, m.slot);
            let (fill, ring, hover) = frame_look(rcx);
            let mut frame = row()
                .key(ICON)
                .w(m.frame)
                .h(m.frame)
                .center()
                .radius(tokens::frame_radius(look))
                .a11y_hidden();
            let parts = (build_shared.parts)(s);
            let thumb = rcx.entry.thumbnail.filter(|_| rcx.entry.glyph.is_none());
            let resolved = match (thumb, parts.thumbs) {
                (Some(key), Some((store, _))) if store.enabled() && !marked => {
                    if parts.grid.thumbs_deferred() {
                        store.existing_thumb(key)
                    } else {
                        Some(store.thumb(key))
                    }
                }
                _ => None,
            };
            frame = match resolved {
                Some(thumb) => frame.child(glyph.live_opacity(thumb.icon_alpha())).child(
                    el().key(THUMB)
                        .absolute(m.frame_pad, m.frame_pad)
                        .w(m.slot)
                        .h(m.slot)
                        .pixels_fit(thumb.pixels(), SurfaceFit::Fit),
                ),
                None => frame.child(glyph),
            };
            let px = build_shared
                .badge
                .as_ref()
                .and_then(|(_, f)| f(s, rcx.entry.id));
            frame = crate::thumbnails::overlay_badge(frame, px, (m.frame_pad, m.slot));
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
            let matches = rcx.state.marks.name_matches;
            let spans = matches.onto(rcx.entry.name, &cut.text).spans(look);
            let mut name = text(cut.text.to_string())
                .spans(spans)
                .key(LABEL)
                .a11y_hidden()
                .font(m.name_font)
                .color(color)
                .align(Align::Center)
                .wrap_anywhere()
                .max_lines(m.name_lines.max(1))
                .px(m.caption_pad)
                .py(m.caption_pad / 2.0)
                .radius(tokens::caption_radius(look))
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

/// The hover tooltip text for a tile, none while it renames, drags, or anchors a folder peek.
fn tile_help<S>(shared: &Shared<S>, b: &Built, s: &S, rcx: &RowCx<'_>) -> Option<String> {
    if rcx.state.renaming
        || rcx.state.marks.peek_anchor
        || rcx.state.marks.dragging
        || rcx.entry.name.is_empty()
    {
        return None;
    }
    let truncated = b.cut(shared, s, rcx.entry.name).cut;
    match &shared.tooltip {
        Some((_, text)) => text(s, rcx, truncated).filter(|t| !t.is_empty()),
        None => truncated.then(|| rcx.entry.name.to_owned()),
    }
}

pub(super) fn content_slot<S: 'static>(shared: &Rc<Shared<S>>, b: Built) -> ContentSlot<S> {
    let (version_shared, build_shared) = (shared.clone(), shared.clone());
    let key = b.key();
    ContentSlot::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut version = key;
            if let Some((details_version, _)) = &version_shared.details {
                version ^= details_version(s, rcx).rotate_left(17);
            }
            if let Some((tooltip_version, _)) = &version_shared.tooltip {
                version ^= tooltip_version(s, rcx).rotate_left(31);
            }
            version
        },
        move |s: &S, rcx: &RowCx<'_>, frame: El, label: El| {
            let m = &b.m;
            let look = rcx.look;
            let help = tile_help(&build_shared, &b, s, rcx);
            let (frame, label) = match &help {
                Some(name) => (frame.help(name.clone()), label.help(name.clone())),
                None => (frame, label),
            };
            let inner = (b.cell_w - 2.0 * (m.margin + m.card_x)).max(0.0);
            let mut card = col()
                .w_full()
                .h_full()
                .items_center()
                .child(frame.mt(m.margin + m.card_y));
            if m.name_lines > 0 {
                let label = if rcx.state.renaming {
                    col().w(inner).h(m.caption_height()).child(label.w_full())
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
