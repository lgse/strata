//! Transport controls: play, previous and next, the elapsed and total times, mute and the
//! volume slider (Strata's `components.rs` `navigation_row`, `media_controls.rs`).

use rune::{Cursor, Drag, El, KeyEvent, Phase, Tx, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::media as slot;

use super::data::MediaKind;
use super::keys::{KeyScope, MediaCommand};
use super::labels::with_keys;
use super::state::MediaState;
use super::time::{self, VOLUME_PAGE, VOLUME_STEP};
use super::{Emit, commit_after};

/// The volume slider's width: Strata's 72.
pub const VOLUME_WIDTH: f32 = 72.0;
const VOLUME_HEIGHT: f32 = 20.0;
const VOLUME_TRACK: f32 = 4.0;
const VOLUME_KNOB: f32 = 12.0;

fn keys_for(st: &MediaState, command: MediaCommand) -> Option<String> {
    st.keys().label(KeyScope::Window, command)
}

/// The play button: "Play or pause", tooltip "Play/Pause (Ctrl+Alt+Space)". The plain bar's
/// takes the tooltip as its name too (`media_controls.rs`, `append_play_toggle`).
pub(crate) fn play<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    look: &Look,
) -> El {
    let labels = st.labels();
    let tooltip = with_keys(
        &labels.play_tooltip,
        keys_for(st, MediaCommand::PlayPause).as_deref(),
    );
    let name = if st.clip().kind == MediaKind::Plain {
        tooltip.clone()
    } else {
        labels.play.clone()
    };
    let icon = if st.playing() {
        IconName::Pause
    } else {
        IconName::Play
    };
    IconButton::new(look.icon(icon))
        .variant(IconButtonVariant::Ghost)
        .label(name)
        .help(tooltip)
        .disabled(st.error().is_some())
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).toggle();
            commit_after(s, access, &emit, tx);
        })
        .build(&look.kit)
        .key(format!("{key}-play"))
}

/// Previous or next: "Previous video", tooltip "Previous video (Ctrl+Alt+<)".
pub(crate) fn step<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    next: bool,
    look: &Look,
) -> El {
    let labels = st.labels();
    let audio = st.clip().kind == MediaKind::Audio;
    let label = match (audio, next) {
        (false, false) => &labels.previous_video,
        (false, true) => &labels.next_video,
        (true, false) => &labels.previous_audio,
        (true, true) => &labels.next_audio,
    };
    let command = if next {
        MediaCommand::Next
    } else {
        MediaCommand::Previous
    };
    let tooltip = with_keys(
        &labels.step_tooltip.replace("{label}", label),
        keys_for(st, command).as_deref(),
    );
    let enabled = if next {
        st.clip().has_next
    } else {
        st.clip().has_previous
    };
    IconButton::new(look.icon(if next {
        IconName::SkipForward
    } else {
        IconName::SkipBack
    }))
    .variant(IconButtonVariant::Ghost)
    .label(label.clone())
    .help(tooltip)
    .disabled(!enabled)
    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
        access(s).command(command);
        commit_after(s, access, &emit, tx);
    })
    .build(&look.kit)
    .key(format!("{key}-{}", if next { "next" } else { "previous" }))
}

/// The time labels' text: the elapsed (the pointer's while it is over the timeline) and the
/// total (`sync_time`).
fn times(st: &MediaState) -> (String, String) {
    let total = st
        .duration()
        .map_or_else(|| st.labels().unknown_total.clone(), time::clock);
    (time::clock(st.shown_time()), total)
}

fn time_label(key: String, s: String, target: bool, look: &Look) -> El {
    let k = &look.kit;
    text(s).key(key).font(k.ty.small).color(if target {
        k.c.text_accent
    } else {
        k.c.text_muted
    })
}

/// The transport row: elapsed on the left, previous, play and next in the middle, the total
/// on the right (`navigation_row` with start and end widgets).
pub(crate) fn transport<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    look: &Look,
) -> El {
    let (elapsed, total) = times(st);
    let target = st.pointer_time().is_some();
    row()
        .key(format!("{key}-transport"))
        .w_full()
        .center_y()
        .child(row().w(0.0).grow(1.0).child(time_label(
            format!("{key}-elapsed"),
            elapsed,
            target,
            look,
        )))
        .child(
            row()
                .gap(look.space.md)
                .center_y()
                .child(step(st, access, emit.clone(), key, false, look))
                .child(play(st, access, emit.clone(), key, look))
                .child(step(st, access, emit, key, true, look)),
        )
        .child(row().w(0.0).grow(1.0).justify_end().child(time_label(
            format!("{key}-total"),
            total,
            false,
            look,
        )))
}

