// SPDX-License-Identifier: MIT

//! C13 [`MediaPlayer`] host for sandboxed audio and video (`PreviewContent::SandboxedMedia`),
//! as GTK's `render_sandboxed_media` (`src/ui/preview/media_render.rs`) and its video and
//! audio views host them:
//!
//! - **The view** is by the file's type: video, audio, or the plain media bar for media that
//!   is neither, as an animated GIF, which loops.
//! - **Playback** ([`stream`]): decoded frames come from the shared sandboxed decoder off the UI
//!   thread and reach Rune through `Later`, while decoded PCM plays through `strata_platform::media`
//!   (PipeWire/PulseAudio, ALSA, or a silent real-time clock fallback when headless) and drives A/V
//!   sync. Autoplay follows Settings, a video opens at the saved start percentage, and one left
//!   partway this session reopens where it was.
//! - **The eyebrow** counts the folder's files of the clip's kind whose bytes back their name
//!   ([`sequence`]), and the previous and next buttons and keys step through them.
//! - **The badges** come from the sandboxed metadata probe ([`details`]).
//! - **The timeline waveform & storyboard** ([`peaks`], [`storyboard`]): videos of at least 4 s
//!   stream keyframe storyboard cells on the shared background decode slot, then hand the slot to
//!   `audio-peaks` (which audio files start directly) so timeline bars grow in from decoded RMS
//!   levels; GIFs and silent videos skip `audio-peaks`.
//! - **The media keys** reach the player where [`super::keys::media_keys_reach`] holds
//!   ([`super::transport`]).
//! - **The volume** is Settings' `preview_volume` and `preview_muted`, saved as it changes.
//!
//! Every element is keyed under [`MEDIA_KEY`].

use std::{cell::Cell, time::Duration};

use rune::{Cx, El, Pixels, Tx};
use strata_components::{
    media_player::{
        Clip, Details, Labels, MediaCommand, MediaError, MediaIntent, MediaKeys, MediaKind,
        MediaPlayer, MediaState, audio::spectrum as spectrum_bars,
    },
    viewers::{layout::Position, viewport::ViewportState},
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::keys::{
    Keys as Shown,
    bindings::{MEDIA_MUTE, MEDIA_NEXT, MEDIA_PLAY_PAUSE, MEDIA_PREVIOUS},
};
use strata_fs::services::{SandboxedMedia, is_raw_video_stream};

use super::{
    labels,
    viewer::{self, Viewer},
};
use crate::{App, location::labels::with_keys, shell, slots::Later};

mod audio_details;
mod cache;
mod details;
mod history;
mod peaks;
mod positions;
pub(super) mod sequence;
mod spectrum;
mod storyboard;
mod stream;

#[cfg(test)]
pub(in crate::preview) use audio_details::{
    PARSE as PARSE_AUDIO_DETAILS, clear_cache as clear_audio_details_cache,
};
#[cfg(test)]
pub(in crate::preview) use peaks::{
    MockPeaks, START as START_PEAKS, clear_cache as clear_peaks_cache,
};
#[cfg(test)]
pub(in crate::preview) use storyboard::{
    MockStoryboard, START as START_STORYBOARD, clear_cache as clear_storyboard_cache,
};
#[cfg(test)]
pub(in crate::preview) use stream::START as START_DECODER;

pub(crate) use sequence::Checks;
use stream::{Inlet, Received, Stream};

/// Key of the media player inside the preview drawer.
pub(crate) const MEDIA_KEY: &str = "preview-media";

thread_local! {
    static NEXT_VIEWER: Cell<u64> = const { Cell::new(1) };
}

/// The kind of file a player steps through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    Audio,
    Video,
}

pub(super) struct MediaViewer {
    pub(super) state: MediaState,
    pixels: Pixels,
    stream: Stream,
    entry: FileEntry,
    family: Option<Family>,
    id: u64,
    details: Option<details::DetailsLoad>,
    audio_details: Option<audio_details::AudioDetailsLoad>,
    spectrum: Option<spectrum::SpectrumFeed>,
    peaks: Option<peaks::PeaksLoad>,
    peaks_started: bool,
    storyboard: Option<storyboard::StoryboardLoad>,
    storyboard_started: bool,
    storyboard_done: bool,
}

/// A sandboxed media preview, as the provider answered it.
pub(super) struct Arrived {
    pub(super) entry: FileEntry,
    pub(super) content_type: String,
    pub(super) media: SandboxedMedia,
}

/// What Settings say a player opens with.
struct Opening {
    start_percent: u32,
    autoplay: bool,
    volume: f32,
    muted: bool,
    tenxer: bool,
    /// A GIF: the frame and its play toggle alone (`MediaKind::Animation`).
    animation: bool,
}

