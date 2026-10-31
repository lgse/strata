// SPDX-License-Identifier: MIT

//! Moves an item to the relative path typed while naming it. Folders
//! are made or reused one directory at a time without following links, the item
//! is moved without replacing anything, and a failure removes the folders this
//! placement made again.

use super::{
    cancelled_event,
    fs_ops::{Entry, FsOps, fs},
    native_fs::{open_local_parent_directory, os_error_detail},
    operation_handle,
    placement::rename_in_place,
    release::{self, ActiveOperation},
    rename_failure_error, run_local_fs_step, typed_names,
};
use crate::{
    model::Location,
    services::{LoadHandle, OperationEvent, PlaceEntryRequest, TypedPath, validate_basename},
};
use gtk::{gio, glib, prelude::*};
use rustix::{
    fs::{AtFlags, FileType, Mode},
    io::Errno,
};
use std::{
    collections::HashSet,
    ffi::OsStr,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    rc::Rc,
};

pub(super) fn start(request: PlaceEntryRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancellable = gio::Cancellable::new();
    let operation_cancellable = cancellable.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let PlaceEntryRequest {
            id,
            entry,
            path,
            new_item,
        } = request;
        let fail = |message: String| {
            emit(OperationEvent::Failed {
                request_id: id,
                message,
                password_failure: None,
            });
        };
        let is_directory = entry.is_directory();
        let item = entry.location;
        let (Some(source), Some(parent)) =
            (item.native_path().map(Path::to_path_buf), item.parent())
        else {
            fail(crate::i18n::tr("Invalid move destination"));
            return;
        };
        let Some(start_folder) = path.start(&parent) else {
            fail(crate::i18n::tr("Invalid move destination"));
            return;
        };
        let into_folder = path.folder && !is_directory;
        if into_folder && !new_item {
            fail(crate::i18n::tr("An existing file can’t become a folder"));
            return;
        }
        let folder = crate::adapters::gio_file_for_location(&start_folder);
        for (index, component) in path.components().enumerate() {
            if let Err(message) = validate_basename(component) {
                fail(crate::i18n::tr(message));
                return;
            }
            let new_folder = index < path.folders.len() || path.folder;
            if let Some(message) = typed_names::refused_in(&folder, component, new_folder).await {
                fail(message);
                return;
            }
        }
        let roots = release::natives([&item]);
        let operation = ActiveOperation::begin(release::parents(&roots));
        operation.release(roots, &operation_cancellable).await;
        if operation_cancellable.is_cancelled() {
            emit(cancelled_event(
                id,
                Vec::new(),
                Vec::new(),
                vec![item],
                HashSet::from([parent]),
            ));
            return;
        }
        let placed_path = path.clone();
        let result =
            run_local_fs_step(move || place(fs(), &source, &placed_path, into_folder)).await;
        match result {
            Ok(first_created) => {
                let placed = path
                    .components()
                    .scan(start_folder, |folder, name| {
                        *folder = folder.child(OsStr::new(name))?;
                        Some(folder.clone())
                    })
                    .skip(first_created)
                    .collect::<Vec<Location>>();
                emit(OperationEvent::EntryPlaced {
                    request_id: id,
                    placed,
                });
            }
            Err(error) => fail(error.message().to_owned()),
        }
    });
    operation_handle(cancellable)
}

/// Returns how many of the path's folders already existed.
fn place(
    fs: &dyn FsOps,
    source: &Path,
    path: &TypedPath,
    into_folder: bool,
) -> Result<usize, String> {
    let (Some(source_parent), Some(source_name)) = (source.parent(), source.file_name()) else {
        return Err(crate::i18n::tr("Invalid move destination"));
    };
    let Some(start) = source_parent.ancestors().nth(path.up) else {
        return Err(crate::i18n::tr("Invalid move destination"));
    };
    let mut placement = Placement {
        fs,
        source_dir: open_local_parent_directory(source_parent)?,
        dirs: vec![open_local_parent_directory(start)?],
        made: Vec::new(),
    };
    let result = placement.run((source, source_name), start, path, into_folder);
    if result.is_err() {
        placement.roll_back();
    }
    result
}

