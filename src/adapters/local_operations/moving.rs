// SPDX-License-Identifier: MIT

use super::*;

pub(super) enum Moved {
    Done(gio::File),
    Copy,
}

pub(super) type MoveAttempt = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        Taken,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<Moved, glib::Error>>>>,
>;

pub(super) type RemoteCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        Taken,
        gio::Cancellable,
        LeftOut,
    ) -> Pin<Box<dyn Future<Output = Result<gio::File, glib::Error>>>>,
>;

pub(super) fn native_path(file: &gio::File) -> Option<PathBuf> {
    file.is_native().then(|| file.path()).flatten()
}

pub(super) struct Transferred {
    pub(super) target: gio::File,
    pub(super) reported: Reported,
    pub(super) owed: Owed,
}

pub(super) async fn move_local_owing(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
    attempt_move: MoveAttempt,
    remote_copy: RemoteCopy,
) -> Result<Transferred, glib::Error> {
    if let Moved::Done(target) =
        attempt_move(source.clone(), target.clone(), taken, cancellable.clone()).await?
    {
        return Ok(Transferred {
            target,
            reported: Reported::default(),
            owed: Owed::default(),
        });
    }
    let Copying {
        progress, names, ..
    } = copying;
    let mut owed = Owed::default();
    if let Some(target_path) = native_path(&target) {
        owed.barrier = Some(copy_engine::barrier(split_local_path(&target_path)?.0).await?);
    }
    let (target, reported, deletion) = match (native_path(&source), native_path(&target)) {
        (Some(source_path), Some(target_path)) => {
            let options = copy_engine::Options {
                names,
                manifest: true,
                durable: true,
                ..copy_engine::Options::default()
            };
            let copied = copy_engine::copy_local(
                source_path,
                copy_engine::Destination::New(target_path.clone(), taken),
                options,
                progress.clone(),
                &cancellable,
            )
            .await?;
            let target = gio::File::for_path(target_path.with_file_name(copied.name));
            let deletion = match copied.manifest {
                Some(sources) => Deletion::Sources(source, sources),
                None => Deletion::Source(source, Vec::new()),
            };
            (target, copied.reported, deletion)
        }
        // Recorded before GIO copies it, so a change during the copy shows.
        (Some(source_path), None) => {
            let sources = copy_engine::snapshot(source_path.clone(), &cancellable).await?;
            let left = LeftOut::default();
            let target = remote_copy(
                source.clone(),
                target,
                taken,
                cancellable.clone(),
                left.clone(),
            )
            .await?;
            left.keep_in(&sources, &source_path);
            (
                target,
                Reported::default(),
                Deletion::Sources(source, sources),
            )
        }
        _ => {
            let left = LeftOut::default();
            let target = copy_new_recursively_on_filesystem(
                source.clone(),
                target,
                taken,
                cancellable.clone(),
                progress.clone(),
                names,
                Some(left.clone()),
            )
            .await?;
            if let Some(path) = native_path(&target)
                && owed
                    .barrier
                    .as_ref()
                    .is_some_and(copy_engine::Barrier::syncs_each_file)
            {
                copy_engine::sync_files(vec![path], &cancellable).await?;
            }
            let kept = left.sources();
            (target, Reported::default(), Deletion::Source(source, kept))
        }
    };
    owed.owe(deletion);
    Ok(Transferred {
        target,
        reported,
        owed,
    })
}

#[cfg(test)]
pub(super) async fn move_local_with_progress(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
    attempt_move: MoveAttempt,
    remote_copy: RemoteCopy,
) -> Result<gio::File, glib::Error> {
    let progress = copying.progress.clone();
    let moved = move_local_owing(
        source,
        target,
        taken,
        cancellable.clone(),
        copying,
        attempt_move,
        remote_copy,
    )
    .await?;
    moved.owed.settle(progress.as_deref(), &cancellable).await?;
    Ok(moved.target)
}