impl Opening {
    fn of(app: &App, content_type: &str) -> Self {
        let prefs = shell::preferences(app);
        // A GIF and a raw stream start at their first frame (`render_sandboxed_media`).
        let from_start = content_type == GIF || is_raw_video_stream(content_type);
        Self {
            start_percent: if from_start {
                0
            } else {
                prefs.preview_video_start
            },
            autoplay: prefs.preview_autoplay,
            volume: prefs.preview_volume as f32,
            muted: prefs.preview_muted,
            tenxer: prefs.tenxer_mode,
            animation: content_type == GIF,
        }
    }
}

const GIF: &str = "image/gif";

/// A sandboxed media preview arrived: the player for its kind, decoding from the start.
pub(super) fn ready(app: &mut App, arrived: Arrived, tx: &mut Tx) -> Viewer {
    let Arrived {
        entry,
        content_type,
        mut media,
    } = arrived;
    let id = NEXT_VIEWER.with(|next| next.replace(next.get().wrapping_add(1)));
    let family = sequence::family_of_type(&content_type);
    media.audio_only = family == Some(Family::Audio);
    let opening = Opening::of(app, &content_type);
    let later = app.preview.loader.later.clone();
    let details = (family == Some(Family::Video))
        .then(|| details::load(&entry, &media, (later.clone(), id), detailed));
    let inlet = Inlet {
        later: later.clone(),
        viewer: id,
        deliver: received,
    };
    let mut stream = Stream::new(media, inlet, opening.start_percent, tx.clock());
    if family == Some(Family::Audio) {
        stream.retain_played_audio();
    }
    stream.set_loop(content_type == GIF);
    stream.set_audio(opening.volume, opening.muted);
    let stepped = app
        .preview
        .stepped
        .take()
        .filter(|stepped| stepped.to == entry.location && family.is_some());
    if opening.autoplay || stepped.as_ref().is_some_and(|stepped| stepped.playing) {
        stream.play();
    }
    if stepped.is_some_and(|stepped| stepped.keys) {
        super::arm_focus_handoff(app, tx);
        app.preview.drawer.focus_content_when_shown(tx);
    }
    let mut viewer = MediaViewer {
        state: player(&entry, family, &opening),
        pixels: Pixels::new(),
        stream,
        entry,
        family,
        id,
        details,
        audio_details: None,
        spectrum: (family == Some(Family::Audio)).then(spectrum::SpectrumFeed::new),
        peaks: None,
        peaks_started: false,
        storyboard: None,
        storyboard_started: false,
        storyboard_done: false,
    };
    viewer.restore_cached_background(tx);
    viewer.load_audio_details(later, tx);
    let (position, previous, next) = place(app, family, &viewer.entry.location);
    viewer.state.set_listing(position, previous, next);
    Viewer::Media(Box::new(viewer))
}

/// A step from one media file to the next (GTK's `step_media`): the file it moved to plays on
/// if the one it left was playing, and takes the keyboard if the player had it
/// (`keep_keys_in_content`).
pub(super) struct Stepped {
    to: Location,
    playing: bool,
    keys: bool,
}

/// The player's state for `entry`: its view, words, keys and volume.
fn player(entry: &FileEntry, family: Option<Family>, opening: &Opening) -> MediaState {
    let mut state = MediaState::new(Clip {
        kind: match family {
            Some(Family::Video) => MediaKind::Video,
            Some(Family::Audio) => MediaKind::Audio,
            None if opening.animation => MediaKind::Animation,
            None => MediaKind::Plain,
        },
        name: entry.display_name.clone(),
        ..Clip::default()
    });
    state.set_labels(media_labels());
    state.set_keys(if opening.tenxer {
        MediaKeys::tenxer()
    } else {
        MediaKeys::default_map()
    });
    state.set_volume(opening.volume, opening.muted);
    state.set_details(match family {
        Some(Family::Video) => Details::Loading,
        _ => Details::Failed,
    });
    state
}

/// The clip's place among its kind's files, and whether it has a previous and a next.
fn place(
    app: &mut App,
    family: Option<Family>,
    location: &Location,
) -> (Option<Position>, bool, bool) {
    family
        .and_then(|family| sequence::listing(app, family, location, None))
        .map_or((None, false, false), |(files, index)| {
            let position = Position {
                index: index + 1,
                count: files.len(),
                results: false,
            };
            (Some(position), index > 0, index + 1 < files.len())
        })
}