struct Placement<'a> {
    fs: &'a dyn FsOps,
    source_dir: OwnedFd,
    /// The folder the path starts from, then each folder on it as it is opened.
    dirs: Vec<OwnedFd>,
    /// Each folder made here, as its parent's index in `dirs` and its name.
    made: Vec<(usize, String)>,
}

impl Placement<'_> {
    fn run(
        &mut self,
        (source, source_name): (&Path, &OsStr),
        start: &Path,
        path: &TypedPath,
        into_folder: bool,
    ) -> Result<usize, String> {
        for name in &path.folders {
            self.make_folder(name, true)?;
            let at = Entry::new(self.last(), OsStr::new(name));
            let folder = self.fs.open_dir(at).map_err(|error| match error {
                Errno::NOTDIR | Errno::LOOP => not_a_folder(name),
                error => could_not_create(name, error),
            })?;
            self.dirs.push(folder);
        }
        if into_folder {
            self.make_folder(&path.name, false)?;
            let source = Entry::new(&self.source_dir, source_name);
            let still_empty = self
                .fs
                .stat(source, true)
                .is_ok_and(|stat| stat.kind == FileType::RegularFile && stat.size == 0);
            if !still_empty {
                return Err(changed(source_name));
            }
            self.fs
                .unlink(source, AtFlags::empty())
                .map_err(|error| could_not_create(&path.name, error))?;
        } else {
            let from = Entry::new(&self.source_dir, source_name);
            let target = Entry::new(self.last(), OsStr::new(&path.name));
            rename_in_place(self.fs, from, target).map_err(|failure| {
                let target_path = path
                    .components()
                    .fold(PathBuf::from(start), |path, name| path.join(name));
                rename_failure_error(failure, source, &target_path)
                    .message()
                    .to_owned()
            })?;
        }
        let made_folders = self.made.len() - usize::from(into_folder);
        Ok(path.folders.len() - made_folders)
    }

    fn last(&self) -> &OwnedFd {
        self.dirs.last().expect("the item's folder is open")
    }

    /// An existing folder on the way is reused; the item itself must be new.
    fn make_folder(&mut self, name: &str, reuse: bool) -> Result<(), String> {
        match self.fs.mkdir(
            Entry::new(self.last(), OsStr::new(name)),
            Mode::from_raw_mode(0o777),
        ) {
            Ok(()) => {
                self.made.push((self.dirs.len() - 1, name.to_owned()));
                Ok(())
            }
            Err(Errno::EXIST) if reuse => Ok(()),
            Err(Errno::EXIST) => {
                Err(rust_i18n::t!("“%{name}” already exists", name = name).into_owned())
            }
            Err(error) => Err(could_not_create(name, error)),
        }
    }

    /// Removes only folders that are still empty, so nothing anyone put in
    /// them meanwhile is lost.
    fn roll_back(&mut self) {
        for (dir, name) in self.made.drain(..).rev() {
            let _ = self.fs.unlink(
                Entry::new(&self.dirs[dir], OsStr::new(&name)),
                AtFlags::REMOVEDIR,
            );
        }
    }
}

fn not_a_folder(name: &str) -> String {
    rust_i18n::t!("“%{name}” is not a folder", name = name).into_owned()
}

fn could_not_create(name: &str, error: Errno) -> String {
    rust_i18n::t!(
        "Could not create “%{name}”: %{error}",
        name = name,
        error = os_error_detail(error)
    )
    .into_owned()
}

fn changed(name: &OsStr) -> String {
    rust_i18n::t!(
        "%{name} changed while the operation was in progress",
        name = name.to_string_lossy()
    )
    .into_owned()
}
