// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{gio, glib};

use crate::{
    model::FileEntry,
    sandbox::{Cancellation, ParseOperation, metadata::MediaMetadata},
    services::SandboxedMedia,
    ui::preview::audio::details::{LOAD_SETTLE, Lru, TrackKey, parse},
};

const DETAILS_CACHE: usize = 12;
const SIDECAR_EXTENSIONS: [&str; 5] = ["srt", "vtt", "ass", "ssa", "sub"];
const SIDECAR_SCAN_LIMIT: usize = 5_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::ui::preview) struct VideoDetails {
    pub(in crate::ui::preview) metadata: MediaMetadata,
    pub(in crate::ui::preview) sidecar_captions: usize,
}

thread_local! {
    static DETAILS: RefCell<Lru<TrackKey, Rc<VideoDetails>>> =
        const { RefCell::new(Lru::new(DETAILS_CACHE)) };
}

pub(super) fn cached_details(key: &TrackKey) -> Option<Rc<VideoDetails>> {
    DETAILS.with_borrow_mut(|cache| cache.get(key))
}

pub(super) fn sidecar_captions(video: &Path) -> usize {
    let (Some(directory), Some(stem)) = (
        video.parent(),
        video.file_stem().and_then(|stem| stem.to_str()),
    ) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let prefix = format!("{}.", stem.to_ascii_lowercase());
    entries
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
        .count()
}

pub(super) struct DetailsLoad(Cancellation);

impl Drop for DetailsLoad {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) fn load_details(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_details: impl Fn(Option<Rc<VideoDetails>>) + 'static,
) -> DetailsLoad {
    load_details_with(entry, source, on_details, parse)
}

fn load_details_with(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_details: impl Fn(Option<Rc<VideoDetails>>) + 'static,
    parse: impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Send + 'static,
) -> DetailsLoad {
    let key = TrackKey::of(entry);
    let cancellation = Cancellation::default();
    let path = source.path.clone();
    // Sidecars sit next to the original, not next to a staged remote copy.
    let original = entry.location.native_path().map(Path::to_path_buf);
    let lease = source.clone();
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let details = gio::spawn_blocking(move || {
            let _lease = lease;
            let metadata = parse(&path, ParseOperation::MediaMetadata, &job)
                .and_then(|json| MediaMetadata::from_json(&json, false).ok())?;
            Some(VideoDetails {
                metadata,
                sidecar_captions: original.as_deref().map_or(0, sidecar_captions),
            })
        })
        .await
        .ok()
        .flatten()
        .map(Rc::new);
        if cancelled.is_cancelled() {
            return;
        }
        if let Some(details) = &details {
            DETAILS.with_borrow_mut(|cache| cache.insert(key, details.clone()));
        }
        on_details(details);
    });
    DetailsLoad(cancellation)
}

#[cfg(test)]
mod tests;
