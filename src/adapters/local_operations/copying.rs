// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct CreatedCopyRoot {
    pub(super) was_created: Cell<bool>,
    pub(super) identity: Cell<Option<Recorded>>,
}

impl CreatedCopyRoot {
    pub(super) fn new() -> Self {
        Self {
            was_created: Cell::new(false),
            identity: Cell::new(None),
        }
    }
}

/// The folder is made without the copy's cancellable: GIO and GVfs can report
/// a cancel after making it, which would leave it behind.
pub(super) async fn make_copy_folder(
    target: &gio::File,
    cancellable: &gio::Cancellable,
    created_root: &Option<Rc<CreatedCopyRoot>>,
) -> Result<(), glib::Error> {
    cancellable.set_error_if_cancelled()?;
    let folder = target.clone();
    run_blocking(move || folder.make_directory(gio::Cancellable::NONE))
        .await
        .map_err(|error| {
            if cancellable.is_cancelled() {
                cancelled_local_operation()
            } else {
                error
            }
        })?;
    if let Some(created_root) = created_root {
        created_root.was_created.set(true);
        created_root
            .identity
            .set(local_file_identity(target).await?);
    }
    Ok(())
}

pub(super) type RemoteFileStageCopy = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

pub(super) type RemoteFileStageCommit = Rc<
    dyn Fn(
        gio::File,
        gio::File,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>,
>;

pub(super) async fn discard_incomplete_copy(stage: gio::File) -> Result<(), glib::Error> {
    match permanently_delete(stage, false, gio::Cancellable::new()).await {
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(()),
        result => result,
    }
}

/// Reserves an unpredictable sibling so a partial remote file is never exposed at the final path.
pub(super) async fn create_remote_file_stage(
    target: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::File, glib::Error> {
    let parent = target
        .parent()
        .ok_or_else(|| translated_io_error("The destination has no parent directory"))?;
    let stage = parent.child(HiddenName::Copy.fresh());
    let stream = match await_cancellable(&stage, cancellable, |stage, cancellable, result| {
        stage.create_async(
            gio::FileCreateFlags::PRIVATE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    {
        Ok(stream) => stream,
        Err(error) if was_cancelled(&error) => {
            let cleanup = discard_incomplete_copy(stage).await;
            return Err(copy_failure_after_cleanup(error, cleanup));
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = await_cancellable(&stream, cancellable, |stream, cancellable, result| {
        stream.close_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
    {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    Ok(stage)
}

pub(super) fn copy_failure_after_cleanup(
    copy_error: glib::Error,
    cleanup_result: Result<(), glib::Error>,
) -> glib::Error {
    match cleanup_result {
        Ok(()) => copy_error,
        Err(cleanup_error) => io_error(rust_i18n::t!(
            "%{error}; the incomplete copy could not be removed: %{cleanup_error}",
            error = crate::services::gio_error_message(&copy_error),
            cleanup_error = crate::services::gio_error_detail(&cleanup_error)
        )),
    }
}

pub(super) async fn copy_new_remote_file_with(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    copy_to_stage: RemoteFileStageCopy,
    commit_stage: RemoteFileStageCommit,
) -> Result<gio::File, glib::Error> {
    let stage = create_remote_file_stage(&target, &cancellable).await?;
    if let Err(error) = copy_to_stage(source, stage.clone(), cancellable.clone()).await {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    if let Err(error) = cancellable.set_error_if_cancelled() {
        let cleanup = discard_incomplete_copy(stage).await;
        return Err(copy_failure_after_cleanup(error, cleanup));
    }
    let committed = first_free_target(&target, (taken, false), &cancellable, |target| {
        commit_stage(stage.clone(), target, cancellable.clone())
    })
    .await;
    match committed {
        Ok((target, ())) => Ok(target),
        Err(error) => {
            let cleanup = discard_incomplete_copy(stage).await;
            Err(copy_failure_after_cleanup(error, cleanup))
        }
    }
}

#[derive(Clone)]
pub(super) enum CopyRecord {
    Nothing,
    Root(Rc<CreatedCopyRoot>),
}

pub(super) fn copy_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    into: CopyInto,
    cancellable: gio::Cancellable,
    record: CopyRecord,
    progress: Option<Rc<TransferProgressTracker>>,
    names: GioNames,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type,standard::size,unix::device",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() == gio::FileType::Directory {
            let existing = into.existing_folder(&target, &cancellable).await;
            if existing.is_none() {
                let created_root = match &record {
                    CopyRecord::Root(root) => Some(root.clone()),
                    _ => None,
                };
                make_copy_folder(&target, &cancellable, &created_root).await?;
            }
            let source_folder = (&source, device(&source, &info));
            let folder = names
                .of(source_folder, &target, existing, &cancellable)
                .await?;
            let record = match record {
                CopyRecord::Root(_) => CopyRecord::Nothing,
                record => record,
            };
            let enumerator = gio_close::enumerate_children(
                &source,
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(&cancellable),
            )
            .await?;
            let names = names.within(folder);
            let mut renamed = Vec::new();
            let mut listed = NamedChildren::new(&enumerator, folder.names);
            let copied = async {
                loop {
                    let children = listed.next(&cancellable).await?;
                    if children.is_empty() {
                        return Ok::<_, glib::Error>(());
                    }
                    for (name, kind, planned) in children {
                        let link = kind == gio::FileType::SymbolicLink;
                        if link && (!folder.names.links || names.left.refused.get()) {
                            names.left.add(source.child(&name), target.child(&name));
                            continue;
                        }
                        let copied = copy_recursively_with_progress(
                            source.child(&name),
                            target.child(&planned.target),
                            into,
                            cancellable.clone(),
                            record.clone(),
                            progress.clone(),
                            names.clone(),
                        )
                        .await;
                        if let Err(error) = &copied
                            && link
                            && unheld(error).is_some()
                        {
                            names.left.refused.set(true);
                            names.left.add(source.child(&name), target.child(&name));
                            continue;
                        }
                        let saved_as = target.child(&planned.target);
                        if let Some(cause) = planned.cause
                            && (copied.is_ok() || query_exists(&saved_as, &cancellable).await)
                        {
                            renamed.extend(renamed_item(&target.child(&name), &saved_as, cause));
                        }
                        copied?;
                    }
                }
            }
            .await;
            if let Some(progress) = &progress {
                progress.report(TransferReport {
                    renamed,
                    ..TransferReport::default()
                });
            }
            copied
        } else {
            let source_size = info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
                .then(|| info.size().max(0) as u64);
            let flags = into.file_flags() | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
            let track_bytes = source_size.is_none_or(|size| size >= BYTE_PROGRESS_MIN_FILE_SIZE);
            let file_progress = progress.as_ref().map(|progress| {
                progress.begin_file(
                    transfer_source_name(&source)
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    track_bytes,
                )
            });
            let reporter = file_progress
                .as_ref()
                .and_then(FileTransferProgress::reporter);
            let link = (info.file_type() == gio::FileType::SymbolicLink).then(|| source.clone());
            let result = copy_file_with_progress(
                source,
                target.clone(),
                flags,
                cancellable.clone(),
                reporter,
            )
            .await
            .map_err(|error| match link {
                // SMB and MTP backends refuse links.
                Some(link) if error.matches(gio::IOErrorEnum::NotSupported) => {
                    let message = rust_i18n::t!(
                        "Could not copy %{name}: %{error}",
                        name = transfer_source_name(&link)
                            .unwrap_or_default()
                            .to_string_lossy(),
                        error = crate::services::gio_error_detail(&error)
                    );
                    glib::Error::new(Unheld::Symlink, &message)
                }
                _ => error,
            });
            if result.is_ok()
                && let Some(file_progress) = file_progress
            {
                file_progress.finish(source_size);
            }
            result
        }
    })
}

#[cfg(test)]
pub(super) fn copy_recursively(
    source: gio::File,
    target: gio::File,
    merge: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if merge {
            return merge_copy(None, NamePolicy::default())(
                source,
                target,
                MergeCopy::default(),
                cancellable,
            )
            .await
            .map(drop);
        }
        copy_new_recursively_on_filesystem(
            source,
            target,
            Taken::Fail,
            cancellable,
            None,
            NamePolicy::default(),
            None,
        )
        .await
        .map(drop)
    })
}

pub(super) async fn copy_new_recursively_on_filesystem(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
    left: Option<LeftOut>,
) -> Result<gio::File, glib::Error> {
    copy_new_with_ownership(source, target, taken, cancellable, progress, names, left)
        .await
        .map(|(target, _)| target)
}

pub(super) async fn copy_new_with_ownership(
    source: gio::File,
    target: gio::File,
    taken: Taken,
    cancellable: gio::Cancellable,
    progress: Option<Rc<TransferProgressTracker>>,
    names: NamePolicy,
    left: Option<LeftOut>,
) -> Result<(gio::File, Option<copy_engine::Ownership>), glib::Error> {
    if source.is_native()
        && target.is_native()
        && let (Some(source_path), Some(target_path)) = (source.path(), target.path())
    {
        let options = copy_engine::Options {
            names,
            ..copy_engine::Options::default()
        };
        let copied = copy_engine::copy_local(
            source_path,
            copy_engine::Destination::New(target_path.clone(), taken),
            options,
            progress,
            &cancellable,
        )
        .await?;
        return Ok((
            gio::File::for_path(target_path.with_file_name(copied.name)),
            copied.ownership,
        ));
    }

    let source_type = if !target.is_native() || taken == Taken::Number {
        let info = await_cancellable(&source, &cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        Some(info.file_type())
    } else {
        None
    };
    let is_directory = source_type == Some(gio::FileType::Directory);
    if !target.is_native() && !is_directory {
        let copy_progress = progress.clone();
        let copy_names = names.clone();
        return copy_new_remote_file_with(
            source,
            target,
            taken,
            cancellable,
            Rc::new(move |source, stage, cancellable| {
                let progress = copy_progress.clone();
                let names = GioNames::new(copy_names.clone(), LeftOut::default());
                Box::pin(async move {
                    copy_recursively_with_progress(
                        source,
                        stage,
                        CopyInto::Existing,
                        cancellable,
                        CopyRecord::Nothing,
                        progress,
                        names,
                    )
                    .await
                })
            }),
            Rc::new(|stage, target, cancellable| {
                Box::pin(async move {
                    await_cancellable(&stage, &cancellable, move |stage, cancellable, result| {
                        stage.move_async(
                            &target,
                            gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            None,
                            move |output| result.resolve(output),
                        );
                    })
                    .await
                })
            }),
        )
        .await
        .map(|target| (target, None));
    }

    let left_out = left.clone().unwrap_or_default();
    let copied = first_free_target(&target, (taken, is_directory), &cancellable, |target| {
        let (source, cancellable, progress) =
            (source.clone(), cancellable.clone(), progress.clone());
        let names = GioNames::new(names.clone(), left_out.clone());
        async move {
            let created_root = Rc::new(CreatedCopyRoot::new());
            let result = copy_recursively_with_progress(
                source,
                target.clone(),
                CopyInto::New,
                cancellable.clone(),
                CopyRecord::Root(created_root.clone()),
                progress,
                names,
            )
            .await;
            match result {
                Err(error) if was_cancelled(&error) && created_root.was_created.get() => {
                    let cleanup_result = permanently_delete_maybe_local_if_unchanged(
                        target,
                        true,
                        created_root.identity.get(),
                        gio::Cancellable::new(),
                    )
                    .await;
                    Err(copy_failure_after_cleanup(error, cleanup_result))
                }
                // Taken inside a folder this copy made, where no other
                // name for the folder helps.
                Err(error)
                    if taken == Taken::Number
                        && error.matches(gio::IOErrorEnum::Exists)
                        && created_root.was_created.get() =>
                {
                    Err(io_error(crate::services::gio_error_message(&error)))
                }
                result => result,
            }
        }
    })
    .await?;
    if left.is_none() {
        left_out.report(progress.as_deref());
    }
    Ok((copied.0, None))
}
