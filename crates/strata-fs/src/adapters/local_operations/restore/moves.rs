// SPDX-License-Identifier: MIT

use super::super::native_fs::LocalFileIdentity;
use super::*;
use crate::adapters::trash_restore::RestorePlan;

/// What every restore of one request shares.
pub(in crate::adapters::local_operations) struct Restoring<'a> {
    pub(in crate::adapters::local_operations) context: &'a RestoreContext,
    pub(in crate::adapters::local_operations) unlinks: &'a Arc<copy_engine::Unlinks>,
    pub(in crate::adapters::local_operations) cancellable: &'a gio::Cancellable,
}

pub(in crate::adapters::local_operations) fn could_not_restore(name: &str, error: &str) -> String {
    rust_i18n::t!(
        "Could not restore %{name}: %{error}",
        name = name,
        error = error
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) async fn move_restore_path(
    source_path: PathBuf,
    target_path: PathBuf,
    allowed_root: PathBuf,
    unlinks: Arc<copy_engine::Unlinks>,
    cancellable: gio::Cancellable,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let paths = RestorePaths::of(&source_path, &target_path)?;
    let restore = RestoreMove {
        display_name: paths.source_name.to_string_lossy().into_owned(),
        paths,
        target_path,
        unlinks,
        cancellable,
    };
    let Some(pinned) = restore.pin(allowed_root).await? else {
        return Err(cancelled_local_operation());
    };
    let trashed = pinned.trashed;
    if !restore.needs_copy(&pinned.placed)? {
        return Ok((TransferReport::default(), trashed));
    }
    let report = restore.copy_across(pinned).await?;
    Ok((report, trashed))
}

struct RestorePaths {
    source_parent: PathBuf,
    source_name: OsString,
    target_parent: PathBuf,
    target_name: OsString,
}

impl RestorePaths {
    fn of(source_path: &Path, target_path: &Path) -> Result<Self, glib::Error> {
        let Some(source_parent) = source_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error("Cannot restore the filesystem root"));
        };
        let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid restore source"));
        };
        let Some(target_parent) = target_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error(
                "The restore destination has no parent directory",
            ));
        };
        let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid restore destination"));
        };
        Ok(Self {
            source_parent,
            source_name,
            target_parent,
            target_name,
        })
    }
}

/// Both folders pinned, and the rename tried between them.
struct Pinned {
    source_parent: OffMain<Arc<OwnedFd>>,
    target_parent: OffMain<Arc<OwnedFd>>,
    source_identity: LocalFileIdentity,
    trashed: ItemIdentity,
    placed: Result<placement::Placed, PlaceError>,
}

struct RestoreMove {
    paths: RestorePaths,
    target_path: PathBuf,
    display_name: String,
    unlinks: Arc<copy_engine::Unlinks>,
    cancellable: gio::Cancellable,
}

impl RestoreMove {
    /// `None` when cancelled before the rename.
    async fn pin(&self, allowed_root: PathBuf) -> Result<Option<Pinned>, glib::Error> {
        let (source_parent_path, target_parent_path) = (
            self.paths.source_parent.clone(),
            self.paths.target_parent.clone(),
        );
        let (source_name, target_name) = (
            self.paths.source_name.clone(),
            self.paths.target_name.clone(),
        );
        let (cancellable, display_name) = (self.cancellable.clone(), self.display_name.clone());
        run_local_fs_step(move || {
            let source_parent = Arc::new(open_local_parent_directory(&source_parent_path)?);
            let target_parent = Arc::new(open_local_parent_beneath(
                &target_parent_path,
                &allowed_root,
            )?);
            let from = Entry::new(&*source_parent, &source_name);
            let to = Entry::new(&*target_parent, &target_name);
            let stat = fs()
                .stat(from, false)
                .map_err(|error| could_not_restore(&display_name, &os_error_detail(error)))?;
            let trashed = trashed_identity(&stat, identity_at(fs(), source_parent.as_fd()));
            if cancellable.is_cancelled() {
                return Ok(None);
            }
            let placed = placement::place(fs(), from, to, Some(stat.identity));
            Ok(Some(Pinned {
                source_parent: OffMain::new(source_parent),
                target_parent: OffMain::new(target_parent),
                source_identity: stat.identity,
                trashed,
                placed,
            }))
        })
        .await
    }

    /// Whether the rename was refused in a way a copy can stand in for.
    fn needs_copy(
        &self,
        placed: &Result<placement::Placed, PlaceError>,
    ) -> Result<bool, glib::Error> {
        let display_name = &self.display_name;
        match placed {
            Ok(_) => Ok(false),
            Err(PlaceError::Unsupported) => Ok(true),
            Err(PlaceError::CrossDevice) => Err(io_error(rust_i18n::t!(
                "Could not restore %{name} across volumes",
                name = display_name
            ))),
            Err(PlaceError::Exists) => Err(restore_destination_exists(display_name)),
            Err(error) => Err(placement_failure(*error, display_name, could_not_restore)),
        }
    }

