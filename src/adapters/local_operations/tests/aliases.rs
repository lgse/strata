// SPDX-License-Identifier: MIT

mod copies;
mod duplicates;
mod listing;
mod top_level;

use super::*;
use crate::services::{RenameCause, RenamedItem};

fn copy_renaming(
    source: &Path,
    target: &Path,
    workers: Option<usize>,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let relative = |location: &Location| {
        location
            .native_path()
            .and_then(|path| path.strip_prefix(target).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    Ok(copy_reporting(source, target, workers)?
        .renamed
        .into_iter()
        .map(
            |RenamedItem {
                 requested,
                 saved_as,
                 cause,
             }| {
                assert_eq!(cause, RenameCause::Alias);
                (relative(&requested), relative(&saved_as))
            },
        )
        .collect())
}

/// A fresh root holding an empty `destination` folder, with the path of its `source`.
fn source_and_destination() -> io::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    Ok((root, source, destination))
}

/// Names never collide on ext4, so the one made last is numbered.
fn casefolded_ext4() -> Quirks {
    Quirks {
        case_insensitive: true,
        ..Quirks::ext4()
    }
}

/// ciopfs folds case like FAT, and holds links, which FAT cannot.
fn ciopfs() -> Quirks {
    Quirks {
        mount_type: Some(("fuse.ciopfs", "rw")),
        case_insensitive: true,
        ..Quirks::default()
    }
}

/// Writes each item under `root` with its own path as contents, or as a folder holding
/// `inside` when its name ends with `folders`.
fn write_items(root: &Path, items: &[&str], folders: &str) -> io::Result<()> {
    for item in items {
        let path = root.join(item);
        if item.ends_with(folders) {
            fs::create_dir(&path)?;
            fs::write(path.join("inside"), item)?;
        } else {
            fs::write(path, item)?;
        }
    }
    Ok(())
}

/// What a written item holds, read from `inside` for a folder.
fn read_item(path: &Path, folder: bool) -> io::Result<String> {
    if folder {
        fs::read_to_string(path.join("inside"))
    } else {
        fs::read_to_string(path)
    }
}

fn expect_contents(expected: &[(PathBuf, &str)], context: &str) -> io::Result<()> {
    for (path, contents) in expected {
        assert_eq!(
            fs::read_to_string(path)?,
            *contents,
            "{context}: {}",
            path.display()
        );
    }
    Ok(())
}

/// Each name and what it holds in `folder`, in name order.
fn held_contents(folder: &Path) -> io::Result<Vec<(String, String)>> {
    names(folder)?
        .into_iter()
        .map(|name| {
            let content = fs::read_to_string(folder.join(&name))?;
            Ok((name, content))
        })
        .collect()
}

fn owned_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).to_owned()))
        .collect()
}

fn fail_if_exists(source: Location) -> PasteItem {
    PasteItem {
        source,
        conflict: TransferConflict::FailIfExists,
        target_name: None,
    }
}

/// A paste of `items` from `source` into `destination` that plans their names itself.
fn paste_of(source: &Path, destination: &Path, items: &[&str], moving: bool) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(1530),
        destination: Location::local(destination),
        items: items
            .iter()
            .map(|item| fail_if_exists(Location::local(source.join(item))))
            .collect(),
        move_sources: moving,
    }
}

/// Runs a paste until it ends, however it ends.
fn paste_until_settled(request: PasteRequest) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_transfer_settles(&events);
    events.take()
}

/// Each name numbered `<name> (1)` for `cause`.
fn numbered_once(names: &[&str], cause: RenameCause) -> Vec<(String, String, RenameCause)> {
    names
        .iter()
        .map(|name| (name.to_string(), format!("{name} (1)"), cause))
        .collect()
}
