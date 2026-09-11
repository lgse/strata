// SPDX-License-Identifier: MIT

//! Local archive operation entry points and worker/event coordination.
//!
//! Compression publishes completed staging files; extraction confines writes to
//! a pinned destination. Format integration stays behind the operation provider.

mod compression;
mod decoders;
mod destination;
mod extraction;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

use crate::{
    model::Location,
    services::{
        ArchiveFormat, CancelledOperation, CompressRequest, ExtractRequest, LoadHandle,
        OperationEvent, OperationRequestId, validate_basename,
    },
};
use compression::{
    compress_7z, compress_tar, compress_zip, count_archive_files, write_staged_archive,
};
use decoders::{extract_7z_from_reader, extract_tar, extract_zip_from_archive};
use extraction::ArchiveOutcome;
use gtk::{gio, glib};
use std::{
    collections::HashSet,
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// Compresses the entries in `request` into a new local archive.
///
/// Validates that the destination is a local path and that
/// [`CompressRequest::archive_name`] passes [`validate_basename`], then writes
/// `{name}.{extension}` through [`write_staged_archive`]. Progress is polled
/// every 100 ms via [`archive_progress_timer`]. Returns a [`LoadHandle`] that
/// cancels in-flight work when dropped.
///
/// Emits [`ArchiveStarted`] immediately, [`ArchiveProgress`] while running,
/// then [`Compressed`], [`Failed`], or [`Cancelled`] depending on the outcome.
///
/// # Concurrency
///
/// Runs on the default [`glib::MainContext`]. Archive encoding happens on a
/// worker thread via [`gio::spawn_blocking`].
///
/// [`ArchiveStarted`]: OperationEvent::ArchiveStarted
/// [`ArchiveProgress`]: OperationEvent::ArchiveProgress
/// [`Compressed`]: OperationEvent::Compressed
/// [`Failed`]: OperationEvent::Failed
/// [`Cancelled`]: OperationEvent::Cancelled
pub(super) fn compress(request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();
    let source_locations = request
        .entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();
    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Archive destination must be a local path".to_owned(),
            });
            return;
        };
        if let Err(message) = validate_basename(&request.archive_name) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: message.to_owned(),
            });
            return;
        }
        let archive_name = format!("{}.{}", request.archive_name, request.format.extension());
        let archive_path = dest_dir.join(&archive_name);
        let entries: Vec<std::path::PathBuf> = request
            .entries
            .iter()
            .filter_map(|e| e.location.native_path().map(Path::to_path_buf))
            .collect();
        if entries.is_empty() {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Nothing to compress".to_owned(),
            });
            return;
        }
        let total = Arc::new(AtomicUsize::new(0));
        let progress = Arc::new(AtomicUsize::new(0));
        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });
        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);
        let format = request.format;
        let password = request.password.clone();
        let work_progress = progress.clone();
        let work_total = total.clone();
        let result = write_staged_archive(
            &dest_dir,
            &archive_path,
            request.conflict,
            &task_cancelled,
            move |file| {
                let count = count_archive_files(&entries, &work_cancelled)?;
                work_total.store(count, Ordering::Relaxed);
                match format {
                    ArchiveFormat::Zip => compress_zip(
                        file,
                        &entries,
                        password.as_deref(),
                        &work_progress,
                        &work_cancelled,
                    ),
                    ArchiveFormat::SevenZ => compress_7z(
                        file,
                        &entries,
                        password.as_deref(),
                        &work_progress,
                        &work_cancelled,
                    ),
                    ArchiveFormat::TarGz => {
                        compress_tar(file, &entries, true, &work_progress, &work_cancelled)
                    }
                    ArchiveFormat::Tar => {
                        compress_tar(file, &entries, false, &work_progress, &work_cancelled)
                    }
                }
            },
        )
        .await;
        timer_id.remove();
        match result {
            Ok(()) => emit(OperationEvent::Compressed {
                request_id: request.id,
                archive_name: archive_name.clone(),
            }),
            Err(ArchiveError::Cancelled) => emit(cancelled_archive_event(
                request.id,
                destination,
                Vec::new(),
                Vec::new(),
                source_locations,
            )),
            Err(ArchiveError::Failed(error)) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: error,
            }),
        }
    });
    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

