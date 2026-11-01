// SPDX-License-Identifier: MIT

use super::{
    fs_ops::{Entry, OffMain, fs},
    holders,
    placement::{PlaceError, RenameFailure},
    release::{self, ActiveOperation},
};
use crate::{
    adapters::{
        gio_file_for_location,
        local_files::trash_thumbnail_path,
        location_for_file,
        trash_restore::{
            RestoreContext, absolute_original, candidate_trash_roots, decode_trashinfo_path,
            plan_restore_for_location,
        },
        trash_support::trash_with_cleanup,
    },
    model::Location,
    services::{
        ItemIdentity, KeptCause, KeptItem, MoveRecord, OperationEvent, OperationRequestId,
        PlacedSnapshot, Proof, RenameCause, RenamedItem, TransferReport,
    },
};
use gtk::{gio, glib, prelude::*};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    future::Future,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::Arc,
};

use super::{
    await_cancellable, cancelled_event, cancelled_local_operation, copy_engine,
    deletion::local_parent_worker_count,
    deletion_error_summary, deletion_undone_when_cancelled, io_error, item_error, kept_items,
    mount_profile::identity_at,
    native_fs::{
        identity_at_path, open_local_parent_beneath, open_local_parent_directory, os_error_detail,
        trashed_identity,
    },
};

use super::{
    owed, placement, placement_failure, progress::TransferProgressTracker, rename_failure_error,
    run_blocking, run_local_fs_step, set_display_name, translated_io_error,
};

use super::was_cancelled;

/// Deepest first, so a renamed folder still has the name its entries' paths use. What keeps
/// another name is reported; what is gone from that name is not. Returns the names given back
/// beneath the original, where it has a local path.
pub(super) async fn restore_renamed(
    record: &MoveRecord,
    progress: &TransferProgressTracker,
) -> Vec<(PathBuf, PathBuf)> {
    let mut renamed = record
        .renamed
        .iter()
        .filter_map(|item| {
            let saved_as = item.saved_as.rebase(&record.current, &record.original)?;
            let requested = item.requested.rebase(&record.current, &record.original)?;
            Some((depth(&item.saved_as), saved_as, requested, item.cause))
        })
        .collect::<Vec<_>>();
    renamed.sort_by_key(|(depth, ..)| std::cmp::Reverse(*depth));
    let native = renamed
        .iter()
        .map(|(_, saved_as, requested, _)| {
            Some((
                saved_as.native_path()?.to_path_buf(),
                requested.native_path()?.to_path_buf(),
            ))
        })
        .collect::<Option<Vec<_>>>();
    let restored = match native {
        Some(paths) => {
            let count = paths.len();
            run_blocking(move || Ok(restore_native_names(&paths)))
                .await
                .unwrap_or_else(|error| {
                    let error = crate::services::gio_error_detail(&error);
                    (0..count)
                        .map(|_| Err(NameKept::Failed(error.clone())))
                        .collect()
                })
        }
        None => {
            let mut restored = Vec::with_capacity(renamed.len());
            for (_, saved_as, requested, _) in &renamed {
                restored.push(restore_name(saved_as, requested).await);
            }
            restored
        }
    };
    let root = gio_file_for_location(&record.original).path();
    let beneath = |location: &Location| {
        let path = gio_file_for_location(location).path()?;
        Some(path.strip_prefix(root.as_ref()?).ok()?.to_path_buf())
    };
    let mut given_back = Vec::new();
    let mut report = TransferReport::default();
    for ((_, saved_as, requested, cause), restored) in renamed.into_iter().zip(restored) {
        let cause = match restored {
            Ok(()) => {
                given_back.extend(beneath(&saved_as).zip(beneath(&requested)));
                continue;
            }
            Err(NameKept::Gone) => continue,
            Err(NameKept::Taken) => RenameCause::Taken,
            Err(NameKept::Refused) => cause,
            Err(NameKept::Failed(error)) => {
                report.kept.push(KeptItem {
                    location: requested,
                    cause: KeptCause::NameNotRestored { saved_as, error },
                });
                continue;
            }
        };
        report.renamed.push(RenamedItem {
            requested,
            saved_as,
            cause,
        });
    }
    progress.report(report);
    given_back
}

