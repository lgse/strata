// SPDX-License-Identifier: GPL-3.0-or-later

//! Restore-target checks for freedesktop.org Trash.
//!
//! `trash::orig-path` / `.trashinfo` `Path=` is untrusted metadata. Restore is
//! allowed only when the destination is on the same volume as the physical
//! trash `files/` entry, using the same GIO filesystem ids as cross-volume
//! drops.

#[cfg(test)]
mod tests;

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        gio_file_for_location,
        volume::{MountTable, REMOTE_QUERY_TIMEOUT, restore_volume_relation},
    },
    model::Location,
    services::VolumeRelation,
};

const MAX_RESTORE_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoreTargetError {
    message: String,
}

impl RestoreTargetError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for RestoreTargetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestorePlan {
    pub(crate) source_path: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) allowed_root: PathBuf,
    pub(crate) trash_info: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct RestoreContext {
    pub(crate) home_trash_root: PathBuf,
    pub(crate) uid: u32,
    pub(crate) mounts: MountTable,
}

impl RestoreContext {
    pub(crate) fn current() -> Self {
        Self {
            home_trash_root: glib::user_data_dir().join("Trash"),
            uid: rustix::process::getuid().as_raw(),
            mounts: MountTable::current(),
        }
    }
}

pub(crate) fn decode_trashinfo_path(encoded: &str) -> Option<PathBuf> {
    let encoded = encoded.trim();
    if encoded.is_empty() {
        return None;
    }
    let encoded = encoded
        .strip_prefix("file://")
        .map(|rest| rest.strip_prefix("//").unwrap_or(rest))
        .unwrap_or(encoded);
    let decoded = percent_decode(encoded.as_bytes())?;
    if decoded.is_empty() || decoded.contains(&0) || decoded.len() > MAX_RESTORE_PATH_BYTES {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

pub(crate) fn plan_restore_from_known_paths(
    source_path: &Path,
    orig_path: &Path,
    trash_root: &Path,
    trash_info: Option<PathBuf>,
    context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    let (destination, allowed_root) =
        resolve_restore_destination(orig_path, source_path, trash_root, context)?;
    if !destination.starts_with(&allowed_root) {
        return Err(escaped_restore_error());
    }
    if context.mounts.mount_point_for(&destination) != Some(allowed_root.as_path()) {
        return Err(RestoreTargetError::new(
            "The original location crosses a bind mount or subvolume boundary and cannot be restored",
        ));
    }
    if restore_volume_relation(
        &Location::local(source_path),
        &Location::local(&destination),
    ) != VolumeRelation::Same
    {
        return Err(escaped_restore_error());
    }
    let trash_tree = trash_tree_root(trash_root);
    if path_is_within(&destination, &trash_tree) {
        return Err(RestoreTargetError::new(
            "The original location must not be inside the trash directory",
        ));
    }
    let trash_tree = trash_tree
        .canonicalize()
        .unwrap_or_else(|_| trash_tree.clone());
    if path_is_within(&destination, &trash_tree) {
        return Err(RestoreTargetError::new(
            "The original location must not be inside the trash directory",
        ));
    }
    Ok(RestorePlan {
        source_path: source_path.to_path_buf(),
        destination,
        allowed_root,
        trash_info,
    })
}

pub(crate) async fn plan_restore_for_location(
    source: &Location,
    original_target: Option<&Location>,
    trash_info: Option<&Path>,
    physical_path: Option<&Path>,
    context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    // GVfs names volume items after their escaped physical path and resolves
    // relative `Path=` values itself, so ask it for the physical entry rather
    // than guessing from the `trash:///` basename.
    let known_files = physical_path
        .and_then(trash_item_from_files_path)
        .or_else(|| source.native_path().and_then(trash_item_from_files_path));
    let gio_item = if known_files.is_none() && trash_info.is_none() {
        Some(with_lookup_timeout(query_gio_trash_item(source)).await?)
    } else {
        None
    };
    let source = source.clone();
    let original_target = original_target.cloned();
    let trash_info = trash_info.map(Path::to_path_buf);
    let gio_orig = gio_item.as_ref().map(|item| item.orig_path.clone());
    let gio_target = gio_item.and_then(|item| item.target_path);
    let known_files = known_files.map(|item| item.source_path);
    let context = context.clone();
    with_lookup_timeout(async move {
        gio::spawn_blocking(move || {
            let orig_path = if let Some(path) = original_target
                .as_ref()
                .and_then(Location::native_path)
                .map(Path::to_path_buf)
            {
                path
            } else if let Some(path) = gio_orig {
                path
            } else {
                orig_path_from_known(&source, trash_info.as_deref(), known_files.as_deref())?
            };
            let discovered = known_files
                .as_deref()
                .and_then(trash_item_from_files_path)
                .or_else(|| gio_target.as_deref().and_then(trash_item_from_files_path))
                .map_or_else(
                    || {
                        discover_trash_item(
                            &source,
                            trash_info.as_deref(),
                            Some(&orig_path),
                            &context,
                        )
                    },
                    Ok,
                )?;
            plan_restore_from_known_paths(
                &discovered.source_path,
                &orig_path,
                &discovered.trash_root,
                discovered.trash_info,
                &context,
            )
        })
        .await
        .map_err(|_| RestoreTargetError::new("Restore lookup failed"))?
    })
    .await
}

/// Shares one mount snapshot and bounds concurrent filesystem lookups. Dropping
/// the batch drops its active futures instead of detaching per-item tasks.
pub(crate) async fn restore_destinations_for_locations(
    items: Vec<(Location, Option<PathBuf>)>,
) -> Vec<Result<PathBuf, RestoreTargetError>> {
    let context = RestoreContext::current();
    resolve_batch(items, |(location, physical_path)| {
        let context = &context;
        async move {
            plan_restore_for_location(&location, None, None, physical_path.as_deref(), context)
                .await
                .map(|plan| plan.destination)
        }
    })
    .await
}

const MAX_RESTORE_LOOKUPS: usize = 8;

async fn resolve_batch<T, R, F: Future<Output = R>>(
    items: Vec<T>,
    resolve: impl Fn(T) -> F,
) -> Vec<R> {
    let mut results = (0..items.len()).map(|_| None).collect::<Vec<_>>();
    let mut queued = items.into_iter().enumerate();
    let mut active = Vec::new();
    std::future::poll_fn(|cx| {
        loop {
            while active.len() < MAX_RESTORE_LOOKUPS {
                let Some((index, item)) = queued.next() else {
                    break;
                };
                active.push((index, Box::pin(resolve(item))));
            }
            if active.is_empty() {
                return std::task::Poll::Ready(());
            }
            let mut completed = false;
            let mut slot = 0;
            while slot < active.len() {
                match active[slot].1.as_mut().poll(cx) {
                    std::task::Poll::Ready(result) => {
                        let (index, _) = active.swap_remove(slot);
                        results[index] = Some(result);
                        completed = true;
                    }
                    std::task::Poll::Pending => slot += 1,
                }
            }
            if !completed {
                return std::task::Poll::Pending;
            }
        }
    })
    .await;
    results
        .into_iter()
        .map(|result| result.expect("every restore lookup completed"))
        .collect()
}

async fn with_lookup_timeout<T>(
    work: impl Future<Output = Result<T, RestoreTargetError>>,
) -> Result<T, RestoreTargetError> {
    futures_lite::future::or(work, async {
        glib::timeout_future(REMOTE_QUERY_TIMEOUT).await;
        Err(RestoreTargetError::new(
            "Timed out looking up the original location",
        ))
    })
    .await
}

fn resolve_restore_destination(
    orig_path: &Path,
    source_path: &Path,
    trash_root: &Path,
    context: &RestoreContext,
) -> Result<(PathBuf, PathBuf), RestoreTargetError> {
    if orig_path.as_os_str().is_empty()
        || orig_path.as_os_str().as_bytes().contains(&0)
        || orig_path.as_os_str().len() > MAX_RESTORE_PATH_BYTES
        || orig_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(RestoreTargetError::new("The original location is invalid"));
    }
    let allowed_root = context
        .mounts
        .mount_point_for(source_path)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            RestoreTargetError::new(
                "Unable to determine the trash volume because the mount table is unavailable",
            )
        })?;
    let absolute = if orig_path.is_absolute() {
        orig_path.to_path_buf()
    } else {
        topdir_for_trash_root(trash_root)
            .ok_or_else(|| RestoreTargetError::new("The original location is invalid"))?
            .join(orig_path)
    };
    let normalized = lexically_normalize(&absolute)
        .ok_or_else(|| RestoreTargetError::new("The original location is invalid"))?;
    let destination = canonical_restore_destination(&normalized)?;
    if destination.file_name().is_none() {
        return Err(RestoreTargetError::new("The original location is invalid"));
    }
    let allowed_root = allowed_root
        .canonicalize()
        .map_err(|_| escaped_restore_error())?;
    Ok((destination, allowed_root))
}

