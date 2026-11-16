// SPDX-License-Identifier: MIT

//! Media and RAW metadata loading and formatting for Single-File Properties
//! (`src/ui/browser/properties/media.rs` and `src/ui/browser/properties.rs` lines 642–663).

use std::path::PathBuf;

use gio::prelude::*;
use rune::Tx;
use strata_core::{
    i18n::{decimal, tr},
    model::FileEntry,
};
use strata_fs::{
    adapters::{CONTENT_TYPE_ATTRIBUTES, content_type_of},
    services::refine_reported_content_type,
};
use strata_preview::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, metadata::MediaMetadata,
};

use super::Dialog;
use crate::{
    App,
    raw_details::{self, RawLoad, RawRows},
    slots::Later,
};

pub(super) enum Section {
    None,
    Media(Vec<(&'static str, String)>),
    Raw(Box<RawRows>),
}

#[derive(Default)]
pub(super) enum ActiveLoad {
    #[default]
    None,
    Raw(RawLoad),
    Media {
        cancel: Cancellation,
        task: glib::JoinHandle<()>,
    },
}

impl ActiveLoad {
    pub(super) fn abort(&mut self) {
        match std::mem::take(self) {
            Self::None => {}
            Self::Raw(mut load) => load.cancel(),
            Self::Media { cancel, task } => {
                cancel.cancel();
                task.abort();
            }
        }
    }
}

pub(super) fn start(
    entry: Option<&FileEntry>,
    is_directory: bool,
    generation: u64,
    later: Later<App>,
) -> (Section, ActiveLoad) {
    let Some(entry) = entry.filter(|_| !is_directory) else {
        return (Section::None, ActiveLoad::None);
    };
    if raw_details::supports(entry) {
        let path = entry.local_thumbnail_path().map(ToOwned::to_owned);
        let load = raw_details::spawn(path, later, move |app, metadata| {
            apply_section(
                app,
                generation,
                Section::Raw(Box::new(raw_details::rows(&metadata))),
            )
        });
        return (
            Section::Raw(Box::new(raw_details::default_rows())),
            ActiveLoad::Raw(load),
        );
    }
    let Some(path) = entry.local_thumbnail_path().map(ToOwned::to_owned) else {
        return (Section::None, ActiveLoad::None);
    };
    (Section::None, spawn_media(later, generation, path))
}

fn spawn_media(later: Later<App>, generation: u64, path: PathBuf) -> ActiveLoad {
    let cancel = Cancellation::default();
    let task_cancel = cancel.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let Some(image) = probe_media_kind(&path, &task_cancel).await else {
            return;
        };
        let loading_later = later.clone();
        loading_later.then(move |app: &mut App, _: &mut Tx| {
            apply_section(
                app,
                generation,
                Section::Media(vec![("MEDIA", tr("Loading…"))]),
            )
        });
        let worker_cancel = task_cancel.clone();
        let result = gio::spawn_blocking(move || {
            sandbox::parse(
                &path,
                ParseOperation::MediaMetadata,
                0,
                MediaPreviewBackend::Software,
                &worker_cancel,
            )
            .and_then(|output| MediaMetadata::from_json(&output.data, image))
        })
        .await;
        if task_cancel.is_cancelled() {
            return;
        }
        let parsed = result
            .ok()
            .and_then(Result::ok)
            .map(|metadata| media_rows(&metadata))
            .unwrap_or_default();
        let final_rows = if parsed.is_empty() {
            vec![("MEDIA", tr("Unavailable"))]
        } else {
            parsed
        };
        later.then(move |app: &mut App, _: &mut Tx| {
            apply_section(app, generation, Section::Media(final_rows))
        });
    });
    ActiveLoad::Media { cancel, task }
}