/// The plain bar: play, "0:05/1:20", the seek slider, mute and volume
/// (`append_media_controls`).
pub(crate) fn plain_bar<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    look: &Look,
) -> El {
    let (elapsed, total) = times(st);
    row()
        .key(format!("{key}-bar"))
        .w_full()
        .gap(look.space.sm)
        .center_y()
        .child(play(st, access, emit.clone(), key, look))
        .child(time_label(
            format!("{key}-time"),
            format!("{elapsed}/{total}"),
            false,
            look,
        ))
        .child(col().grow(1.0).min_w(0.0).child(super::timeline::timeline(
            st,
            access,
            emit.clone(),
            key,
            true,
            look,
        )))
        .child(volume(st, access, emit, key, look))
}

/// Mute and the volume slider (`VolumeControls`).
pub(crate) fn volume<S: 'static>(
    st: &MediaState,
    access: fn(&mut S) -> &mut MediaState,
    emit: Emit<S>,
    key: &str,
    look: &Look,
) -> El {
    let k = &look.kit;
    let labels = st.labels();
    let mute_label = with_keys(&labels.mute, keys_for(st, MediaCommand::Mute).as_deref());
    let mute = IconButton::new(look.icon(if st.muted() {
        IconName::VolumeMute
    } else {
        IconName::Volume
    }))
    .variant(IconButtonVariant::Ghost)
    .label(mute_label.clone())
    .help(mute_label)
    .on_press_tx({
        let emit = emit.clone();
        move |s: &mut S, tx: &mut Tx| {
            access(s).toggle_mute();
            commit_after(s, access, &emit, tx);
        }
    })
    .build(k)
    .key(format!("{key}-mute"));

    let value = if st.muted() { 0.0 } else { st.volume() };
    let fill = value * VOLUME_WIDTH;
    let y = (VOLUME_HEIGHT - VOLUME_TRACK) / 2.0;
    let set_at = |w: f32, x: f32| (x / w.max(1.0)).clamp(0.0, 1.0);
    let slider = el()
        .key(format!("{key}-volume"))
        .label(labels.volume.clone())
        .range_value((f64::from(value) * 1000.0).round() / 1000.0, 0.0, 1.0)
        .range_step((f64::from(VOLUME_STEP) * 1000.0).round() / 1000.0)
        .a11y_value(format!("{}%", (value * 100.0).round() as u32))
        .w(VOLUME_WIDTH)
        .h(VOLUME_HEIGHT)
        .cursor(Cursor::Pointer)
        // Keys step on the 0.1 grid; Home mutes and End is full volume (`volume_slider`).
        .on_key({
            let emit = emit.clone();
            move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
                use rune::Key;
                if e.mods.ctrl || e.mods.alt || e.mods.cmd || e.mods.logo || e.mods.shift {
                    return false;
                }
                let st = access(s);
                match e.key {
                    Key::Left | Key::Down => st.step_volume(-1.0),
                    Key::Right | Key::Up => st.step_volume(1.0),
                    Key::PageDown => st.step_volume(-VOLUME_PAGE / VOLUME_STEP),
                    Key::PageUp => st.step_volume(VOLUME_PAGE / VOLUME_STEP),
                    Key::Home => st.set_user_volume(0.0),
                    Key::End => st.set_user_volume(1.0),
                    _ => return false,
                }
                commit_after(s, access, &emit, tx);
                true
            }
        })
        .on_step({
            let emit = emit.clone();
            move |s: &mut S, steps: i32, tx: &mut Tx| {
                access(s).step_volume(steps as f32);
                commit_after(s, access, &emit, tx);
            }
        })
        .on_step_limit({
            let emit = emit.clone();
            move |s: &mut S, end: bool, tx: &mut Tx| {
                access(s).set_user_volume(if end { 1.0 } else { 0.0 });
                commit_after(s, access, &emit, tx);
            }
        })
        // A drag sets any value, without snapping; a cancelled one puts the volume back.
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let st = access(s);
            if d.phase == Phase::Up {
                st.end_volume_drag(d.cancelled());
            } else {
                st.drag_volume(set_at(d.w, d.x));
            }
            commit_after(s, access, &emit, tx);
        })
        .child(
            el().pin(Some(0.0), Some(y), None, None)
                .size(VOLUME_WIDTH, VOLUME_TRACK)
                .radius(VOLUME_TRACK / 2.0)
                .fill(slot::waveform_unplayed(look))
                .a11y_hidden(),
        )
        .child(
            el().pin(Some(0.0), Some(y), None, None)
                .size(fill, VOLUME_TRACK)
                .radius(VOLUME_TRACK / 2.0)
                .fill(slot::waveform_played(look))
                .a11y_hidden(),
        )
        .child(
            el().pin(
                Some((fill - VOLUME_KNOB / 2.0).clamp(0.0, VOLUME_WIDTH - VOLUME_KNOB)),
                Some((VOLUME_HEIGHT - VOLUME_KNOB) / 2.0),
                None,
                None,
            )
            .size(VOLUME_KNOB, VOLUME_KNOB)
            .radius(VOLUME_KNOB / 2.0)
            .fill(k.c.knob)
            .border(1.0, k.c.border_base)
            .a11y_hidden(),
        );
    row()
        .key(format!("{key}-volume-row"))
        .gap(look.space.xs)
        .center_y()
        .child(mute)
        .child(slider)
}
