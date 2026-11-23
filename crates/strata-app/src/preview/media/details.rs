// SPDX-License-Identifier: MIT

use std::{path::Path, time::Duration};

use rune::Tx;
use strata_components::media_player::{Details, badges::VideoDetails};
use strata_core::{file_activity, model::FileEntry};
use strata_fs::services::SandboxedMedia;
use strata_preview::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, metadata::MediaMetadata,
};

use crate::{App, slots::Later};

const LOAD_SETTLE: Duration = Duration::from_millis(50);
const SIDECAR_EXTENSIONS: [&str; 5] = ["srt", "vtt", "ass", "ssa", "sub"];
const SIDECAR_SCAN_LIMIT: usize = 5_000;

/// The probe of one shown video; dropping it cancels the probe.
pub(super) struct DetailsLoad(Cancellation);

impl Drop for DetailsLoad {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Probes `source` for the viewer `viewer`; `deliver` hands the answer over.
pub(super) fn load(
    entry: &FileEntry,
    source: &SandboxedMedia,
    (later, viewer): (Later<App>, u64),
    deliver: fn(&mut App, u64, Details) -> bool,
) -> DetailsLoad {
    let cancellation = Cancellation::default();
    let (job, cancelled) = (cancellation.clone(), cancellation.clone());
    let path = source.path.clone();
    // Sidecars sit next to the original, not next to a staged remote copy.
    let original = entry.location.native_path().map(Path::to_path_buf);
    let lease = source.clone();
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let probed = gio::spawn_blocking(move || {
            let _lease = lease;
            probe(&path, original.as_deref(), &job)
        })
        .await
        .ok()
        .flatten();
        if cancelled.is_cancelled() {
            return;
        }
        let details = probed.map_or(Details::Failed, Details::Ready);
        later.then(move |app: &mut App, _: &mut Tx| deliver(app, viewer, details));
    });
    DetailsLoad(cancellation)
}

fn probe(path: &Path, original: Option<&Path>, job: &Cancellation) -> Option<VideoDetails> {
    let output = sandbox::parse(
        path,
        ParseOperation::MediaMetadata,
        0,
        MediaPreviewBackend::Software,
        job,
    )
    .ok()?;
    let metadata = MediaMetadata::from_json(&output.data, false).ok()?;
    let sidecar_captions = original.map_or(Some(0), sidecar_captions)?;
    Some(video_details(metadata, sidecar_captions))
}

fn video_details(metadata: MediaMetadata, sidecar_captions: usize) -> VideoDetails {
    VideoDetails {
        dimensions: metadata.dimensions,
        color_transfer: metadata.color_transfer,
        pixel_format: metadata.pixel_format,
        frame_rate: metadata.frame_rate,
        video_codec: metadata.video_codec,
        audio_codec: metadata.audio_codec,
        channels: metadata.channels,
        channel_layout: metadata.channel_layout,
        subtitle_tracks: metadata.subtitle_tracks.len(),
        sidecar_captions,
    }
}

/// Caption files named after `video` beside it; `None` when the folder can't be read now.
fn sidecar_captions(video: &Path) -> Option<usize> {
    let (Some(directory), Some(stem)) = (
        video.parent(),
        video.file_stem().and_then(|stem| stem.to_str()),
    ) else {
        return Some(0);
    };
    let mut entries = match file_activity::list(directory) {
        Ok(entries) => entries,
        Err(error) if file_activity::is_refusal(&error) => return None,
        Err(_) => return Some(0),
    };
    let prefix = format!("{}.", stem.to_ascii_lowercase());
    let count = entries
        .by_ref()
        .take(SIDECAR_SCAN_LIMIT)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with(&prefix)
                && SIDECAR_EXTENSIONS
                    .iter()
                    .any(|extension| name.ends_with(&format!(".{extension}")))
        })
        .count();
    (!entries.stopped()).then_some(count)
}
