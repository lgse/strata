// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) struct Copied {
    pub ownership: Option<Ownership>,
    pub name: OsString,
    pub reported: Reported,
    pub manifest: Option<Manifest>,
    /// Barriers of the mounts nested in a durably merged folder.
    pub nested: Vec<Barrier>,
}

#[derive(Clone, Default)]
pub(in crate::adapters::local_operations) struct Options {
    pub names: NamePolicy,
    pub workers: Option<usize>,
    pub manifest: bool,
    /// What only the copy will hold is deleted once its [`Barrier`] is reached,
    /// so files that barrier cannot reach are synced as written.
    pub durable: bool,
}

pub(in crate::adapters::local_operations) async fn copy_local(
    source: PathBuf,
    destination: Destination,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
) -> Result<Copied, glib::Error> {
    let (Destination::New(target, _) | Destination::Merge(target, _)) = &destination;
    let target = target.clone();
    let merging = matches!(destination, Destination::Merge(..));
    let workers = options.workers;
    let followed = following(
        Item::Path(source),
        options,
        progress.clone(),
        cancellable,
        move |copy, source| match destination {
            Destination::New(target, taken) => {
                let (parent, name) = split(&target).map_err(Failure::Error)?;
                let new = NewItem::Published(taken);
                let (name, ownership) = copy_new(copy, source, (&parent, &name), new, workers)?;
                let published = Some(Published::proven(parent, name.clone(), ownership));
                Ok((name, Vec::new(), published))
            }
            Destination::Merge(folder, placing) => {
                let name = folder.file_name().unwrap_or_default().to_os_string();
                merge(copy, source, &folder, workers, placing)
                    .map(|(nested, published)| (name, nested, published))
            }
        },
    )
    .await?;
    match followed.value {
        Ok((name, nested, published)) => {
            report(
                progress.as_deref(),
                &target.with_file_name(&name),
                followed.left,
            );
            let ownership = published.as_ref().map(|copy| copy.ownership().clone());
            let mut manifest = followed.manifest;
            if let (Some(manifest), Some(published)) = (&mut manifest, published) {
                manifest.copied_to(published);
            }
            Ok(Copied {
                ownership,
                name,
                reported: followed.reported,
                manifest,
                nested,
            })
        }
        // What a stopped merge placed stays.
        Err(error) => {
            if merging {
                report(progress.as_deref(), &target, followed.left);
            }
            Err(error)
        }
    }
}

pub(in crate::adapters::local_operations) async fn copy_to_stage(
    source: PathBuf,
    target: PathBuf,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
) -> Result<(StagedSibling, Option<Manifest>, TransferReport), glib::Error> {
    let root = target.clone();
    let workers = options.workers;
    let followed = following(
        Item::Path(source),
        options,
        progress.clone(),
        cancellable,
        move |copy, source| {
            let staged = copy_stage(copy, source, &target, workers)?;
            match copy.check_cancelled() {
                Ok(()) => Ok(staged),
                Err(failure) => Err(copy.discarding(staged, failure)),
            }
        },
    )
    .await?;
    let staged = followed.value?;
    Ok((staged, followed.manifest, left_report(&root, followed.left)))
}

/// For a mount that can place neither the item nor a stage. A failed copy
/// removes only what it made.
pub(in crate::adapters::local_operations) async fn copy_in_place(
    (source, name, expected): (Arc<OwnedFd>, OsString, LocalFileIdentity),
    (parent, target): (Arc<OwnedFd>, PathBuf),
    options: Options,
    cancellable: &gio::Cancellable,
) -> Result<(Manifest, TransferReport), glib::Error> {
    let Some(target_name) = target.file_name().map(OsStr::to_os_string) else {
        return Err(io_error(crate::i18n::tr("Invalid local filesystem target")));
    };
    let options = Options {
        manifest: true,
        ..options
    };
    let workers = options.workers;
    let copied_into = OffMain::new(parent.clone());
    let followed = following(
        Item::Pinned(source, name),
        options,
        None,
        cancellable,
        move |copy, source| {
            let new = NewItem::InPlace(expected);
            copy_new(copy, source, (&parent, &target_name), new, workers)
        },
    )
    .await?;
    let (name, ownership) = followed.value?;
    let mut manifest = followed
        .manifest
        .ok_or_else(|| translated_io_error("The copy stopped unexpectedly"))?;
    manifest.copied_to(Published::proven(copied_into.into_inner(), name, ownership));
    Ok((manifest, left_report(&target, followed.left)))
}

