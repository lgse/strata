// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn copy_failure_on_fat32(
    error: &glib::Error,
    fat32_destination: bool,
) -> (Option<String>, bool) {
    if was_cancelled(error) {
        return (None, false);
    }
    if !fat32_destination {
        return (Some(crate::services::gio_error_message(error)), false);
    }
    let description = if error.message().contains("File too large") {
        crate::i18n::tr(
            "A file is too large for this FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
        )
    } else {
        crate::services::gio_error_message(error)
    };
    let details = rust_i18n::t!(
        "Earlier completed copies may still be writing; wait for safe eject before unplugging. Details: %{error}",
        error = crate::services::gio_error_detail(error)
    );
    (
        Some(crate::i18n::join_sentences(&description, &details)),
        true,
    )
}

/// Size probes never fail with NoSpace on their own, so the code marks this refusal.
pub(super) fn fat32_file_too_large(name: &str) -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NoSpace,
        &rust_i18n::t!(
            "%{name} is too large for a FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
            name = name
        ),
    )
}

#[derive(Clone)]
pub(super) struct TransferEstimate {
    pub(super) bytes: Option<u64>,
    pub(super) files: usize,
    pub(super) kind: gio::FileType,
    pub(super) filesystem: Option<glib::GString>,
}

pub(super) fn transfer_size(
    file: gio::File,
    cancellable: gio::Cancellable,
    max_file_size: Option<u64>,
) -> Pin<Box<dyn Future<Output = Result<TransferEstimate, glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type,standard::size,id::filesystem",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let filesystem = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM);
        if info.file_type() != gio::FileType::Directory {
            let size = info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
                .then(|| info.size().max(0) as u64);
            if info.file_type() == gio::FileType::Regular
                && let (Some(limit), Some(size)) = (max_file_size, size)
                && size > limit
            {
                return Err(fat32_file_too_large(
                    &transfer_source_name(&file)
                        .unwrap_or_default()
                        .to_string_lossy(),
                ));
            }
            return Ok(TransferEstimate {
                bytes: size,
                files: 1,
                kind: info.file_type(),
                filesystem,
            });
        }

        let enumerator = gio_close::enumerate_children(
            &file,
            "standard::name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(&cancellable),
        )
        .await?;
        let mut total = Some(0_u64);
        let mut files = 0_usize;
        loop {
            let children = await_cancellable(
                &*enumerator,
                &cancellable,
                |enumerator, cancellable, result| {
                    enumerator.next_files_async(
                        64,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                },
            )
            .await?;
            if children.is_empty() {
                return Ok(TransferEstimate {
                    bytes: total,
                    files,
                    kind: gio::FileType::Directory,
                    filesystem,
                });
            }
            for chunk in children.chunks(MAX_LOCAL_IO_WORKERS) {
                let context = glib::MainContext::default();
                let tasks = chunk
                    .iter()
                    .map(|child| {
                        context.spawn_local(transfer_size(
                            file.child(child.name()),
                            cancellable.clone(),
                            max_file_size,
                        ))
                    })
                    .collect::<Vec<_>>();
                for task in tasks {
                    let child_size = task
                        .await
                        .map_err(|_| io_error("Transfer-size worker failed"))??;
                    total = total.and_then(|total| {
                        child_size.bytes.and_then(|size| total.checked_add(size))
                    });
                    files = files.saturating_add(child_size.files);
                }
            }
        }
    })
}

#[derive(Debug)]
pub(super) struct TransferSizes {
    pub(super) sizes: Vec<Option<u64>>,
    pub(super) total: Option<u64>,
    pub(super) files: Vec<Option<usize>>,
    pub(super) total_files: Option<usize>,
    pub(super) kinds: Vec<Option<gio::FileType>>,
    pub(super) filesystems: Vec<Option<glib::GString>>,
}

