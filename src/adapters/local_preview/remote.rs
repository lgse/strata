// SPDX-License-Identifier: MIT

use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use gtk::{gio, glib, prelude::*};
use tempfile::NamedTempFile;

use crate::model::{FileEntry, MetadataValue};

const MAX_REMOTE_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STAGED_PREVIEWS: usize = 4;
const REMOTE_PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_BYTES: usize = 64 * 1024;
static STAGED_PREVIEWS: AtomicUsize = AtomicUsize::new(0);

pub(super) struct StagedPreview {
    file: NamedTempFile,
    _permit: StagingPermit,
}

impl StagedPreview {
    pub(super) fn path(&self) -> &Path {
        self.file.path()
    }
}

struct StagingPermit;

impl StagingPermit {
    fn acquire() -> Result<Self, String> {
        STAGED_PREVIEWS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_STAGED_PREVIEWS).then_some(active + 1)
            })
            .map(|_| Self)
            .map_err(|_| "Too many remote previews are active; try again shortly".into())
    }
}

impl Drop for StagingPermit {
    fn drop(&mut self) {
        STAGED_PREVIEWS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct CancelOnDrop(gio::Cancellable);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) async fn stage(entry: &FileEntry) -> Result<StagedPreview, String> {
    stage_with_limit(entry, MAX_REMOTE_PREVIEW_BYTES, REMOTE_PREVIEW_TIMEOUT).await
}

pub(super) async fn stage_video(entry: &FileEntry) -> Result<StagedPreview, String> {
    stage_with_limit(entry, 256 * 1024 * 1024, Duration::from_secs(60)).await
}

async fn stage_with_limit(
    entry: &FileEntry,
    byte_limit: u64,
    timeout: Duration,
) -> Result<StagedPreview, String> {
    if matches!(entry.size, MetadataValue::Known(size) if size > byte_limit) {
        return Err(format!(
            "Remote preview exceeds the {} MiB download limit",
            byte_limit / (1024 * 1024)
        ));
    }
    let file = crate::adapters::gio_file_for_location(&entry.location);
    let suffix = Path::new(&entry.native_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            (1..=8).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    transfer(suffix, timeout, move |staged, cancellation| {
        let stream = file
            .read(Some(cancellation))
            .map_err(|error| error.to_string())?;
        copy_bounded(&stream, staged, byte_limit, cancellation)
    })
    .await
}

async fn transfer(
    suffix: String,
    timeout: Duration,
    copy: impl FnOnce(&mut NamedTempFile, &gio::Cancellable) -> Result<(), String> + Send + 'static,
) -> Result<StagedPreview, String> {
    let permit = StagingPermit::acquire()?;
    let guard = CancelOnDrop(gio::Cancellable::new());
    let cancellation = guard.0.clone();
    let worker = gio::spawn_blocking(move || {
        let mut staged = StagedPreview {
            file: tempfile::Builder::new()
                .prefix("strata-remote-preview-")
                .suffix(&suffix)
                .tempfile()
                .map_err(|error| error.to_string())?,
            _permit: permit,
        };
        if cancellation.is_cancelled() {
            return Err("Remote preview cancelled".into());
        }
        copy(&mut staged.file, &cancellation)?;
        if cancellation.is_cancelled() {
            return Err("Remote preview cancelled".into());
        }
        Ok(staged)
    });
    // Aborted loads cancel GIO, but the worker retains its slot and partial file
    // until the backend acknowledges cancellation. Decoding retains the slot too.
    futures_lite::future::race(
        async {
            worker
                .await
                .map_err(|_| "Remote preview transfer failed".to_owned())?
        },
        async {
            glib::timeout_future(timeout).await;
            Err("Remote preview download timed out".to_owned())
        },
    )
    .await
}

fn copy_bounded(
    stream: &impl IsA<gio::InputStream>,
    output: &mut impl Write,
    byte_limit: u64,
    cancellation: &gio::Cancellable,
) -> Result<(), String> {
    let mut total = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err("Remote preview cancelled".into());
        }
        let remaining = byte_limit - total;
        let count = CHUNK_BYTES.min(remaining.saturating_add(1) as usize);
        let bytes = stream
            .read_bytes(count, Some(cancellation))
            .map_err(|error| error.to_string())?;
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() as u64 > remaining {
            return Err(format!(
                "Remote preview exceeds the download limit ({} MiB maximum)",
                byte_limit / (1024 * 1024)
            ));
        }
        output
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        total += bytes.len() as u64;
    }
}

#[cfg(test)]
mod tests;