pub(super) async fn delete_moved_sources(
    source: &gio::File,
    sources: copy_engine::Manifest,
    unlinks: Arc<copy_engine::Unlinks>,
    progress: Option<&TransferProgressTracker>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (kept, deleted) =
        copy_engine::delete_unchanged_sources(sources, unlinks, cancellable).await;
    if let (Some(progress), Some(parent)) =
        (progress, source.path().as_deref().and_then(Path::parent))
    {
        progress.report(TransferReport {
            kept: kept_items(parent, kept),
            ..TransferReport::default()
        });
    }
    deleted
}

pub(super) fn kept_items(parent: &Path, kept: Vec<(PathBuf, KeptCause)>) -> Vec<KeptItem> {
    kept.into_iter()
        .map(|(path, cause)| KeptItem {
            location: Location::local(parent.join(path)),
            cause,
        })
        .collect()
}

#[cfg(test)]
pub(super) async fn move_local_with(
    source: gio::File,
    target: gio::File,
    cancellable: gio::Cancellable,
    attempt_move: MoveAttempt,
) -> Result<gio::File, glib::Error> {
    move_local_with_progress(
        source,
        target,
        Taken::Fail,
        cancellable,
        Copying::default(),
        attempt_move,
        remote_copy(Copying::default()),
    )
    .await
}

/// Standalone reason for a placement failure, for a `…: %{error}` slot.
pub(super) fn placement_reason(error: PlaceError) -> String {
    match error {
        PlaceError::Exists => crate::i18n::tr("An item with that name already exists"),
        PlaceError::InvalidName => crate::i18n::tr(INVALID_NAME_REASON),
        PlaceError::IntoItself => crate::i18n::tr("Can\u{2019}t put a folder inside itself"),
        PlaceError::Changed => {
            crate::i18n::tr("The target changed while the operation was in progress")
        }
        PlaceError::Unconfirmed => crate::i18n::tr(UNCONFIRMED_REASON),
        PlaceError::CrossDevice => os_error_text(rustix::io::Errno::XDEV),
        PlaceError::Unsupported => crate::i18n::tr(UNSUPPORTED_REASON),
        PlaceError::Io(error) => os_error_text(error),
    }
}

pub(super) fn placement_error_kind(error: PlaceError) -> gio::IOErrorEnum {
    match error {
        PlaceError::Exists => gio::IOErrorEnum::Exists,
        PlaceError::InvalidName => gio::IOErrorEnum::InvalidFilename,
        _ => gio::IOErrorEnum::Failed,
    }
}

/// A placement failure as a complete message about `name`, using `template`
/// (with `%{name}` and `%{error}`) unless the failure has its own sentence.
pub(super) fn placement_failure(
    error: PlaceError,
    name: &str,
    template: fn(&str, &str) -> String,
) -> glib::Error {
    let message = match error {
        PlaceError::IntoItself => placement_reason(error),
        PlaceError::Changed => rust_i18n::t!(
            "%{name} changed while the operation was in progress",
            name = name
        )
        .into_owned(),
        error => template(
            name,
            &crate::services::error_detail(placement_reason(error)),
        ),
    };
    io_error(message)
}

pub(super) fn could_not_move(name: &str, error: &str) -> String {
    rust_i18n::t!(
        "Could not move %{name}: %{error}",
        name = name,
        error = error
    )
    .into_owned()
}

