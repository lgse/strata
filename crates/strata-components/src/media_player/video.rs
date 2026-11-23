// SPDX-License-Identifier: MIT

use rune::{Cx, El, Pixels, SurfaceFit, Tx, col, el, text};
use strata_kit::look::Look;
use strata_kit::slots::media as slot;

use super::ambient;
use super::badges::badges;
use super::data::{Details, MediaKind};
use super::state::MediaState;
use super::time;
use super::{Emit, commit_after};
use crate::viewers::layout::{self, Areas};
use crate::viewers::viewport::{Interaction, Viewport, ViewportState};

/// The stand-in poster's opacity (`frame.rs`, `POSTER_OPACITY`).
const POSTER_OPACITY: f32 = 0.55;
/// The seek cover's opacity (`video.rs`, `SEEK_COVER_OPACITY`).
const SEEK_COVER_OPACITY: f32 = 0.9;
/// The bubble's margin over the frame's bottom (`BUBBLE_MARGIN`).
const BUBBLE_MARGIN: f32 = 10.0;
/// A bubble with no storyboard is this wide on its long edge (`CELL_EDGE`).
const CELL_EDGE: f32 = 128.0;
/// Empty pills while the probe runs (`SKELETON_BADGE_WIDTHS`).
const SKELETON: [f32; 3] = [44.0, 56.0, 38.0];
const DEFAULT_ASPECT: f32 = 16.0 / 9.0;

#[expect(
    clippy::too_many_arguments,
    reason = "the builder's parts, passed on as the other views take them"
)]
pub(crate) fn video<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    viewport: fn(&mut S) -> &mut ViewportState,
    emit: Emit<S>,
    key: &str,
    pixels: Option<&Pixels>,
    plain: bool,
    look: &Look,
    cx: &mut Cx<S>,
) -> El {
    let content = frame(st, access, viewport, emit.clone(), key, pixels, look, cx);
    let aspect = st.aspect().unwrap_or(DEFAULT_ASPECT);
    if plain {
        // Strata's plain media shows no title, and a GIF's bar is its play toggle alone.
        let controls = if st.clip().kind == MediaKind::Animation {
            super::transport::play(st, access, emit, key, look)
        } else {
            super::transport::plain_bar(st, access, emit, key, look)
        };
        return layout::section(
            key.to_owned(),
            Areas {
                content,
                aspect,
                details: col(),
                badges: None,
                timeline: None,
                controls,
            },
            look,
        );
    }
    // The volume sits at the eyebrow's end, the title under both (Strata's `DetailsHeader`).
    let volume = super::transport::volume(st, access, emit.clone(), key, look);
    let details = col()
        .w_full()
        .min_w(0.0)
        .child(layout::details_header_beside(
            key,
            (st.clip().position.map(st.labels().position), Some(volume)),
            time::stem(&st.clip().name),
            look,
        ))
        .child(super::error(st, key, look));
    let glow_inset = if look.glow.on && look.animations {
        ambient::BAND
    } else {
        0.0
    };
    layout::section_with_inset(
        key.to_owned(),
        Areas {
            content,
            aspect,
            details,
            badges: Some(badge_row(st, key, look)),
            timeline: Some(super::timeline::timeline(
                st,
                access,
                emit.clone(),
                key,
                false,
                look,
            )),
            controls: super::transport::transport(st, access, emit, key, look),
        },
        glow_inset,
        look,
    )
}

fn badge_row(st: &MediaState, key: &str, look: &Look) -> El {
    let k = &look.kit;
    let items: Vec<El> = match st.details() {
        Details::Loading => SKELETON
            .iter()
            .map(|w| {
                el().w(*w)
                    .h(k.ty.small.size + 2.0 * look.space.xxs + 2.0)
                    .radius(look.round(k.radius.sm))
                    .fill(k.c.layer_02)
                    .a11y_hidden()
            })
            .collect(),
        Details::Failed => Vec::new(),
        Details::Ready(d) => badges(d, st.labels())
            .into_iter()
            .enumerate()
            .map(|(i, b)| layout::badge(format!("{key}-badge-{i}"), b.label, look))
            .collect(),
    };
    layout::badge_row(key, items, look)
}

fn frame_keys<S: 'static>(
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    play: String,
) -> impl Fn(&mut S, &rune::KeyEvent, &mut Tx) -> bool + 'static {
    move |s: &mut S, k: &rune::KeyEvent, tx: &mut Tx| {
        let st = access(s);
        if st.clip().kind != MediaKind::Video {
            return plain_left(k, &play, tx);
        }
        let Some(command) = st.keys().focused_command(k, tx) else {
            return false;
        };
        if !st.command(command) {
            return false;
        }
        commit_after(s, access, &emit, tx);
        true
    }
}

/// A plain Left on plain media's frame, which moves to its play toggle, `play`.
fn plain_left(k: &rune::KeyEvent, play: &str, tx: &mut Tx) -> bool {
    let left = k.key == rune::Key::Left && k.mods == rune::Mods::default();
    if left {
        tx.focus(play.to_owned(), true);
    }
    left
}