/// Extracts the archive in `request` into a local destination directory.
///
/// Requires both the archive and destination to be local paths. Format is
/// inferred from [`FileEntry::display_name`] via [`ArchiveFormat::from_extension`].
/// Returns a [`LoadHandle`] that cancels in-flight work when dropped.
///
/// Emits [`ArchiveStarted`] immediately, [`ArchiveProgress`] while running,
/// then [`Extracted`], [`Failed`], or [`Cancelled`]. A cancel after some
/// members have been written reports completed, failed, and not-attempted
/// locations through [`CancelledOperation`].
///
/// # Concurrency
///
/// Runs on the default [`glib::MainContext`]. Decoding happens on a worker
/// thread via [`gio::spawn_blocking`].
///
/// [`FileEntry::display_name`]: crate::model::FileEntry::display_name
/// [`ArchiveStarted`]: OperationEvent::ArchiveStarted
/// [`ArchiveProgress`]: OperationEvent::ArchiveProgress
/// [`Extracted`]: OperationEvent::Extracted
/// [`Failed`]: OperationEvent::Failed
/// [`Cancelled`]: OperationEvent::Cancelled
pub(super) fn extract(request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(archive_path) = request.entry.location.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Archive must be a local file".to_owned(),
            });
            return;
        };
        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Extract destination must be a local path".to_owned(),
            });
            return;
        };
        let created_dest = !dest_dir.exists();
        if created_dest && let Err(e) = std::fs::create_dir_all(&dest_dir) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: format!("Could not create folder: {e}"),
            });
            return;
        }
        let dest_dir_for_cleanup = dest_dir.clone();
        let format = ArchiveFormat::from_extension(&request.entry.display_name);
        let password = request.password.clone();
        let display_name = request.entry.display_name.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });
        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);
        let work_progress = progress.clone();
        let work_total = total.clone();
        let result = gio::spawn_blocking(move || match format {
            Some(ArchiveFormat::Zip) => {
                let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                let mut archive = zip::ZipArchive::new(file).map_err(decoders::zip_error)?;
                work_total.store(archive.len(), Ordering::Relaxed);
                extract_zip_from_archive(
                    &mut archive,
                    &dest_dir,
                    password.as_deref(),
                    &work_progress,
                    &work_cancelled,
                )
            }
            Some(ArchiveFormat::SevenZ) => {
                let pw = password
                    .as_deref()
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default();
                let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                extract_7z_from_reader(file, &dest_dir, pw, &work_progress, &work_cancelled)
            }
            Some(ArchiveFormat::TarGz) => extract_tar(
                &archive_path,
                &dest_dir,
                true,
                &work_progress,
                &work_cancelled,
            ),
            Some(ArchiveFormat::Tar) => extract_tar(
                &archive_path,
                &dest_dir,
                false,
                &work_progress,
                &work_cancelled,
            ),
            None => Err(archive_failed(format!(
                "Unsupported archive format: {display_name}"
            ))),
        })
        .await;
        timer_id.remove();
        if created_dest
            && !matches!(result, Ok(Ok(ArchiveOutcome::Completed(_))))
            && std::fs::read_dir(&dest_dir_for_cleanup)
                .is_ok_and(|mut entries| entries.next().is_none())
        {
            let _ = std::fs::remove_dir(&dest_dir_for_cleanup);
        }
        match result {
            Ok(Ok(ArchiveOutcome::Completed(first_name))) => emit(OperationEvent::Extracted {
                request_id: request.id,
                first_name,
            }),
            Ok(Ok(ArchiveOutcome::Cancelled {
                completed,
                failed,
                not_attempted,
            })) => emit(cancelled_archive_event(
                request.id,
                destination,
                completed,
                failed,
                not_attempted,
            )),
            Ok(Err(ArchiveError::Cancelled)) => emit(cancelled_archive_event(
                request.id,
                destination,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
            Ok(Err(ArchiveError::Failed(error))) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: error,
            }),
            Err(_) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: "Extraction task panicked".to_owned(),
            }),
        }
    });
    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