pub(super) async fn transfer_sizes(
    files: &[gio::File],
    cancellable: &gio::Cancellable,
    max_file_size: Option<u64>,
) -> Result<TransferSizes, glib::Error> {
    let mut sizes = Vec::with_capacity(files.len());
    let mut counts = Vec::with_capacity(files.len());
    let mut total = Some(0_u64);
    let mut total_files = Some(0_usize);
    let mut kinds = Vec::with_capacity(files.len());
    let mut filesystems = Vec::with_capacity(files.len());
    for chunk in files.chunks(MAX_LOCAL_IO_WORKERS) {
        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .map(|file| {
                context.spawn_local(transfer_size(
                    file.clone(),
                    cancellable.clone(),
                    max_file_size,
                ))
            })
            .collect::<Vec<_>>();
        for task in tasks {
            let size = match task.await {
                Ok(Ok(size)) => Some(size),
                Ok(Err(error))
                    if was_cancelled(&error) || error.matches(gio::IOErrorEnum::NoSpace) =>
                {
                    return Err(error);
                }
                Ok(Err(_)) | Err(_) => None,
            };
            total = total.and_then(|total| {
                size.as_ref()
                    .and_then(|size| size.bytes.and_then(|bytes| total.checked_add(bytes)))
            });
            total_files = total_files
                .and_then(|total| size.as_ref().map(|size| total.saturating_add(size.files)));
            counts.push(size.as_ref().map(|size| size.files));
            sizes.push(size.as_ref().and_then(|size| size.bytes));
            kinds.push(size.as_ref().map(|size| size.kind));
            filesystems.push(size.and_then(|size| size.filesystem));
        }
    }
    Ok(TransferSizes {
        sizes,
        total,
        files: counts,
        total_files,
        kinds,
        filesystems,
    })
}

pub(super) fn validated_child(parent: &gio::File, name: &str) -> Result<gio::File, &'static str> {
    validate_basename(name)?;
    Ok(parent.child(name))
}

pub(super) fn transfer_is_noop(
    source: &gio::File,
    destination: &gio::File,
    target: &gio::File,
) -> bool {
    source.equal(target)
        || source.equal(destination)
        || destination.has_prefix(source)
        || target_is_within_source(source, target) == Some(true)
}

