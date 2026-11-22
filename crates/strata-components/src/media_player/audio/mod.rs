// SPDX-License-Identifier: MIT

//! The audio view (Strata's `src/ui/preview/audio.rs`): art (or the record), the tags, the
//! live spectrum, the waveform timeline and the transport.

pub mod layout;
pub mod spectrum;

use rune::{El, LoopSpec, Shape, Span, SurfaceFit, Tx, canvas, col, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::media as slot;

use super::data::{Art, AudioTags};
use super::state::MediaState;
use super::time;
use super::{Emit, commit_after};
use layout::{ART_ASPECT, PANEL_MAX, plan};

/// The record turns once in this long while playing (`SECONDS_PER_TURN`).
const SECONDS_PER_TURN: f64 = 2.4;
/// The record's label, as a share of its radius (`LABEL`).
const LABEL: f32 = 0.34;
/// The disc's share of its box (`DISC`).
const DISC: f32 = 0.94;
/// The peeking disc's resting offset behind the album cover, as a share of cover side (`TUCKED`).
pub const DISC_TUCKED: f32 = 0.06;
/// The peeking disc's playing offset behind the album cover, as a share of cover side (`SLID_OUT`).
pub const DISC_SLID_OUT: f32 = 0.3;
/// Exponential slide-out rate while playing (`SLIDE_RATE`).
pub(crate) const DISC_SLIDE_RATE: f32 = 7.0;
/// Exponential tuck-in rate when paused (`TUCK_RATE`).
pub(crate) const DISC_TUCK_RATE: f32 = 14.0;

/// Strata's `.preview-audio-title`: 1.85× the base UI text size, extra-bold, tight tracking.
fn audio_title_style(look: &Look) -> rune::TextStyle {
    let size = (look.kit.ty.ui.size * 1.85).round();
    rune::TextStyle {
        size,
        weight: 800.0,
        tracking: -size * 0.01,
        ..look.ty.title
    }
    .lh((size * 1.2).round())
}

/// The height the tags, timeline and transport take: the title line, the byline, the
/// eyebrow, the waveform, the transport and the gaps between them.
fn fixed_height(look: &Look) -> f32 {
    let k = &look.kit;
    let lh = |t: rune::TextStyle| t.line_height.max(t.size * 1.2);
    lh(audio_title_style(look))
        + 2.0 * lh(k.ty.small)
        + slot::waveform_height(look)
        + 32.0
        + 4.0 * look.space.sm
        + 2.0 * look.space.xl
}

pub(crate) fn audio<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    look: &Look,
) -> El {
    let k = &look.kit;
    let p = st.plan();
    let keys_emit = emit.clone();
    let fixed = fixed_height(look);
    let gap = look.space.lg;
    let tags = st.tags();
    let title = tags
        .title
        .clone()
        .unwrap_or_else(|| time::stem(&st.clip().name).to_owned());
    let caption = time::track_caption(
        tags.track,
        tags.track_total,
        st.clip().position,
        st.labels(),
    );
    // Rows keep their space while tags load, so changing tracks doesn't move the layout.
    let volume = super::transport::volume(st, access, emit.clone(), key, look);
    let header = row().w_full().gap(look.space.md).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(look.space.xxs)
            .child(caption_row(key, caption.unwrap_or_default(), volume, look))
            .child(
                text(title)
                    .key(format!("{key}-title"))
                    .font(audio_title_style(look))
                    .color(k.c.text_base)
                    .truncate(),
            )
            .child(byline_el(key, tags, look))
            .child(super::error(st, key, look)),
    );

    let mut panel = col()
        .w_full()
        .max_w(PANEL_MAX)
        .gap(look.space.sm)
        .child(header);
    if let Some(h) = p.spectrum {
        panel = panel.child(spectrum_el(st, access, emit.clone(), key, h, look));
    }
    panel = panel
        .child(super::timeline::timeline(
            st,
            access,
            emit.clone(),
            key,
            false,
            look,
        ))
        .child(super::transport::transport(
            st,
            access,
            emit.clone(),
            key,
            look,
        ));
    let art = p
        .art
        .map(|side| art(st, access, emit.clone(), key, side, look));

    let body = if p.wide {
        row()
            .w_full()
            .gap(gap)
            .center_y()
            .children(art)
            .child(col().grow(1.0).min_w(0.0).child(panel))
    } else {
        col()
            .w_full()
            .gap(gap)
            .items_center()
            .children(art)
            .child(panel)
    };
    // The view takes the player's keys when it has the keyboard, as Strata's drawer content
    // does for audio (`media_key_command`): Right from the listing lands here.
    col()
        .key(key.to_owned())
        .focusable(true)
        .no_focus_ring()
        .on_key(move |s: &mut S, k: &rune::KeyEvent, tx: &mut Tx| {
            let st = access(s);
            let Some(command) = st.keys().focused_command(k, tx) else {
                return false;
            };
            if !st.command(command) {
                return false;
            }
            commit_after(s, access, &keys_emit, tx);
            true
        })
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(look.space.xl)
        .center()
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            let st = access(s);
            let next = plan(r[2], r[3], fixed, gap, st.was_wide());
            if !st.set_audio_box(next) {
                tx.live_only();
            }
        })
        .child(body)
}

