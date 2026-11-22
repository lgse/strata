// SPDX-License-Identifier: MIT

//! The media player's host (`preview::media`): a sandboxed video's frames and clock reach the
//! player and the media key plays and pauses it; the eyebrow counts only the files whose
//! probe found media, and the next key steps past text named as video, keeps playing and keeps
//! the keyboard in the player;
//! a decode that fails shows its error inside the player; and a GIF loops, its bar the play
//! toggle alone.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use rune::Tx;
use rune::{Key, Mods};
use strata_components::{
    media_player::{MediaKind, MediaState},
    viewers::layout::Position,
};
use strata_core::model::{Location, MetadataValue};
use strata_fs::services::{MetadataUpdate, PreviewContent, SandboxedMedia};
use strata_preview::{
    media::{Header, peaks as preview_peaks, storyboard as preview_storyboard},
    sandbox::{
        MediaPreviewBackend,
        media::{PeaksEvent, Session, StoryboardEvent, fixtures},
    },
};

use super::{
    MODULE, Viewer, disable_auto_preview, focus_view, focused_key, press,
    provider::{Recorded, install, listed_with},
    select, space,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    preview::{
        media::{
            MEDIA_KEY, MockPeaks, MockStoryboard, START_DECODER, START_PEAKS, START_STORYBOARD,
            clear_peaks_cache, clear_storyboard_cache, deliver_peaks,
        },
        session,
    },
    test_support::{PATIENCE, hosted},
};

/// Bytes no text is: what a listing reads as backing a video's name.
const MP4: &str = "\0\0\0\u{18}ftypmp42\0\0\0\0";

fn state(host: &Offscreen) -> Option<&MediaState> {
    match &host.app().preview.viewer {
        Viewer::Media(media) => Some(&media.state),
        _ => None,
    }
}

fn audio_levels(host: &Offscreen) -> Option<(f32, bool)> {
    match &host.app().preview.viewer {
        Viewer::Media(media) => media.audio_levels(),
        _ => None,
    }
}

fn shows(host: &mut Offscreen, done: impl Fn(&MediaState) -> bool, what: &str) {
    let shown = host.run_until(PATIENCE, |host| state(host).is_some_and(&done));
    assert!(
        shown,
        "the media player: {what}: {:?}",
        state(host).map(|s| (
            s.clip().position,
            s.clip().has_next,
            s.playing(),
            s.position(),
            s.duration(),
            s.error().map(|e| e.detail.clone()),
        ))
    );
}

fn decode_with(start: fn(SandboxedMedia, u32) -> Result<Session, String>) {
    START_DECODER.set(Some(start));
}

/// Ten seconds of 16 × 16 frames.
fn ten_seconds(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 16,
        height: 16,
        audio: false,
        duration_us: 10_000_000,
        start_tick,
    })
}

/// The provider's answer for the media file at `path`.
fn sandboxed(path: PathBuf) -> PreviewContent {
    PreviewContent::SandboxedMedia {
        media: SandboxedMedia {
            path,
            size: strata_fs::services::MediaPreviewSize::new(320, 180),
            backend: MediaPreviewBackend::Software,
            input_owner: None,
            audio_only: false,
        },
    }
}

/// Gives the listed `names` a length, as a metadata probe of real media does.
fn probed(host: &mut Offscreen, names: &'static [&'static str]) {
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        let listed: Vec<_> = (0..).map_while(|at| app.browser.entry_at(0, at)).collect();
        for (position, entry) in listed.into_iter().enumerate() {
            if names.contains(&entry.display_name.as_str()) {
                let update = MetadataUpdate {
                    location: entry.location,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    created_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Known((16, 16)),
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Known(10),
                    listable: MetadataValue::Unknown,
                };
                app.browser.apply_thumbnail_metadata(0, position, update);
            }
        }
    });
    while host.turn() {}
}