pub(super) async fn rename_native(
    source: PathBuf,
    target: PathBuf,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (Some(source_parent), Some(source_name), Some(target_parent), Some(target_name)) = (
        source.parent().map(Path::to_path_buf),
        source.file_name().map(OsStr::to_os_string),
        target.parent().map(Path::to_path_buf),
        target.file_name().map(OsStr::to_os_string),
    ) else {
        return Err(translated_io_error("Invalid move destination"));
    };
    let stopped = cancellable.clone();
    run_local_fs_step(move || {
        let source_dir = open_local_parent_directory(&source_parent)?;
        let target_dir = if target_parent == source_parent {
            None
        } else {
            Some(open_local_parent_directory(&target_parent)?)
        };
        let from = Entry::new(&source_dir, &source_name);
        let to = Entry::new(target_dir.as_ref().unwrap_or(&source_dir), &target_name);
        let Ok(renamed) = deferred_close::retry_folder_rename(
            fs(),
            from,
            &Deadline::default(),
            &|| stopped.is_cancelled(),
            |failure| {
                matches!(
                    failure,
                    RenameFailure::Place(PlaceError::Io(rustix::io::Errno::ACCESS))
                )
            },
            || placement::rename_in_place(fs(), from, to),
        ) else {
            return Ok(Err(cancelled_local_operation()));
        };
        Ok(renamed.map(drop).map_err(|failure| match failure {
            RenameFailure::Place(error) => still_open_rename(from, error)
                .map_or_else(|| rename_failure_error(failure, &source, &target), io_error),
            failure => rename_failure_error(failure, &source, &target),
        }))
    })
    .await?
}

pub(super) fn rename_failure_error(
    failure: RenameFailure,
    source: &Path,
    target: &Path,
) -> glib::Error {
    let name = |path: &Path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    match failure {
        RenameFailure::Place(PlaceError::Exists) => glib::Error::new(
            gio::IOErrorEnum::Exists,
            &rust_i18n::t!("“%{name}” already exists", name = name(target)),
        ),
        RenameFailure::Place(error) => placement_error(error),
        RenameFailure::Stranded { temporary, error } => io_error(rust_i18n::t!(
            "Could not rename “%{name}”; it was left as “%{temporary}”: %{error}",
            name = name(source),
            temporary = temporary.to_string_lossy(),
            error = crate::services::error_detail(placement_reason(error))
        )),
    }
}

pub(super) fn move_local_path(
    source_path: PathBuf,
    target_path: PathBuf,
    taken: Taken,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Moved, glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let Some(source_parent_path) = source_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error("Cannot move the filesystem root"));
        };
        let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move source"));
        };
        let Some(target_parent_path) = target_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error(
                "The move destination has no parent directory",
            ));
        };
        let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move destination"));
        };

        let display_name = source_name.to_string_lossy().into_owned();
        let stopped = cancellable.clone();
        run_local_fs_step(move || {
            let source_parent = open_local_parent_directory(&source_parent_path)?;
            let target_parent = open_local_parent_directory(&target_parent_path)?;
            let from = Entry::new(&source_parent, &source_name);
            let moved = || {
                let is_directory = match taken {
                    Taken::Fail => false,
                    Taken::Number => match fs().stat(from, false) {
                        Ok(stat) => stat.is_directory(),
                        Err(error) => return Ok(Err(PlaceError::Io(error))),
                    },
                };
                let limit = || mount_profile::name_limit(fs(), target_parent.as_fd());
                let deadline = Deadline::default();
                for name in taken.names(&target_name, is_directory, limit) {
                    let to = Entry::new(&target_parent, &name);
                    let placed = deferred_close::retry_folder_rename(
                        fs(),
                        from,
                        &deadline,
                        &|| stopped.is_cancelled(),
                        |error| matches!(error, PlaceError::Io(rustix::io::Errno::ACCESS)),
                        || placement::place(fs(), from, to, None),
                    )?;
                    if taken == Taken::Number && matches!(placed, Err(PlaceError::Exists)) {
                        continue;
                    }
                    return Ok(match placed {
                        Ok(_) => Ok(Moved::Done(gio::File::for_path(
                            target_parent_path.join(&name),
                        ))),
                        Err(PlaceError::CrossDevice | PlaceError::Unsupported) => {
                            match fs().stat(from, false) {
                                Ok(stat)
                                    if stat.is_directory()
                                        && placement::contains(fs(), stat.identity, to.dir)
                                            != Some(false) =>
                                {
                                    Err(PlaceError::IntoItself)
                                }
                                Ok(_) => Ok(Moved::Copy),
                                Err(error) => Err(PlaceError::Io(error)),
                            }
                        }
                        Err(error) => Err(error),
                    });
                }
                Ok(Err(PlaceError::Exists))
            };
            Ok(match moved() {
                Ok(moved) => moved.map_err(|error| match still_open_rename(from, error) {
                    Some(reason) => io_error(could_not_move(
                        &display_name,
                        &crate::services::error_detail(reason),
                    )),
                    None => placement_failure(error, &display_name, could_not_move),
                }),
                Err(WaitCancelled) => Err(cancelled_local_operation()),
            })
        })
        .await?
    })
}