fn depth(location: &Location) -> usize {
    match location.native_path() {
        Some(path) => path.components().count(),
        None => location
            .uri_value()
            .unwrap_or_default()
            .matches('/')
            .count(),
    }
}

pub(super) enum NameKept {
    Taken,
    Gone,
    Refused,
    Failed(String),
}

fn restore_native_names(paths: &[(PathBuf, PathBuf)]) -> Vec<Result<(), NameKept>> {
    let mut folders = HashMap::<&Path, Result<Arc<OwnedFd>, String>>::new();
    let mut open = |folder| {
        folders
            .entry(folder)
            .or_insert_with(|| open_local_parent_directory(folder).map(Arc::new))
            .clone()
    };
    paths
        .iter()
        .map(|(from, to)| {
            let (Some(from_folder), Some(from_name), Some(to_folder), Some(to_name)) =
                (from.parent(), from.file_name(), to.parent(), to.file_name())
            else {
                return Err(NameKept::Failed(crate::i18n::tr(
                    "Invalid move destination",
                )));
            };
            let folders = open(from_folder).and_then(|from| Ok((from, open(to_folder)?)));
            let (from_dir, to_dir) = match folders {
                Ok(folders) => folders,
                Err(_)
                    if std::fs::symlink_metadata(from)
                        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
                {
                    return Err(NameKept::Gone);
                }
                Err(error) => {
                    return Err(NameKept::Failed(crate::services::gio_error_detail(
                        &io_error(error),
                    )));
                }
            };
            let renamed = placement::rename_in_place(
                fs(),
                Entry::new(&*from_dir, from_name),
                Entry::new(&*to_dir, to_name),
            );
            match renamed {
                Ok(_) => Ok(()),
                Err(RenameFailure::Place(PlaceError::Exists)) => Err(NameKept::Taken),
                Err(RenameFailure::Place(error)) if error.refuses_name() => Err(NameKept::Refused),
                Err(RenameFailure::Place(PlaceError::Io(rustix::io::Errno::NOENT))) => {
                    Err(NameKept::Gone)
                }
                Err(failure) => Err(NameKept::Failed(crate::services::gio_error_detail(
                    &rename_failure_error(failure, from, to),
                ))),
            }
        })
        .collect()
}

pub(super) async fn restore_name(
    saved_as: &Location,
    requested: &Location,
) -> Result<(), NameKept> {
    // GIO renames take a name as text.
    let Ok(name) = requested.file_name().unwrap_or_default().into_string() else {
        return Err(NameKept::Refused);
    };
    let file = gio_file_for_location(saved_as);
    match set_display_name(&file, &name, &gio::Cancellable::new()).await {
        Ok(_) => Ok(()),
        Err(error) if error.matches(gio::IOErrorEnum::Exists) => Err(NameKept::Taken),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Err(NameKept::Gone),
        Err(error)
            if error.matches(gio::IOErrorEnum::InvalidFilename)
                || error.matches(gio::IOErrorEnum::FilenameTooLong) =>
        {
            Err(NameKept::Refused)
        }
        Err(error) => Err(NameKept::Failed(crate::services::gio_error_detail(&error))),
    }
}

pub(super) fn could_not_restore(name: &str, error: &str) -> String {
    rust_i18n::t!(
        "Could not restore %{name}: %{error}",
        name = name,
        error = error
    )
    .into_owned()
}