/// The artist (accent, bold) and album (muted), matching `.preview-audio-artist` and
/// `.preview-audio-album` in a single accessible text node.
fn byline_el(key: &str, tags: &AudioTags, look: &Look) -> El {
    let k = &look.kit;
    let byline = time::byline(tags.artist.as_deref(), tags.album.as_deref());
    let mut e = text(byline)
        .key(format!("{key}-byline"))
        .font(k.ty.small)
        .color(k.c.text_muted)
        .truncate()
        .min_h(k.ty.small.size);
    if let Some(artist) = tags.artist.as_deref()
        && !artist.is_empty()
    {
        e = e.spans([Span::new(0, artist.len())
            .weight(700.0)
            .color(k.c.text_accent)]);
    }
    e
}

/// The art (square sleeve with peeking grooved vinyl disc behind its right side), or the
/// centered record when the track has none. A press toggles playback. Art keeps its key
/// while its id stays, so a track with the same art keeps it in place.
fn art<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    side: f32,
    look: &Look,
) -> El {
    let e = match st.art() {
        Some(a) => sleeve_art(st, a, key, side, look),
        None => record(st, key, side, look),
    };
    e.role("image")
        .label(st.labels().artwork.clone())
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).toggle();
            commit_after(s, access, &emit, tx);
        })
}

/// Square album cover on the left with the grooved vinyl disc peeking from behind its right
/// side (`artwork.rs`: `ASPECT = 1.3`, sliding from `TUCKED` when paused to `SLID_OUT` while
/// playing via `live_offset`).
fn sleeve_art(st: &MediaState, a: &Art, key: &str, side: f32, look: &Look) -> El {
    let k = &look.kit;
    let radius = (side * 0.035).clamp(4.0, 14.0);
    let width = (side * ART_ASPECT).round();
    let peeking_disc = canvas(disc_shape(side, look))
        .key(format!("{key}-peeking-disc"))
        .size(side, side)
        .pin(Some(0.0), Some(0.0), None, None)
        .live_offset(&st.live.disc)
        .a11y_hidden();
    let sleeve = el()
        .size(side, side)
        .pin(Some(0.0), Some(0.0), None, None)
        .fill(k.c.bg_base)
        .radius(radius)
        .shadow(side * 0.03, side * 0.07, k.alpha(k.c.black, 0.55))
        .child(cover_image(a, side, radius))
        .child(
            el().size(side, side)
                .pin(Some(0.0), Some(0.0), None, None)
                .radius(radius)
                .border(1.0, k.alpha(k.c.text_base, 0.08))
                .a11y_hidden(),
        );
    el().key(format!("{key}-art-{}", a.id))
        .size(width, side)
        .child(peeking_disc)
        .child(sleeve)
}

/// Center-cropped cover image clipped to the sleeve's rounded corners (`paint_cover`).
fn cover_image(a: &Art, side: f32, radius: f32) -> El {
    let [w, h] = a.size;
    let (drawn_w, drawn_h) = if w > 0 && h > 0 {
        let scale = (side / w as f32).max(side / h as f32);
        (w as f32 * scale, h as f32 * scale)
    } else {
        (side, side)
    };
    el().size(side, side)
        .clip_rounded(radius)
        .center()
        .a11y_hidden()
        .child(
            el().size(drawn_w, drawn_h)
                .pixels_fit(&a.pixels, SurfaceFit::Fill)
                .a11y_hidden(),
        )
}

