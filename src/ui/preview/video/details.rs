// SPDX-License-Identifier: MIT

//! The sandboxed probe behind the badges, settled and cached like audio tags.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use gtk::{gio, glib};

use crate::{
    model::FileEntry,
    sandbox::{Cancellation, ParseOperation, metadata::MediaMetadata},
    services::SandboxedMedia,
    ui::preview::audio::details::{LOAD_SETTLE, Lru, TrackKey, parse},
};

const DETAILS_CACHE: usize = 12;

thread_local! {
    static DETAILS: RefCell<Lru<TrackKey, Rc<MediaMetadata>>> =
        const { RefCell::new(Lru::new(DETAILS_CACHE)) };
}

pub(super) fn cached_details(key: &TrackKey) -> Option<Rc<MediaMetadata>> {
    DETAILS.with_borrow_mut(|cache| cache.get(key))
}

/// Cancels its sandbox job when dropped.
pub(super) struct DetailsLoad(Cancellation);

impl Drop for DetailsLoad {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// `on_details` receives `None` when the probe fails, so the skeleton clears.
pub(super) fn load_details(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_details: impl Fn(Option<Rc<MediaMetadata>>) + 'static,
) -> DetailsLoad {
    load_details_with(entry, source, on_details, parse)
}

fn load_details_with(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_details: impl Fn(Option<Rc<MediaMetadata>>) + 'static,
    parse: impl Fn(&Path, ParseOperation, &Cancellation) -> Option<Vec<u8>> + Send + 'static,
) -> DetailsLoad {
    let key = TrackKey::of(entry);
    let cancellation = Cancellation::default();
    let path = source.path.clone();
    let lease = source.clone();
    let job = cancellation.clone();
    let cancelled = cancellation.clone();
    let published = Cell::new(false);
    glib::MainContext::default().spawn_local(async move {
        glib::timeout_future(LOAD_SETTLE).await;
        if cancelled.is_cancelled() {
            return;
        }
        let details = gio::spawn_blocking(move || {
            let _lease = lease;
            parse(&path, ParseOperation::MediaMetadata, &job)
                .and_then(|json| MediaMetadata::from_json(&json, false).ok())
        })
        .await
        .ok()
        .flatten()
        .map(Rc::new);
        // A reused view may already show another clip.
        if cancelled.is_cancelled() || published.replace(true) {
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