pub(super) async fn move_restore_path(
    source_path: PathBuf,
    target_path: PathBuf,
    allowed_root: PathBuf,
    unlinks: Arc<copy_engine::Unlinks>,
    cancellable: gio::Cancellable,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let Some(source_parent_path) = source_path.parent().map(Path::to_path_buf) else {
        return Err(translated_io_error("Cannot restore the filesystem root"));
    };
    let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
        return Err(translated_io_error("Invalid restore source"));
    };
    let Some(target_parent_path) = target_path.parent().map(Path::to_path_buf) else {
        return Err(translated_io_error(
            "The restore destination has no parent directory",
        ));
    };
    let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
        return Err(translated_io_error("Invalid restore destination"));
    };

    let display_name = source_name.to_string_lossy().into_owned();
    let step_source_name = source_name.clone();
    let step_cancellable = cancellable.clone();
    let step_display_name = display_name.clone();
    let (trash_path, barrier_path) = (source_parent_path.clone(), target_parent_path.clone());
    let placed = run_local_fs_step(move || {
        let source_parent = Arc::new(open_local_parent_directory(&source_parent_path)?);
        let target_parent = Arc::new(open_local_parent_beneath(
            &target_parent_path,
            &allowed_root,
        )?);
        let from = Entry::new(&*source_parent, &step_source_name);
        let to = Entry::new(&*target_parent, &target_name);
        let stat = fs()
            .stat(from, false)
            .map_err(|error| could_not_restore(&step_display_name, &os_error_detail(error)))?;
        let trashed = trashed_identity(&stat, identity_at(fs(), source_parent.as_fd()));
        if step_cancellable.is_cancelled() {
            return Ok(None);
        }
        let placed = placement::place(fs(), from, to, Some(stat.identity));
        Ok(Some((
            OffMain::new(source_parent),
            OffMain::new(target_parent),
            stat.identity,
            trashed,
            placed,
        )))
    })
    .await?;
    let Some((source_parent, target_parent, source_identity, trashed, placed)) = placed else {
        return Err(cancelled_local_operation());
    };
    match placed {
        Ok(_) => return Ok((TransferReport::default(), trashed)),
        Err(PlaceError::Unsupported) => {}
        Err(PlaceError::CrossDevice) => {
            return Err(io_error(rust_i18n::t!(
                "Could not restore %{name} across volumes",
                name = display_name
            )));
        }
        Err(PlaceError::Exists) => return Err(restore_destination_exists(&display_name)),
        Err(error) => return Err(placement_failure(error, &display_name, could_not_restore)),
    }

    let barrier = copy_engine::barrier(barrier_path).await?;
    let options = copy_engine::Options {
        durable: true,
        ..copy_engine::Options::default()
    };
    let (sources, mut report) = copy_engine::copy_in_place(
        (source_parent.into_inner(), source_name, source_identity),
        (target_parent.into_inner(), target_path),
        options,
        &cancellable,
    )
    .await
    .map_err(|error| {
        if was_cancelled(&error) {
            error
        } else if error.matches(gio::IOErrorEnum::Exists) {
            restore_destination_exists(&display_name)
        } else {
            io_error(could_not_restore(
                &display_name,
                &crate::services::gio_error_detail(&error),
            ))
        }
    })?;
    owed::flush(barrier).await.map_err(io_error)?;
    // The copy is complete, so a cancel no longer stops this restore.
    let (kept, deleted) =
        copy_engine::delete_unchanged_sources(sources, unlinks, &gio::Cancellable::new()).await;
    deleted.map_err(|error| {
        io_error(rust_i18n::t!(
            "The item was restored, but its trash copy could not be removed: %{error}",
            error = crate::services::gio_error_detail(&error)
        ))
    })?;
    report.kept_in_trash = kept_items(&trash_path, kept);
    Ok((report, trashed))
}

pub(super) fn restore_destination_exists(display_name: &str) -> glib::Error {
    io_error(rust_i18n::t!(
        "Could not restore %{name}: something already exists at the destination",
        name = display_name
    ))
}

pub(super) async fn move_restore(
    source: gio::File,
    target: gio::File,
    allowed_root: PathBuf,
    unlinks: Arc<copy_engine::Unlinks>,
    cancellable: gio::Cancellable,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    if source.is_native()
        && target.is_native()
        && let (Some(source_path), Some(target_path)) = (source.path(), target.path())
    {
        return move_restore_path(source_path, target_path, allowed_root, unlinks, cancellable)
            .await;
    }
    Err(io_error(crate::i18n::tr(
        "Trash restore requires a local source and destination",
    )))
}

fn vouched(expected: &ItemIdentity, path: &Path) -> Option<bool> {
    let current = identity_at_path(path)?;
    (expected.proof(&current) != Proof::Different).then(|| expected.unchanged(&current))
}

async fn trashed_as(expected: ItemIdentity, path: PathBuf) -> Option<bool> {
    gio::spawn_blocking(move || vouched(&expected, &path))
        .await
        .ok()
        .flatten()
}