/// The grooved vinyl disc shape (`paint_disc` + `paint_label`).
fn disc_shape(side: f32, look: &Look) -> Shape {
    let r = side * DISC / 2.0;
    let c = side / 2.0;
    let circle = |radius: f32| -> Vec<[f32; 2]> {
        (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                [c + radius * a.cos(), c + radius * a.sin()]
            })
            .collect()
    };
    let groove = slot::record_groove(look);
    let rim = look.kit.alpha(groove, (groove.a * 2.8).min(1.0));
    let mut shape = Shape::new().fill(circle(r), slot::record_disc(look));
    let mut ring = 0.42;
    while ring < 0.97 {
        let mut pts = circle(r * ring);
        pts.push(pts[0]);
        shape = shape.stroke(pts, 1.0, groove);
        ring += 0.055;
    }
    let mut outline = circle(r);
    outline.push(outline[0]);
    shape
        .stroke(outline, 1.0, rim)
        .fill(circle(r * LABEL), slot::record_label(look))
        .fill(circle((r * 0.035).max(1.5)), look.kit.c.bg_base)
}

/// Strata's record (`paint_disc`, `paint_label`), simplified (deviation D4): a dark disc with
/// grooves and the accent label, whose mark turns while playing. Only icon quads can turn
/// without a view commit at this Rune rev (`LoopSpec::rotation`), so the turning mark is an
/// icon; under reduced motion it holds still.
fn record(st: &MediaState, key: &str, side: f32, look: &Look) -> El {
    let r = side * DISC / 2.0;
    let mark = r * LABEL * 1.2;
    let mut turning = icon(look.icon(IconName::Music), mark)
        .key(format!("{key}-record-mark"))
        .color(slot::spectrum_bright(look))
        .a11y_hidden();
    if st.playing() && st.motion.get() {
        turning = turning.looping(LoopSpec::rotation(SECONDS_PER_TURN));
    }
    el().key(format!("{key}-record"))
        .size(side, side)
        .center()
        .child(
            canvas(disc_shape(side, look))
                .size(side, side)
                .pin(Some(0.0), Some(0.0), None, None)
                .a11y_hidden(),
        )
        .child(turning)
}

/// The spectrum: bars, peak caps and reflections bound to live values. Its bar count follows
/// its width; a press toggles playback.
fn spectrum_el<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    h: f32,
    look: &Look,
) -> El {
    let (base, bright) = (slot::spectrum_base(look), slot::spectrum_bright(look));
    let bar_w = slot::spectrum_bar(look);
    let gap = slot::spectrum_gap(look);
    let peak = slot::spectrum_peak(look);
    let reflection = slot::spectrum_reflection(look);
    let clear = look.kit.alpha(reflection, 0.0);
    let base_live = rune::Live::new(base);
    st.spectrum.set_colors(base, bright);
    let mut e = el()
        .key(format!("{key}-spectrum"))
        .role("image")
        .label(st.labels().spectrum.clone())
        .w_full()
        .h(h)
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            let st = access(s);
            if !st.spectrum.set_size([r[2], r[3]], bar_w, gap) {
                tx.live_only();
            }
        })
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).toggle();
            commit_after(s, access, &emit, tx);
        });
    for (i, b) in st.spectrum.bars.iter().enumerate() {
        let at = |e: El| e.pin(Some(0.0), Some(0.0), None, None).a11y_hidden();
        e = e
            .child(at(el()
                .key(format!("{key}-bar-{i}"))
                .gradient(base, base)
                .live_rect(&b.bar)
                .live_gradient(&b.top, &base_live)))
            .child(at(el()
                .gradient(reflection, clear)
                .live_rect(&b.reflection)))
            .child(at(el().fill(peak).live_rect(&b.peak)));
    }
    e
}

/// The caption with the volume at its end, the title and byline going under both (Strata's
/// `DetailsHeader`).
fn caption_row(key: &str, caption: String, volume: El, look: &Look) -> El {
    let k = &look.kit;
    row()
        .w_full()
        .gap(look.space.sm)
        .child(
            text(caption)
                .key(format!("{key}-eyebrow"))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .truncate()
                .min_h(k.ty.small.size)
                .min_w(0.0)
                .grow(1.0),
        )
        .child(volume)
}
