// SPDX-License-Identifier: MIT

//! Where a paste would put each of its items, which the conflict prompt
//! shows before the paste starts.

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::Path,
};

use gtk::{gio, glib, prelude::*};

use super::{
    MAX_LOCAL_IO_WORKERS, await_cancellable,
    copy_engine::stage::collision_key,
    duplicates,
    fs_ops::{Entry, fs},
    mount_profile::NamePolicy,
    open_local_parent_directory, paste_targets, placement_reason, run_blocking,
    transfer_source_name, was_cancelled,
};
use crate::{
    adapters::gio_file_for_location,
    model::Location,
    services::{NameMode, ShownName},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransferTarget {
    pub name: Option<ShownName>,
    pub collision: Option<Collision>,
    pub fixes_inside: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Collision {
    pub mergeable: bool,
}

/// Nothing collides with a link the paste leaves out, as the destination is known not to hold
/// links.
pub(crate) async fn transfer_targets(
    destination: &Location,
    sources: &[Location],
    move_sources: bool,
    cancellable: &gio::Cancellable,
) -> Result<Vec<TransferTarget>, glib::Error> {
    let destination = gio_file_for_location(destination);
    let sources = sources
        .iter()
        .map(gio_file_for_location)
        .collect::<Vec<_>>();
    let names = NamePolicy::new(NameMode::Fix);
    let pasted_into = names.destination(&destination, &sources, cancellable).await;
    let remote = sources
        .iter()
        .map(|source| (!source.is_native()).then(|| source.clone()))
        .collect::<Vec<_>>();
    let remote = query_each(&remote, REMOTE_ITEM_ATTRIBUTES, cancellable)
        .await?
        .into_iter()
        .map(|info| info.map(|info| kind_and_filesystem(&info)))
        .collect::<Vec<_>>();
    let blocking = cancellable.clone();
    let into = destination.clone();
    let (targets, folders, existing) = run_blocking(move || {
        let (kinds, filesystems) = item_kinds(&sources, remote, &blocking)?;
        let profile = pasted_into.profile;
        let rules = names.applied(profile.names, profile.strictness);
        let crossing = pasted_into.fixing(&filesystems);
        let own_names = sources.iter().map(transfer_source_name).collect::<Vec<_>>();
        let renamable = vec![true; own_names.len()];
        let duplicate = duplicates(&sources, &into, move_sources);
        let planned = paste_targets(
            &own_names,
            &rules,
            (&crossing, &renamable, &duplicate),
            &kinds,
        )
        .map_err(|error| glib::Error::new(gio::IOErrorEnum::Exists, &placement_reason(error)))?;
        let left_out = |index: usize| {
            kinds[index] == Some(gio::FileType::SymbolicLink)
                && profile.symlinks == Some(false)
                && (crossing[index] || !move_sources)
        };
        let looked_up = planned
            .iter()
            .enumerate()
            .map(|(index, planned)| {
                let name = planned.as_ref()?.target.clone();
                let duplicate = sources[index].equal(&into.child(&name));
                (!duplicate && !left_out(index)).then_some(name)
            })
            .collect::<Vec<_>>();
        let existing = match into.path().filter(|_| into.is_native()) {
            Some(path) => Existing::Found(existing_folders(&path, &looked_up)),
            None => Existing::Remote(looked_up),
        };
        let folders = kinds
            .iter()
            .map(|kind| *kind == Some(gio::FileType::Directory))
            .collect::<Vec<_>>();
        let targets = planned
            .into_iter()
            .enumerate()
            .map(|(index, planned)| TransferTarget {
                name: planned.map(|planned| ShownName {
                    name: planned.target,
                    cause: planned.cause,
                }),
                collision: None,
                fixes_inside: folders[index] && crossing[index] && rules.changes_characters(),
            })
            .collect::<Vec<_>>();
        Ok((targets, folders, existing))
    })
    .await?;
    let existing = match existing {
        Existing::Found(existing) => existing,
        Existing::Remote(names) => {
            let children = names
                .iter()
                .map(|name| name.as_ref().map(|name| destination.child(name)))
                .collect::<Vec<_>>();
            query_each(&children, gio::FILE_ATTRIBUTE_STANDARD_TYPE, cancellable)
                .await?
                .into_iter()
                .map(|info| info.map(|info| info.file_type() == gio::FileType::Directory))
                .collect()
        }
    };
    Ok(targets
        .into_iter()
        .zip(folders)
        .zip(existing)
        .map(|((target, folder), existing)| TransferTarget {
            collision: existing.map(|existing_folder| Collision {
                mergeable: folder && existing_folder,
            }),
            ..target
        })
        .collect())
}

const REMOTE_ITEM_ATTRIBUTES: &str = "standard::type,id::filesystem";

enum Existing {
    Found(Vec<Option<bool>>),
    Remote(Vec<Option<OsString>>),
}

async fn query_each(
    files: &[Option<gio::File>],
    attributes: &'static str,
    cancellable: &gio::Cancellable,
) -> Result<Vec<Option<gio::FileInfo>>, glib::Error> {
    let context = glib::MainContext::default();
    let mut infos = Vec::with_capacity(files.len());
    for chunk in files.chunks(MAX_LOCAL_IO_WORKERS) {
        let tasks = chunk
            .iter()
            .map(|file| {
                let file = file.clone()?;
                let cancellable = cancellable.clone();
                Some(context.spawn_local(async move {
                    await_cancellable(&file, &cancellable, move |file, cancellable, result| {
                        file.query_info_async(
                            attributes,
                            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                            glib::Priority::DEFAULT,
                            Some(cancellable),
                            move |output| result.resolve(output),
                        );
                    })
                    .await
                }))
            })
            .collect::<Vec<_>>();
        for task in tasks {
            let info = match task {
                Some(task) => match task.await {
                    Ok(Ok(info)) => Some(info),
                    Ok(Err(error)) if was_cancelled(&error) => return Err(error),
                    Ok(Err(_)) | Err(_) => None,
                },
                None => None,
            };
            infos.push(info);
        }
    }
    Ok(infos)
}

type KindAndFilesystem = (gio::FileType, Option<glib::GString>);
type ItemKinds = (Vec<Option<gio::FileType>>, Vec<Option<glib::GString>>);

fn kind_and_filesystem(info: &gio::FileInfo) -> KindAndFilesystem {
    (
        info.file_type(),
        info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM),
    )
}

fn item_kinds(
    items: &[gio::File],
    remote: Vec<Option<KindAndFilesystem>>,
    cancellable: &gio::Cancellable,
) -> Result<ItemKinds, glib::Error> {
    let mut kinds = Vec::with_capacity(items.len());
    let mut filesystems = Vec::with_capacity(items.len());
    for (item, remote) in items.iter().zip(remote) {
        let read = if item.is_native() {
            match item.query_info(
                gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(cancellable),
            ) {
                Ok(info) => Some(kind_and_filesystem(&info)),
                Err(error) if was_cancelled(&error) => return Err(error),
                Err(_) => None,
            }
        } else {
            remote
        };
        let (kind, filesystem) = read.unzip();
        kinds.push(kind);
        filesystems.push(filesystem.flatten());
    }
    Ok((kinds, filesystems))
}

/// Up to this many names are looked up one by one; more are first looked
/// for in the folder's listing.
pub(super) const LOOKUPS_WITHOUT_LISTING: usize = 32;

fn existing_folders(path: &Path, names: &[Option<OsString>]) -> Vec<Option<bool>> {
    let fs = fs();
    let wanted = names.iter().flatten().count();
    let folder = (wanted > 0)
        .then(|| open_local_parent_directory(path).ok())
        .flatten();
    let Some(folder) = folder else {
        return vec![None; names.len()];
    };
    let listed = (wanted > LOOKUPS_WITHOUT_LISTING)
        .then(|| fs.names(Entry::new(&folder, OsStr::new("."))).ok())
        .flatten()
        .map(|listed| {
            listed
                .iter()
                .map(|name| collision_key(name))
                .collect::<HashSet<_>>()
        });
    names
        .iter()
        .map(|name| {
            let name = name.as_deref()?;
            if listed
                .as_ref()
                .is_some_and(|listed| !listed.contains(&collision_key(name)))
            {
                return None;
            }
            fs.stat(Entry::new(&folder, name), false)
                .ok()
                .map(|stat| stat.is_directory())
        })
        .collect()
}