#[derive(Clone)]
pub(super) struct RestoreEntry {
    pub(super) source: Location,
    pub(super) display_name: String,
    pub(super) original_target: Option<Location>,
    pub(super) trash_info: Option<PathBuf>,
    pub(super) confirmed_destination: Option<PathBuf>,
    pub(super) physical_path: Option<PathBuf>,
}

pub(super) async fn trashed_entries_for_originals(
    original_locations: &[Location],
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> Result<Vec<Result<RestoreEntry, (Location, glib::Error)>>, glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let requested = original_locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    // GVfs can miss an item re-trashed under the same basename after a restore, so prefer the
    // authoritative freedesktop.org metadata before consulting trash:///.
    let context = RestoreContext::current().await;
    let fallback_context = context.clone();
    let fallback_requested = requested.clone();
    let fallback_cancellable = cancellable.clone();
    let fallback_originals = originals.clone();
    let mut fallback = gio::spawn_blocking(move || {
        trash_entries(
            &fallback_context,
            &fallback_requested,
            &fallback_cancellable,
            &fallback_originals,
        )
    })
    .await
    .map_err(|_| glib::Error::new(gio::IOErrorEnum::Failed, "Trash lookup task failed"))?;
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    if fallback.len() == requested.len() && requested.len() == original_locations.len() {
        return Ok(original_locations
            .iter()
            .filter_map(|location| {
                let found = fallback.remove(location.native_path()?)?;
                Some(found.map_err(|error| (location.clone(), error)))
            })
            .collect());
    }

    let trash = gio::File::for_uri("trash:///");
    let enumerated = await_cancellable(&trash, cancellable, |trash, cancellable, result| {
        trash.enumerate_children_async(
            "standard::name,standard::display-name,standard::target-uri,trash::orig-path,trash::deletion-date",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |value| result.resolve(value),
        );
    })
    .await;
    let enumerator = match enumerated {
        Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => {
            return Err(trashed_items_unavailable());
        }
        enumerator => enumerator?,
    };
    let mut newest = HashMap::new();
    loop {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let infos = await_cancellable(
            &enumerator,
            cancellable,
            |enumerator, cancellable, result| {
                enumerator.next_files_async(
                    64,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |value| result.resolve(value),
                );
            },
        )
        .await?;
        if infos.is_empty() {
            break;
        }
        for info in infos {
            let Some(original_path) = info.attribute_byte_string("trash::orig-path") else {
                continue;
            };
            let original_path = PathBuf::from(original_path.as_str());
            if !requested.contains(&original_path) || fallback.contains_key(&original_path) {
                continue;
            }
            let deletion_date = info
                .attribute_string("trash::deletion-date")
                .map(|value| value.to_string())
                .unwrap_or_default();
            let Some(location) = location_for_file(&trash.child(info.name())) else {
                continue;
            };
            let original = Location::local(&original_path);
            let recorded = originals.get(&original);
            let vouched = match recorded {
                Some(&expected) => match plan_restore_for_location(
                    &location,
                    Some(&original),
                    None,
                    None,
                    &context,
                )
                .await
                {
                    Ok(plan) => trashed_as(expected, plan.source_path).await,
                    Err(_) => None,
                },
                None => Some(true),
            };
            let Some(vouched) = vouched else {
                continue;
            };
            let entry = gvfs_trash_entry(location, &info, &original_path);
            keep_newest(
                &mut newest,
                original_path,
                (vouched, deletion_date),
                recorded.is_some(),
                entry,
            );
        }
    }

    if requested
        .iter()
        .any(|path| !fallback.contains_key(path) && !newest.contains_key(path))
        || requested.len() != original_locations.len()
    {
        return Err(trashed_items_unavailable());
    }
    Ok(original_locations
        .iter()
        .filter_map(|location| {
            let path = location.native_path()?;
            let found = fallback
                .remove(path)
                .or_else(|| newest.remove(path).map(|newest| chosen(path, newest)))?;
            Some(found.map_err(|error| (location.clone(), error)))
        })
        .collect())
}

struct Newest {
    rank: (bool, String),
    entry: RestoreEntry,
    tied: bool,
}

