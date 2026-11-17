// SPDX-License-Identifier: MIT

//! Validation and submission of 10xer's `create ›` (`a`) and `rename ›` (`r` / `F2`) prompts
//! (`src/ui/window/keyboard/prompts.rs`, `src/ui/browser/file_commands.rs`).

use std::{ffi::OsStr, path::PathBuf};

use rune::Tx;
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::{
    adapters::{
        gio_file_for_location, typed_folder_blocked, typed_name_refusal, typed_rename_is_taken,
    },
    services::{TypedPath, validate_basename},
};

use super::{Asked, close_and_return, spec::PromptKind};
use crate::{App, new_item, rename, shell};

#[derive(Debug, PartialEq, Eq)]
enum CreateRefusal {
    Unsupported,
    Invalid(&'static str),
    Exists(String),
    Unstorable(String),
    NotAFolder(String),
}

struct TypedEntry {
    parent: Location,
    name: String,
    directory: bool,
}

impl TypedEntry {
    async fn refusal(&self) -> Option<CreateRefusal> {
        let folder = gio_file_for_location(&self.parent);
        if let Some(message) = typed_name_refusal(&folder, &self.name, self.directory).await {
            return Some(CreateRefusal::Unstorable(message));
        }
        let path = self.parent.native_path()?.join(&self.name);
        let exists = gio::spawn_blocking(move || std::fs::symlink_metadata(path).is_ok())
            .await
            .unwrap_or(false);
        exists.then(|| CreateRefusal::Exists(self.name.clone()))
    }
}

struct TypedRename {
    depth: usize,
    entry: FileEntry,
    typed: String,
    name: String,
    path: Option<TypedPath>,
}

impl TypedRename {
    fn components(&self) -> Vec<(&str, bool)> {
        let Some(path) = &self.path else {
            return vec![(self.name.as_str(), false)];
        };
        path.folders
            .iter()
            .map(|folder| (folder.as_str(), true))
            .chain([(path.name.as_str(), path.folder)])
            .collect()
    }

    fn start_folder(&self) -> Option<Location> {
        let parent = self.entry.location.parent()?;
        match &self.path {
            Some(path) => path.start(&parent),
            None => Some(parent),
        }
    }

    fn native_paths(&self) -> Option<(PathBuf, PathBuf)> {
        let current = self.entry.location.native_path()?;
        let wanted = self.components().into_iter().fold(
            self.start_folder()?.native_path()?.to_path_buf(),
            |path, (name, _)| path.join(name),
        );
        Some((current.to_path_buf(), wanted))
    }

    fn folders(&self) -> Vec<String> {
        self.path
            .as_ref()
            .map(|path| path.folders.clone())
            .unwrap_or_default()
    }
}

fn prepare_typed_entry(app: &App, text: &str) -> Result<TypedEntry, CreateRefusal> {
    let (name, directory) = match text.strip_suffix('/') {
        Some(name) => (name, true),
        None => (text, false),
    };
    validate_basename(name).map_err(CreateRefusal::Invalid)?;
    let Some((_, parent)) = new_item::focused_create_parent(app) else {
        return Err(CreateRefusal::Unsupported);
    };
    Ok(TypedEntry {
        parent,
        name: name.to_owned(),
        directory,
    })
}

/// Whether `name` is already `entry`'s, as shown or on disk.
fn is_named(entry: &FileEntry, name: &str) -> bool {
    entry.display_name == name || entry.native_name == OsStr::new(name)
}

fn prepare_typed_rename(
    depth: usize,
    entry: FileEntry,
    text: &str,
) -> Result<Option<TypedRename>, CreateRefusal> {
    if !rename::can_rename(&entry) {
        return Err(CreateRefusal::Unsupported);
    }
    let (name, path) = if let Some(parent) = entry
        .location
        .native_path()
        .and_then(|_| entry.location.parent())
    {
        let path = TypedPath::parse(text, entry.is_directory(), false)
            .map_err(CreateRefusal::Invalid)?
            .within(&parent);
        if path.is_rename_of(entry.is_directory()) {
            (path.name, None)
        } else {
            (text.to_owned(), Some(path))
        }
    } else {
        validate_basename(text).map_err(CreateRefusal::Invalid)?;
        (text.to_owned(), None)
    };
    if path.is_none() && is_named(&entry, &name) {
        return Ok(None);
    }
    Ok(Some(TypedRename {
        depth,
        entry,
        typed: text.to_owned(),
        name,
        path,
    }))
}

fn refuse(app: &mut App, asked: Asked, refusal: CreateRefusal, tx: &mut Tx) {
    let hint = match refusal {
        CreateRefusal::Invalid(message) => tr(message),
        CreateRefusal::Exists(name) => {
            rust_i18n::t!("\u{201c}%{name}\u{201d} already exists", name = name).into_owned()
        }
        CreateRefusal::Unstorable(message) => message,
        CreateRefusal::NotAFolder(name) => {
            rust_i18n::t!("\u{201c}%{name}\u{201d} is not a folder", name = name).into_owned()
        }
        CreateRefusal::Unsupported => {
            close_and_return(app, tx);
            let msg = if asked.kind == PromptKind::Create {
                "Can\u{2019}t create items here"
            } else {
                "Can\u{2019}t rename items here"
            };
            app.tenxer.flash.show(tr(msg), tx);
            return;
        }
    };
    let _ = app.tenxer.prompt.refuse(asked.serial, hint);
}

pub(super) fn submit_create(app: &mut App, serial: u64, text: &str, tx: &mut Tx) {
    let asked = Asked {
        kind: PromptKind::Create,
        serial,
    };
    let entry = match prepare_typed_entry(app, text) {
        Ok(entry) => entry,
        Err(refusal) => return refuse(app, asked, refusal, tx),
    };
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let refusal = entry.refusal().await;
        later.then(move |app, tx| {
            if !asked.holds(app) {
                return false;
            }
            match refusal {
                Some(refusal) => refuse(app, asked, refusal, tx),
                None => {
                    app.browser
                        .create_exact_entry(entry.parent, entry.name, entry.directory);
                    close_and_return(app, tx);
                    shell::settle(app, tx);
                }
            }
            true
        });
    });
}