pub(in crate::adapters::local_operations) async fn snapshot(
    source: PathBuf,
    cancellable: &gio::Cancellable,
) -> Result<Manifest, glib::Error> {
    let recorded = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&source)?;
        manifest::snapshot(fs(), parent, &name, &cancelled)
            .map_err(|failure| failure.into_error(&name))
    })
    .await;
    recorded.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

pub(in crate::adapters::local_operations) async fn delete_unchanged_sources(
    manifest: Manifest,
    unlinks: Arc<Unlinks>,
    cancellable: &gio::Cancellable,
) -> (Vec<(PathBuf, KeptCause)>, Result<(), glib::Error>) {
    let deleted = run_cancellable(cancellable, move |cancelled| {
        manifest::delete(fs(), manifest, unlinks, cancelled)
    })
    .await;
    deleted.unwrap_or_else(|_| {
        (
            Vec::new(),
            Err(translated_io_error("The deletion stopped unexpectedly")),
        )
    })
}

pub(in crate::adapters::local_operations) async fn sync_files(
    paths: Vec<PathBuf>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let synced = run_cancellable(cancellable, move |cancelled| {
        for path in paths {
            let (parent, name) = split(&path)?;
            sync_tree(fs(), Entry::new(&*parent, &name), &cancelled)
                .map_err(|failure| failure.into_error(&name))?;
        }
        Ok(())
    })
    .await;
    synced.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

pub(super) fn sync_tree(
    fs: &dyn FsOps,
    at: Entry<'_>,
    cancelled: &AtomicBool,
) -> Result<(), Failure> {
    if cancelled.load(Ordering::Acquire) {
        return Err(Failure::Cancelled);
    }
    match fs.stat(at, false)?.kind {
        FileType::Directory => {
            let dir = fs.open_dir(at)?;
            for (name, _) in fs.list(dir.as_fd())? {
                sync_tree(fs, Entry::new(&dir, &name), cancelled)
                    .map_err(|failure| failure.named(&name))?;
            }
        }
        FileType::RegularFile => {
            // An SMB server flushes only for a handle that may write.
            let file = match fs.open_write(at) {
                Err(Errno::ACCESS | Errno::PERM) => fs.open_source(at, true)?.map(|(file, _)| file),
                opened => Some(opened?),
            };
            if let Some(file) = file {
                fs.sync_data(file.as_fd())?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(in crate::adapters::local_operations) fn file_ownership(stat: &EntryStat) -> Ownership {
    Ownership::of(stat)
}

pub(in crate::adapters::local_operations) fn staged_ownership(staged: &StagedSibling) -> Ownership {
    staged
        .ownership
        .clone()
        .unwrap_or_else(|| Ownership::new(staged.identity))
}

pub(in crate::adapters::local_operations) fn published(
    parent: Arc<OwnedFd>,
    name: OsString,
    ownership: Ownership,
) -> Published {
    Published::proven(parent, name, ownership)
}

pub(in crate::adapters::local_operations) async fn placed_proven(
    path: PathBuf,
    ownership: Ownership,
    left: Arc<AtomicUsize>,
    cancellable: &gio::Cancellable,
) -> Option<PlacedSnapshot> {
    run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&path).ok()?;
        manifest::placed_proven(fs(), (parent.as_fd(), &name), &cancelled, &left, &ownership)
            .ok()
            .map(PlacedSnapshot::new)
    })
    .await
    .ok()
    .flatten()
}

pub(in crate::adapters::local_operations) fn placed_at_proven(
    fs: &dyn FsOps,
    at: (BorrowedFd<'_>, &OsStr),
    ownership: &Ownership,
    cancelled: &AtomicBool,
) -> Option<PlacedSnapshot> {
    manifest::placed_proven(fs, at, cancelled, &manifest::placed_entries(), ownership)
        .ok()
        .map(PlacedSnapshot::new)
}

/// Must be opened before a copy writes beneath `path`.
pub(in crate::adapters::local_operations) async fn barrier(
    path: PathBuf,
) -> Result<Barrier, glib::Error> {
    run_blocking(move || Barrier::open(fs(), &path)).await
}

pub(in crate::adapters::local_operations) async fn reach(
    barrier: Barrier,
    removable: bool,
) -> Result<(), String> {
    match gio::spawn_blocking(move || barrier.reach(fs(), removable)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(os_error_text(error)),
        Err(error) => Err(blocking_join_message(error)),
    }
}