fn keep_newest(
    newest: &mut HashMap<PathBuf, Newest>,
    original: PathBuf,
    rank: (bool, String),
    recorded: bool,
    entry: RestoreEntry,
) {
    if let Some(kept) = newest.get_mut(&original)
        && kept.rank >= rank
    {
        kept.tied |= recorded && rank.0 && kept.rank == rank;
        return;
    }
    newest.insert(
        original,
        Newest {
            rank,
            entry,
            tied: false,
        },
    );
}

fn chosen(original: &Path, newest: Newest) -> Result<RestoreEntry, glib::Error> {
    if !newest.tied {
        return Ok(newest.entry);
    }
    let name = Location::local(original).display_name();
    Err(glib::Error::new(
        gio::IOErrorEnum::Failed,
        &rust_i18n::t!(
            "More than one item in Trash could be “%{name}”, so they were left there",
            name = name
        ),
    ))
}

fn trashed_items_unavailable() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NotFound,
        &crate::i18n::tr("One or more recently trashed items are no longer available"),
    )
}

pub(super) fn gvfs_trash_entry(
    source: Location,
    info: &gio::FileInfo,
    original_path: &Path,
) -> RestoreEntry {
    RestoreEntry {
        physical_path: trash_thumbnail_path(&source, info),
        source,
        display_name: info.display_name().to_string(),
        original_target: Some(Location::local(original_path)),
        trash_info: None,
        confirmed_destination: None,
    }
}

pub(super) async fn release_trashed(
    entries: &[RestoreEntry],
    cancellable: &gio::Cancellable,
) -> ActiveOperation {
    let roots: Vec<PathBuf> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .physical_path
                .clone()
                .or_else(|| entry.source.native_path().map(Path::to_path_buf))
        })
        .collect();
    let destinations: Vec<PathBuf> = entries
        .iter()
        .filter_map(|entry| {
            restored_destination(entry)
                .native_path()
                .map(Path::to_path_buf)
        })
        .collect();
    let operation = ActiveOperation::begin(release::parents(&destinations));
    operation.release(roots, cancellable).await;
    operation
}

pub(super) fn restored_destination(entry: &RestoreEntry) -> Location {
    entry
        .confirmed_destination
        .as_ref()
        .map(|path| Location::local(path.clone()))
        .or_else(|| entry.original_target.clone())
        .unwrap_or_else(|| entry.source.clone())
}

pub(super) async fn restore_worker_count(entries: &[RestoreEntry]) -> usize {
    local_parent_worker_count(entries.iter().filter_map(|entry| {
        restored_destination(entry)
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }))
    .await
}

/// The entry's Trash metadata stays while anything of it does.
pub(super) async fn restore_trash_entry(
    entry: &RestoreEntry,
    context: &RestoreContext,
    affected_locations: &mut HashSet<Location>,
    unlinks: &Arc<copy_engine::Unlinks>,
    cancellable: &gio::Cancellable,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    let plan = match plan_restore_for_location(
        &entry.source,
        entry.original_target.as_ref(),
        entry.trash_info.as_deref(),
        entry.physical_path.as_deref(),
        context,
    )
    .await
    {
        Ok(plan)
            if entry
                .confirmed_destination
                .as_ref()
                .is_some_and(|confirmed| plan.destination != *confirmed) =>
        {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &crate::i18n::tr(
                    "The original location changed and no longer matches the confirmed destination",
                ),
            ));
        }
        Ok(plan) => plan,
        Err(error) => {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                &error.user_message(),
            ));
        }
    };
    if let Some(parent) = plan.destination.parent() {
        affected_locations.insert(Location::local(parent));
    }
    let source = gio::File::for_path(&plan.source_path);
    let target = gio::File::for_path(&plan.destination);
    let (report, trashed) = move_restore(
        source,
        target,
        plan.allowed_root,
        unlinks.clone(),
        cancellable.clone(),
    )
    .await?;
    if report.kept_in_trash.is_empty()
        && let Some(info_path) = plan.trash_info.or_else(|| entry.trash_info.clone())
    {
        let item = plan.source_path;
        let removed = gio::spawn_blocking(move || match std::fs::symlink_metadata(&item) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                std::fs::remove_file(info_path)
            }
            // The Trash still holds something under the item's name.
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        })
        .await;
        if let Ok(Err(error)) = removed {
            tracing::warn!(%error, "unable to remove restored trash metadata");
        }
    }
    Ok((report, trashed))
}

