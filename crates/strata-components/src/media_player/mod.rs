//! MediaPlayer (C13): the media preview's interface, driven by a clock and frames it doesn't
//! produce.
//!
//! Work package: C13, `docs/work/phase-2-composites/C13-media-player.md`. Plan §4 C13,
//! `preview/preview-panel/media`, `.../audio`, `.../video`.
//!
//! ```ignore
//! MediaPlayer::new(&s.media, |s: &mut App| &mut s.media, |s: &mut App| &mut s.media.viewport)
//!     .key("media")
//!     .video(&s.frames)                     // P7's pacer writes the frames in place
//!     .on_intent(|s: &mut App, intent, _tx| s.clock.apply(&intent))
//!     .build(&s.look, cx)
//! // and in the app's poll, in this order:
//! s.clock.advance(now);
//! changed |= s.media.tick(&s.clock, now, tx);
//! for intent in s.media.drain_intents() { s.clock.apply(&intent) }
//! ```
//!
//! Intents from the view's handlers go to `on_intent` at once. Without it they wait for the
//! poll's drain, which comes after the tick: a seek sent then is held on the timeline at its
//! target until the clock reports it (or for at most a second, should the host drop it), so
//! no frame shows the old position, and the handler asks for the wake that brings the poll.
//!
//! **Rune dev 5d421fd: partly.** `Pixels` updated in place (C14a's viewport), `Live`
//! offsets, rects, opacity and gradients, `canvas` shapes, the slider role (`on_step`,
//! `on_step_limit`, `range_value`, `a11y_value`), an element's `on_key` ahead of the slider's
//! step keys, `wake_after` and `LoopSpec::rotation` cover it. Missing: rune-kit has no Slider
//! (anomalyco/rune#149), accessibility values don't follow `Live` (C16's U-C16-3), a shape's
//! geometry can't be bound live (U-C13-1), `on_step`'s page is a fixed 10 steps (U-C13-2),
//! and only icon quads turn without a commit (U-C13-3). `Player` has no Linux backend
//! (anomalyco/rune#30, #112); P7 replaces it. The timeline follows the playback clock (P7),
//! not Rune's, so `Tx::clock` (c650fe0c) retires nothing here.
//!
//! # What it does
//!
//! Three views, by [`MediaKind`], each from Strata's (`src/ui/preview/video.rs`, `audio.rs`,
//! `media_controls.rs` at lgse/strata `upstream/main` e9a8993), for behaviour, not names:
//!
//! - **Video**: the frame in C14b's shared player layout ([`crate::viewers::layout`]) on a
//!   C14a viewport (fixed, fitted) whose `Pixels` the host writes; before the first frame the
//!   listing's thumbnail dimmed, or a plain surface at the probed aspect, never a spinner,
//!   fading out over 150 ms (at once with animations off); while a seek runs, the nearest
//!   storyboard cell covers the frame. Under it the position caption and title, the volume,
//!   the badges ([`badges`]: empty pills until the probe answers), the timeline and the
//!   transport. A press on the frame toggles playback.
//! - **Audio**: the art, or a record when there's none, the caption ("Track 3 of 12"), title
//!   (the file's name without its extension when there's no title tag) and byline, the live
//!   spectrum, the timeline and the transport. Wide panes put the art beside the rest
//!   ([`audio::layout`]); short ones drop the spectrum first, then the art. A press on the
//!   art or the spectrum toggles playback.
//! - **Plain** (an animated GIF, say): the frame over Strata's plain media bar: play, the
//!   time, a seek slider, mute and volume.
//!
//! The **timeline** is a slider named "Playback position" whose value is "1:05 of 3:20". In
//! the views with audio it is the waveform (`timeline`) with chapter ticks; the plain bar's
//! is a track and knob. Hovering shows the pointer's time in the elapsed label (and, over a
//! video, a bubble with the nearest storyboard cell, the time and its chapter); a press-drag
//! shows the drag position and seeks on release; keys ([`MediaKeys`]) and the wheel (plain
//! bar) seek at once, never past the last frame. A clip whose duration isn't known yet can't
//! seek: the slider is read-only.
//!
//! **Volume**: mute and a 72 pt slider. Keys step on the 0.1 grid (Page Up and Down 0.2), so
//! repeated Down reaches 0 and mutes; Home mutes, End is full; a drag sets any value.
//!
//! **Hand-off**: Return over a video, or the host's Open button through
//! [`MediaState::hand_off`], pauses and sends [`MediaIntent::HandOff`] with the position, or
//! none within 1 s of either end. Which players take a position is Strata's.
//!
//! # What stays with the host
//!
//! Decoding, the audio sink and its clock (P7), autoplay and its fade, stall recovery, which
//! files join the previous and next steps, the four-player cap, art and sidecar lookup,
//! saving the volume, the 50 ms before the waveform decodes, which clips get a storyboard,
//! the player list for hand-off, Element glow (deferred, D7). The host gets intents and
//! applies them.
//!
//! # No commit per frame
//!
//! [`MediaState::tick`] reads the clock each frame and moves `Live` values: the played clip,
//! the playhead, the pointer's line and tint, the bubble's place, the stand-in's fade and the
//! spectrum's bars, caps and reflections. Video frames are the host's `Pixels::set`. None
//! re-run the view. The view commits only when what it shows changes ([`MediaState::tick`]'s
//! answer, and each handler's): the elapsed and total seconds and the slider's spoken value,
//! at most once a second while playing (Strata's cadence); play, pause, seeking, the end, an
//! error; the stand-in going; the bubble's second, cell or chapter; the bar counts; the audio
//! layout; the volume; and the waveform's grow-in, at most 30 times a second while bars move
//! (D3). The time labels are committed text, not `LiveText`, which would turn off the
//! listing's virtual-window fast path beside the drawer (U-C1f-6).
//!
//! # What assistive technology hears
//!
//! The timeline: a slider named "Playback position" with its range in seconds and the value
//! "1:05 of 3:20", updated at most once a second while playing, read-only while the clip
//! can't seek; its increment and decrement seek 5 s. Named transport buttons ("Play or
//! pause", "Previous video", …; the plain bar's play button "Play/Pause (Ctrl+Alt+Space)"),
//! each with its tooltip as its description, "Mute/unmute (Ctrl+Alt+M)", and the volume
//! slider ("Volume", "60%"). The frame is a button named "Video frame" (a press plays or
//! pauses); the art and the spectrum are images named "Album art" and "Audio spectrum". The
//! elapsed and total times and the bubble's time are plain labels, as Strata's are. Nothing is
//! a live region, so playback announces nothing on its own.
//!
//! # Deviations
//!
//! - **D1** (OWNER ruling): the spectrum is `Live`-bound elements with Strata's gradient, not
//!   `canvas` shapes as the WP says: a shape can't change without a commit.
//! - **D2**: the playhead cuts the bar it crosses; Strata colours a whole bar by its middle.
//! - **D3** (OWNER ruling): the waveform's grow-in commits at most 30 times a second.
//! - **D4** (OWNER ruling): the record is a static disc whose label mark turns; no sleeve,
//!   slide-out or tuck, and new art replaces the old at once, without Strata's crossfade.
//! - **D5** (OWNER ruling): C13 owns its seek and volume sliders; C20a adopts or replaces
//!   them later.
//! - **D6**: the time labels are committed text (U-C1f-6).
//! - **D7** (OWNER ruling): no Element glow.
//! - **D8**: [`MediaKeys`] is a table of its own, separate from C1c's `KeyMap`.
//! - **D9** (OWNER ruling): the video frame is a button named "Video frame", since a press
//!   plays or pauses and Rune names a pressable element so; Strata's is a focusable group.
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c13/`, in-process tests in `crates/strata-gallery/tests/c13.rs`
//! (`c13::…`), unit tests in this module's `tests.rs`.
//!
//! `preview/preview-panel/media`:
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | Ctrl+Alt+Space, Ctrl+Alt+Left/Right, Up/Down, M | scripts `default-keys-seek`, `default-keys-volume`, `default-keys-mute`, `default-keys-play`; unit `the_window_keys_are_ctrl_alt` |
//! | The plain bar's play button reads "Play/Pause (…)" | script `plain-names`; golden `c13-plain` |
//! | The mute button reads "Mute/unmute (…)" | script `mute-name` |
//! | A focused volume slider steps 0.1 and 0.2; Home mutes, End full | scripts `volume-arrows`, `volume-pages`, `volume-home`, `volume-end` |
//! | Stepped volumes snap to the 0.1 grid | script `volume-snap`; unit |
//! | Dragging the volume slider sets any value | script `volume-drag` |
//! | Neither audio nor video: the seek slider steps 5 s and 30 s | script `plain-seek-keys` |
//! | That slider's Home and End | script `plain-seek-ends` |
//! | Keys and the wheel seek at once; a drag on release | script `plain-seek-drag` |
//! | Ignores scroll jumps under a frame | unit `the_wheel_seeks_a_step_a_notch_and_ignores_jumps_under_a_tick` (partial: a touchpad's deltas, not frames; scripts send only whole wheel deltas usefully) |
//! | No duration: seeking disabled until known | script `unknown-duration` (partial: playing to the end is P7's) |
//! | Decodes at the pane's size × scale, capped at 1280, never past the source; regrows by 8 %, never on a shrink | test `the_decode_size_follows_growth_of_eight_percent_and_never_a_shrink`; units (partial: the decoder is the host's) |
//! | An isolated seek shows the slider at the target | the seek scripts (partial: coalescing within 200 ms is P7's) |
//! | No focus border when clicked | Rune's focus ring is keyboard-only; no script |
//!
//! `preview/preview-panel/media/audio`:
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | Without art, a record spins while playing | golden `c13-audio-record` (D4) |
//! | Title, artist and album; no title tag shows the name without its extension | script `audio-tags` |
//! | "Track 3 of 12", "Track 3", "3 of 12 in folder" | scripts `audio-caption`, `audio-caption-tagged` |
//! | The play button's name and tooltip | script `audio-play-name` |
//! | Rows keep their space while loading | goldens (partial: the eyebrow and byline hold a line) |
//! | Same art stays; different art crossfades | test `the_same_art_keeps_its_element_and_new_art_replaces_it` (D4: no crossfade) |
//! | Art beside at 1.12, on top at 0.95 | unit `wide_panes_put_art_beside_with_hysteresis`; golden `c13-audio-wide` |
//! | Short and stacked: the spectrum hides first, then the art | scripts `audio-hide-spectrum`, `audio-hide-art` |
//! | Clicking the art or the spectrum toggles | scripts `audio-click-toggles`, `audio-click-art` |
//! | Bars follow the audio; peak caps hold before falling | unit `peak_caps_hold_then_fall…` (the analysis is the host's) |
//! | The waveform grows in | test `the_grow_in_commits_at_most_30_times_a_second`; unit (the 50 ms wait is the host's) |
//! | Hovering shows the target time; clicking or dragging seeks | scripts `waveform-seek`, `waveform-drag` |
//! | No audio or no duration keeps the plain line | golden `c13-video-loading` |
//! | Previous disabled on the first, next on the last; no wrap | scripts `track-ends`, `track-ends-first` |
//! | `<` and `>` in 10xer, Ctrl+Alt+`<` and `>` by default | scripts `track-keys`, `track-keys-tenxer` |
//! | Tracks never advance on their own | test `a_track_that_ends_stays_put` |
//! | An undecodable track shows its error; previous and next work | scripts `audio-error`, `audio-error-steps` |
//! | Resizing doesn't restart playback | test `resizing_audio_sends_no_intent` |
//!
//! `preview/preview-panel/media/video`:
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | Before the first frame: the thumbnail dimmed, or a plain surface; never a spinner | golden `c13-video-loading`; test `the_stand_in_fades…` |
//! | The first frame fades the stand-in over 150 ms, or at once | test `the_stand_in_fades_over_150_ms_or_goes_at_once_from_wake_to_wake` |
//! | Clicking the frame toggles playback | script `video-click-toggles` |
//! | The play button's name and tooltip | script `video-play-name` |
//! | "2 of 5 in folder" | script `video-caption` |
//! | An error shows inside; play off, previous and next work | scripts `video-error`, `video-error-steps` |
//! | Badges: resolution, HDR, depth, rate, codecs | script `video-badges`; unit `badges_follow_stratas_rules` |
//! | "CC" or "CC ×N" | script `video-captions-badge` |
//! | Empty pills until the probe answers | golden `c13-video-loading` (partial: the pills are hidden from AT) |
//! | The timeline is the waveform slider with chapter ticks | script `timeline-waveform`; goldens |
//! | Hover or drag: a bubble with the cell, time and chapter | scripts `storyboard-hover`, `storyboard-leave` |
//! | While seeking the nearest cell covers the frame | test `a_seek_shows_the_nearest_cell_until_it_lands`; golden `c13-video-seeking` |
//! | Previous and next keys | script `video-step-keys` |
//! | Return pauses and opens the default player | script `handoff` (the Open button: the host calls [`MediaState::hand_off`]) |
//! | Within 1 s of either end, the file opens plainly | script `handoff-margin` (which players take a position is Strata's) |
//! | A resumed clip shows its nearest cell | partial: the host passes the cell as the poster ([`Storyboard::nearest`]) |
//!
//! `preview/quick-preview` (media timeline keys):
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | Left/Right 5 s; Shift+Left/Right, Page Down/Up 30 s | scripts `timeline-seek-arrows`, `timeline-seek-shift`, `timeline-seek-page-up`, `timeline-seek-page-down` (the pages take 30 s, not Rune's 10 steps: U-C13-2); unit |
//! | Home, `0`, Shift+0 to the start; End, `G` to the last frame | script `timeline-jumps`; unit |
//! | They hold after the listing's keys are rebound | by construction (D8): [`MediaKeys`] reads nothing of C1c's `KeyMap` |
//! | Caps Lock g doesn't seek | script `timeline-capslock-g`; unit |
//! | `+`, `=`, `-` and the keypad's change the volume; 0 mutes and keeps the volume | scripts `timeline-volume-plus`, `-equals`, `-minus`, `-keypad`, `-mute-restore`, `-restore`; unit |
//! | No Ctrl+Alt form | script `timeline-no-ctrl-alt`; unit |
//! | A key added to the seek-to-end command | script `timeline-seek-end-binding`; unit |
//! | Removing `+` leaves `=` | script `timeline-volume-alias`; unit |
//!
//! Not here (Strata's, the host's or P7's): stepping and which files join the sequence,
//! autoplay and its fade, stall recovery, reopening at a position, the sink's start-up,
//! resources and the four-player cap, hardware decoding, GIF looping and raw streams, volume
//! shared across players, art and sidecar lookup, the double-click hold, clips that get no
//! storyboard, subtitles (never drawn: there is no API for them).
//!
//! # Known gaps
//!
//! - After a click on the timeline the elapsed label shows the position, not the pointer's
//!   time, until the pointer moves: Rune ends the hover with the press (script
//!   `waveform-seek` moves away and back).
//! - P7 isn't built: everything here runs on [`FakeClock`]. The hand test with P7 waits for it.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C13-1, no live shape geometry.** A shape's points are set at build, so a waveform
//!   that grows in, or a spectrum drawn as shapes, rebuilds the view each frame. Repro: a
//!   `canvas` of 96 bars whose heights follow `Live<f32>`s needs 96 elements with `live_rect`
//!   instead. Ask: points (or a polygon's scale) bound to `Live` values.
//! - **U-C13-2, `on_step` pages are 10 steps.** A media timeline steps 5 s and pages 30 s:
//!   Page Up and Down through `on_step` would move 50 s. Repro: a slider with `on_step` and
//!   `range_step(5.0)`, focused, Page Up: `f` hears +10. Ask: `El::range_page(steps)`. This
//!   player takes the keys in `on_key` first.
//! - **U-C13-3, only icons turn live.** `LoopSpec::rotation` turns icon quads; any other
//!   element turns only through `El::rotate`, which needs a commit each frame (a comment
//!   candidate on anomalyco/rune#420, which notes rotation can't be bound live).
//! - **U-C13-4, `aspect_fit` with `max_w` overflows its column.** The cap is taken for the
//!   cross room: a 16:9 box in a 628 pt column whose free height is 452 pt draws 803 pt wide
//!   with `.aspect_fit(16.0 / 9.0).max_w(1280.0)`, 628 pt without the `max_w` (probe P8). Ask:
//!   bound the fit by the smaller of `max_w` and the cross room. C14b's shared layout gives the
//!   cap as a height instead ([`crate::viewers::layout`]).
//! - Reused: U-C16-3 (accessibility values don't follow `Live`), U-C1f-6, anomalyco/rune#149.

pub mod audio;
pub mod badges;
pub mod clock;
pub mod data;
pub mod intent;
pub mod keys;
pub mod labels;
mod state;
pub mod storyboard;
pub mod time;
pub(crate) mod timeline;
mod transport;
mod video;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, El, Pixels, Tx, col, text};
use strata_kit::look::Look;

pub use clock::{FakeClock, PlaybackClock, PlaybackState};
pub use data::{Art, AudioTags, Chapter, Clip, Details, MediaError, MediaKind};
pub use intent::MediaIntent;
pub use keys::{KeyScope, MediaBinding, MediaCommand, MediaKeys, Seek};
pub use labels::Labels;
pub use state::{GROW_INTERVAL, MediaState, STAND_IN_FADE};
pub use storyboard::{Sheet, Storyboard};
pub use timeline::levels::Levels;

use crate::viewers::viewport::ViewportState;

/// Where the view's handlers send intents.
pub(crate) type Emit<S> = Option<Rc<dyn Fn(&mut S, MediaIntent, &mut Tx)>>;

/// Hands the state's intents to the host's callback, if it gave one (else they wait for
/// [`MediaState::drain_intents`]). True when any went.
pub(crate) fn flush<S>(
    s: &mut S,
    access: fn(&mut S) -> &mut MediaState,
    emit: &Emit<S>,
    tx: &mut Tx,
) -> bool {
    let Some(f) = emit else { return false };
    let intents = access(s).drain_intents();
    for i in &intents {
        f(s, *i, tx);
    }
    !intents.is_empty()
}

/// After a handler changed the state: the intents go, and the view commits when it shows
/// something new or the host was asked something (its answer may change what shows). With no
/// callback, the intents wait for the host's poll, which a wake brings.
pub(crate) fn commit_after<S>(
    s: &mut S,
    access: fn(&mut S) -> &mut MediaState,
    emit: &Emit<S>,
    tx: &mut Tx,
) {
    let pending = !access(s).outbox_is_empty();
    let sent = flush(s, access, emit, tx);
    if pending && !sent {
        // No callback: the host drains them in its poll, which this wake brings.
        tx.wake_after(std::time::Duration::ZERO);
    }
    if !sent {
        access(s).finish(tx);
    }
}

/// The error, inside the view: its title and detail.
pub(crate) fn error(st: &MediaState, key: &str, look: &Look) -> El {
    let k = &look.kit;
    let Some(e) = st.error() else {
        return col().key(format!("{key}-error"));
    };
    col()
        .key(format!("{key}-error"))
        .gap(look.space.xxs)
        .pt(look.space.xs)
        .child(
            text(e.title.clone())
                .key(format!("{key}-error-title"))
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .role("heading")
                .wrap(),
        )
        .child(
            text(e.detail.clone())
                .key(format!("{key}-error-detail"))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
        )
}

/// The player: `MediaPlayer::new(&s.media, access, viewport) … .build(look, cx)`. It fills its
/// parent.
pub struct MediaPlayer<'a, S> {
    state: &'a MediaState,
    access: fn(&mut S) -> &mut MediaState,
    viewport: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    emit: Emit<S>,
}

impl<'a, S: 'static> MediaPlayer<'a, S> {
    /// `viewport` reaches [`MediaState::viewport`]: C14a's viewport takes a plain `fn`.
    pub fn new(
        state: &'a MediaState,
        access: fn(&mut S) -> &mut MediaState,
        viewport: fn(&mut S) -> &mut ViewportState,
    ) -> Self {
        MediaPlayer {
            state,
            access,
            viewport,
            key: "media".into(),
            pixels: None,
            emit: None,
        }
    }

    /// The view's key ("media" unless set). Its parts are `<key>-frame`, `<key>-picture`,
    /// `<key>-stand-in`, `<key>-cover-<cell>`, `<key>-bubble`, `<key>-bubble-time`,
    /// `<key>-timeline`, `<key>-played`, `<key>-head`, `<key>-chapters`, `<key>-play`,
    /// `<key>-previous`, `<key>-next`, `<key>-elapsed`, `<key>-total`, `<key>-time` (the plain
    /// bar's), `<key>-mute`, `<key>-volume`, `<key>-title`, `<key>-eyebrow`, `<key>-byline`,
    /// `<key>-badges`, `<key>-badge-<n>`, `<key>-error`, `<key>-art-<id>`, `<key>-record`,
    /// `<key>-record-mark`, `<key>-spectrum` and `<key>-bar-<n>`.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The frames the host writes in place (video and plain).
    pub fn video(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    /// Where intents from the view's handlers go, as they happen: the host applies each
    /// (play, seek, volume) in the same handler, so nothing waits a frame. Without one they
    /// wait in [`MediaState::drain_intents`].
    pub fn on_intent(mut self, f: impl Fn(&mut S, MediaIntent, &mut Tx) + 'static) -> Self {
        self.emit = Some(Rc::new(f));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let MediaPlayer {
            state: st,
            access,
            viewport,
            key,
            pixels,
            emit,
        } = self;
        st.motion.set(look.animations);
        let e = match st.clip().kind {
            MediaKind::Video => {
                video::video(st, access, viewport, emit, &key, pixels, false, look, cx)
            }
            MediaKind::Plain => {
                video::video(st, access, viewport, emit, &key, pixels, true, look, cx)
            }
            MediaKind::Audio => audio::audio(st, access, emit, &key, look),
        };
        st.note_built();
        e
    }
}
