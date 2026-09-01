// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    future::Future,
    io,
    path::Path,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    services::{
        ArchiveFormat, CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
        ExtractRequest, LoadHandle, OperationEvent, OperationProvider, OperationRequestId,
        PasteRequest, RenameRequest, RestoreRequest, TransferConflict, validate_basename,
    },
};

fn gio_file(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

fn validated_child(parent: &gio::File, name: &str) -> Result<gio::File, &'static str> {
    validate_basename(name)?;
    Ok(parent.child(name))
}

fn transfer_is_noop(source: &gio::File, destination: &gio::File, target: &gio::File) -> bool {
    source.equal(target) || source.equal(destination) || destination.has_prefix(source)
}

fn copy_recursively(
    source: gio::File,
    target: gio::File,
    overwrite_existing: bool,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let info = source
            .query_info_future(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await?;
        if info.file_type() == gio::FileType::Directory {
            if !overwrite_existing || !target.query_exists(None::<&gio::Cancellable>) {
                target
                    .make_directory_future(glib::Priority::DEFAULT)
                    .await?;
            }
            let enumerator = source
                .enumerate_children_future(
                    "standard::name",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                )
                .await?;
            loop {
                let children = enumerator
                    .next_files_future(64, glib::Priority::DEFAULT)
                    .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    copy_recursively(
                        source.child(child.name()),
                        target.child(child.name()),
                        overwrite_existing,
                    )
                    .await?;
                }
            }
            Ok(())
        } else {
            let flags = gio::FileCopyFlags::ALL_METADATA
                | gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                | if overwrite_existing {
                    gio::FileCopyFlags::OVERWRITE
                } else {
                    gio::FileCopyFlags::NONE
                };
            let (copy, _progress) = source.copy_future(&target, flags, glib::Priority::DEFAULT);
            copy.await
        }
    })
}

enum StagedSibling {
    File(tempfile::TempPath),
    Directory(tempfile::TempDir),
}

impl StagedSibling {
    fn create(parent: &Path, directory: bool) -> io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".strata-replacement-");
        if directory {
            builder.tempdir_in(parent).map(Self::Directory)
        } else {
            builder
                .tempfile_in(parent)
                .map(tempfile::NamedTempFile::into_temp_path)
                .map(Self::File)
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Directory(directory) => directory.path(),
        }
    }
}

fn io_error(error: impl std::fmt::Display) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

type StageCopy = Rc<
    dyn Fn(gio::File, gio::File, bool) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

async fn replace_local_with(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    copy_to_stage: StageCopy,
) -> Result<(), glib::Error> {
    if source.path().is_none() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable for this source",
        ));
    }
    let target_path = target.path().ok_or_else(|| {
        glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "Safe replacement is unavailable at this destination",
        )
    })?;
    let parent = target_path
        .parent()
        .ok_or_else(|| io_error("The destination has no parent directory"))?;
    let source_type = source
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?
        .file_type();
    let target_type = target
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?
        .file_type();
    let source_is_directory = source_type == gio::FileType::Directory;
    let target_is_directory = target_type == gio::FileType::Directory;
    if source_is_directory != target_is_directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "A file and a folder cannot safely replace one another",
        ));
    }

    let staged = StagedSibling::create(parent, source_is_directory).map_err(io_error)?;
    let staged_file = gio::File::for_path(staged.path());
    copy_to_stage(source.clone(), staged_file.clone(), source_is_directory).await?;

    let staged_path = staged.path().to_owned();
    let exchanged = gio::spawn_blocking(move || {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &staged_path,
            rustix::fs::CWD,
            &target_path,
            rustix::fs::RenameFlags::EXCHANGE,
        )
    })
    .await
    .map_err(|_| io_error("The replacement worker stopped unexpectedly"))?;
    exchanged.map_err(|error| io_error(format!("Could not safely replace the item: {error}")))?;

    permanently_delete(staged_file, target_is_directory).await?;
    if move_source {
        permanently_delete(source, source_is_directory).await?;
    }
    Ok(())
}

async fn replace_local(
    source: gio::File,
    target: gio::File,
    move_source: bool,
) -> Result<(), glib::Error> {
    replace_local_with(
        source,
        target,
        move_source,
        Rc::new(|source, staged, directory| {
            Box::pin(async move {
                if directory {
                    copy_recursively(source, staged, true).await
                } else {
                    let flags = gio::FileCopyFlags::ALL_METADATA
                        | gio::FileCopyFlags::NOFOLLOW_SYMLINKS
                        | gio::FileCopyFlags::OVERWRITE;
                    let (copy, _progress) =
                        source.copy_future(&staged, flags, glib::Priority::DEFAULT);
                    copy.await
                }
            })
        }),
    )
    .await
}