async fn probe_media_kind(path: &PathBuf, cancel: &Cancellation) -> Option<bool> {
    let file = gio::File::for_path(path);
    let attrs = format!("{CONTENT_TYPE_ATTRIBUTES},standard::type");
    let info = file
        .query_info_future(
            &attrs,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    if cancel.is_cancelled() || info.file_type() != gio::FileType::Regular {
        return None;
    }
    let reported = content_type_of(&info).and_then(|ct| gio::content_type_get_mime_type(&ct))?;
    let mime = refine_reported_content_type(&reported, path);
    let image = mime.starts_with("image/");
    (image || mime.starts_with("audio/") || mime.starts_with("video/")).then_some(image)
}

fn apply_section(app: &mut App, generation: u64, section: Section) -> bool {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_mut() else {
        return false;
    };
    if single.generation != generation {
        return false;
    }
    single.media = section;
    true
}

pub(super) fn media_rows(metadata: &MediaMetadata) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    append_stream_rows(&mut rows, metadata);
    append_audio_rows(&mut rows, metadata);
    append_track_rows(&mut rows, metadata);
    rows
}

fn append_stream_rows(rows: &mut Vec<(&'static str, String)>, metadata: &MediaMetadata) {
    if let Some((width, height)) = metadata.dimensions {
        rows.push((
            "RESOLUTION",
            rust_i18n::t!(
                "%{width} × %{height} pixels",
                width = width,
                height = height
            )
            .into_owned(),
        ));
    }
    if let Some(duration) = metadata.duration {
        let seconds = duration.round() as u64;
        rows.push((
            "DURATION",
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            ),
        ));
    }
    if let Some(bitrate) = metadata.bitrate {
        rows.push(("BITRATE", bitrate_text(bitrate)));
    }
    if let Some(codec) = &metadata.video_codec {
        rows.push(("VIDEO CODEC", codec.clone()));
    }
    if let Some(hdr) = metadata.hdr_format() {
        rows.push(("HDR", hdr.into()));
    }
    if let Some(rate) = metadata.frame_rate {
        rows.push((
            "FRAME RATE",
            rust_i18n::t!("%{rate} fps", rate = decimal(rate, 2)).into_owned(),
        ));
    }
}

fn bitrate_text(bitrate: f64) -> String {
    if bitrate >= 1_000_000.0 {
        rust_i18n::t!("%{rate} Mb/s", rate = decimal(bitrate / 1_000_000.0, 2)).into_owned()
    } else {
        rust_i18n::t!("%{rate} kb/s", rate = decimal(bitrate / 1000.0, 0)).into_owned()
    }
}

fn append_audio_rows(rows: &mut Vec<(&'static str, String)>, metadata: &MediaMetadata) {
    if let Some(codec) = &metadata.audio_codec {
        rows.push(("AUDIO CODEC", codec.clone()));
    }
    if let Some(rate) = metadata.sample_rate {
        rows.push((
            "SAMPLE RATE",
            rust_i18n::t!("%{rate} kHz", rate = decimal(rate / 1000.0, 1)).into_owned(),
        ));
    }
    if let Some(channels) = metadata.channels {
        rows.push((
            "CHANNELS",
            match channels {
                1 => tr("1 (Mono)"),
                2 => tr("2 (Stereo)"),
                _ => channels.to_string(),
            },
        ));
    }
}

fn append_track_rows(rows: &mut Vec<(&'static str, String)>, metadata: &MediaMetadata) {
    if !metadata.subtitle_tracks.is_empty() {
        let languages: Vec<&str> = metadata
            .subtitle_tracks
            .iter()
            .filter_map(|track| track.language.as_deref())
            .collect();
        let text = if languages.is_empty() {
            metadata.subtitle_tracks.len().to_string()
        } else {
            format!(
                "{} ({})",
                metadata.subtitle_tracks.len(),
                languages.join(", ")
            )
        };
        rows.push(("SUBTITLES", text));
    }
    if !metadata.chapters.is_empty() {
        rows.push(("CHAPTERS", metadata.chapters.len().to_string()));
    }
}
