// SPDX-License-Identifier: MIT

mod aside;
mod moves;
mod staging;
mod substitution;

use std::os::unix::fs::MetadataExt;

use rustix::{fs::FsWord, io::Errno};

use super::super::{Copying, StageContent, publish_staged_replacement};
use super::fs_quirks::{Births, Exchange, Op, Racer, Support, Trash};
use super::*;
use crate::{
    adapters::volume::{Family, mount_capabilities},
    services::ReplacedOriginal,
};

const NFS_SUPER_MAGIC: FsWord = 0x6969;
const FUSE_SUPER_MAGIC: FsWord = 0x6573_5546;
const TMPFS_MAGIC: FsWord = 0x0102_1994;

fn without_trash(fs_type: FsWord, quirks: Quirks) -> Quirks {
    Quirks {
        trash: Trash::Unsupported,
        fs_type: Some(fs_type),
        ..quirks
    }
}

fn inode(path: &Path) -> Result<u64, io::Error> {
    Ok(fs::symlink_metadata(path)?.ino())
}

fn replace(source: &Path, target: &Path, move_source: bool) -> Result<(), glib::Error> {
    glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
    ))
}

fn replace_reporting(
    source: &Path,
    target: &Path,
    move_source: bool,
) -> (Result<(), glib::Error>, Vec<ReplacedOriginal>) {
    let replaced = RefCell::new(Vec::new());
    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
        Copying::default(),
        &|original, _| replaced.borrow_mut().push(original),
    ));
    (result, replaced.into_inner())
}

/// Replaces while a progress tracker collects every item the report says was kept or
/// skipped.
fn replace_collecting<T: Clone + 'static>(
    source: &Path,
    target: &Path,
    move_source: bool,
    collect: fn(crate::services::TransferReport) -> Vec<T>,
) -> (Result<(), glib::Error>, Vec<T>) {
    let collected = Rc::new(RefCell::new(Vec::new()));
    let reported = collected.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| {
            if let OperationEvent::ItemsReported { report, .. } = event {
                reported.borrow_mut().extend(collect(report));
            }
        }),
    );
    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
        Copying {
            progress: Some(progress),
            names: NamePolicy::default(),
            ..Default::default()
        },
        &|_, _| {},
    ));
    let collected = collected.borrow().clone();
    (result, collected)
}

fn originals_folder(folder: &Path) -> Result<Option<String>, io::Error> {
    Ok(names(folder)?
        .into_iter()
        .find(|name| name.starts_with(".strata-originals-")))
}

fn write_item(path: &Path, directory: bool, contents: &[u8]) -> Result<(), io::Error> {
    if directory {
        fs::create_dir(path)?;
        fs::write(path.join("item"), contents)
    } else {
        fs::write(path, contents)
    }
}

fn read_item(path: &Path, directory: bool) -> Result<Vec<u8>, io::Error> {
    fs::read(if directory {
        path.join("item")
    } else {
        path.to_path_buf()
    })
}

/// A root under the system temporary directory, outside any `tempfile` cleanup.
fn unique_root(stem: &str) -> Result<PathBuf, Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!("{stem}-{unique}")))
}

fn cancelling(cancellable: &gio::Cancellable) -> Racer {
    let cancellable = cancellable.clone();
    Box::new(move || cancellable.cancel())
}

/// Moves the item at `path` to `kept` and saves another in its place.
fn substitute(path: &Path, kept: &Path) -> Racer {
    let (path, kept) = (path.to_path_buf(), kept.to_path_buf());
    Box::new(move || {
        fs::rename(&path, &kept).expect("move the item away");
        fs::write(&path, b"new arrival").expect("save a new item");
    })
}

/// What a replacement finds at its target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Item {
    File,
    Folder,
    ReadOnlyFolder,
    EmptyReadOnlyFolder,
    Symlink,
}

impl Item {
    fn is_directory(self) -> bool {
        matches!(
            self,
            Item::Folder | Item::ReadOnlyFolder | Item::EmptyReadOnlyFolder
        )
    }

    fn write_target(self, target: &Path) -> Result<(), io::Error> {
        let read_only = fs::Permissions::from_mode(0o555);
        match self {
            Item::Symlink => std::os::unix::fs::symlink("old", target),
            Item::EmptyReadOnlyFolder => {
                fs::create_dir(target)?;
                fs::set_permissions(target, read_only)
            }
            Item::ReadOnlyFolder => {
                write_item(target, true, b"old")?;
                fs::set_permissions(target, read_only)
            }
            Item::File | Item::Folder => write_item(target, self.is_directory(), b"old"),
        }
    }
}

fn expect_contents(expected: &[(PathBuf, &[u8])]) {
    for (path, contents) in expected {
        let read = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(read, *contents, "{}", path.display());
    }
}

/// Asserts that `folder` lists exactly `expected`.
fn expect_names(folder: &Path, expected: &[&str], context: &str) -> Result<(), io::Error> {
    assert_eq!(names(folder)?, expected, "{context}");
    Ok(())
}

/// Asserts that `target` holds `contents` and its folder lists exactly `listed`.
fn expect_left(target: &Path, directory: bool, contents: &[u8], listed: &[&str]) {
    let folder = target.parent().expect("the target has a folder");
    let left = read_item(target, directory).and_then(|read| Ok((read, names(folder)?)));
    let left = left.unwrap_or_else(|error| panic!("{}: {error}", target.display()));
    let listed = listed.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(left, (contents.to_vec(), listed), "{}", target.display());
}

/// Asserts that the original was set aside under `root`, which now lists `listed`.
fn expect_set_aside(quirks: &FsQuirks, root: &Path, listed: &[&str]) -> Result<(), io::Error> {
    assert!(quirks.used_name(".strata-originals-*"));
    expect_names(root, listed, "set aside")
}