/// Opens the preview on `name` in a folder holding `files`, and answers it as `content_type`
/// media; the provider keeps answering what follows.
fn open(
    host: &mut Offscreen,
    files: &[(&str, &str)],
    name: &str,
    content_type: &str,
) -> (Recorded, tempfile::TempDir) {
    let provider = Recorded::default();
    install(host, &provider);
    let folder = listed_with(host, files);
    probed(host, &["alpha.mp4", "beta.mp4"]);
    disable_auto_preview(host);
    select(host, &Location::local(folder.path().join(name)));
    focus_view(host);
    space(host);
    provider.wait_for(host, 0);
    provider.answer_as(0, content_type, sandboxed(folder.path().join(name)));
    (provider, folder)
}

fn ctrl_alt() -> Mods {
    Mods {
        ctrl: true,
        alt: true,
        ..Mods::default()
    }
}

fn play_pause(host: &mut Offscreen) {
    press(host, Key::Text(" ".to_owned()), ctrl_alt());
}

#[test]
fn a_video_plays_on_the_media_key_and_its_clock_moves() {
    hosted(
        MODULE,
        "media::a_video_plays_on_the_media_key_and_its_clock_moves",
        |mut host| {
            decode_with(ten_seconds);
            let (_provider, _folder) =
                open(&mut host, &[("alpha.mp4", MP4)], "alpha.mp4", "video/mp4");
            let ready =
                |s: &MediaState| s.has_frame() && s.duration() == Some(Duration::from_secs(10));
            shows(&mut host, ready, "the first frame and the duration");
            assert_eq!(state(&host).map(|s| s.clip().kind), Some(MediaKind::Video));
            assert!(
                state(&host).is_some_and(|s| !s.playing()),
                "paused, autoplay being off"
            );

            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "playing on Ctrl+Alt+Space");
            let moved = |s: &MediaState| s.position() >= Duration::from_secs(1);
            shows(&mut host, moved, "the clock moving while it plays");
            play_pause(&mut host);
            shows(&mut host, |s| !s.playing(), "paused on Ctrl+Alt+Space");
        },
    );
}

#[test]
fn next_steps_past_text_named_as_video_and_keeps_playing() {
    hosted(
        MODULE,
        "media::next_steps_past_text_named_as_video_and_keeps_playing",
        |mut host| {
            decode_with(ten_seconds);
            let files = [
                ("alpha.mp4", MP4),
                ("bait.mp4", "not a video\n"),
                ("beta.mp4", MP4),
            ];
            let (provider, _folder) = open(&mut host, &files, "alpha.mp4", "video/mp4");
            let first = Position {
                index: 1,
                count: 2,
                results: false,
            };
            let counted = |s: &MediaState| s.clip().position == Some(first) && s.clip().has_next;
            shows(&mut host, counted, "1 of 2, the bait left out");
            assert!(state(&host).is_some_and(|s| !s.clip().has_previous));
            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "playing before the step");
            // The keyboard in the player: the next one takes it (GTK's `keep_keys_in_content`).
            press(&mut host, Key::Right, Mods::default());
            let in_player = |host: &mut Offscreen| {
                focused_key(host).is_some_and(|key| key.starts_with(MEDIA_KEY))
            };
            assert!(in_player(&mut host), "Right gives the player the keyboard");

            let next = Mods {
                shift: true,
                ..ctrl_alt()
            };
            press(&mut host, Key::Text(">".to_owned()), next);
            provider.wait_for(&mut host, 1);
            assert_eq!(provider.request(1).entry.display_name, "beta.mp4");
            let selected = session::selected_entry(host.app()).1;
            assert_eq!(
                selected.map(|entry| entry.display_name).as_deref(),
                Some("beta.mp4")
            );
            let beta = provider
                .request(1)
                .entry
                .location
                .native_path()
                .map(Path::to_path_buf);
            provider.answer_as(1, "video/mp4", sandboxed(beta.expect("a local file")));
            let second = Position { index: 2, ..first };
            let stepped = |s: &MediaState| s.clip().position == Some(second) && s.playing();
            shows(&mut host, stepped, "2 of 2, playing on");
            let mut took = in_player(&mut host);
            for _ in 0..600 {
                if took {
                    break;
                }
                host.turn();
                took = in_player(&mut host);
            }
            assert!(took, "the next player has the keyboard");
        },
    );
}

