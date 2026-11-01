// SPDX-License-Identifier: MIT

use super::*;

mod tree;

pub(super) use tree::*;

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
pub(in crate::adapters::local_operations) enum CopyRecord {
    Nothing,
    Root(Rc<CreatedCopyRoot>),
}

#[cfg(test)]
pub(super) fn copy_recursively_with_progress(
    source: gio::File,
    target: gio::File,
    into: CopyInto,
    cancellable: gio::Cancellable,
    record: CopyRecord,
    progress: Option<Rc<TransferProgressTracker>>,
    names: GioNames,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    let copy = GioCopy {
        into,
        cancellable,
        record,
        progress,
        names,
    };
    copy_tree(source, target, copy)
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
    let copy = NewCopy {
        taken,
        cancellable,
        progress,
        names,
        left,
    };
    copy_new_with_ownership(source, target, copy)
        .await
        .map(|(target, _)| target)
}

/// A copy to a name nothing has yet.
pub(super) struct NewCopy {
    pub(super) taken: Taken,
    pub(super) cancellable: gio::Cancellable,
    pub(super) progress: Option<Rc<TransferProgressTracker>>,
    pub(super) names: NamePolicy,
    pub(super) left: Option<LeftOut>,
}

pub(super) async fn copy_new_with_ownership(
    source: gio::File,
    target: gio::File,
    copy: NewCopy,
) -> Result<(gio::File, Option<copy_engine::Ownership>), glib::Error> {
    if let Some((source_path, target_path)) = native_paths(&source, &target) {
        return copy_native(source_path, target_path, copy).await;
    }
    let source_type = if !target.is_native() || copy.taken == Taken::Number {
        Some(file_type(&source, &copy.cancellable).await?)
    } else {
        None
    };
    let is_directory = source_type == Some(gio::FileType::Directory);
    if !target.is_native() && !is_directory {
        return copy_remote_file(source, target, copy)
            .await
            .map(|target| (target, None));
    }
    copy_new_tree(source, target, is_directory, copy).await
}

fn native_paths(source: &gio::File, target: &gio::File) -> Option<(PathBuf, PathBuf)> {
    if !source.is_native() || !target.is_native() {
        return None;
    }
    let (Some(source_path), Some(target_path)) = (source.path(), target.path()) else {
        return None;
    };
    Some((source_path, target_path))
}

async fn copy_native(
    source_path: PathBuf,
    target_path: PathBuf,
    copy: NewCopy,
) -> Result<(gio::File, Option<copy_engine::Ownership>), glib::Error> {
    let options = copy_engine::Options {
        names: copy.names,
        ..copy_engine::Options::default()
    };
    let copied = copy_engine::copy_local(
        source_path,
        copy_engine::Destination::New(target_path.clone(), copy.taken),
        options,
        copy.progress,
        &copy.cancellable,
    )
    .await?;
    Ok((
        gio::File::for_path(target_path.with_file_name(copied.name)),
        copied.ownership,
    ))
}

async fn file_type(
    source: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::FileType, glib::Error> {
    let info = await_cancellable(source, cancellable, |source, cancellable, result| {
        source.query_info_async(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?;
    Ok(info.file_type())
}

async fn copy_remote_file(
    source: gio::File,
    target: gio::File,
    copy: NewCopy,
) -> Result<gio::File, glib::Error> {
    let NewCopy {
        taken,
        cancellable,
        progress,
        names,
        ..
    } = copy;
    copy_new_remote_file_with(
        source,
        target,
        taken,
        cancellable,
        Rc::new(move |source, stage, cancellable| {
            let copy = GioCopy {
                into: CopyInto::Existing,
                cancellable,
                record: CopyRecord::Nothing,
                progress: progress.clone(),
                names: GioNames::new(names.clone(), LeftOut::default()),
            };
            Box::pin(async move { copy_tree(source, stage, copy).await })
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
}

async fn copy_new_tree(
    source: gio::File,
    target: gio::File,
    is_directory: bool,
    copy: NewCopy,
) -> Result<(gio::File, Option<copy_engine::Ownership>), glib::Error> {
    let left_out = copy.left.clone().unwrap_or_default();
    let copied = first_free_target(
        &target,
        (copy.taken, is_directory),
        &copy.cancellable,
        |target| {
            let tree = GioCopy {
                into: CopyInto::New,
                cancellable: copy.cancellable.clone(),
                record: CopyRecord::Nothing,
                progress: copy.progress.clone(),
                names: GioNames::new(copy.names.clone(), left_out.clone()),
            };
            copy_new_root(source.clone(), target, tree, copy.taken)
        },
    )
    .await?;
    if copy.left.is_none() {
        left_out.report(copy.progress.as_deref());
    }
    Ok((copied.0, None))
}

async fn copy_new_root(
    source: gio::File,
    target: gio::File,
    mut tree: GioCopy,
    taken: Taken,
) -> Result<(), glib::Error> {
    let created_root = Rc::new(CreatedCopyRoot::new());
    tree.record = CopyRecord::Root(created_root.clone());
    let Err(error) = copy_tree(source, target.clone(), tree).await else {
        return Ok(());
    };
    if !created_root.was_created.get() {
        return Err(error);
    }
    if was_cancelled(&error) {
        let cleanup_result = permanently_delete_maybe_local_if_unchanged(
            target,
            true,
            created_root.identity.get(),
            gio::Cancellable::new(),
        )
        .await;
        return Err(copy_failure_after_cleanup(error, cleanup_result));
    }
    // Taken inside a folder this copy made, where no other
    // name for the folder helps.
    if taken == Taken::Number && error.matches(gio::IOErrorEnum::Exists) {
        return Err(io_error(crate::services::gio_error_message(&error)));
    }
    Err(error)
}