/// Reads that answered refresh the shown player's place. True when one shows.
pub(super) fn refresh_listing(app: &mut App) -> bool {
    let Viewer::Media(media) = &app.preview.viewer else {
        return false;
    };
    let (family, location) = (media.family, media.entry.location.clone());
    let (position, previous, next) = place(app, family, &location);
    let Viewer::Media(media) = &mut app.preview.viewer else {
        return false;
    };
    media.state.set_listing(position, previous, next);
    true
}

/// A pump's events for the player `viewer`, dropped when it has gone.
fn received(app: &mut App, (viewer, session): (u64, u64), events: Received, tx: &mut Tx) -> bool {
    let later = app.preview.loader.later.clone();
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    let frame = media.stream.receive(session, events);
    media.show(frame);
    media.sync_error();
    if media.family.is_some() && !media.stream.has_video() {
        media.maybe_start_peaks(later, tx);
    }
    // The poll's tick reads the clock: the duration, the first frame, an error.
    tx.wake_after(Duration::ZERO);
    true
}

/// The probe's answer for the player `viewer`.
fn detailed(app: &mut App, viewer: u64, details: Details) -> bool {
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    media.state.set_details(details);
    true
}

/// Audio tags or cover art for the player `viewer`, dropped when it has gone.
fn audio_detailed(app: &mut App, viewer: u64, update: audio_details::Update, tx: &mut Tx) -> bool {
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    match update {
        audio_details::Update::Tags(tags) => media.state.set_tags(tags),
        audio_details::Update::Cover(art) => {
            media.state.set_art(art);
            tx.wake_after(Duration::ZERO);
        }
    }
    true
}

/// Waveform peak batches for the player `viewer`, dropped when it has gone.
fn peaks_received(app: &mut App, viewer: u64, batch: peaks::Batch, tx: &mut Tx) -> bool {
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    for (start, levels) in batch {
        apply_levels(&mut media.state, start, &levels);
    }
    tx.wake_after(Duration::ZERO);
    true
}

#[cfg(test)]
pub(in crate::preview) fn deliver_peaks(
    app: &mut App,
    viewer: u64,
    batch: peaks::Batch,
    tx: &mut Tx,
) -> bool {
    peaks_received(app, viewer, batch, tx)
}

/// Storyboard progress for the player `viewer`, dropped when it has gone.
fn storyboard_updated(app: &mut App, viewer: u64, update: storyboard::Update, tx: &mut Tx) -> bool {
    let later = app.preview.loader.later.clone();
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    match update {
        storyboard::Update::Board(board) => {
            media.state.set_storyboard(Some(board));
            tx.wake_after(Duration::ZERO);
            true
        }
        storyboard::Update::Done => {
            media.storyboard = None;
            media.storyboard_done = true;
            media.maybe_start_peaks(later, tx);
            true
        }
    }
}

fn apply_levels(state: &mut MediaState, start: u32, levels: &[u8]) {
    let rms: Vec<f32> = levels
        .iter()
        .copied()
        .map(strata_preview::media::peaks::rms)
        .collect();
    state.set_levels(
        strata_preview::media::peaks::BUCKETS as usize,
        start as usize,
        &rms,
    );
}

fn shown(app: &mut App, viewer: u64) -> Option<&mut MediaViewer> {
    match &mut app.preview.viewer {
        Viewer::Media(media) if media.id == viewer => Some(media),
        _ => None,
    }
}

impl MediaViewer {
    fn restore_cached_background(&mut self, tx: &mut Tx) {
        if self.family.is_some()
            && let Some(cached) = peaks::cached(&self.entry)
        {
            apply_levels(&mut self.state, 0, &cached);
            self.peaks_started = true;
            tx.wake_after(Duration::ZERO);
        }
        if self.family == Some(Family::Video)
            && let Some((board, complete)) = storyboard::cached(&self.entry)
        {
            self.state.set_storyboard(Some(board));
            self.storyboard_done = complete;
        }
    }

    fn load_audio_details(&mut self, later: Later<App>, tx: &mut Tx) {
        if self.family != Some(Family::Audio) {
            return;
        }
        if let Some(cached) = audio_details::cached(&self.entry) {
            self.state.set_tags(cached.tags);
            self.state.set_art(cached.art);
            tx.wake_after(Duration::ZERO);
            return;
        }
        let inlet = audio_details::Inlet {
            later,
            viewer: self.id,
            deliver: audio_detailed,
        };
        self.audio_details = Some(audio_details::load(
            &self.entry,
            self.stream.source(),
            inlet,
        ));
    }

