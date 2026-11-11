//! The timeline: a slider over the waveform, with chapter ticks and the playhead (Strata's
//! `src/ui/preview/waveform.rs`), or the plain bar's seek slider (`media_controls.rs`).
//!
//! The waveform is a `canvas` shape built when the levels or the width change. The played
//! part is the same shape in the accent under a clip whose width is a `Live` rect, the part
//! between the playhead and the pointer a tinted copy under another, and the playhead and the
//! pointer's line `Live` offsets: playback and hover move them with no view commit. A bar the
//! playhead crosses is cut where it is, where Strata colours a whole bar by its middle
//! (deviation D2).

pub(crate) mod levels;

use rune::{Color, Cursor, Drag, El, KeyEvent, Phase, Shape, Tx, WheelEvent, canvas, el};
use strata_kit::look::Look;
use strata_kit::slots::media as slot;

use super::state::MediaState;
use super::{Emit, commit_after};

/// The plain bar's track height and knob.
const PLAIN_HEIGHT: f32 = 20.0;
const PLAIN_TRACK: f32 = 4.0;
const KNOB: f32 = 12.0;
/// A bar is never lower than this: the plain progress line (`LINE_HEIGHT`).
const LINE_HEIGHT: f32 = 3.0;
/// Chapter ticks rise this far from the bottom (`CHAPTER_TICK`).
const CHAPTER_TICK: f32 = 6.0;

/// The timeline slider, keyed `<key>-timeline`.
pub(crate) fn timeline<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    plain: bool,
    look: &Look,
) -> El {
    let h = if plain {
        PLAIN_HEIGHT
    } else {
        slot::waveform_height(look)
    };
    st.wave_h.set(h);
    st.bar_pitch
        .set(slot::waveform_bar(look) + slot::waveform_gap(look));
    let w = st.timeline_width();
    let secs = |d: std::time::Duration| d.as_secs() as f64;
    let total = st.duration().map_or(secs(st.position()), secs);
    let labels = st.labels();
    let mut e = el()
        .key(format!("{key}-timeline"))
        .label(labels.timeline.clone())
        .range_value(secs(st.position()), 0.0, total)
        .range_step(super::time::SEEK_STEP.as_secs_f64())
        .a11y_value(st.value_text())
        .read_only(!st.seekable())
        .w_full()
        .h(h)
        .min_w(0.0)
        .cursor(Cursor::Pointer)
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            let st = access(s);
            let moved = (st.timeline_width() - r[2]).abs() > 0.01;
            let rebuilt = st.set_timeline_width(r[2]);
            if !moved && !rebuilt {
                tx.live_only();
            }
        })
        .on_pointer_move_local(move |s: &mut S, p, tx: &mut Tx| {
            if !access(s).hover_at(Some(p.x)) {
                tx.live_only();
            }
        })
        .on_hover_change(move |s: &mut S, inside, tx: &mut Tx| {
            if inside || !access(s).hover_at(None) {
                tx.live_only();
            }
        })
        .on_drag({
            let emit = emit.clone();
            move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                let st = access(s);
                match d.phase {
                    Phase::Up => st.drag_at(None, !d.cancelled()),
                    _ => st.drag_at(Some(d.x), false),
                };
                commit_after(s, access, &emit, tx);
            }
        })
        .on_key({
            let emit = emit.clone();
            move |s: &mut S, k: &KeyEvent, tx: &mut Tx| {
                let st = access(s);
                let Some(command) = st.keys().focused_command(k, tx) else {
                    return false;
                };
                if !st.command(command) {
                    return false;
                }
                commit_after(s, access, &emit, tx);
                true
            }
        })
        .on_step({
            let emit = emit.clone();
            move |s: &mut S, steps: i32, tx: &mut Tx| {
                let by = super::time::SEEK_STEP * steps.unsigned_abs();
                let seek = if steps < 0 {
                    super::keys::Seek::Back(by)
                } else {
                    super::keys::Seek::Forward(by)
                };
                access(s).seek_by(seek);
                commit_after(s, access, &emit, tx);
            }
        })
        .on_step_limit({
            let emit = emit.clone();
            move |s: &mut S, end: bool, tx: &mut Tx| {
                access(s).seek_by(if end {
                    super::keys::Seek::End
                } else {
                    super::keys::Seek::Start
                });
                commit_after(s, access, &emit, tx);
            }
        });
    if plain {
        e = e.on_wheel(move |s: &mut S, w: WheelEvent, tx: &mut Tx| {
            let taken = access(s).wheel(w.dy, w.precise);
            if taken {
                commit_after(s, access, &emit, tx);
            } else {
                tx.live_only();
            }
            taken
        });
        return e.children(plain_parts(st, key, w, look));
    }
    e.children(wave_parts(st, key, w, h, look))
}