pub(super) fn trashed_merge_original(
    location: Location,
    cancellable: gio::Cancellable,
    originals: HashMap<Location, ItemIdentity>,
) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>> {
    Box::pin(async move {
        let entries = trashed_entries_for_originals(
            std::slice::from_ref(&location),
            &cancellable,
            &originals,
        )
        .await?;
        match entries.into_iter().next() {
            Some(found) => found.map_err(|(_, error)| error),
            None => Err(glib::Error::new(
                gio::IOErrorEnum::NotFound,
                &crate::i18n::tr("The original is no longer in Trash"),
            )),
        }
    })
}

pub(super) enum Reverted {
    Restored(TransferReport, Box<ItemIdentity>),
    Kept,
}

pub(super) async fn remove_unchanged(
    path: &Path,
    placed: &PlacedSnapshot,
    unlinks: &Arc<copy_engine::Unlinks>,
    cancellable: &gio::Cancellable,
    deleting: &mut bool,
) -> Result<bool, glib::Error> {
    let removed = copy_engine::remove_placed(
        path.to_path_buf(),
        placed.clone(),
        unlinks.clone(),
        cancellable,
    )
    .await;
    *deleting = removed.removed;
    removed.result?;
    if let Some((path, error)) = removed.kept.iter().find_map(|(path, cause)| match cause {
        KeptCause::Undeletable(error) => Some((
            path,
            crate::services::io_error_detail(&io::Error::from_raw_os_error(*error)),
        )),
        KeptCause::StillOpen(holder) => Some((
            path,
            crate::services::error_detail(holders::held(holder.as_ref())),
        )),
        _ => None,
    }) {
        return Err(io_error(rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = path.file_name().unwrap_or_default().to_string_lossy(),
            error = error
        )));
    }
    Ok(removed.kept.is_empty())
}

