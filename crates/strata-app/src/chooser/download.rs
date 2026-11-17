// SPDX-License-Identifier: MIT

//! Remote URL download progress, cancellation, and cached download reuse (`src/ui/chooser/state.rs`
//! and `src/ui/chooser/download.rs`).

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};

use rune::Tx;
use strata_core::{
    i18n::{file_size, percent, tr},
    model::{EntryKind, FileEntry, Location, MetadataValue},
};
use strata_fs::services::{RemoteDownload, download_remote, remote_file_name};

use super::image_conversion;
use crate::{App, portal::ChooserKind, shell};

#[derive(Clone, Debug)]
pub(crate) struct DownloadProgress {
    pub(crate) name: String,
    pub(crate) status: Option<String>,
    pub(crate) fraction: Option<f64>,
    pub(crate) value_text: Option<String>,
    pub(crate) cancel_label: String,
}

impl DownloadProgress {
    pub(crate) fn for_url(url: &str) -> Self {
        Self {
            name: remote_file_name(url).unwrap_or_else(|| tr("Download")),
            status: Some(tr("Connecting…")),
            fraction: None,
            value_text: None,
            cancel_label: tr("Cancel download"),
        }
    }

    pub(crate) fn for_image(activity: &str) -> Self {
        Self {
            name: tr("Image"),
            status: Some(tr(activity)),
            fraction: None,
            value_text: None,
            cancel_label: tr("Cancel image processing"),
        }
    }

    /// What a batch of the download thread's reports changed: the file's name, and how far it
    /// has come.
    pub(crate) fn advance(&mut self, name: Option<String>, progress: Option<(u64, Option<u64>)>) {
        if let Some(name) = name {
            self.name = name;
        }
        if let Some((downloaded, total)) = progress {
            self.update(downloaded, total);
        }
    }

    pub(crate) fn update(&mut self, downloaded: u64, total: Option<u64>) {
        match total.filter(|total| *total > 0) {
            Some(total) => {
                #[expect(clippy::cast_precision_loss, reason = "progress ratio in [0.0, 1.0]")]
                let ratio = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped percentage in 0..=100"
                )]
                let pct = (ratio * 100.0) as usize;
                self.fraction = Some(ratio);
                self.value_text = Some(
                    rust_i18n::t!("%{percent} downloaded", percent = percent(pct)).into_owned(),
                );
                self.status = None;
            }
            None => {
                self.fraction = None;
                self.value_text = None;
                self.status = Some(
                    rust_i18n::t!("%{size} downloaded", size = file_size(downloaded)).into_owned(),
                );
            }
        }
    }
}

pub(crate) fn open_remote(app: &mut App, url: &str, tx: &mut Tx) {
    let modal = shell::modal_open(app);
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    if !matches!(
        active.request.kind,
        ChooserKind::Open {
            directory: false,
            ..
        }
    ) {
        active.show_error(tr("Remote links are only supported when opening files"));
        return;
    }
    if active.completion.is_none() || modal {
        return;
    }
    active.accept_generation = active.accept_generation.wrapping_add(1);
    active.cancel_download();
    active.clear_error();
    let cached = active
        .downloaded_file
        .as_ref()
        .filter(|(previous, _)| previous == url)
        .map(|(_, path)| path.clone());
    match cached {
        Some(path) => image_conversion::reuse_download(app, url.to_owned(), path),
        None => start_download(app, url.to_owned(), tx),
    }
}

