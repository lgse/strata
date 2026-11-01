// SPDX-License-Identifier: MIT

//! Top-level items of one paste whose names differ only in case. On a
//! case-insensitive destination whose rules do not fold, such as a casefolded
//! folder, only looking a later name up shows that an earlier item took it.

use std::{collections::HashMap, ffi::OsString, path::PathBuf};

use gtk::{gio, prelude::*};

use super::{
    copy_engine::stage::collision_key,
    fs_ops::{Entry, fs},
    native_fs::{LocalFileIdentity, open_local_parent_directory},
    placement,
};

pub(super) type Saved = (PathBuf, LocalFileIdentity);

pub(super) struct CaseVariants {
    /// The other items of each one, by index, whose names differ only in case.
    others: HashMap<usize, Vec<usize>>,
    saved: HashMap<usize, Saved>,
}

impl CaseVariants {
    pub(super) fn new(names: impl IntoIterator<Item = (usize, OsString)>) -> Self {
        let mut keyed = HashMap::<OsString, Vec<(usize, OsString)>>::new();
        for (index, name) in names {
            keyed
                .entry(collision_key(&name))
                .or_default()
                .push((index, name));
        }
        let mut others = HashMap::new();
        for items in keyed.values().filter(|items| items.len() > 1) {
            for (index, name) in items {
                let spelled_otherwise = items
                    .iter()
                    .filter(|(_, other)| other != name)
                    .map(|(other, _)| *other)
                    .collect::<Vec<_>>();
                if !spelled_otherwise.is_empty() {
                    others.insert(*index, spelled_otherwise);
                }
            }
        }
        Self {
            others,
            saved: HashMap::new(),
        }
    }

    pub(super) fn follows(&self, index: usize, earlier: usize) -> bool {
        earlier < index
            && self
                .others
                .get(&index)
                .is_some_and(|others| others.contains(&earlier))
    }

    /// Records what `index` was saved as, where a later item may find it.
    pub(super) async fn saved(&mut self, index: usize, target: &gio::File) {
        let precedes = self
            .others
            .get(&index)
            .is_some_and(|others| others.iter().any(|&other| other > index));
        if precedes
            && let Some(path) = target.path()
            && let Some(identity) = identity(target).await
        {
            self.saved.insert(index, (path, identity));
        }
    }

    pub(super) fn earlier(&self, index: usize) -> Vec<Saved> {
        self.others
            .get(&index)
            .into_iter()
            .flatten()
            .filter(|&&other| other < index)
            .filter_map(|other| self.saved.get(other).cloned())
            .collect()
    }
}

pub(super) async fn taken_by(target: &gio::File, earlier: Vec<Saved>) -> bool {
    let Some(path) = target.path().filter(|_| !earlier.is_empty()) else {
        return false;
    };
    gio::spawn_blocking(move || {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        let Ok(dir) = open_local_parent_directory(parent) else {
            return false;
        };
        let to = Entry::new(&dir, name);
        let Ok(found) = fs().stat(to, false) else {
            return false;
        };
        earlier.iter().any(|(saved, identity)| {
            found.identity == *identity
                || saved
                    .parent()
                    .zip(saved.file_name())
                    .and_then(|(parent, name)| {
                        let saved_dir = open_local_parent_directory(parent).ok()?;
                        placement::alias_of(fs(), Entry::new(&saved_dir, name), to)
                    })
                    .is_some_and(|alias| alias == *identity)
        })
    })
    .await
    .unwrap_or(false)
}

async fn identity(target: &gio::File) -> Option<LocalFileIdentity> {
    let path = target.path()?;
    gio::spawn_blocking(move || {
        let (parent, name) = (path.parent()?, path.file_name()?);
        let dir = open_local_parent_directory(parent).ok()?;
        fs().stat(Entry::new(&dir, name), false)
            .ok()
            .map(|stat| stat.identity)
    })
    .await
    .ok()
    .flatten()
}