/// Compares device/inode identities so symlink, bind-mount, and
/// case-insensitive aliases of the source are caught. A symlink source keeps
/// its own identity, so moving a link into the folder it targets is allowed.
/// `None` means an ancestor could not be inspected.
pub(super) fn target_is_within_source(source: &gio::File, target: &gio::File) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;

    fs_ops::assert_off_main_thread();
    let (Some(source_path), Some(target_path)) = (source.path(), target.path()) else {
        return Some(false);
    };
    let identity = |metadata: &std::fs::Metadata| (metadata.dev(), metadata.ino());
    let source_identity = identity(&std::fs::symlink_metadata(source_path).ok()?);
    if std::fs::symlink_metadata(&target_path)
        .is_ok_and(|metadata| identity(&metadata) == source_identity)
    {
        return Some(true);
    }
    for ancestor in target_path.ancestors().skip(1) {
        match std::fs::metadata(ancestor) {
            Ok(metadata) if identity(&metadata) == source_identity => return Some(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    Some(false)
}

pub(crate) fn transfer_source_name(source: &gio::File) -> Option<OsString> {
    crate::model::transfer_file_name(source)
}

/// Items a copy pastes into their own folder. Each is numbered as it is copied, so it takes no
/// name in the plan and another item of its name meets the original.
pub(super) fn duplicates(
    sources: &[gio::File],
    destination: &gio::File,
    moving: bool,
) -> Vec<bool> {
    sources
        .iter()
        .map(|source| {
            !moving
                && source
                    .parent()
                    .is_some_and(|parent| parent.equal(destination))
        })
        .collect()
}

/// Items whose names the plan gives are copied first, then duplicates, then the conflicts the
/// prompt resolved, so an item numbered as it is copied never takes a planned name and a duplicate
/// is copied before Replace can remove its original.
pub(super) fn in_copy_order(mut request: PasteRequest, destination: &gio::File) -> PasteRequest {
    let sources = request
        .items
        .iter()
        .map(|item| gio_file_for_location(&item.source))
        .collect::<Vec<_>>();
    let duplicate = duplicates(&sources, destination, request.move_sources);
    let mut ranked = std::mem::take(&mut request.items)
        .into_iter()
        .zip(duplicate)
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(item, duplicate)| match (*duplicate, item.conflict) {
        (false, TransferConflict::FailIfExists) => 0,
        (true, _) => 1,
        (false, _) => 2,
    });
    request.items = ranked.into_iter().map(|(item, _)| item).collect();
    request
}

/// A `renamable` item is numbered where an earlier item has its name, and gets the destination's
/// rules where it is `crossing` too. A `duplicate` keeps its name.
pub(super) fn paste_targets(
    names: &[Option<OsString>],
    rules: &NameProfile,
    (crossing, renamable, duplicate): (&[bool], &[bool], &[bool]),
    kinds: &[Option<gio::FileType>],
) -> Result<Vec<Option<Planned>>, PlaceError> {
    let entries = (0..names.len())
        .filter(|index| !duplicate[*index])
        .filter_map(|index| {
            Some(PlanEntry {
                name: names[index].as_deref()?,
                is_directory: kinds[index] == Some(gio::FileType::Directory),
                fixing: crossing[index] && renamable[index],
                repeatable: renamable[index],
            })
        })
        .collect::<Vec<_>>();
    let mut planned = plan_names(&entries, rules, Key::Folded)?.into_iter();
    Ok(names
        .iter()
        .zip(duplicate)
        .map(|(name, duplicate)| match (name, duplicate) {
            (Some(name), true) => Some(Planned {
                target: name.clone(),
                cause: None,
            }),
            (Some(_), false) => planned.next(),
            (None, _) => None,
        })
        .collect())
}

pub(super) fn planned_paste(
    items: &[PasteItem],
    sources: &[gio::File],
    rules: &NameProfile,
    (crossing, duplicate): (&[bool], &[bool]),
    kinds: &[Option<gio::FileType>],
) -> Result<Vec<Option<Planned>>, PlaceError> {
    let own = sources.iter().map(transfer_source_name).collect::<Vec<_>>();
    let names = items
        .iter()
        .zip(&own)
        .map(|(item, own)| match &item.target_name {
            Some(shown) => plain_name(&shown.name).then(|| shown.name.clone()),
            None => own.clone(),
        })
        .collect::<Vec<_>>();
    // Replace and Merge act on the name the user saw, which is the item's
    // own where no prompt showed another.
    let renamable = items
        .iter()
        .map(|item| {
            item.target_name.is_none()
                && !matches!(
                    item.conflict,
                    TransferConflict::ReplaceExisting | TransferConflict::Merge
                )
        })
        .collect::<Vec<_>>();
    let mut planned = paste_targets(&names, rules, (crossing, &renamable, duplicate), kinds)?;
    for (index, item) in items.iter().enumerate() {
        let (Some(shown), Some(planned), Some(name)) =
            (&item.target_name, &mut planned[index], &own[index])
        else {
            continue;
        };
        let is_directory = kinds[index] == Some(gio::FileType::Directory);
        planned.cause = (*name != shown.name).then(|| {
            shown
                .cause
                .or_else(|| fix_name(name, rules, is_directory).cause)
                .unwrap_or(RenameCause::Alias)
        });
    }
    Ok(planned)
}

pub(super) fn plain_name(name: &OsStr) -> bool {
    !matches!(name.as_bytes(), b"" | b"." | b"..")
        && !name
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'/' | b'\0'))
}

#[derive(Clone)]
pub(super) struct ParallelCopyPlan {
    pub(super) index: usize,
    pub(super) source_path: PathBuf,
    pub(super) target_location: Location,
    pub(super) written_path: PathBuf,
    pub(super) renamed: Option<RenameCause>,
}