async fn check_typed_rename(rename: &TypedRename) -> Option<CreateRefusal> {
    match unstorable(rename).await {
        Some(refusal) => Some(refusal),
        None => blocked_or_taken(rename).await,
    }
}

/// A name along the typed path that the start folder's file system cannot store.
async fn unstorable(rename: &TypedRename) -> Option<CreateRefusal> {
    let folder = gio_file_for_location(&rename.start_folder()?);
    for (name, new_folder) in rename.components() {
        if let Some(message) = typed_name_refusal(&folder, name, new_folder).await {
            return Some(CreateRefusal::Unstorable(message));
        }
    }
    None
}

/// A file in the way of the typed folders, or a destination already taken.
async fn blocked_or_taken(rename: &TypedRename) -> Option<CreateRefusal> {
    let paths = rename.native_paths();
    let start = rename
        .start_folder()
        .and_then(|start| start.native_path().map(std::path::Path::to_path_buf));
    let folders = rename.folders();
    let (blocked, taken) = match paths.zip(start) {
        Some(((current, wanted), start)) => {
            gio::spawn_blocking(move || match typed_folder_blocked(&start, &folders) {
                Some(blocked) => (Some(blocked), false),
                None => (None, typed_rename_is_taken(&current, &wanted)),
            })
            .await
            .unwrap_or_default()
        }
        None => (None, false),
    };
    match blocked {
        Some(folder) => Some(CreateRefusal::NotAFolder(folder)),
        None => taken.then(|| CreateRefusal::Exists(rename.typed.clone())),
    }
}

/// Whether the rename prompt is still the one for `entry`.
fn renames(app: &App, entry: &FileEntry) -> bool {
    app.tenxer
        .prompt_state
        .rename_target
        .as_ref()
        .is_some_and(|(_, target)| target.location == entry.location)
}

pub(super) fn submit_rename(app: &mut App, serial: u64, text: &str, tx: &mut Tx) {
    let Some((depth, entry)) = app.tenxer.prompt_state.rename_target.clone() else {
        close_and_return(app, tx);
        return;
    };
    let asked = Asked {
        kind: PromptKind::Rename,
        serial,
    };
    let prepared = match prepare_typed_rename(depth, entry, text) {
        Ok(None) => return close_and_return(app, tx),
        Ok(Some(prepared)) => prepared,
        Err(refusal) => return refuse(app, asked, refusal, tx),
    };
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let refusal = check_typed_rename(&prepared).await;
        later.then(move |app, tx| {
            if !asked.holds(app) || !renames(app, &prepared.entry) {
                return false;
            }
            match refusal {
                Some(refusal) => refuse(app, asked, refusal, tx),
                None => {
                    rename::start_typed_rename(app, prepared.depth, prepared.entry, prepared.typed);
                    close_and_return(app, tx);
                    shell::settle(app, tx);
                }
            }
            true
        });
    });
}