#[test]
fn a_decode_that_fails_shows_its_error_in_the_player() {
    hosted(
        MODULE,
        "media::a_decode_that_fails_shows_its_error_in_the_player",
        |mut host| {
            decode_with(|_, _| fixtures::failing_stream());
            let (_provider, _folder) =
                open(&mut host, &[("alpha.mp4", MP4)], "alpha.mp4", "video/mp4");
            let failed = |s: &MediaState| {
                s.error().is_some_and(|error| {
                    error
                        .detail
                        .starts_with("Unable to play this media preview")
                })
            };
            shows(&mut host, failed, "the decoder's error");
            play_pause(&mut host);
            assert!(
                state(&host).is_some_and(|s| !s.playing()),
                "no play over an error"
            );
        },
    );
}

#[test]
fn a_gif_plays_in_the_plain_bar_and_loops() {
    hosted(
        MODULE,
        "media::a_gif_plays_in_the_plain_bar_and_loops",
        |mut host| {
            decode_with(half_a_second_of_pictures);
            let (_provider, _folder) =
                open(&mut host, &[("spin.gif", MP4)], "spin.gif", "image/gif");
            shows(&mut host, MediaState::has_frame, "the first frame");
            assert_eq!(
                state(&host).map(|s| s.clip().kind),
                Some(MediaKind::Animation)
            );
            // Strata's GIF bar is its play toggle alone: no time, seek or volume.
            let keys: Vec<String> = host
                .runtime()
                .elements()
                .into_iter()
                .filter_map(|el| el.key)
                .filter(|key| key.starts_with(MEDIA_KEY))
                .collect();
            assert!(keys.iter().any(|key| key.ends_with("-play")), "{keys:?}");
            for part in ["-time", "-timeline", "-mute", "-volume", "-title"] {
                assert!(
                    !keys.iter().any(|key| key.ends_with(part)),
                    "{part} in {keys:?}"
                );
            }
            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "playing");
            let late = host.run_until(PATIENCE, |host| {
                state(host).is_some_and(|s| s.position() >= Duration::from_millis(300))
            });
            assert!(late, "past the middle");
            let looped = host.run_until(PATIENCE, |host| {
                state(host)
                    .is_some_and(|s| s.playing() && s.position() < Duration::from_millis(200))
            });
            assert!(looped, "back at the start and playing, not ended");
        },
    );
}

/// Ten seconds of sound and no picture.
fn ten_seconds_of_sound(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 0,
        height: 0,
        audio: true,
        duration_us: 10_000_000,
        start_tick,
    })
}

/// Half a second of 16 by 16 pictures and no sound, as a GIF decodes.
fn half_a_second_of_pictures(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 16,
        height: 16,
        audio: false,
        duration_us: 500_000,
        start_tick,
    })
}

/// Whether the player has the keyboard.
fn in_player(host: &mut Offscreen) -> bool {
    focused_key(host).is_some_and(|key| key.starts_with(MEDIA_KEY))
}

/// With the keyboard in the drawer, Strata's audio player takes the player's keys, Left and
/// Right seeking, and keeps the keyboard.
#[test]
fn an_audio_player_keeps_left_and_right() {
    hosted(
        MODULE,
        "media::an_audio_player_keeps_left_and_right",
        |mut host| {
            decode_with(ten_seconds_of_sound);
            let files = [("tone.wav", MP4)];
            let (_provider, _folder) = open(&mut host, &files, "tone.wav", "audio/vnd.wave");
            let known = |s: &MediaState| s.duration() == Some(Duration::from_secs(10));
            shows(&mut host, known, "the audio's length");
            press(&mut host, Key::Right, Mods::default());
            assert!(
                in_player(&mut host),
                "Right gives the audio player the keyboard"
            );
            // Left at the start seeks nowhere, and the player still keeps it.
            press(&mut host, Key::Left, Mods::default());
            assert!(in_player(&mut host), "Left at 0:00 stays in the player");
            press(&mut host, Key::Right, Mods::default());
            let ahead = |s: &MediaState| s.position() >= Duration::from_secs(5);
            shows(&mut host, ahead, "Right seeks forward");
            press(&mut host, Key::Left, Mods::default());
            let back = |s: &MediaState| s.position() < Duration::from_secs(5);
            shows(&mut host, back, "Left seeks back");
            assert!(in_player(&mut host), "the keyboard stays in the player");
        },
    );
}

