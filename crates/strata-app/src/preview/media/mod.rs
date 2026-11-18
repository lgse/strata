// SPDX-License-Identifier: MIT

//! C13 [`MediaPlayer`] host for sandboxed audio and video (`PreviewContent::SandboxedMedia`),
//! as GTK's `render_sandboxed_media` (`src/ui/preview/media_render.rs`) and its video and
//! audio views host them:
//!
//! - **The view** is by the file's type: video, audio, or the plain media bar for media that
//!   is neither, as an animated GIF, which loops.
//! - **Playback** is silent ([`stream`]): Rune has no audio output yet (packet entry 26,
//!   decision 15). Frames come from the shared sandboxed decoder off the UI thread and reach
//!   Rune through `Later`. Autoplay follows Settings, a video opens at the saved start
//!   percentage, and one left partway this session reopens where it was.
//! - **The eyebrow** counts the folder's files of the clip's kind whose bytes back their name
//!   ([`sequence`]), and the previous and next buttons and keys step through them.
//! - **The badges** come from the sandboxed metadata probe ([`details`]).
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
        MediaPlayer, MediaState,
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
use crate::{App, location::labels::with_keys, shell};

mod details;
mod positions;
pub(super) mod sequence;
mod stream;

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
    _details: Option<details::DetailsLoad>,
}

/// A sandboxed media preview arrived: the player for its kind, decoding from the start.
pub(super) fn ready(
    app: &mut App,
    entry: FileEntry,
    content_type: &str,
    mut media: SandboxedMedia,
    tx: &mut Tx,
) -> Viewer {
    let id = NEXT_VIEWER.with(|next| next.replace(next.get().wrapping_add(1)));
    let gif = content_type == "image/gif";
    let family = sequence::family_of_type(content_type);
    media.audio_only = family == Some(Family::Audio);
    let prefs = shell::preferences(app);
    let start_percent = if gif || is_raw_video_stream(content_type) {
        0
    } else {
        prefs.preview_video_start
    };
    let (autoplay, volume, muted, tenxer) = (
        prefs.preview_autoplay,
        prefs.preview_volume,
        prefs.preview_muted,
        prefs.tenxer_mode,
    );
    let later = app.preview.loader.later.clone();
    let inlet = Inlet {
        later: later.clone(),
        viewer: id,
        deliver: received,
    };
    let details = (family == Some(Family::Video))
        .then(|| details::load(&entry, &media, (later, id), detailed));
    let mut stream = Stream::new(media, inlet, start_percent, tx.clock());
    stream.set_loop(gif);
    let continues = app
        .preview
        .continue_playback
        .take()
        .is_some_and(|location| location == entry.location);
    if autoplay || (continues && family.is_some()) {
        stream.play();
    }
    let mut state = MediaState::new(Clip {
        kind: match family {
            Some(Family::Video) => MediaKind::Video,
            Some(Family::Audio) => MediaKind::Audio,
            None => MediaKind::Plain,
        },
        name: entry.display_name.clone(),
        ..Clip::default()
    });
    state.set_labels(media_labels());
    state.set_keys(if tenxer {
        MediaKeys::tenxer()
    } else {
        MediaKeys::default_map()
    });
    state.set_volume(volume as f32, muted);
    let mut viewer = MediaViewer {
        state,
        pixels: Pixels::new(),
        stream,
        entry,
        family,
        id,
        _details: details,
    };
    viewer.state.set_details(match family {
        Some(Family::Video) => Details::Loading,
        _ => Details::Failed,
    });
    let (position, previous, next) = place(app, family, &viewer.entry.location);
    viewer.state.set_listing(position, previous, next);
    Viewer::Media(Box::new(viewer))
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
fn received(app: &mut App, viewer: u64, session: u64, events: Received, tx: &mut Tx) -> bool {
    let Some(media) = shown(app, viewer) else {
        return false;
    };
    let frame = media.stream.receive(session, events);
    media.show(frame);
    media.sync_error();
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

fn shown(app: &mut App, viewer: u64) -> Option<&mut MediaViewer> {
    match &mut app.preview.viewer {
        Viewer::Media(media) if media.id == viewer => Some(media),
        _ => None,
    }
}

impl MediaViewer {
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

    pub(super) fn stop(&mut self) {
        if self.stream.playing() {
            self.stream.pause();
        }
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
    let Viewer::Media(media) = &mut app.preview.viewer else {
        return false;
    };
    match intent {
        MediaIntent::Play => media.stream.play(),
        MediaIntent::Pause => media.stream.pause(),
        MediaIntent::Seek(to) => media.stream.seek(to),
        MediaIntent::SetVolume { volume, muted } => media.state.set_volume(volume, muted),
        MediaIntent::DecodeSize(size) => media.stream.resize(size),
        // No storyboard yet: A16's "Open".
        MediaIntent::StoryboardWanted | MediaIntent::Step(_) | MediaIntent::HandOff { .. } => {}
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
    let Some(target) = sequence::step(app, family, &current, by, tx) else {
        return false;
    };
    app.preview.continue_playback = playing.then_some(target);
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