pub(super) async fn undo_merged_overwrite(
    (location, placed): (&Location, Option<&PlacedSnapshot>),
    staged: &RestoreEntry,
    context: &RestoreContext,
    affected_locations: &mut HashSet<Location>,
    unlinks: &Arc<copy_engine::Unlinks>,
    cancellable: &gio::Cancellable,
    deleting: &mut bool,
) -> Result<Reverted, glib::Error> {
    let freed = match (placed, location.native_path()) {
        (Some(placed), Some(path)) => {
            remove_unchanged(path, placed, unlinks, cancellable, deleting).await?
        }
        // Nothing tells what is there, so only an empty path takes the
        // original back.
        _ => {
            let file = gio_file_for_location(location);
            let queried = await_cancellable(&file, cancellable, |file, cancellable, result| {
                file.query_info_async(
                    "standard::type",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            })
            .await;
            match queried {
                Err(error) if error.matches(gio::IOErrorEnum::NotFound) => true,
                Err(error) => return Err(error),
                Ok(_) => false,
            }
        }
    };
    if !freed {
        return Ok(Reverted::Kept);
    }
    restore_trash_entry(staged, context, affected_locations, unlinks, cancellable)
        .await
        .map(|(report, trashed)| Reverted::Restored(report, Box::new(trashed)))
}

pub(super) type StagedOriginalLookup = Rc<
    dyn Fn(
        Location,
        gio::Cancellable,
    ) -> Pin<Box<dyn Future<Output = Result<RestoreEntry, glib::Error>>>>,
>;

pub(super) async fn run_merge_undo(
    request_id: OperationRequestId,
    (created, overwritten, placed): (
        Vec<Location>,
        Vec<Location>,
        HashMap<Location, PlacedSnapshot>,
    ),
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellable: gio::Cancellable,
    staged_original: StagedOriginalLookup,
) {
    let total = overwritten.len() + created.len();
    let pending: Vec<Location> = overwritten.iter().chain(&created).cloned().collect();
    let mut affected_locations = HashSet::from([Location::uri("trash:///")]);
    for location in &pending {
        if let Some(parent) = location.parent() {
            affected_locations.insert(parent);
        }
    }
    let mut deleted_pins = crate::adapters::bookmarks::DeletedPins::default();
    let context = RestoreContext::current().await;
    let unlinks = Arc::default();
    let mut completed_locations = Vec::new();
    let mut restored = Vec::new();
    let mut removed = Vec::new();
    let mut failed_locations = Vec::new();
    let mut errors = Vec::new();
    let mut completed = 0usize;
    let remaining = |index: usize| pending[index..].to_vec();
    // Copies only: the incoming item duplicates the surviving source, so it
    // is deleted when unchanged before the original comes back from Trash.
    for (index, location) in overwritten.iter().enumerate() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed_locations,
                failed_locations,
                remaining(index),
                affected_locations,
            ));
            return;
        }
        let mut deleting = false;
        let result = match staged_original(location.clone(), cancellable.clone()).await {
            Ok(staged) => {
                undo_merged_overwrite(
                    (location, placed.get(location)),
                    &staged,
                    &context,
                    &mut affected_locations,
                    &unlinks,
                    &cancellable,
                    &mut deleting,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let restored_location = match result {
            Ok(Reverted::Restored(report, identity)) => {
                if !report.is_empty() {
                    emit(OperationEvent::ItemsReported { request_id, report });
                }
                emit(OperationEvent::ItemRestored {
                    request_id,
                    location: location.clone(),
                    identity: *identity,
                });
                completed_locations.push(location.clone());
                restored.push(location.clone());
                Some(location.clone())
            }
            Ok(Reverted::Kept) => {
                emit(OperationEvent::ItemsReported {
                    request_id,
                    report: TransferReport {
                        kept_in_trash: vec![KeptItem {
                            location: location.clone(),
                            cause: KeptCause::ReplacementChanged,
                        }],
                        ..TransferReport::default()
                    },
                });
                emit(OperationEvent::ItemKept {
                    request_id,
                    location: location.clone(),
                });
                completed_locations.push(location.clone());
                None
            }
            Err(error) if was_cancelled(&error) => {
                if deleting {
                    failed_locations.push(location.clone());
                }
                emit(cancelled_event(
                    request_id,
                    completed_locations,
                    failed_locations,
                    remaining(index + usize::from(deleting)),
                    affected_locations,
                ));
                return;
            }
            Err(error) => {
                errors.push(item_error(
                    &location.display_name(),
                    crate::services::gio_error_detail(&error),
                ));
                failed_locations.push(location.clone());
                None
            }
        };
        completed += 1;
        emit(OperationEvent::RestoreProgress {
            request_id,
            completed,
            total,
            restored_location,
        });
    }
    for (index, location) in created.iter().enumerate() {
        if cancellable.is_cancelled() {
            emit(cancelled_event(
                request_id,
                completed_locations,
                failed_locations,
                remaining(overwritten.len() + index),
                affected_locations,
            ));
            return;
        }
        let file = gio_file_for_location(location);
        let queried = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await;
        let found = queried.is_ok();
        let mut trashed_identity = None;
        let mut deleting = false;
        let mut kept = false;
        let result = match queried {
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(()),
            Err(error) => Err(error),
            Ok(_) => match trash_with_cleanup(&file, &cancellable).await {
                Err(error) if error.is_unsupported() => {
                    match (placed.get(location), location.native_path()) {
                        (Some(placed), Some(path)) => {
                            remove_unchanged(path, placed, &unlinks, &cancellable, &mut deleting)
                                .await
                        }
                        _ => Ok(false),
                    }
                    .and_then(|removed| {
                        // What changed once part of it was deleted holds its folder.
                        if !removed && deleting {
                            return Err(io_error(crate::services::io_error_detail(
                                &io::Error::from_raw_os_error(
                                    rustix::io::Errno::NOTEMPTY.raw_os_error(),
                                ),
                            )));
                        }
                        kept = !removed;
                        Ok(())
                    })
                }
                result => result
                    .map(|identity| trashed_identity = identity)
                    .map_err(|error| error.error),
            },
        };
        let deleted_locations = match result {
            Ok(()) if kept => {
                emit(OperationEvent::ItemKept {
                    request_id,
                    location: location.clone(),
                });
                errors.push(item_error(
                    &location.display_name(),
                    crate::i18n::tr("It was kept, as it may have changed since it was copied"),
                ));
                failed_locations.push(location.clone());
                Vec::new()
            }
            Ok(()) => {
                if found {
                    deleted_pins.push(location.clone());
                }
                if let Some(identity) = trashed_identity {
                    emit(OperationEvent::ItemTrashed {
                        request_id,
                        location: location.clone(),
                        identity,
                    });
                }
                completed_locations.push(location.clone());
                removed.push(location.clone());
                vec![location.clone()]
            }
            Err(error) if was_cancelled(&error) => {
                // Without Trash, an item stays whole until anything of it is deleted.
                let untouched = !found || deletion_undone_when_cancelled(location, deleting);
                if !untouched {
                    failed_locations.push(location.clone());
                }
                emit(cancelled_event(
                    request_id,
                    completed_locations,
                    failed_locations,
                    remaining(overwritten.len() + index + usize::from(!untouched)),
                    affected_locations,
                ));
                return;
            }
            Err(error) => {
                errors.push(item_error(
                    &location.display_name(),
                    crate::services::gio_error_detail(&error),
                ));
                failed_locations.push(location.clone());
                Vec::new()
            }
        };
        completed += 1;
        emit(OperationEvent::DeleteProgress {
            request_id,
            completed,
            total,
            deleted_locations,
        });
    }
    if cancellable.is_cancelled() {
        emit(cancelled_event(
            request_id,
            completed_locations,
            failed_locations,
            Vec::new(),
            affected_locations,
        ));
    } else if errors.is_empty() {
        emit(OperationEvent::Restored {
            request_id,
            locations: removed,
            restored,
        });
    } else {
        emit(OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations: removed,
            retryable_locations: Vec::new(),
            has_non_retryable_failures: true,
            message: deletion_error_summary(&errors, true),
        });
    }
}