/// A GIF's overlay takes none of the player's keys, as Strata's does: Left moves to its play
/// toggle, as GTK's directional focus does, and a second Left goes back to the listing.
#[test]
fn a_gif_gives_left_to_its_play_toggle_then_back() {
    hosted(
        MODULE,
        "media::a_gif_gives_left_to_its_play_toggle_then_back",
        |mut host| {
            decode_with(half_a_second_of_pictures);
            let (_provider, _folder) =
                open(&mut host, &[("spin.gif", MP4)], "spin.gif", "image/gif");
            let animation = |s: &MediaState| s.clip().kind == MediaKind::Animation;
            shows(&mut host, animation, "the GIF");
            press(&mut host, Key::Right, Mods::default());
            assert!(
                in_player(&mut host),
                "Right gives the GIF's frame the keyboard"
            );
            press(&mut host, Key::Left, Mods::default());
            let play = format!("{MEDIA_KEY}-play");
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(play.as_str()),
                "Left moves to the play toggle"
            );
            press(&mut host, Key::Left, Mods::default());
            assert!(
                !in_player(&mut host),
                "a second Left goes back to the listing"
            );
        },
    );
}

/// Half a second of audio-only PCM.
fn half_a_second_of_sound(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 0,
        height: 0,
        audio: true,
        duration_us: 500_000,
        start_tick,
    })
}

/// One and a half seconds of 16 by 16 video frames with interleaved audio PCM.
fn video_with_sound(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 16,
        height: 16,
        audio: true,
        duration_us: 1_500_000,
        start_tick,
    })
}

#[test]
fn audio_preview_advances_on_pcm_clock_applies_mute_and_reaches_eof() {
    hosted(
        MODULE,
        "media::audio_preview_advances_on_pcm_clock_applies_mute_and_reaches_eof",
        |mut host| {
            decode_with(half_a_second_of_sound);
            let files = [("tone.wav", MP4)];
            let (_provider, _folder) = open(&mut host, &files, "tone.wav", "audio/vnd.wave");
            let ready = |s: &MediaState| s.duration() == Some(Duration::from_millis(500));
            shows(&mut host, ready, "the audio duration");

            // Mute shortcut reaches the live PCM sink immediately.
            press(&mut host, Key::Text("m".to_owned()), ctrl_alt());
            let muted = host.run_until(PATIENCE, |host| {
                audio_levels(host).is_some_and(|(_, muted)| muted)
            });
            assert!(muted, "mute state propagates to PcmOutput");

            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "audio playing on PCM clock");
            let moved = |s: &MediaState| s.position() >= Duration::from_millis(120);
            shows(&mut host, moved, "audio clock advancing");

            // Pause freezes playback; play resumes and drains cleanly to EOF.
            play_pause(&mut host);
            shows(&mut host, |s| !s.playing(), "audio paused");
            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "audio resumed");
            let ended = |s: &MediaState| !s.playing() && s.position() >= Duration::from_millis(450);
            shows(&mut host, ended, "audio drains to EOF");
        },
    );
}

#[test]
fn video_with_audio_paces_frames_to_pcm_clock_and_reaches_eof() {
    hosted(
        MODULE,
        "media::video_with_audio_paces_frames_to_pcm_clock_and_reaches_eof",
        |mut host| {
            decode_with(video_with_sound);
            let (_provider, _folder) =
                open(&mut host, &[("clip.mp4", MP4)], "clip.mp4", "video/mp4");
            let ready = |s: &MediaState| {
                s.has_frame() && s.duration() == Some(Duration::from_millis(1_500))
            };
            shows(&mut host, ready, "first frame and duration with audio");
            assert!(
                audio_levels(&host).is_some(),
                "video with audio opens PcmOutput"
            );

            play_pause(&mut host);
            shows(&mut host, MediaState::playing, "video+audio playing");
            let moved = |s: &MediaState| s.position() >= Duration::from_millis(300);
            shows(&mut host, moved, "video+audio clock advancing");
            let ended =
                |s: &MediaState| !s.playing() && s.position() >= Duration::from_millis(1_450);
            shows(&mut host, ended, "video+audio reaches EOF");
        },
    );
}

