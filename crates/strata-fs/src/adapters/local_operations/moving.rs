// SPDX-License-Identifier: MIT

use super::*;

mod errors;
mod native;

pub(super) use errors::*;
pub(super) use native::*;

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

/// A move of one item, and how it is tried before it falls back to a copy.
pub(super) struct Moving {
    pub(super) taken: Taken,
    pub(super) cancellable: gio::Cancellable,
    pub(super) copying: Copying,
    pub(super) attempt_move: MoveAttempt,
    pub(super) remote_copy: RemoteCopy,
}

impl Moving {
    pub(super) fn new(taken: Taken, cancellable: gio::Cancellable, copying: Copying) -> Self {
        Self {
            attempt_move: attempt_move(copying.progress.clone()),
            remote_copy: remote_copy(copying.clone()),
            taken,
            cancellable,
            copying,
        }
    }
}

pub(super) async fn move_owing(
    source: gio::File,
    target: gio::File,
    moving: Moving,
) -> Result<Transferred, glib::Error> {
    let Moving {
        taken,
        cancellable,
        copying,
        attempt_move,
        remote_copy,
    } = moving;
    if let Moved::Done(target) =
        attempt_move(source.clone(), target.clone(), taken, cancellable.clone()).await?
    {
        return Ok(Transferred {
            target,
            reported: Reported::default(),
            owed: Owed::default(),
        });
    }
    let mut owed = Owed::default();
    if let Some(target_path) = native_path(&target) {
        owed.barrier = Some(copy_engine::barrier(split_local_path(&target_path)?.0).await?);
    }
    let copy = MoveCopy {
        taken,
        cancellable,
        copying,
        remote_copy,
    };
    let (target, reported, deletion) = match (native_path(&source), native_path(&target)) {
        (Some(source_path), Some(target_path)) => {
            copy.local(source, source_path, target_path).await?
        }
        (Some(source_path), None) => copy.onto_remote(source, source_path, target).await?,
        _ => copy.through_gio(source, target, &owed).await?,
    };
    owed.owe(deletion);
    Ok(Transferred {
        target,
        reported,
        owed,
    })
}

/// The copy a move falls back to, and what deleting its source then owes.
struct MoveCopy {
    taken: Taken,
    cancellable: gio::Cancellable,
    copying: Copying,
    remote_copy: RemoteCopy,
}

type MoveCopied = (gio::File, Reported, Deletion);

impl MoveCopy {
    async fn local(
        self,
        source: gio::File,
        source_path: PathBuf,
        target_path: PathBuf,
    ) -> Result<MoveCopied, glib::Error> {
        let options = copy_engine::Options {
            names: self.copying.names,
            manifest: true,
            durable: true,
            ..copy_engine::Options::default()
        };
        let copied = copy_engine::copy_local(
            source_path,
            copy_engine::Destination::New(target_path.clone(), self.taken),
            options,
            self.copying.progress.clone(),
            &self.cancellable,
        )
        .await?;
        let target = gio::File::for_path(target_path.with_file_name(copied.name));
        let deletion = match copied.manifest {
            Some(sources) => Deletion::Sources(source, sources),
            None => Deletion::Source(source, Vec::new()),
        };
        Ok((target, copied.reported, deletion))
    }

    /// Recorded before GIO copies it, so a change during the copy shows.
    async fn onto_remote(
        self,
        source: gio::File,
        source_path: PathBuf,
        target: gio::File,
    ) -> Result<MoveCopied, glib::Error> {
        let sources = copy_engine::snapshot(source_path.clone(), &self.cancellable).await?;
        let left = LeftOut::default();
        let target = (self.remote_copy)(
            source.clone(),
            target,
            self.taken,
            self.cancellable.clone(),
            left.clone(),
        )
        .await?;
        left.keep_in(&sources, &source_path);
        Ok((
            target,
            Reported::default(),
            Deletion::Sources(source, sources),
        ))
    }

    async fn through_gio(
        self,
        source: gio::File,
        target: gio::File,
        owed: &Owed,
    ) -> Result<MoveCopied, glib::Error> {
        let left = LeftOut::default();
        let target = copy_new_recursively_on_filesystem(
            source.clone(),
            target,
            self.taken,
            self.cancellable.clone(),
            self.copying.progress.clone(),
            self.copying.names,
            Some(left.clone()),
        )
        .await?;
        let syncs_each_file = owed
            .barrier
            .as_ref()
            .is_some_and(copy_engine::Barrier::syncs_each_file);
        if let Some(path) = native_path(&target).filter(|_| syncs_each_file) {
            copy_engine::sync_files(vec![path], &self.cancellable).await?;
        }
        let kept = left.sources();
        Ok((target, Reported::default(), Deletion::Source(source, kept)))
    }
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
    let moving = Moving {
        taken,
        cancellable: cancellable.clone(),
        copying,
        attempt_move,
        remote_copy,
    };
    let moved = move_owing(source, target, moving).await?;
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