fn trash_entries(
    context: &RestoreContext,
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> HashMap<PathBuf, Result<RestoreEntry, glib::Error>> {
    let mut roots = requested
        .iter()
        .flat_map(|path| candidate_trash_roots(context, Some(path)))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    trash_entries_at(&roots, requested, cancellable, originals)
}

pub(super) fn trash_entries_at(
    roots: &[PathBuf],
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> HashMap<PathBuf, Result<RestoreEntry, glib::Error>> {
    let mut newest = HashMap::new();
    for trash_root in roots {
        newest_in_trash(trash_root, requested, cancellable, originals, &mut newest);
    }
    newest
        .into_iter()
        .map(|(path, newest)| {
            let entry = chosen(&path, newest);
            (path, entry)
        })
        .collect()
}

fn newest_in_trash(
    trash_root: &Path,
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
    newest: &mut HashMap<PathBuf, Newest>,
) {
    let info_root = trash_root.join("info");
    let files_root = trash_root.join("files");
    let Ok(infos) = std::fs::read_dir(info_root) else {
        return;
    };
    for info in infos.flatten() {
        if cancellable.is_cancelled() {
            break;
        }
        let info_path = info.path();
        let Some(name) = info_path.file_name() else {
            continue;
        };
        let bytes = name.as_bytes();
        let Some(file_name) = bytes.strip_suffix(b".trashinfo") else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&info_path) else {
            continue;
        };
        let encoded_path = contents.lines().find_map(|line| line.strip_prefix("Path="));
        let deletion_date = contents
            .lines()
            .find_map(|line| line.strip_prefix("DeletionDate="))
            .unwrap_or_default();
        let Some(original_path) = encoded_path
            .and_then(decode_trashinfo_path)
            .and_then(|path| absolute_original(trash_root, path))
        else {
            continue;
        };
        if !requested.contains(&original_path) {
            continue;
        }
        let source_path = files_root.join(OsString::from_vec(file_name.to_vec()));
        let recorded = originals.get(&Location::local(&original_path));
        let vouched = match recorded {
            Some(expected) => vouched(expected, &source_path),
            None => std::fs::symlink_metadata(&source_path)
                .is_ok()
                .then_some(true),
        };
        let Some(vouched) = vouched else {
            continue;
        };
        let entry = RestoreEntry {
            source: Location::local(&source_path),
            display_name: source_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::i18n::tr("Trashed item")),
            original_target: Some(Location::local(&original_path)),
            trash_info: Some(info_path),
            confirmed_destination: None,
            physical_path: Some(source_path.clone()),
        };
        keep_newest(
            newest,
            original_path,
            (vouched, deletion_date.to_owned()),
            recorded.is_some(),
            entry,
        );
    }
}