struct DiscoveredTrashItem {
    trash_root: PathBuf,
    source_path: PathBuf,
    trash_info: Option<PathBuf>,
}

fn discover_trash_item(
    source: &Location,
    known_info: Option<&Path>,
    orig_path: Option<&Path>,
    context: &RestoreContext,
) -> Result<DiscoveredTrashItem, RestoreTargetError> {
    if let Some(info) = known_info
        && let Some(trash_root) = trash_root_from_info_path(info)
    {
        let source_path = source
            .native_path()
            .map(Path::to_path_buf)
            .or_else(|| files_path_for_info_path(info))
            .ok_or_else(|| RestoreTargetError::new("The original location is unavailable"))?;
        return Ok(DiscoveredTrashItem {
            trash_root,
            source_path,
            trash_info: Some(info.to_path_buf()),
        });
    }
    if let Some(discovered) = source.native_path().and_then(trash_item_from_files_path) {
        return Ok(discovered);
    }

    let name = source.file_name().ok_or_else(|| {
        RestoreTargetError::new("Unable to determine where this item was deleted from")
    })?;
    let mut named = Vec::new();
    let mut by_orig = None;
    for trash_root in candidate_trash_roots(context, orig_path) {
        if by_orig.is_none()
            && let Some(orig_path) = orig_path
            && let Some(found) = find_trash_item_by_orig_path(&trash_root, orig_path)
        {
            by_orig = Some(found);
        }
        let source_path = trash_root.join("files").join(&name);
        if std::fs::symlink_metadata(&source_path).is_ok() {
            named.push(DiscoveredTrashItem {
                trash_info: info_path_for_files_path(&source_path),
                trash_root: trash_root.clone(),
                source_path,
            });
        }
    }
    if let Some(orig) = by_orig {
        return Ok(orig);
    }
    if named.len() == 1 {
        return Ok(named.remove(0));
    }
    Err(RestoreTargetError::new(
        "Unable to determine where this item was deleted from",
    ))
}