    async fn copy_across(self, pinned: Pinned) -> Result<TransferReport, glib::Error> {
        let barrier = copy_engine::barrier(self.paths.target_parent.clone()).await?;
        let options = copy_engine::Options {
            durable: true,
            ..copy_engine::Options::default()
        };
        let display_name = &self.display_name;
        let (sources, mut report) = copy_engine::copy_in_place(
            (
                pinned.source_parent.into_inner(),
                self.paths.source_name,
                pinned.source_identity,
            ),
            (pinned.target_parent.into_inner(), self.target_path),
            options,
            &self.cancellable,
        )
        .await
        .map_err(|error| restore_copy_failure(error, display_name))?;
        owed::flush(barrier).await.map_err(io_error)?;
        // The copy is complete, so a cancel no longer stops this restore.
        let (kept, deleted) =
            copy_engine::delete_unchanged_sources(sources, self.unlinks, &gio::Cancellable::new())
                .await;
        deleted.map_err(|error| {
            io_error(rust_i18n::t!(
                "The item was restored, but its trash copy could not be removed: %{error}",
                error = crate::services::gio_error_detail(&error)
            ))
        })?;
        report.kept_in_trash = kept_items(&self.paths.source_parent, kept);
        Ok(report)
    }
}

fn restore_copy_failure(error: glib::Error, display_name: &str) -> glib::Error {
    if was_cancelled(&error) {
        error
    } else if error.matches(gio::IOErrorEnum::Exists) {
        restore_destination_exists(display_name)
    } else {
        io_error(could_not_restore(
            display_name,
            &crate::services::gio_error_detail(&error),
        ))
    }
}

pub(in crate::adapters::local_operations) fn restore_destination_exists(
    display_name: &str,
) -> glib::Error {
    io_error(rust_i18n::t!(
        "Could not restore %{name}: something already exists at the destination",
        name = display_name
    ))
}

async fn move_restore(
    (source, target): (gio::File, gio::File),
    allowed_root: PathBuf,
    restoring: &Restoring<'_>,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    if restoring.cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    if !source.is_native() || !target.is_native() {
        return Err(local_restore_required());
    }
    let (Some(source_path), Some(target_path)) = (source.path(), target.path()) else {
        return Err(local_restore_required());
    };
    move_restore_path(
        source_path,
        target_path,
        allowed_root,
        restoring.unlinks.clone(),
        restoring.cancellable.clone(),
    )
    .await
}

fn local_restore_required() -> glib::Error {
    io_error(crate::i18n::tr(
        "Trash restore requires a local source and destination",
    ))
}

/// The entry's Trash metadata stays while anything of it does.
pub(in crate::adapters::local_operations) async fn restore_trash_entry(
    entry: &RestoreEntry,
    restoring: &Restoring<'_>,
    affected_locations: &mut HashSet<Location>,
) -> Result<(TransferReport, ItemIdentity), glib::Error> {
    let plan = confirmed_plan(entry, restoring.context).await?;
    if let Some(parent) = plan.destination.parent() {
        affected_locations.insert(Location::local(parent));
    }
    let source = gio::File::for_path(&plan.source_path);
    let target = gio::File::for_path(&plan.destination);
    let (report, trashed) = move_restore((source, target), plan.allowed_root, restoring).await?;
    let info_path = plan
        .trash_info
        .or_else(|| entry.trash_info.clone())
        .filter(|_| report.kept_in_trash.is_empty());
    if let Some(info_path) = info_path {
        remove_trash_info(plan.source_path, info_path).await;
    }
    Ok((report, trashed))
}

async fn confirmed_plan(
    entry: &RestoreEntry,
    context: &RestoreContext,
) -> Result<RestorePlan, glib::Error> {
    let plan = plan_restore_for_location(
        &entry.source,
        entry.original_target.as_ref(),
        entry.trash_info.as_deref(),
        entry.physical_path.as_deref(),
        context,
    )
    .await
    .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.user_message()))?;
    let moved = entry
        .confirmed_destination
        .as_ref()
        .is_some_and(|confirmed| plan.destination != *confirmed);
    if moved {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            &crate::i18n::tr(
                "The original location changed and no longer matches the confirmed destination",
            ),
        ));
    }
    Ok(plan)
}

async fn remove_trash_info(item: PathBuf, info_path: PathBuf) {
    let removed = gio::spawn_blocking(move || match std::fs::symlink_metadata(&item) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => std::fs::remove_file(info_path),
        // The Trash still holds something under the item's name.
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    })
    .await;
    if let Ok(Err(error)) = removed {
        tracing::warn!(%error, "unable to remove restored trash metadata");
    }
}
