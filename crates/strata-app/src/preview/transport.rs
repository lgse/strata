// SPDX-License-Identifier: MIT

//! The preview's media keys at the dispatcher's `video_controls` stage: GTK's
//! `handle_video_key` (`src/ui/preview/drawer.rs`), which hands the shown player or route its
//! command, one action per catalog command. They reach where
//! [`super::keys::media_keys_reach`] holds and a player or a route shows: a route takes play,
//! pause and the seeks, a player every one.

use std::time::Duration;

use rune::Tx;
use strata_components::{
    media_player::{MediaCommand, Seek as MediaSeek},
    viewers::route::{RouteCommand, Seek as RouteSeek},
};
use strata_desktop::keys::{
    Modifiers,
    bindings::{
        MEDIA_MUTE, MEDIA_NEXT, MEDIA_PLAY_PAUSE, MEDIA_PREVIOUS, MEDIA_SEEK_BACK,
        MEDIA_SEEK_FORWARD, MEDIA_VOLUME_DOWN, MEDIA_VOLUME_UP,
    },
};

use super::{keys, media, route, viewer::Viewer};
use crate::{
    App,
    commands::{Action, Call, Keys, Outcome, Situation, Stage},
};

/// A media key's seek: GTK's `MEDIA_SEEK_STEP_US`.
const SEEK_STEP: Duration = Duration::from_secs(5);

/// What a key asks of a route, if a route takes it, and of a player.
struct Asked {
    route: Option<RouteCommand>,
    media: MediaCommand,
}

/// One media key's action: its catalog command, exactly as bound, at `video_controls`.
macro_rules! media_key {
    ($name:literal, $command:expr, $run:expr $(,)?) => {
        Action::new($name, $run).command($command).keys(&[Keys::at(
            Stage::VideoControls,
            &[$command.ignoring(Modifiers::empty())],
        )
        .when(plays_here)])
    };
}

pub(super) const PLAY_PAUSE: Action<App> =
    media_key!("preview-media-play-pause", MEDIA_PLAY_PAUSE, play_pause);
pub(super) const SEEK_BACK: Action<App> =
    media_key!("preview-media-seek-back", MEDIA_SEEK_BACK, seek_back);
pub(super) const SEEK_FORWARD: Action<App> = media_key!(
    "preview-media-seek-forward",
    MEDIA_SEEK_FORWARD,
    seek_forward,
);
pub(super) const VOLUME_UP: Action<App> =
    media_key!("preview-media-volume-up", MEDIA_VOLUME_UP, volume_up);
pub(super) const VOLUME_DOWN: Action<App> =
    media_key!("preview-media-volume-down", MEDIA_VOLUME_DOWN, volume_down);
pub(super) const MUTE: Action<App> = media_key!("preview-media-mute", MEDIA_MUTE, mute);
pub(super) const PREVIOUS: Action<App> =
    media_key!("preview-media-previous", MEDIA_PREVIOUS, previous);
pub(super) const NEXT: Action<App> = media_key!("preview-media-next", MEDIA_NEXT, next);

/// A player or a route shows, where the preview's media keys reach (GTK's `has_video`).
fn plays_here(app: &App, at: &Situation) -> bool {
    matches!(app.preview.viewer, Viewer::Route(_) | Viewer::Media(_))
        && keys::media_keys_reach(app, at)
}

fn play_pause(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(
        app,
        Asked {
            route: Some(RouteCommand::PlayPause),
            media: MediaCommand::PlayPause,
        },
        tx,
    )
}

fn seek_back(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, seek(-1), tx)
}

fn seek_forward(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, seek(1), tx)
}

/// A seek of one step back (-1) or forward (1), for a route and for a player.
fn seek(way: i8) -> Asked {
    let route = RouteSeek::By(f64::from(way) * SEEK_STEP.as_secs_f64());
    let media = if way < 0 {
        MediaSeek::Back(SEEK_STEP)
    } else {
        MediaSeek::Forward(SEEK_STEP)
    };
    Asked {
        route: Some(RouteCommand::Seek(route)),
        media: MediaCommand::Seek(media),
    }
}

fn player_only(media: MediaCommand) -> Asked {
    Asked { route: None, media }
}

fn volume_up(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, player_only(MediaCommand::VolumeUp), tx)
}

fn volume_down(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, player_only(MediaCommand::VolumeDown), tx)
}

fn mute(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, player_only(MediaCommand::Mute), tx)
}

fn previous(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, player_only(MediaCommand::Previous), tx)
}

fn next(app: &mut App, _: &Call<'_>, tx: &mut Tx) -> Outcome {
    run(app, player_only(MediaCommand::Next), tx)
}

fn run(app: &mut App, asked: Asked, tx: &mut Tx) -> Outcome {
    let taken = match &app.preview.viewer {
        Viewer::Route(_) => asked
            .route
            .is_some_and(|command| route::command(app, command)),
        Viewer::Media(_) => media::command(app, asked.media, tx),
        _ => false,
    };
    if taken {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}