fn viewer_id(host: &Offscreen) -> Option<u64> {
    match &host.app().preview.viewer {
        Viewer::Media(media) => Some(media.viewer_id()),
        _ => None,
    }
}

type PeaksProbe = (mpsc::Sender<PeaksEvent>, Arc<AtomicBool>);
type StoryboardProbe = (mpsc::Sender<StoryboardEvent>, Arc<AtomicBool>);

thread_local! {
    static PEAKS_PROBES: RefCell<Vec<PeaksProbe>> = const { RefCell::new(Vec::new()) };
    static STORYBOARD_PROBES: RefCell<Vec<StoryboardProbe>> = const { RefCell::new(Vec::new()) };
}

struct BackgroundHooks;

impl BackgroundHooks {
    fn install(peaks: bool, storyboard: bool) -> Self {
        clear_peaks_cache();
        clear_storyboard_cache();
        PEAKS_PROBES.with_borrow_mut(Vec::clear);
        STORYBOARD_PROBES.with_borrow_mut(Vec::clear);
        START_PEAKS
            .set(peaks.then_some(record_peaks_start as fn(SandboxedMedia) -> Option<MockPeaks>));
        START_STORYBOARD.set(storyboard.then_some(
            record_storyboard_start as fn(SandboxedMedia, u32) -> Option<MockStoryboard>,
        ));
        Self
    }
}

impl Drop for BackgroundHooks {
    fn drop(&mut self) {
        START_PEAKS.set(None);
        START_STORYBOARD.set(None);
        PEAKS_PROBES.with_borrow_mut(Vec::clear);
        STORYBOARD_PROBES.with_borrow_mut(Vec::clear);
        clear_peaks_cache();
        clear_storyboard_cache();
    }
}

fn record_peaks_start(_: SandboxedMedia) -> Option<MockPeaks> {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    PEAKS_PROBES.with_borrow_mut(|probes| probes.push((sender, cancelled.clone())));
    Some(MockPeaks::new(receiver, cancelled))
}

fn record_storyboard_start(_: SandboxedMedia, _: u32) -> Option<MockStoryboard> {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    STORYBOARD_PROBES.with_borrow_mut(|probes| probes.push((sender, cancelled.clone())));
    Some(MockStoryboard::new(receiver, cancelled))
}

fn peaks_started() -> usize {
    PEAKS_PROBES.with_borrow(Vec::len)
}

fn storyboard_started() -> usize {
    STORYBOARD_PROBES.with_borrow(Vec::len)
}

fn wait_for_probe(host: &mut Offscreen, ready: impl Fn() -> bool, what: &str) {
    let arrived = host.run_until(PATIENCE, |_| ready());
    assert!(arrived, "background media decode: {what}");
}

fn dwell(host: &mut Offscreen, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        host.turn();
    }
}

/// Ten seconds of 16 by 16 video frames with audio, long enough for a storyboard.
fn ten_seconds_video_with_sound(_: SandboxedMedia, start_tick: u32) -> Result<Session, String> {
    fixtures::stream(Header {
        width: 16,
        height: 16,
        audio: true,
        duration_us: 10_000_000,
        start_tick,
    })
}

/// Video whose audio depends on the filename: `alpha.mp4` carries audio, `beta.mp4` is silent.
fn alpha_with_audio_beta_silent(
    source: SandboxedMedia,
    start_tick: u32,
) -> Result<Session, String> {
    let audio = source
        .path
        .file_name()
        .is_some_and(|name| name == "alpha.mp4");
    fixtures::stream(Header {
        width: 16,
        height: 16,
        audio,
        duration_us: 1_500_000,
        start_tick,
    })
}

