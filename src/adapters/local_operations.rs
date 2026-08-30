// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{future::Future, io, path::Path, pin::Pin, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    services::{
        CreateDirectoryRequest, CreateFileRequest, DeleteRequest, LoadHandle, OperationEvent,
        OperationProvider, PasteRequest, RenameRequest, RestoreRequest, TransferConflict,
        validate_basename,
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
                    errors.push(format!("{}: {error}", entry.display_name));
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
}