fn permanently_delete(
    file: gio::File,
    directory: bool,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if directory {
            let enumerator = file
                .enumerate_children_future(
                    "standard::name,standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                )
                .await?;
            loop {
                let children = enumerator
                    .next_files_future(64, glib::Priority::DEFAULT)
                    .await?;
                if children.is_empty() {
                    break;
                }
                for child in children {
                    permanently_delete(
                        file.child(child.name()),
                        child.file_type() == gio::FileType::Directory,
                    )
                    .await?;
                }
            }
        }
        file.delete_future(glib::Priority::DEFAULT).await
    })
}

fn operation_error_summary(errors: &[String], action: &str) -> String {
    let mut summary = format!(
        "{} could not be {action}. The remaining items were processed.",
        if errors.len() == 1 {
            "1 item".to_owned()
        } else {
            format!("{} items", errors.len())
        }
    );
    for error in errors.iter().take(8) {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if errors.len() > 8 {
        summary.push_str(&format!("\n\n…and {} more", errors.len() - 8));
    }
    summary
}

fn deletion_error_summary(errors: &[String]) -> String {
    operation_error_summary(errors, "deleted")
}

/// Backends without Trash support (most remote filesystems, including SMB)
/// fail a move-to-trash with `NOT_SUPPORTED`. Give an actionable message for
/// that specific case instead of the raw GIO error text.
fn deletion_error_message(name: &str, permanent: bool, error: &glib::Error) -> String {
    if !permanent && error.matches(gio::IOErrorEnum::NotSupported) {
        format!("{name}: This location doesn't support Trash. Delete permanently instead.")
    } else {
        format!("{name}: {error}")
    }
}

#[derive(Default)]
pub struct LocalOperationProvider;

impl OperationProvider for LocalOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            if let Err(message) = validate_basename(&request.new_name) {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: message.to_owned(),
                });
                return;
            }
            let file = request
                .entry
                .location
                .native_path()
                .map(gio::File::for_path)
                .unwrap_or_else(|| {
                    gio::File::for_uri(request.entry.location.uri_value().unwrap_or_default())
                });
            match file
                .set_display_name_future(&request.new_name, glib::Priority::DEFAULT)
                .await
            {
                Ok(_) => emit(OperationEvent::Renamed {
                    request_id: request.id,
                }),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let parent = gio_file(&request.parent);
            let folder = match validated_child(&parent, &request.name) {
                Ok(folder) => folder,
                Err(message) => {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: message.to_owned(),
                    });
                    return;
                }
            };
            match folder.make_directory_future(glib::Priority::DEFAULT).await {
                Ok(()) => emit(OperationEvent::Created {
                    request_id: request.id,
                }),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let parent = gio_file(&request.parent);
            let file = match validated_child(&parent, &request.name) {
                Ok(file) => file,
                Err(message) => {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: message.to_owned(),
                    });
                    return;
                }
            };
            match file
                .create_future(gio::FileCreateFlags::NONE, glib::Priority::DEFAULT)
                .await
            {
                Ok(_) => emit(OperationEvent::Created {
                    request_id: request.id,
                }),
                Err(error) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.to_string(),
                }),
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let destination = gio_file(&request.destination);
            for item in &request.items {
                let source = gio_file(&item.source);
                let Some(name) = source.basename() else {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: "A clipboard item has no file name".to_owned(),
                    });
                    return;
                };
                let target = destination.child(name);
                if transfer_is_noop(&source, &destination, &target) {
                    continue;
                }
                let result = if item.conflict == TransferConflict::ReplaceExisting {
                    replace_local(source, target, request.move_sources).await
                } else if request.move_sources {
                    let flags =
                        gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
                    let (transfer, _progress) =
                        source.move_future(&target, flags, glib::Priority::DEFAULT);
                    transfer.await
                } else {
                    copy_recursively(source, target, false).await
                };
                if let Err(error) = result {
                    emit(OperationEvent::Failed {
                        request_id: request.id,
                        message: error.to_string(),
                    });
                    return;
                }
            }
            emit(OperationEvent::Pasted {
                request_id: request.id,
            });
        });
        LoadHandle::new(move || task.abort())
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let mut errors = Vec::new();
            let mut deleted_locations = Vec::new();
            let total = request.entries.len();
            for (index, entry) in request.entries.iter().enumerate() {
                let file = gio_file(&entry.location);
                let result = if request.permanent {
                    if entry
                        .location
                        .uri_value()
                        .is_some_and(|uri| uri.starts_with("trash:"))
                    {
                        file.delete_future(glib::Priority::DEFAULT).await
                    } else {
                        permanently_delete(file, entry.is_directory()).await
                    }
                } else {
                    file.trash_future(glib::Priority::DEFAULT).await
                };
                let deleted_location = if let Err(error) = result {
                    errors.push(deletion_error_message(
                        &entry.display_name,
                        request.permanent,
                        &error,
                    ));
                    None
                } else {
                    deleted_locations.push(entry.location.clone());
                    Some(entry.location.clone())
                };
                emit(OperationEvent::DeleteProgress {
                    request_id: request.id,
                    completed: index + 1,
                    total,
                    deleted_location,
                });
            }
            if errors.is_empty() {
                emit(OperationEvent::Deleted {
                    request_id: request.id,
                    locations: deleted_locations,
                });
            } else {
                emit(OperationEvent::CompletedWithErrors {
                    request_id: request.id,
                    deleted_locations,
                    message: deletion_error_summary(&errors),
                });
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let total = request.entries.len();
            let mut errors = Vec::new();
            let mut restored_locations = Vec::new();
            for (index, entry) in request.entries.iter().enumerate() {
                let source = gio_file(&entry.location);
                let result = match source
                    .query_info_future(
                        "trash::orig-path",
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    )
                    .await
                {
                    Ok(info) => match info.attribute_byte_string("trash::orig-path") {
                        Some(original_path) => {
                            let target =
                                gio::File::for_path(std::path::Path::new(original_path.as_str()));
                            let (restore, _progress) = source.move_future(
                                &target,
                                gio::FileCopyFlags::ALL_METADATA
                                    | gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
                                glib::Priority::DEFAULT,
                            );
                            restore.await
                        }
                        None => Err(glib::Error::new(
                            gio::IOErrorEnum::NotFound,
                            "The original location is unavailable",
                        )),
                    },
                    Err(error) => Err(error),
                };
                let restored_location = if let Err(error) = result {
                    errors.push(format!("{}: {error}", entry.display_name));
                    None
                } else {
                    restored_locations.push(entry.location.clone());
                    Some(entry.location.clone())
                };
                emit(OperationEvent::RestoreProgress {
                    request_id: request.id,
                    completed: index + 1,
                    total,
                    restored_location,
                });
            }
            if errors.is_empty() {
                emit(OperationEvent::Restored {
                    request_id: request.id,
                    locations: restored_locations,
                });
            } else {
                emit(OperationEvent::RestoreCompletedWithErrors {
                    request_id: request.id,
                    restored_locations,
                    message: operation_error_summary(&errors, "restored"),
                });
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let Some(dest_dir) = request.destination.native_path() else {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "Archive destination must be a local path".to_owned(),
                });
                return;
            };
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
            let timer_id = archive_progress_timer(request.id, &progress, &total, &emit);
            let format = request.format;
            let password = request.password.clone();
            let work_progress = progress.clone();
            let work_total = total.clone();
            let result = gio::spawn_blocking(move || {
                let count = count_files(&entries);
                work_total.store(count, Ordering::Relaxed);
                match format {
                    ArchiveFormat::Zip => {
                        compress_zip(&archive_path, &entries, password.as_deref(), &work_progress)
                    }
                    ArchiveFormat::SevenZ => {
                        compress_7z(&archive_path, &entries, password.as_deref(), &work_progress)
                    }
                    ArchiveFormat::TarGz => {
                        compress_tar(&archive_path, &entries, true, &work_progress)
                    }
                    ArchiveFormat::Tar => {
                        compress_tar(&archive_path, &entries, false, &work_progress)
                    }
                }
            })
            .await;
            timer_id.remove();
            match result {
                Ok(Ok(())) => emit(OperationEvent::Compressed {
                    request_id: request.id,
                    archive_name: archive_name.clone(),
                }),
                Ok(Err(error)) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error,
                }),
                Err(_) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "Compression task panicked".to_owned(),
                }),
            }
        });
        LoadHandle::new(move || task.abort())
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let Some(archive_path) = request.entry.location.native_path().map(Path::to_path_buf)
            else {
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
            let format = ArchiveFormat::from_extension(&request.entry.display_name);
            let password = request.password.clone();
            let display_name = request.entry.display_name.clone();
            let progress = Arc::new(AtomicUsize::new(0));
            let total = Arc::new(AtomicUsize::new(0));
            emit(OperationEvent::ArchiveStarted {
                request_id: request.id,
                total: 0,
            });
            let timer_id = archive_progress_timer(request.id, &progress, &total, &emit);
            let work_progress = progress.clone();
            let work_total = total.clone();
            let result = gio::spawn_blocking(move || match format {
                Some(ArchiveFormat::Zip) => {
                    let file = std::fs::File::open(&archive_path).map_err(|e| e.to_string())?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                    work_total.store(archive.len(), Ordering::Relaxed);
                    extract_zip_from_archive(
                        &mut archive,
                        &dest_dir,
                        password.as_deref(),
                        &work_progress,
                    )
                }
                Some(ArchiveFormat::SevenZ) => {
                    let pw = password
                        .as_deref()
                        .map(sevenz_rust2::Password::from)
                        .unwrap_or_default();
                    let reader = sevenz_rust2::ArchiveReader::open(&archive_path, pw)
                        .map_err(|e| e.to_string())?;
                    extract_7z_from_reader(reader, &dest_dir, &work_progress)
                }
                Some(ArchiveFormat::TarGz) => {
                    extract_tar(&archive_path, &dest_dir, true, &work_progress)
                }
                Some(ArchiveFormat::Tar) => {
                    extract_tar(&archive_path, &dest_dir, false, &work_progress)
                }
                None => Err(format!("Unsupported archive format: {}", display_name)),
            })
            .await;
            timer_id.remove();
            match result {
                Ok(Ok(first_name)) => emit(OperationEvent::Extracted {
                    request_id: request.id,
                    first_name,
                }),
                Ok(Err(error)) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error,
                }),
                Err(_) => emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "Extraction task panicked".to_owned(),
                }),
            }
        });
        LoadHandle::new(move || task.abort())
    }
}