#[test]
fn audio_preview_streams_waveform_peaks_into_timeline_levels() {
    hosted(
        MODULE,
        "media::audio_preview_streams_waveform_peaks_into_timeline_levels",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, false);
            decode_with(ten_seconds_of_sound);
            let (_provider, _folder) = open(
                &mut host,
                &[("tone.wav", MP4)],
                "tone.wav",
                "audio/vnd.wave",
            );
            shows(
                &mut host,
                |s| s.duration() == Some(Duration::from_secs(10)),
                "audio duration ready",
            );
            wait_for_probe(&mut host, || peaks_started() == 1, "audio-peaks starts");

            let sender = PEAKS_PROBES.with_borrow(|probes| probes[0].0.clone());
            // First half arrives at mid loudness while paused; second half is still empty.
            sender
                .send(PeaksEvent::Levels {
                    start: 0,
                    levels: vec![128; 512],
                })
                .expect("send first peaks run");
            shows(
                &mut host,
                |s| {
                    let bars = s.levels().bars(4);
                    bars[0] > 0.9 && bars[1] > 0.9 && bars[2] == 0.0 && bars[3] == 0.0
                },
                "first half of waveform bars populated progressively",
            );

            // Second half arrives at full loudness; first half rescales proportionally.
            sender
                .send(PeaksEvent::Levels {
                    start: 512,
                    levels: vec![255; 512],
                })
                .expect("send second peaks run");
            sender.send(PeaksEvent::Finished).expect("finish peaks");
            let expected_ratio = preview_peaks::rms(128) / preview_peaks::rms(255);
            shows(
                &mut host,
                |s| {
                    let bars = s.levels().bars(4);
                    (bars[0] - expected_ratio).abs() < 0.02 && (bars[3] - 1.0).abs() < 0.02
                },
                "full waveform reflects relative loudness",
            );
        },
    );
}

#[test]
fn video_with_audio_streams_storyboard_then_waveform_peaks() {
    hosted(
        MODULE,
        "media::video_with_audio_streams_storyboard_then_waveform_peaks",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, true);
            decode_with(ten_seconds_video_with_sound);
            let (_provider, _folder) =
                open(&mut host, &[("clip.mp4", MP4)], "clip.mp4", "video/mp4");
            shows(&mut host, MediaState::has_frame, "video first frame ready");
            wait_for_probe(
                &mut host,
                || storyboard_started() == 1,
                "video-storyboard starts after first frame",
            );
            assert_eq!(
                peaks_started(),
                0,
                "audio-peaks waits for video-storyboard to release the shared background slot"
            );

            let sb_sender = STORYBOARD_PROBES.with_borrow(|probes| probes[0].0.clone());
            let sheet = preview_storyboard::Sheet {
                width: 4,
                height: 4,
                count: 8,
                duration_us: 10_000_000,
            };
            sb_sender
                .send(StoryboardEvent::Sheet(sheet))
                .expect("send storyboard sheet");
            sb_sender
                .send(StoryboardEvent::Cell {
                    index: 4,
                    pixels: vec![0x80; sheet.cell_bytes()],
                })
                .expect("send storyboard cell");
            shows(
                &mut host,
                |s| s.storyboard().is_some_and(|board| board.loaded() == 1),
                "storyboard cell ingested",
            );

            sb_sender
                .send(StoryboardEvent::Finished)
                .expect("finish storyboard");
            wait_for_probe(
                &mut host,
                || peaks_started() == 1,
                "audio-peaks starts once storyboard finishes",
            );

            let peaks_sender = PEAKS_PROBES.with_borrow(|probes| probes[0].0.clone());
            peaks_sender
                .send(PeaksEvent::Levels {
                    start: 0,
                    levels: vec![200; preview_peaks::BUCKETS as usize],
                })
                .expect("send video peaks");
            peaks_sender.send(PeaksEvent::Finished).expect("finish");
            shows(
                &mut host,
                |s| s.levels().bars(4).iter().all(|&level| level > 0.9),
                "video timeline waveform populated",
            );
        },
    );
}

#[test]
fn gif_skips_audio_peaks_and_storyboard() {
    hosted(
        MODULE,
        "media::gif_skips_audio_peaks_and_storyboard",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, false);
            decode_with(half_a_second_of_pictures);
            let (_provider, _folder) =
                open(&mut host, &[("spin.gif", MP4)], "spin.gif", "image/gif");
            shows(&mut host, MediaState::has_frame, "gif frame ready");
            dwell(&mut host, Duration::from_millis(160));
            assert_eq!(peaks_started(), 0, "GIF never starts audio-peaks");
            assert_eq!(storyboard_started(), 0, "GIF never starts video-storyboard");
        },
    );
}