pub(super) fn still_open_rename(from: Entry<'_>, error: PlaceError) -> Option<String> {
    match error {
        PlaceError::Io(errno) => holders::refused_rename(fs(), from, errno),
        _ => None,
    }
}

#[cfg(test)]
pub(super) async fn move_local(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
) -> Result<gio::File, glib::Error> {
    let copying = Copying {
        progress: progress.clone(),
        names,
        ..Copying::default()
    };
    move_local_with_progress(
        source,
        target,
        taken,
        cancellable,
        copying.clone(),
        attempt_move(progress),
        remote_copy(copying),
    )
    .await
}

pub(super) async fn move_owing(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
) -> Result<Transferred, glib::Error> {
    let (attempt, remote) = (
        attempt_move(copying.progress.clone()),
        remote_copy(copying.clone()),
    );
    move_local_owing(source, target, taken, cancellable, copying, attempt, remote).await
}

pub(super) fn attempt_move(progress: Option<Rc<TransferProgressTracker>>) -> MoveAttempt {
    Rc::new(move |source, target, taken, cancellable| {
        if let (Some(source_path), Some(target_path)) = (native_path(&source), native_path(&target))
        {
            return move_local_path(source_path, target_path, taken, cancellable);
        }
        // GIO would delete a native source itself, without checking what it copied.
        if source.is_native() {
            return Box::pin(async { Ok(Moved::Copy) });
        }
        let move_progress = progress.as_ref().map(|progress| {
            progress.begin_file(
                transfer_source_name(&source)
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                true,
            )
        });
        Box::pin(async move {
            let is_directory = taken == Taken::Number
                && await_cancellable(&source, &cancellable, |source, cancellable, result| {
                    source.query_info_async(
                        "standard::type",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?
                .file_type()
                    == gio::FileType::Directory;
            let flags = gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
            let moved = first_free_target(&target, (taken, is_directory), &cancellable, |target| {
                let reporter = move_progress
                    .as_ref()
                    .and_then(FileTransferProgress::reporter);
                move_file_with_progress(
                    source.clone(),
                    target,
                    flags,
                    cancellable.clone(),
                    reporter,
                )
            })
            .await;
            match moved {
                Ok((target, ())) => {
                    if let Some(move_progress) = move_progress {
                        move_progress.finish(None);
                    }
                    Ok(Moved::Done(target))
                }
                Err(error) if error.matches(gio::IOErrorEnum::WouldRecurse) => {
                    let (guard_source, guard_target) = (source.clone(), target.clone());
                    let within = run_local_fs_step(move || {
                        Ok(target_is_within_source(&guard_source, &guard_target))
                    })
                    .await?;
                    if within != Some(false) {
                        return Err(placement_error(PlaceError::IntoItself));
                    }
                    Ok(Moved::Copy)
                }
                Err(error) => Err(error),
            }
        })
    })
}

pub(super) fn remote_copy(copying: Copying) -> RemoteCopy {
    Rc::new(move |source, target, taken, cancellable, left| {
        let Copying {
            progress, names, ..
        } = copying.clone();
        Box::pin(copy_new_recursively_on_filesystem(
            source,
            target,
            taken,
            cancellable,
            progress,
            names,
            Some(left),
        ))
    })
}