pub(crate) fn start_download(app: &mut App, url: String, _tx: &mut Tx) {
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    active.downloaded_file = None;
    let cancelled = Arc::new(AtomicBool::new(false));
    active.download_cancel = Some(cancelled.clone());
    active.download_progress = Some(DownloadProgress::for_url(&url));
    active.accept_sensitive = false;
    let receiver = download_remote(url.clone(), cancelled.clone());
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let mut keep_polling = true;
        match drain_batch(&receiver) {
            BatchOutcome::Idle => {}
            outcome => {
                keep_polling = outcome.continues();
                let flag = cancelled.clone();
                let url_copy = url.clone();
                later.then(move |app: &mut App, _: &mut Tx| {
                    apply_batch(app, (&flag, &url_copy), outcome)
                });
            }
        }
        if keep_polling && !cancelled.load(Ordering::SeqCst) {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

enum BatchOutcome {
    Idle,
    Progress {
        name: Option<String>,
        progress: Option<(u64, Option<u64>)>,
    },
    Finished(PathBuf),
    Failed(String),
    Disconnected,
}

impl BatchOutcome {
    fn continues(&self) -> bool {
        matches!(self, Self::Idle | Self::Progress { .. })
    }
}

fn drain_batch(receiver: &Receiver<RemoteDownload>) -> BatchOutcome {
    let mut latest_name = None;
    let mut latest_progress = None;
    loop {
        match receiver.try_recv() {
            Ok(RemoteDownload::Named(name)) => latest_name = Some(name),
            Ok(RemoteDownload::Progress { downloaded, total }) => {
                latest_progress = Some((downloaded, total));
            }
            Ok(RemoteDownload::Finished(path)) => return BatchOutcome::Finished(path),
            Ok(RemoteDownload::Failed(message)) => return BatchOutcome::Failed(message),
            Err(TryRecvError::Empty) => {
                if latest_name.is_none() && latest_progress.is_none() {
                    return BatchOutcome::Idle;
                }
                return BatchOutcome::Progress {
                    name: latest_name,
                    progress: latest_progress,
                };
            }
            Err(TryRecvError::Disconnected) => return BatchOutcome::Disconnected,
        }
    }
}

/// What the download thread reported, applied while the download it belongs to is still the
/// chooser's own: `cancelled` names it.
fn apply_batch(
    app: &mut App,
    (cancelled, url): (&Arc<AtomicBool>, &str),
    outcome: BatchOutcome,
) -> bool {
    let is_current = app
        .chooser
        .active
        .as_ref()
        .is_some_and(|active| active.download_is_current(cancelled));
    if !is_current {
        return false;
    }
    match outcome {
        BatchOutcome::Idle => return false,
        BatchOutcome::Progress { name, progress } => {
            if let Some(bar) = app
                .chooser
                .active
                .as_mut()
                .and_then(|active| active.download_progress.as_mut())
            {
                bar.advance(name, progress);
            }
        }
        BatchOutcome::Finished(path) => {
            if let Some(active) = app.chooser.active.as_mut() {
                active.keep_download(url, &path);
            }
            image_conversion::complete_download(app, path);
        }
        BatchOutcome::Failed(message) => {
            if let Some(active) = app.chooser.active.as_mut() {
                active.fail_download(message);
            }
        }
        BatchOutcome::Disconnected => {
            if let Some(active) = app.chooser.active.as_mut() {
                active.fail_download(tr("The download stopped unexpectedly. Try again."));
            }
        }
    }
    true
}

pub(crate) fn after_download_step<T: Send + 'static>(
    app: &mut App,
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut App, T, &mut Tx) + 'static,
) {
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    active.cancel_download();
    let step = Arc::new(AtomicBool::new(false));
    active.download_cancel = Some(step.clone());
    active.accept_sensitive = false;
    glib::MainContext::default().spawn_local(async move {
        let answer = gio::spawn_blocking(work)
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        later.then(move |app: &mut App, tx: &mut Tx| {
            let current = app
                .chooser
                .active
                .as_ref()
                .is_some_and(|active| active.download_is_current(&step));
            if !current {
                return false;
            }
            if let Some(active) = app.chooser.active.as_mut() {
                active.download_cancel = None;
                active.accept_sensitive = true;
            }
            then(app, answer, tx);
            true
        });
    });
}

pub(crate) fn finish_remote(app: &mut App, path: PathBuf, tx: &mut Tx) {
    let name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let entry = FileEntry {
        location: Location::local(&path),
        thumbnail_path: None,
        native_name: name.clone(),
        display_name: name.to_string_lossy().into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    };
    if !app.browser.allows_entry(&entry) {
        if let Some(active) = app.chooser.active.as_mut() {
            active.accept_sensitive = true;
            active.show_error(tr("The file does not match the selected filter"));
        }
        return;
    }
    super::accept::complete_paths_writable(app, vec![path], tx);
}