fn compress_zip(
    archive_path: &Path,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    let file = std::fs::File::create(archive_path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::with_capacity(COPY_BUF, file);
    let mut writer = zip::ZipWriter::new(writer);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = if let Some(pw) = password {
        deflated.with_aes_encryption(zip::AesMode::Aes256, pw)
    } else {
        deflated
    };
    let stored = if let Some(pw) = password {
        stored.with_aes_encryption(zip::AesMode::Aes256, pw)
    } else {
        stored
    };
    for entry in entries {
        let name = entry
            .file_name()
            .ok_or("Entry has no file name")?
            .to_string_lossy()
            .to_string();
        if entry.is_dir() {
            add_dir_to_zip(&mut writer, entry, &name, &deflated, &stored, progress)?;
        } else {
            let opts = if is_incompressible(entry) {
                &stored
            } else {
                &deflated
            };
            writer.start_file(&name, *opts).map_err(|e| e.to_string())?;
            let f = std::fs::File::open(entry).map_err(|e| e.to_string())?;
            let f = std::io::BufReader::with_capacity(COPY_BUF, f);
            copy_with_big_buf(f, &mut writer).map_err(|e| e.to_string())?;
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    deflated: &zip::write::FileOptions<'_, ()>,
    stored: &zip::write::FileOptions<'_, ()>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel_name = format!("{}/{}", prefix, entry.file_name().to_string_lossy());
        if path.is_dir() {
            add_dir_to_zip(writer, &path, &rel_name, deflated, stored, progress)?;
        } else {
            let opts = if is_incompressible(&path) {
                stored
            } else {
                deflated
            };
            writer
                .start_file(&rel_name, *opts)
                .map_err(|e| e.to_string())?;
            let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let f = std::io::BufReader::with_capacity(COPY_BUF, f);
            copy_with_big_buf(f, writer).map_err(|e| e.to_string())?;
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn compress_tar(
    archive_path: &Path,
    entries: &[std::path::PathBuf],
    gzip: bool,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    let file = std::fs::File::create(archive_path).map_err(|e| e.to_string())?;
    let writer: Box<dyn std::io::Write> = if gzip {
        Box::new(std::io::BufWriter::with_capacity(
            COPY_BUF,
            flate2::write::GzEncoder::new(file, flate2::Compression::default()),
        ))
    } else {
        Box::new(std::io::BufWriter::with_capacity(COPY_BUF, file))
    };
    let mut builder = tar::Builder::new(writer);
    for entry in entries {
        let name = entry
            .file_name()
            .ok_or("Entry has no file name")?
            .to_string_lossy()
            .to_string();
        if entry.is_dir() {
            builder
                .append_dir_all(&name, entry)
                .map_err(|e| e.to_string())?;
        } else {
            builder
                .append_path_with_name(entry, &name)
                .map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    builder.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn safe_extract_path(dest_dir: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let outpath = dest_dir.join(name);
    if !outpath.starts_with(dest_dir) {
        return Err(format!("Refusing to extract outside destination: {name}"));
    }
    Ok(outpath)
}

/// Returns a path that doesn't conflict with existing files. If `outpath` exists,
/// appends " (2)", " (3)", etc. to the stem.
fn unique_path(outpath: &Path) -> std::path::PathBuf {
    if !outpath.exists() {
        return outpath.to_path_buf();
    }
    let parent = outpath.parent().unwrap_or(Path::new("."));
    let stem = outpath
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = outpath
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 2.. {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    outpath.to_path_buf()
}

/// Counts all files (not directories) under the given paths recursively.
fn count_files(entries: &[std::path::PathBuf]) -> usize {
    let mut count = 0;
    for entry in entries {
        if entry.is_dir() {
            if let Ok(rd) = std::fs::read_dir(entry) {
                for child in rd.flatten() {
                    count += count_files(&[child.path()]);
                }
            }
        } else {
            count += 1;
        }
    }
    count
}

const COPY_BUF: usize = 1 << 20; // 1 MiB

fn copy_with_big_buf(
    mut reader: impl std::io::Read,
    writer: &mut (impl std::io::Write + ?Sized),
) -> std::io::Result<u64> {
    let mut buf = vec![0u8; COPY_BUF];
    let mut total = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    Ok(total)
}

/// Spawns a 100ms timer that polls progress counters and emits ArchiveProgress events.
fn archive_progress_timer(
    request_id: OperationRequestId,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    emit: &Rc<dyn Fn(OperationEvent)>,
) -> glib::SourceId {
    let timer_progress = progress.clone();
    let timer_total = total.clone();
    let timer_emit = emit.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        timer_emit(OperationEvent::ArchiveProgress {
            request_id,
            completed: timer_progress.load(Ordering::Relaxed),
            total: timer_total.load(Ordering::Relaxed),
        });
        glib::ControlFlow::Continue
    })
}

/// File extensions that are already compressed — storing them raw saves CPU with zero size gain.
const INCOMPRESSIBLE_EXTS: &[&str] = &[
    "zip", "7z", "gz", "bz2", "xz", "zst", "tar", "rar", "lz", "lz4", "br", "mp4", "mkv", "avi",
    "mov", "webm", "flv", "wmv", "jpg", "jpeg", "png", "webp", "gif", "heic", "avif", "bmp", "mp3",
    "flac", "aac", "ogg", "opus", "wma", "m4a", "pdf", "epub", "docx", "xlsx", "pptx", "odt",
    "ods", "odp", "iso", "dmg", "deb", "rpm", "apk", "jar", "war",
];

fn is_incompressible(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| INCOMPRESSIBLE_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Tracks renamed top-level entries so child paths follow the rename.
struct ExtractNameResolver {
    renames: std::collections::HashMap<String, String>,
}

impl ExtractNameResolver {
    fn new() -> Self {
        Self {
            renames: std::collections::HashMap::new(),
        }
    }

    /// Resolves an archive entry name to a safe, conflict-free filesystem path.
    /// If the top-level component already exists, it's renamed to "name (2)", etc.
    fn resolve(&mut self, dest_dir: &Path, name: &str) -> Result<std::path::PathBuf, String> {
        let top = name.split('/').next().unwrap_or(name);
        let rest = &name[top.len()..];
        let resolved_top = if let Some(existing) = self.renames.get(top) {
            existing.clone()
        } else if dest_dir.join(top).exists() {
            let renamed = unique_path(&dest_dir.join(top));
            let new_name = renamed
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| top.to_string());
            self.renames.insert(top.to_string(), new_name.clone());
            new_name
        } else {
            self.renames.insert(top.to_string(), top.to_string());
            top.to_string()
        };
        let resolved_name = format!("{resolved_top}{rest}");
        safe_extract_path(dest_dir, &resolved_name)
    }
}

fn extract_zip_from_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let pw_bytes = password.map(|p| p.as_bytes());
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    for i in 0..archive.len() {
        let read_options = zip::read::ZipReadOptions::new().password(pw_bytes);
        let mut entry = archive
            .by_index_with_options(i, read_options)
            .map_err(|e| e.to_string())?;
        let name = entry.name().trim_end_matches('/').to_owned();
        let outpath = resolver.resolve(dest_dir, &name)?;
        if first_name.is_none() {
            first_name = outpath
                .strip_prefix(dest_dir)
                .ok()
                .and_then(|p| p.components().next())
                .map(|c| c.as_os_str().to_string_lossy().to_string());
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let outpath = unique_path(&outpath);
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
            copy_with_big_buf(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(first_name)
}

fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    let mut resolver = ExtractNameResolver::new();
    let mut first_name = None;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let name = entry.path().map_err(|e| e.to_string())?;
        let name = name.to_string_lossy().trim_end_matches('/').to_string();
        let outpath = resolver.resolve(dest_dir, &name)?;
        if first_name.is_none() {
            first_name = outpath
                .strip_prefix(dest_dir)
                .ok()
                .and_then(|p| p.components().next())
                .map(|c| c.as_os_str().to_string_lossy().to_string());
        }
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let outpath = unique_path(&outpath);
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
            copy_with_big_buf(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(first_name)
}

fn compress_7z(
    archive_path: &Path,
    entries: &[std::path::PathBuf],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options};
    let mut writer =
        sevenz_rust2::ArchiveWriter::create(archive_path).map_err(|e| e.to_string())?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let lzma2 =
        sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2).with_options(
            EncoderOptions::Lzma2(Lzma2Options::from_level_mt(6, threads, 1 << 26)),
        );
    if let Some(pw) = password {
        let methods = vec![lzma2, AesEncoderOptions::new(pw.into()).into()];
        writer.set_content_methods(methods);
    } else {
        writer.set_content_methods(vec![lzma2]);
    }
    for entry in entries {
        add_path_to_7z(&mut writer, entry, entry, progress)?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn add_path_to_7z(
    writer: &mut sevenz_rust2::ArchiveWriter<std::fs::File>,
    base: &Path,
    path: &Path,
    progress: &Arc<AtomicUsize>,
) -> Result<(), String> {
    if path.is_dir() {
        for child in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            add_path_to_7z(writer, base, &child.path(), progress)?;
        }
    } else {
        let name = path
            .strip_prefix(base.parent().unwrap_or(base))
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let entry = sevenz_rust2::ArchiveEntry::from_path(path, name);
        let reader = std::fs::File::open(path).map_err(|e| e.to_string())?;
        writer
            .push_archive_entry(entry, Some(reader))
            .map_err(|e| e.to_string())?;
        progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

fn extract_7z_from_reader(
    mut reader: sevenz_rust2::ArchiveReader<std::fs::File>,
    dest_dir: &Path,
    progress: &Arc<AtomicUsize>,
) -> Result<Option<String>, String> {
    let resolver = std::cell::RefCell::new(ExtractNameResolver::new());
    let first_name = std::cell::RefCell::new(None::<String>);
    let dest = dest_dir.to_path_buf();
    let progress = progress.clone();
    reader
        .for_each_entries(|entry, reader| {
            let name = entry.name.trim_end_matches('/');
            let outpath = resolver
                .borrow_mut()
                .resolve(&dest, name)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            if first_name.borrow().is_none() {
                *first_name.borrow_mut() = outpath
                    .strip_prefix(&dest)
                    .ok()
                    .and_then(|p| p.components().next())
                    .map(|c| c.as_os_str().to_string_lossy().to_string());
            }
            if entry.is_directory {
                std::fs::create_dir_all(&outpath)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                }
                let outpath = unique_path(&outpath);
                let mut file = std::fs::File::create(&outpath)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                copy_with_big_buf(reader, &mut file)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            }
            progress.fetch_add(1, Ordering::Relaxed);
            Ok(false)
        })
        .map_err(|e| e.to_string())?;
    Ok(first_name.into_inner())
}
