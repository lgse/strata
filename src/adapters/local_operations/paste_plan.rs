// SPDX-License-Identifier: MIT

use super::*;

mod parallel;
mod sizes;

pub(super) use parallel::*;
pub(super) use sizes::*;

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
    (crossing, duplicate, kinds): (&[bool], &[bool], &[Option<gio::FileType>]),
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

pub(super) async fn first_free_target<T, F: Future<Output = Result<T, glib::Error>>>(
    target: &gio::File,
    (taken, is_directory): (Taken, bool),
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
            if let Some(result) = try_free_target(candidate, cancellable, &mut attempt).await {
                return result;
            }
        }
    }
    cancellable.set_error_if_cancelled()?;
    Err(translated_io_error(
        "Could not find an unused duplicate name",
    ))
}

/// `None` when the candidate is taken.
async fn try_free_target<T, F: Future<Output = Result<T, glib::Error>>>(
    candidate: gio::File,
    cancellable: &gio::Cancellable,
    attempt: &mut impl FnMut(gio::File) -> F,
) -> Option<Result<(gio::File, T), glib::Error>> {
    if query_exists(&candidate, cancellable).await {
        return None;
    }
    match attempt(candidate.clone()).await {
        Err(error) if error.matches(gio::IOErrorEnum::Exists) => None,
        result => Some(result.map(|value| (candidate, value))),
    }
}
