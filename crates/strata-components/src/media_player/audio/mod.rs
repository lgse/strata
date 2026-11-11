//! The audio view (Strata's `src/ui/preview/audio.rs`): art (or the record), the tags, the
//! live spectrum, the waveform timeline and the transport.

pub mod layout;
pub mod spectrum;

use rune::{El, LoopSpec, Shape, SurfaceFit, Tx, canvas, col, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::media as slot;

use super::state::MediaState;
use super::time;
use super::{Emit, commit_after};
use layout::{PANEL_MAX, plan};

/// The record turns once in this long while playing (`SECONDS_PER_TURN`).
const SECONDS_PER_TURN: f64 = 2.4;
/// The record's label, as a share of its radius (`LABEL`).
const LABEL: f32 = 0.34;
/// The disc's share of its box (`DISC`).
const DISC: f32 = 0.94;

/// The height the tags, timeline and transport take: the title line, the byline, the
/// eyebrow, the waveform, the transport and the gaps between them.
fn fixed_height(look: &Look) -> f32 {
    let k = &look.kit;
    let lh = |t: rune::TextStyle| t.line_height.max(t.size * 1.2);
    lh(look.ty.title)
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
    let byline = time::byline(tags.artist.as_deref(), tags.album.as_deref());
    // Rows keep their space while tags load, so changing tracks doesn't move the layout.
    let header = row()
        .w_full()
        .gap(look.space.md)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(look.space.xxs)
                .child(
                    text(caption.unwrap_or_default())
                        .key(format!("{key}-eyebrow"))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .truncate()
                        .min_h(k.ty.small.size),
                )
                .child(
                    text(title)
                        .key(format!("{key}-title"))
                        .font(look.ty.title)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(
                    text(byline)
                        .key(format!("{key}-byline"))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .truncate()
                        .min_h(k.ty.small.size),
                )
                .child(super::error(st, key, look)),
        )
        .child(super::transport::volume(
            st,
            access,
            emit.clone(),
            key,
            look,
        ));

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
    col()
        .key(key.to_owned())
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

/// The art, or the record when the track has none. A press toggles playback. Art keeps its
/// key while its id stays, so a track with the same art keeps it in place; new art mounts
/// fresh (deviation D4: Strata crossfades).
fn art<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    side: f32,
    look: &Look,
) -> El {
    let k = &look.kit;
    let mut e = match st.art() {
        Some(a) => {
            let radius = (side * 0.035).clamp(4.0, 14.0);
            el().key(format!("{key}-art-{}", a.id))
                .size(side, side)
                .radius(radius)
                .clip_rounded(radius)
                .border(1.0, k.alpha(k.c.text_base, 0.08))
                .pixels_fit(&a.pixels, SurfaceFit::Fill)
        }
        None => record(st, key, side, look),
    };
    e = e
        .role("image")
        .label(st.labels().artwork.clone())
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).toggle();
            commit_after(s, access, &emit, tx);
        });
    e
}

/// Strata's record (`paint_disc`, `paint_label`), simplified (deviation D4): a dark disc with
/// grooves and the accent label, whose mark turns while playing. Only icon quads can turn
/// without a view commit at this Rune rev (`LoopSpec::rotation`), so the turning mark is an
/// icon; under reduced motion it holds still.
fn record(st: &MediaState, key: &str, side: f32, look: &Look) -> El {
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
    let mut shape = Shape::new().fill(circle(r), slot::record_disc(look));
    let mut ring = 0.42;
    while ring < 0.97 {
        let mut pts = circle(r * ring);
        pts.push(pts[0]);
        shape = shape.stroke(pts, 1.0, groove);
        ring += 0.055;
    }
    shape = shape
        .fill(circle(r * LABEL), slot::record_label(look))
        .fill(circle((r * 0.035).max(1.5)), look.kit.c.bg_base);
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
            canvas(shape)
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
