// SPDX-License-Identifier: MIT

use super::*;

/// A rename a move made beneath its target, deepest first.
type Rename = (usize, Location, Location, RenameCause);

/// Deepest first, so a renamed folder still has the name its entries' paths use. What keeps
/// another name is reported; what is gone from that name is not. Returns the names given back
/// beneath the original, where it has a local path.
pub(in crate::adapters::local_operations) async fn restore_renamed(
    record: &MoveRecord,
    progress: &TransferProgressTracker,
) -> Vec<(PathBuf, PathBuf)> {
    let renamed = deepest_first(record);
    let restored = restore_names(&renamed).await;
    let (given_back, report) = sort_restored(record, renamed, restored);
    progress.report(report);
    given_back
}

fn deepest_first(record: &MoveRecord) -> Vec<Rename> {
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
    renamed
}

async fn restore_names(renamed: &[Rename]) -> Vec<Result<(), NameKept>> {
    let native = renamed
        .iter()
        .map(|(_, saved_as, requested, _)| {
            Some((
                saved_as.native_path()?.to_path_buf(),
                requested.native_path()?.to_path_buf(),
            ))
        })
        .collect::<Option<Vec<_>>>();
    let Some(paths) = native else {
        let mut restored = Vec::with_capacity(renamed.len());
        for (_, saved_as, requested, _) in renamed {
            restored.push(restore_name(saved_as, requested).await);
        }
        return restored;
    };
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

fn sort_restored(
    record: &MoveRecord,
    renamed: Vec<Rename>,
    restored: Vec<Result<(), NameKept>>,
) -> (Vec<(PathBuf, PathBuf)>, TransferReport) {
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
    (given_back, report)
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

pub(in crate::adapters::local_operations) enum NameKept {
    Taken,
    Gone,
    Refused,
    Failed(String),
}

type OpenFolder = Result<Arc<OwnedFd>, String>;

fn restore_native_names(paths: &[(PathBuf, PathBuf)]) -> Vec<Result<(), NameKept>> {
    let mut folders = HashMap::<&Path, OpenFolder>::new();
    let mut open = |folder| {
        folders
            .entry(folder)
            .or_insert_with(|| open_local_parent_directory(folder).map(Arc::new))
            .clone()
    };
    paths
        .iter()
        .map(|(from, to)| restore_native_name(&mut open, from, to))
        .collect()
}

fn restore_native_name<'a>(
    open: &mut impl FnMut(&'a Path) -> OpenFolder,
    from: &'a Path,
    to: &'a Path,
) -> Result<(), NameKept> {
    let (Some(from_folder), Some(from_name), Some(to_folder), Some(to_name)) =
        (from.parent(), from.file_name(), to.parent(), to.file_name())
    else {
        return Err(NameKept::Failed(crate::i18n::tr(
            "Invalid move destination",
        )));
    };
    let folders = open(from_folder).and_then(|from| Ok((from, open(to_folder)?)));
    let (from_dir, to_dir) = folders.map_err(|error| unopened(from, error))?;
    let renamed = placement::rename_in_place(
        fs(),
        Entry::new(&*from_dir, from_name),
        Entry::new(&*to_dir, to_name),
    );
    match renamed {
        Ok(_) => Ok(()),
        Err(RenameFailure::Place(PlaceError::Exists)) => Err(NameKept::Taken),
        Err(RenameFailure::Place(error)) if error.refuses_name() => Err(NameKept::Refused),
        Err(RenameFailure::Place(PlaceError::Io(rustix::io::Errno::NOENT))) => Err(NameKept::Gone),
        Err(failure) => Err(NameKept::Failed(crate::services::gio_error_detail(
            &rename_failure_error(failure, from, to),
        ))),
    }
}

fn unopened(from: &Path, error: String) -> NameKept {
    if std::fs::symlink_metadata(from).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) {
        return NameKept::Gone;
    }
    NameKept::Failed(crate::services::gio_error_detail(&io_error(error)))
}

pub(in crate::adapters::local_operations) async fn restore_name(
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