fn hidden(e: El) -> El {
    e.a11y_hidden()
}

/// The waveform's bars as one shape, in `color`, `w` wide and `h` tall.
fn wave_shape(bars: &[f32], w: f32, h: f32, look: &Look, color: Color) -> El {
    let bar = slot::waveform_bar(look);
    let pitch = bar + slot::waveform_gap(look);
    let center = (h / 2.0).round();
    let shape = bars.iter().enumerate().fold(Shape::new(), |s, (i, level)| {
        let x = i as f32 * pitch;
        let b = (level * (h - 2.0)).max(LINE_HEIGHT);
        let (top, bottom) = (center - b / 2.0, center + b / 2.0);
        s.fill(
            vec![[x, top], [x + bar, top], [x + bar, bottom], [x, bottom]],
            color,
        )
    });
    hidden(canvas(shape).w(w).h(h))
}

fn wave_parts(st: &MediaState, key: &str, w: f32, h: f32, look: &Look) -> Vec<El> {
    if w <= 0.0 {
        return Vec::new();
    }
    let bars = st.levels.bars(st.bars());
    let clipped = |k: &str, live: &rune::Live<[f32; 4]>, color: Color| {
        hidden(
            el().key(format!("{key}-{k}"))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, h)
                .clip_soft(0.0)
                .live_rect(live)
                .child(wave_shape(&bars, w, h, look, color)),
        )
    };
    let mut parts = vec![
        hidden(
            el().pin(Some(0.0), Some(0.0), None, None)
                .size(w, h)
                .child(wave_shape(&bars, w, h, look, slot::waveform_unplayed(look))),
        ),
        clipped("hovered", &st.live.hovered, slot::waveform_hover(look)),
        clipped("played", &st.live.played, slot::waveform_played(look)),
    ];
    if let Some(d) = st.duration().filter(|d| !d.is_zero())
        && !st.chapters().is_empty()
    {
        let color = slot::waveform_chapter(look);
        let ticks = st.chapters().iter().fold(Shape::new(), |s, c| {
            let x = ((c.start.as_secs_f64() / d.as_secs_f64()) as f32 * w).clamp(0.5, w - 0.5);
            s.fill(
                vec![
                    [x - 0.5, h - CHAPTER_TICK],
                    [x + 0.5, h - CHAPTER_TICK],
                    [x + 0.5, h],
                    [x - 0.5, h],
                ],
                color,
            )
        });
        parts.push(hidden(
            canvas(ticks)
                .key(format!("{key}-chapters"))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, h),
        ));
    }
    parts.push(hidden(
        el().key(format!("{key}-pointer"))
            .pin(Some(0.0), Some(0.0), None, None)
            .size(1.0, h)
            .fill(slot::waveform_chapter(look))
            .live_offset(&st.live.pointer)
            .live_opacity(&st.live.pointer_shown),
    ));
    parts.push(hidden(
        el().key(format!("{key}-head"))
            .pin(Some(0.0), Some(0.0), None, None)
            .size(2.0, h)
            .fill(slot::waveform_playhead(look))
            .live_offset(&st.live.head),
    ));
    parts
}

fn plain_parts(st: &MediaState, key: &str, w: f32, look: &Look) -> Vec<El> {
    if w <= 0.0 {
        return Vec::new();
    }
    let k = &look.kit;
    let y = (PLAIN_HEIGHT - PLAIN_TRACK) / 2.0;
    let track = |e: El| {
        e.pin(Some(0.0), Some(y), None, None)
            .size(w, PLAIN_TRACK)
            .radius(PLAIN_TRACK / 2.0)
    };
    vec![
        hidden(track(el()).fill(slot::waveform_unplayed(look))),
        hidden(
            el().key(format!("{key}-played"))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, PLAIN_HEIGHT)
                .clip_soft(0.0)
                .live_rect(&st.live.played)
                .child(track(el()).fill(slot::waveform_played(look))),
        ),
        hidden(
            el().key(format!("{key}-head"))
                .pin(
                    Some(-KNOB / 2.0 + 1.0),
                    Some((PLAIN_HEIGHT - KNOB) / 2.0),
                    None,
                    None,
                )
                .size(KNOB, KNOB)
                .radius(KNOB / 2.0)
                .fill(k.c.knob)
                .border(1.0, k.c.border_base)
                .live_offset(&st.live.head),
        ),
    ]
}