pub(super) async fn parallel_copy_plans(
    request: &PasteRequest,
    sources: &[gio::File],
    destination: &gio::File,
    planned: &[Option<Planned>],
    kinds: &[Option<gio::FileType>],
    windows_disk: bool,
) -> Option<(Vec<ParallelCopyPlan>, usize)> {
    if request.items.len() <= MAX_LOCAL_IO_WORKERS
        || !destination.is_native()
        || request
            .items
            .iter()
            .any(|item| item.conflict != TransferConflict::FailIfExists)
    {
        return None;
    }

    let targets = planned
        .iter()
        .zip(kinds)
        .map(|(planned, kind)| {
            planned
                .as_ref()
                .filter(|_| *kind == Some(gio::FileType::Regular))
                .map(|planned| (destination.child(&planned.target), planned.cause))
        })
        .collect::<Vec<_>>();
    let (sources, destination) = (sources.to_vec(), destination.clone());
    gio::spawn_blocking(move || {
        let plans = sources
            .iter()
            .zip(targets)
            .enumerate()
            .map(|(index, (source, target))| {
                let (target, renamed) = target.filter(|_| source.is_native())?;
                if transfer_is_noop(source, &destination, &target) {
                    return None;
                }
                Some(ParallelCopyPlan {
                    index,
                    source_path: source.path()?,
                    target_location: location_for_file(&target)?,
                    written_path: target.path()?,
                    renamed,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        // A rotational disk on either side reads or writes one file at a time;
        // a disk Windows formats is taken for one where its device does not say.
        let source_folders: HashSet<&Path> = plans
            .iter()
            .filter_map(|plan| plan.source_path.parent())
            .collect();
        let rotational = destination
            .path()
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|directory| fs().rotational(directory.as_fd()))
            .unwrap_or(windows_disk)
            || source_folders.into_iter().any(|folder| {
                open_local_parent_directory(folder)
                    .ok()
                    .and_then(|folder| fs().rotational(folder.as_fd()))
                    == Some(true)
            });
        Some((plans, bounded_local_worker_count(available, rotational)))
    })
    .await
    .ok()
    .flatten()
}

pub(super) struct ParallelCopyOutcome {
    pub(super) failed: Vec<(usize, glib::Error)>,
    pub(super) next: usize,
    pub(super) written_paths: Vec<PathBuf>,
}

pub(super) async fn execute_parallel_copy_plans(
    plans: &[ParallelCopyPlan],
    (item_sizes, item_files): (&[Option<u64>], &[Option<usize>]),
    (worker_count, moving): (usize, bool),
    cancellable: &gio::Cancellable,
    copying: &Copying,
    (settling, variants): (&mut Settling, &mut CaseVariants),
) -> ParallelCopyOutcome {
    let mut outcome = ParallelCopyOutcome {
        failed: Vec::new(),
        next: 0,
        written_paths: Vec::new(),
    };
    let last = plans.last().map(|plan| plan.index);
    while outcome.next < plans.len() && !cancellable.is_cancelled() {
        let start = outcome.next;
        let full = (start + worker_count).min(plans.len());
        // An item waits for the earlier ones whose names differ from its own only in case.
        let end = (start + 1..full)
            .find(|&position| {
                plans[start..position]
                    .iter()
                    .any(|earlier| variants.follows(plans[position].index, earlier.index))
            })
            .unwrap_or(full);
        let chunk = &plans[start..end];
        outcome
            .written_paths
            .extend(chunk.iter().map(|plan| plan.written_path.clone()));
        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .cloned()
            .map(|plan| {
                let (cancellable, copying, earlier) = (
                    cancellable.clone(),
                    copying.clone(),
                    variants.earlier(plan.index),
                );
                context.spawn_local(async move {
                    let target = gio::File::for_path(&plan.written_path);
                    let taken = if case_variants::taken_by(&target, earlier).await {
                        Taken::Number
                    } else {
                        Taken::Fail
                    };
                    transfer_planned(&plan, taken, moving, &cancellable, &copying)
                        .await
                        .map(|transferred| (transferred, taken))
                })
            })
            .collect::<Vec<_>>();
        let mut renamed = Vec::new();
        for (plan, task) in chunk.iter().zip(tasks) {
            match task.await {
                Ok(Ok((transferred, taken))) => {
                    variants.saved(plan.index, &transferred.target).await;
                    let aliased = taken == Taken::Number;
                    let cause = plan.renamed.or(aliased.then_some(RenameCause::Alias));
                    if let (Some(cause), Some(name)) = (cause, plan.source_path.file_name()) {
                        let requested = gio::File::for_path(plan.written_path.with_file_name(name));
                        let saved_as = if aliased {
                            transferred.target.clone()
                        } else {
                            gio::File::for_path(&plan.written_path)
                        };
                        renamed.extend(renamed_item(&requested, &saved_as, cause));
                    }
                    if let Some(progress) = &copying.progress {
                        progress.finish_item(
                            transferred.reported,
                            item_sizes[plan.index],
                            item_files[plan.index],
                            (!moving)
                                .then(|| location_for_file(&transferred.target))
                                .flatten(),
                        );
                    }
                    let done = Done {
                        index: plan.index,
                        moved_to: location_for_file(&transferred.target),
                    };
                    let copied = transferred.reported.bytes;
                    let is_last = Some(plan.index) == last;
                    settling.owe(done, transferred.owed, copied, is_last).await;
                }
                Ok(Err(error)) => outcome.failed.push((plan.index, error)),
                Err(_) => outcome
                    .failed
                    .push((plan.index, io_error("Copy worker failed"))),
            }
        }
        if let Some(progress) = &copying.progress {
            progress.report(TransferReport {
                renamed,
                ..TransferReport::default()
            });
        }
        outcome.next = end;
        if !outcome.failed.is_empty() || settling.failure().is_some() {
            break;
        }
    }
    outcome
}

pub(super) async fn transfer_planned(
    plan: &ParallelCopyPlan,
    taken: Taken,
    moving: bool,
    cancellable: &gio::Cancellable,
    copying: &Copying,
) -> Result<Transferred, glib::Error> {
    if moving {
        let source = gio::File::for_path(&plan.source_path);
        let target = gio::File::for_path(&plan.written_path);
        return move_owing(source, target, taken, cancellable.clone(), copying.clone()).await;
    }
    let options = copy_engine::Options {
        names: copying.names.clone(),
        ..copy_engine::Options::default()
    };
    let copied = copy_engine::copy_local(
        plan.source_path.clone(),
        copy_engine::Destination::New(plan.written_path.clone(), taken),
        options,
        copying.progress.clone(),
        cancellable,
    )
    .await?;
    Ok(Transferred {
        target: gio::File::for_path(plan.written_path.with_file_name(copied.name)),
        reported: copied.reported,
        owed: Owed::default(),
    })
}

pub(super) async fn first_free_target<T, F: Future<Output = Result<T, glib::Error>>>(
    target: &gio::File,
    taken: Taken,
    is_directory: bool,
    cancellable: &gio::Cancellable,
    mut attempt: impl FnMut(gio::File) -> F,
) -> Result<(gio::File, T), glib::Error> {
    if taken == Taken::Fail {
        return attempt(target.clone())
            .await
            .map(|value| (target.clone(), value));
    }
    if let (Some(parent), Some(name)) = (target.parent(), target.basename()) {
        let limit = mount_profile::folder_name_limit(&parent).await;
        for name in numbered_names(name.as_os_str(), is_directory, limit) {
            cancellable.set_error_if_cancelled()?;
            let candidate = parent.child(&name);
            if query_exists(&candidate, cancellable).await {
                continue;
            }
            match attempt(candidate.clone()).await {
                Err(error) if error.matches(gio::IOErrorEnum::Exists) => {}
                result => return result.map(|value| (candidate, value)),
            }
        }
    }
    cancellable.set_error_if_cancelled()?;
    Err(translated_io_error(
        "Could not find an unused duplicate name",
    ))
}