fn orig_path_from_known(
    location: &Location,
    known_info: Option<&Path>,
    physical_path: Option<&Path>,
) -> Result<PathBuf, RestoreTargetError> {
    if let Some(info) = known_info
        && let Some(path) = orig_path_from_trashinfo(info)
    {
        return Ok(path);
    }
    for path in [location.native_path(), physical_path]
        .into_iter()
        .flatten()
    {
        if let Some(info) = info_path_for_files_path(path)
            && let Some(orig) = orig_path_from_trashinfo(&info)
        {
            return Ok(orig);
        }
    }
    Err(RestoreTargetError::new(
        "The original location is unavailable",
    ))
}

struct GioTrashItem {
    orig_path: PathBuf,
    target_path: Option<PathBuf>,
}

async fn query_gio_trash_item(location: &Location) -> Result<GioTrashItem, RestoreTargetError> {
    let file = gio_file_for_location(location);
    let info = file
        .query_info_future(
            "trash::orig-path,standard::target-uri",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
        .map_err(|error| RestoreTargetError::new(error.to_string()))?;
    let Some(original) = info.attribute_byte_string("trash::orig-path") else {
        return Err(RestoreTargetError::new(
            "The original location is unavailable",
        ));
    };
    let orig_path = PathBuf::from(original.as_str());
    if orig_path.as_os_str().is_empty() {
        return Err(RestoreTargetError::new(
            "The original location is unavailable",
        ));
    }
    let target_path = info
        .attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)
        .and_then(|target| glib::filename_from_uri(&target).ok())
        .and_then(|(path, hostname)| {
            hostname
                .is_none_or(|host| host.eq_ignore_ascii_case("localhost"))
                .then_some(path)
        });
    Ok(GioTrashItem {
        orig_path,
        target_path,
    })
}

/// Accepts only a `<trash root>/files/<name>` entry so a reported target that
/// is not shaped like a trash item never becomes the restore source.
fn trash_item_from_files_path(path: &Path) -> Option<DiscoveredTrashItem> {
    let trash_root = trash_root_from_files_path(path)?;
    Some(DiscoveredTrashItem {
        trash_info: info_path_for_files_path(path),
        trash_root,
        source_path: path.to_path_buf(),
    })
}

fn orig_path_from_trashinfo(info_path: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(info_path).ok()?;
    contents
        .lines()
        .find_map(|line| line.strip_prefix("Path="))
        .and_then(decode_trashinfo_path)
}

fn find_trash_item_by_orig_path(
    trash_root: &Path,
    orig_path: &Path,
) -> Option<DiscoveredTrashItem> {
    let info_root = trash_root.join("info");
    let files_root = trash_root.join("files");
    let topdir = topdir_for_trash_root(trash_root)?;
    let wanted = lexically_normalize(orig_path)?;
    let infos = std::fs::read_dir(info_root).ok()?;
    for info in infos.flatten() {
        let info_path = info.path();
        let Some(name) = info_path.file_name() else {
            continue;
        };
        let Some(file_name) = name.as_bytes().strip_suffix(b".trashinfo") else {
            continue;
        };
        let Some(path) = orig_path_from_trashinfo(&info_path) else {
            continue;
        };
        let path = if path.is_absolute() {
            path
        } else {
            topdir.join(path)
        };
        if lexically_normalize(&path).as_deref() != Some(wanted.as_path()) {
            continue;
        }
        let source_path = files_root.join(OsStr::from_bytes(file_name));
        if std::fs::symlink_metadata(&source_path).is_err() {
            continue;
        }
        return Some(DiscoveredTrashItem {
            trash_root: trash_root.to_path_buf(),
            source_path,
            trash_info: Some(info_path),
        });
    }
    None
}