    fn feed_spectrum(&mut self) {
        let Some(feed) = &mut self.spectrum else {
            return;
        };
        if !self.stream.played_samples(feed.window_mut()) {
            return;
        }
        let bands = self
            .state
            .spectrum()
            .count()
            .clamp(spectrum_bars::MIN_BARS, spectrum_bars::MAX_BARS);
        let levels = feed.analyze(bands);
        self.state.feed_bands(levels);
    }

    fn show(&mut self, frame: Option<stream::Shown>) {
        if let Some(frame) = frame {
            let [width, height] = frame.size;
            self.pixels.set(width, height, frame.pixels);
            self.state.set_frame(Some(frame.size));
        }
    }

    fn sync_error(&mut self) {
        if self.state.error().is_none()
            && let Some(message) = self.stream.error()
        {
            self.state.set_error(Some(error_feedback(message)));
        }
    }

    fn start_storyboard(&mut self, later: Later<App>, tx: &mut Tx) {
        if self.storyboard_started || self.family != Some(Family::Video) {
            return;
        }
        self.storyboard_started = true;
        let long_enough = self
            .stream
            .duration_us()
            .is_some_and(|duration| duration >= storyboard::MIN_DURATION_US);
        if self.storyboard_done || !long_enough {
            self.storyboard_done = true;
            self.maybe_start_peaks(later, tx);
            return;
        }
        let inlet = storyboard::Inlet {
            later,
            viewer: self.id,
            deliver: storyboard_updated,
        };
        self.storyboard = Some(storyboard::load(
            &self.entry,
            self.stream.source().clone(),
            inlet,
        ));
    }

    fn maybe_start_peaks(&mut self, later: Later<App>, tx: &mut Tx) {
        if self.peaks_started || !self.stream.can_decode_peaks() {
            return;
        }
        self.peaks_started = true;
        if let Some(cached) = peaks::cached(&self.entry) {
            apply_levels(&mut self.state, 0, &cached);
            tx.wake_after(Duration::ZERO);
            return;
        }
        let inlet = peaks::Inlet {
            later,
            viewer: self.id,
            deliver: peaks_received,
        };
        self.peaks = Some(peaks::load(
            &self.entry,
            self.stream.source().clone(),
            inlet,
        ));
    }

    pub(super) fn stop(&mut self) {
        if self.stream.playing() {
            self.stream.pause();
        }
        self.details = None;
        self.audio_details = None;
        self.peaks = None;
        self.storyboard = None;
    }

    #[cfg(test)]
    pub(super) fn audio_levels(&self) -> Option<(f32, bool)> {
        self.stream.audio_levels()
    }

    #[cfg(test)]
    pub(super) fn viewer_id(&self) -> u64 {
        self.id
    }
}

/// Playback's frame, from the preview's poll. True when the view changed.
pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let Viewer::Media(media) = &mut app.preview.viewer else {
        return false;
    };
    let frame = media.stream.advance(clock, tx);
    media.show(frame);
    media.sync_error();
    media.feed_spectrum();
    let changed = media.state.tick(&media.stream, clock, tx);
    let intents = media.state.drain_intents();
    intents
        .into_iter()
        .fold(changed, |changed, intent| apply(app, intent, tx) | changed)
}

/// A media key's command on the shown player (GTK's `media_command`). False when it does
/// nothing here, so the key goes on.
pub(super) fn command(app: &mut App, command: MediaCommand, tx: &mut Tx) -> bool {
    let Viewer::Media(media) = &mut app.preview.viewer else {
        return false;
    };
    if !media.state.command(command) {
        return false;
    }
    for intent in media.state.drain_intents() {
        apply(app, intent, tx);
    }
    tx.wake_after(Duration::ZERO);
    true
}

/// What the player asked: to the clock, to Settings, to the listing, or to the desktop.
fn apply(app: &mut App, intent: MediaIntent, tx: &mut Tx) -> bool {
    match intent {
        MediaIntent::Step(by) => return step(app, by, tx),
        MediaIntent::SetVolume { volume, muted } => {
            shell::saved(app).change(|settings| {
                settings.preview_volume = f64::from(volume);
                settings.preview_muted = muted;
            });
        }
        MediaIntent::HandOff { .. } => super::open_current(app),
        _ => {}
    }
    let later = app.preview.loader.later.clone();
    let Viewer::Media(media) = &mut app.preview.viewer else {
        return false;
    };
    match intent {
        MediaIntent::Play => media.stream.play(),
        MediaIntent::Pause => media.stream.pause(),
        MediaIntent::Seek(to) => media.stream.seek(to),
        MediaIntent::SetVolume { volume, muted } => {
            media.state.set_volume(volume, muted);
            media.stream.set_audio(volume, muted);
        }
        MediaIntent::DecodeSize(size) => media.stream.resize(size),
        MediaIntent::StoryboardWanted => media.start_storyboard(later, tx),
        MediaIntent::Step(_) | MediaIntent::HandOff { .. } => {}
    }
    tx.wake_after(Duration::ZERO);
    true
}