#[test]
fn silent_video_skips_audio_peaks() {
    hosted(
        MODULE,
        "media::silent_video_skips_audio_peaks",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, false);
            decode_with(ten_seconds);
            let (_provider, _folder) =
                open(&mut host, &[("silent.mp4", MP4)], "silent.mp4", "video/mp4");
            shows(&mut host, MediaState::has_frame, "silent video frame ready");
            dwell(&mut host, Duration::from_millis(160));
            assert_eq!(peaks_started(), 0, "silent video skips audio-peaks");
            assert!(
                state(&host).is_some_and(|s| s.levels().is_empty()),
                "silent video keeps empty waveform levels"
            );
        },
    );
}

#[test]
fn switching_media_cancels_peaks_and_discards_stale_events() {
    hosted(
        MODULE,
        "media::switching_media_cancels_peaks_and_discards_stale_events",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, false);
            decode_with(alpha_with_audio_beta_silent);
            let files = [("alpha.mp4", MP4), ("beta.mp4", MP4)];
            let (provider, _folder) = open(&mut host, &files, "alpha.mp4", "video/mp4");
            shows(&mut host, MediaState::has_frame, "alpha video ready");
            wait_for_probe(
                &mut host,
                || peaks_started() == 1,
                "alpha starts audio-peaks",
            );
            let old_id = viewer_id(&host).expect("alpha viewer id");
            let (old_sender, old_cancelled) = PEAKS_PROBES.with_borrow(|probes| probes[0].clone());
            assert!(!old_cancelled.load(Ordering::Acquire));

            // Step to beta.mp4 (silent) while alpha's audio-peaks handle is still active.
            let next = Mods {
                shift: true,
                ..ctrl_alt()
            };
            press(&mut host, Key::Text(">".to_owned()), next);
            provider.wait_for(&mut host, 1);
            let beta = provider
                .request(1)
                .entry
                .location
                .native_path()
                .map(Path::to_path_buf)
                .expect("beta path");
            provider.answer_as(1, "video/mp4", sandboxed(beta));
            shows(
                &mut host,
                |s| s.clip().name == "beta.mp4" && s.has_frame(),
                "beta video ready",
            );
            assert!(
                old_cancelled.load(Ordering::Acquire),
                "switching media cancels previous audio-peaks handle immediately"
            );

            // Late events on the old channel and direct stale-id deliveries are both discarded.
            let _ = old_sender.send(PeaksEvent::Levels {
                start: 0,
                levels: vec![255; 256],
            });
            let mut accepted = true;
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                accepted = deliver_peaks(app, old_id, vec![(0, vec![255; 256])], tx);
            });
            assert!(!accepted, "stale viewer generation batch is discarded");
            dwell(&mut host, Duration::from_millis(120));
            assert!(
                state(&host).is_some_and(|s| s.levels().is_empty()),
                "beta's silent timeline stays unaffected by alpha's stale peaks"
            );
        },
    );
}

#[test]
fn closing_preview_cancels_active_peaks() {
    hosted(
        MODULE,
        "media::closing_preview_cancels_active_peaks",
        |mut host| {
            let _hooks = BackgroundHooks::install(true, false);
            decode_with(ten_seconds_of_sound);
            let (_provider, _folder) = open(
                &mut host,
                &[("tone.wav", MP4)],
                "tone.wav",
                "audio/vnd.wave",
            );
            shows(
                &mut host,
                |s| s.duration() == Some(Duration::from_secs(10)),
                "audio ready",
            );
            wait_for_probe(
                &mut host,
                || peaks_started() == 1,
                "audio starts audio-peaks",
            );
            let cancelled = PEAKS_PROBES.with_borrow(|probes| probes[0].1.clone());
            assert!(!cancelled.load(Ordering::Acquire));

            space(&mut host);
            let closed = host.run_until(PATIENCE, |host| state(host).is_none());
            assert!(closed, "preview drawer closed");
            assert!(
                cancelled.load(Ordering::Acquire),
                "closing preview cancels active audio-peaks handle immediately"
            );
        },
    );
}