fn candidate_trash_roots(context: &RestoreContext, orig_path: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec![context.home_trash_root.clone()];
    let mounts: Vec<&Path> = if let Some(orig) = orig_path.filter(|path| path.is_absolute()) {
        context.mounts.mount_point_for(orig).into_iter().collect()
    } else {
        context.mounts.trash_scan_mounts().collect()
    };
    for mount in mounts {
        roots.push(mount.join(format!(".Trash-{}", context.uid)));
        if let Some(shared) = valid_shared_trash_dir(mount, context.uid) {
            roots.push(shared);
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

fn valid_shared_trash_dir(mount: &Path, uid: u32) -> Option<PathBuf> {
    let trash = mount.join(".Trash");
    let meta = std::fs::symlink_metadata(&trash).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    if meta.mode() & 0o1000 == 0 {
        return None;
    }
    Some(trash.join(uid.to_string()))
}

/// The shared `$topdir/.Trash` directory that holds `$uid` trash roots.
fn shared_trash_dir(trash_root: &Path) -> Option<&Path> {
    trash_root
        .parent()
        .filter(|parent| parent.file_name() == Some(OsStr::new(".Trash")))
}

/// Outermost directory that belongs to the trash: the shared `.Trash` dir for
/// the shared layout, otherwise the trash root itself.
fn trash_tree_root(trash_root: &Path) -> PathBuf {
    shared_trash_dir(trash_root)
        .unwrap_or(trash_root)
        .to_path_buf()
}

pub(crate) fn trash_root_from_files_path(path: &Path) -> Option<PathBuf> {
    let files = path.parent()?;
    if files.file_name()? != "files" {
        return None;
    }
    files.parent().map(Path::to_path_buf)
}

/// Directory a relative `.trashinfo` `Path=` is resolved against. That is the
/// trash directory's parent, except for the shared `$topdir/.Trash/$uid`
/// layout, where the spec anchors relative paths at `$topdir` rather than at
/// the intervening `.Trash`.
fn topdir_for_trash_root(trash_root: &Path) -> Option<&Path> {
    match shared_trash_dir(trash_root) {
        Some(shared) => shared.parent(),
        None => trash_root.parent(),
    }
}

fn trash_root_from_info_path(path: &Path) -> Option<PathBuf> {
    let info = path.parent()?;
    if info.file_name()? != "info" {
        return None;
    }
    info.parent().map(Path::to_path_buf)
}

fn info_path_for_files_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let trash_root = trash_root_from_files_path(path)?;
    let mut info_name = name.as_bytes().to_vec();
    info_name.extend_from_slice(b".trashinfo");
    Some(trash_root.join("info").join(OsStr::from_bytes(&info_name)))
}

fn files_path_for_info_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let file_name = name.as_bytes().strip_suffix(b".trashinfo")?;
    let trash_root = trash_root_from_info_path(path)?;
    Some(trash_root.join("files").join(OsStr::from_bytes(file_name)))
}

fn percent_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'%' => {
                let hex = input.get(index + 1..index + 3)?;
                let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                decoded.push(value);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Some(decoded)
}

fn lexically_normalize(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
            Component::Normal(part) => {
                if part.as_bytes().is_empty() || part.as_bytes().contains(&0) {
                    return None;
                }
                normalized.push(part);
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return None;
    }
    Some(normalized)
}

fn canonical_restore_destination(path: &Path) -> Result<PathBuf, RestoreTargetError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| RestoreTargetError::new("The original location is invalid"))?;
    let parent = path
        .parent()
        .ok_or_else(|| RestoreTargetError::new("The original location is invalid"))?;
    let mut existing = parent.to_path_buf();
    let mut missing = Vec::new();
    while !existing.as_os_str().is_empty() && !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        missing.push(name.to_os_string());
        match existing.parent() {
            Some(parent) => existing = parent.to_path_buf(),
            None => break,
        }
    }
    let mut canonical = if existing.exists() {
        existing
            .canonicalize()
            .map_err(|_| escaped_restore_error())?
    } else {
        existing
    };
    for name in missing.into_iter().rev() {
        if name.as_bytes() == b".." || name.as_bytes() == b"." || name.as_bytes().contains(&0) {
            return Err(RestoreTargetError::new("The original location is invalid"));
        }
        canonical.push(name);
    }
    canonical.push(file_name);
    Ok(canonical)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn escaped_restore_error() -> RestoreTargetError {
    RestoreTargetError::new(
        "The original location is outside the trash volume and cannot be restored.",
    )
}