/// Steps to the previous or next file of the clip's kind, which keeps playing if this one
/// was (GTK's `step_media`).
fn step(app: &mut App, by: i8, tx: &mut Tx) -> bool {
    let Viewer::Media(media) = &app.preview.viewer else {
        return false;
    };
    let Some(family) = media.family else {
        return false;
    };
    let (current, playing) = (media.entry.location.clone(), media.stream.playing());
    let keys = tx.focused().is_some_and(|key| key.starts_with(MEDIA_KEY));
    let Some(to) = sequence::step(app, (family, &current), by, tx) else {
        return false;
    };
    app.preview.stepped = Some(Stepped { to, playing, keys });
    true
}

/// GTK's `media_error_feedback`: a missing decoder asks for the plugins, anything else says
/// the preview can't play.
fn error_feedback(message: &str) -> MediaError {
    let normalized = message.to_ascii_lowercase();
    let plugin = [
        "gstreamer",
        "plug-in",
        "plugin",
        "missing decoder",
        "no decoder",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if plugin {
        return MediaError {
            title: tr("Additional media support required"),
            detail: tr(
                "On Arch or Omarchy, install the required GStreamer plugins, then restart Strata.",
            ),
        };
    }
    MediaError {
        title: tr("Preview unavailable"),
        detail: rust_i18n::t!(
            "Unable to play this media preview: %{message}",
            message = tr(message)
        )
        .into_owned(),
    }
}

/// A catalog message whose placeholders C13 fills: `%{rate}` becomes `{rate}`.
fn template(message: &str) -> String {
    tr(message).replace("%{", "{")
}

fn media_labels() -> Labels {
    let step = |message, keyless, binding: strata_desktop::keys::Command| {
        with_keys(message, Some(keyless), Shown::One(binding.shown()))
    };
    let mut labels = Labels::default();
    labels.timeline = tr("Playback position");
    labels.play = tr("Play or pause");
    labels.play_tooltip = with_keys(
        "Play/Pause (%{keys})",
        None,
        Shown::One(MEDIA_PLAY_PAUSE.shown()),
    );
    labels.previous_video = tr("Previous video");
    labels.next_video = tr("Next video");
    labels.previous_audio = tr("Previous audio file");
    labels.next_audio = tr("Next audio file");
    labels.previous_video_tooltip =
        step("Previous video (%{keys})", "Previous video", MEDIA_PREVIOUS);
    labels.next_video_tooltip = step("Next video (%{keys})", "Next video", MEDIA_NEXT);
    labels.previous_audio_tooltip = step(
        "Previous audio file (%{keys})",
        "Previous audio file",
        MEDIA_PREVIOUS,
    );
    labels.next_audio_tooltip = step("Next audio file (%{keys})", "Next audio file", MEDIA_NEXT);
    labels.mute = with_keys(
        "Mute/unmute (%{keys})",
        None,
        Shown::One(MEDIA_MUTE.shown()),
    );
    labels.volume = tr("Volume");
    labels.video_frame = tr("Video frame");
    labels.spectrum = tr("Audio spectrum");
    labels.artwork = tr("Album art");
    labels.value = template("%{elapsed} of %{duration}");
    labels.fps = template("%{rate} fps");
    labels.mono = tr("Mono");
    labels.stereo = tr("Stereo");
    labels.channels = template("%{channels} ch");
    labels.track_of = template("Track %{track} of %{total}");
    labels.track = template("Track %{track}");
    labels.position = labels::position_caption;
    labels
}

pub(super) fn build_view(media: &MediaViewer, app: &App, cx: &mut Cx<App>) -> El {
    MediaPlayer::new(&media.state, media_mut, viewport_mut)
        .key(MEDIA_KEY)
        .video(&media.pixels)
        .on_intent(|app: &mut App, intent, tx| {
            apply(app, intent, tx);
        })
        .build(&app.look, cx)
}

/// The player's state, or, for a handler Rune queued before the viewer changed, the
/// preview's detached one (`viewer::Detached`).
fn media_mut(app: &mut App) -> &mut MediaState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Media(media) => &mut media.state,
        _ => viewer::spare("media", &mut preview.detached.media),
    }
}

fn viewport_mut(app: &mut App) -> &mut ViewportState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Media(media) => &mut media.state.viewport,
        _ => viewer::spare("media", &mut preview.detached.viewport),
    }
}