/// Byte size of the reusable read/write buffer used by [`copy_with_big_buf`].
const COPY_BUF: usize = 1 << 20;
/// Sentinel message used to round-trip cancellation through `sevenz_rust2`.
const ARCHIVE_CANCELLED: &str = "Operation cancelled";

/// Failure or cooperative cancellation of a compress or extract step.
#[derive(Debug, PartialEq, Eq)]
enum ArchiveError {
    /// The [`LoadHandle`] cancelled the operation before it finished.
    Cancelled,
    /// Encoding, decoding, or filesystem work failed with this message.
    Failed(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str(ARCHIVE_CANCELLED),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<String> for ArchiveError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for ArchiveError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_owned())
    }
}

fn archive_failed(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Failed(error.to_string())
}

/// Returns [`ArchiveError::Cancelled`] when the `cancelled` flag is set.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set
///
/// [`Cancelled`]: ArchiveError::Cancelled
fn check_archive_cancelled(cancelled: &AtomicBool) -> Result<(), ArchiveError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(ArchiveError::Cancelled)
    } else {
        Ok(())
    }
}

/// Builds an [`OperationEvent::Cancelled`] for a compress or extract request.
///
/// # Arguments
///
/// * `request_id` - Identifier of the cancelled request
/// * `destination` - Archive path or extract directory, recorded as affected
/// * `completed` - Members fully written before cancellation
/// * `failed` - Members left in a partial or unremovable state
/// * `not_attempted` - Members not started, including a cleaned-up partial write
fn cancelled_archive_event(
    request_id: OperationRequestId,
    destination: Location,
    completed: Vec<Location>,
    failed: Vec<Location>,
    not_attempted: Vec<Location>,
) -> OperationEvent {
    OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed,
            failed,
            not_attempted,
            affected_locations: HashSet::from([destination]),
        },
    }
}

/// Copies `reader` to `writer`, checking `cancelled` between 1 MiB chunks.
///
/// Returns the number of bytes written.
///
/// # Performance
///
/// Uses a [`COPY_BUF`]-sized heap buffer so large members are not copied
/// through the default 8 KiB [`std::io::copy`] path.
///
/// # Errors
///
/// - [`Cancelled`] if `cancelled` is set between reads
/// - [`Failed`] if a read or write fails
///
/// [`Cancelled`]: ArchiveError::Cancelled
/// [`Failed`]: ArchiveError::Failed
fn copy_with_big_buf(
    mut reader: impl std::io::Read,
    writer: &mut (impl std::io::Write + ?Sized),
    cancelled: &AtomicBool,
) -> Result<u64, ArchiveError> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut total = 0;
    loop {
        check_archive_cancelled(cancelled)?;
        let n = reader.read(&mut buf).map_err(archive_failed)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).map_err(archive_failed)?;
        total += n as u64;
    }
    Ok(total)
}

/// Starts a 100 ms timer that emits [`OperationEvent::ArchiveProgress`].
///
/// The source stays attached until the caller removes the returned
/// [`glib::SourceId`]. Returning [`glib::ControlFlow::Break`] from the
/// callback would double-remove that source.
///
/// # Arguments
///
/// * `request_id` - Identifier included in each progress event
/// * `progress` - Completed member count polled each tick
/// * `total` - Expected member count, which may still be zero during counting
/// * `cancelled` - When set, ticks stop emitting but the source stays attached
/// * `emit` - Callback that receives progress events on the main context
fn archive_progress_timer(
    request_id: OperationRequestId,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &Arc<AtomicBool>,
    emit: &Rc<dyn Fn(OperationEvent)>,
) -> glib::SourceId {
    let timer_progress = progress.clone();
    let timer_total = total.clone();
    let timer_cancelled = cancelled.clone();
    let timer_emit = emit.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        // Keep the source until the task calls remove(); Break would double-remove.
        if !timer_cancelled.load(Ordering::Relaxed) {
            timer_emit(OperationEvent::ArchiveProgress {
                request_id,
                completed: timer_progress.load(Ordering::Relaxed),
                total: timer_total.load(Ordering::Relaxed),
            });
        }
        glib::ControlFlow::Continue
    })
}