#[expect(
    clippy::too_many_arguments,
    reason = "the builder's parts, passed on as the other views take them"
)]
fn frame<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    viewport: fn(&mut S) -> &mut ViewportState,
    emit: Emit<S>,
    key: &str,
    pixels: Option<&Pixels>,
    look: &Look,
    cx: &mut Cx<S>,
) -> El {
    let radius = slot::frame_radius(look);
    let mut surface = Viewport::new(&st.viewport, viewport)
        .key(format!("{key}-picture"))
        .interaction(Interaction::Fixed);
    if let Some(px) = pixels.filter(|_| st.has_frame()) {
        surface = surface.pixels(px);
    }
    let surface = surface.build(look, cx).a11y_hidden();
    let name = match st.clip().kind {
        MediaKind::Video => st.labels().video_frame.clone(),
        MediaKind::Audio | MediaKind::Plain | MediaKind::Animation => String::new(),
    };
    let mut frame = el()
        .key(format!("{key}-frame"))
        .label(name)
        .w_full()
        .h_full()
        .radius(radius)
        .clip_rounded(radius)
        .focusable(true)
        .no_focus_ring()
        .child_exit(rune::Transition::none())
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !access(s).set_frame_box([r[2], r[3]]) {
                tx.live_only();
            }
        })
        .on_press_tx({
            let emit = emit.clone();
            move |s: &mut S, tx: &mut Tx| {
                access(s).toggle();
                commit_after(s, access, &emit, tx);
            }
        })
        .on_key(frame_keys(access, emit, format!("{key}-play")))
        .child(surface);
    if st.stand_in_shown() {
        let mut stand_in = el()
            .key(format!("{key}-stand-in"))
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(slot::frame_fill(look))
            .live_opacity(&st.live.stand_in)
            .a11y_hidden();
        if let Some(poster) = st.poster() {
            stand_in = stand_in
                .fill(look.transparency.opaque(look.kit.c.bg_base))
                .child(
                    el().w_full()
                        .h_full()
                        .pixels_fit(poster, SurfaceFit::Fit)
                        .opacity(POSTER_OPACITY),
                );
        }
        frame = frame.child(stand_in);
    }
    if let Some((i, cell)) = st.cover() {
        frame = frame.child(
            el().key(format!("{key}-cover-{i}"))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(look.transparency.opaque(look.kit.c.bg_base))
                .a11y_hidden()
                .child(
                    el().w_full()
                        .h_full()
                        .pixels_fit(cell, SurfaceFit::Fit)
                        .opacity(SEEK_COVER_OPACITY),
                ),
        );
    }
    if let Some(bubble) = bubble(st, key, look) {
        frame = frame.child(bubble);
    }
    wrap_with_ambient(st, key, frame, look)
}

fn wrap_with_ambient(st: &MediaState, key: &str, frame: El, look: &Look) -> El {
    if st.clip().kind != MediaKind::Video {
        return frame;
    }
    let radius = slot::frame_radius(look) + ambient::OUTSET;
    let glow = st.ambient_shown().then(|| {
        el().key(format!("{key}-ambient"))
            .pin(
                Some(-ambient::OUTSET),
                Some(-ambient::OUTSET),
                Some(-ambient::OUTSET),
                Some(-ambient::OUTSET),
            )
            .radius(radius)
            .pixels(st.ambient().pixels())
            .blur(ambient::BLUR_SIGMA)
            .live_opacity(&st.live.ambient)
            .inert()
            .a11y_hidden()
    });
    el().key(format!("{key}-frame-wrap"))
        .w_full()
        .h_full()
        .child_exit(rune::Transition::none())
        .children(glow)
        .child(frame)
}

fn bubble(st: &MediaState, key: &str, look: &Look) -> Option<El> {
    let (cell, t, chapter) = st.bubble()?;
    let k = &look.kit;
    let [cw, ch] = match st.storyboard() {
        Some(b) => {
            let [w, h] = b.sheet().cell;
            [w as f32, h as f32]
        }
        None => {
            let a = st.aspect().unwrap_or(DEFAULT_ASPECT);
            if a >= 1.0 {
                [CELL_EDGE, (CELL_EDGE / a).round()]
            } else {
                [(CELL_EDGE * a).round(), CELL_EDGE]
            }
        }
    };
    // The cell, the padding and a 1 pt border on each side (Strata's `BUBBLE_CHROME`).
    let pad = slot::bubble_pad(look);
    let bubble_w = cw + 2.0 * (pad + 1.0);
    st.set_bubble_width(bubble_w);
    st.place();
    let label = match chapter {
        Some(c) => format!("{} · {}", time::clock(t), c.title),
        None => time::clock(t),
    };
    let mut picture = el()
        .key(format!("{key}-bubble-cell"))
        .size(cw, ch)
        .radius(slot::bubble_cell_radius(look))
        .clip_rounded(slot::bubble_cell_radius(look))
        .fill(slot::bubble_cell_fill(look));
    if let Some((_, px)) = cell {
        picture = picture.pixels_fit(px, SurfaceFit::Fill);
    }
    Some(
        col()
            .key(format!("{key}-bubble"))
            .pin(Some(0.0), None, None, Some(BUBBLE_MARGIN))
            .w(bubble_w)
            .padding(pad)
            .gap(2.0)
            .radius(slot::bubble_radius(look))
            .fill(slot::bubble_fill(look))
            .border(1.0, slot::bubble_border(look))
            .live_offset(&st.live.bubble)
            .child(picture.a11y_hidden())
            .child(
                text(label)
                    .key(format!("{key}-bubble-time"))
                    .font(rune::TextStyle {
                        weight: 700.0,
                        ..k.ty.small
                    })
                    .color(k.c.text_base)
                    .truncate(),
            ),
    )
}
