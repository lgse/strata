// SPDX-License-Identifier: MIT

use std::{os::unix::fs::MetadataExt, sync::atomic::Ordering};

use rustix::io::Errno;

use super::super::{fs_ops::System, placement::probe};
use super::fs_quirks::{Op, OverEmptyDirectory, Racer, Support, Trash, hold};
use super::*;
use crate::{
    adapters::volume::learn_mount_capability,
    services::{KeptCause, KeptItem, PlacedSnapshot, ReplacedOriginal},
};

mod applying;
mod undo_cleanup;
mod undo_restore;

fn real_copy() -> CopyStep {
    super::super::merge_copy(None, NamePolicy::default())
}

/// Substitutes Trash staging with a rename into `staging`, recording the
/// staged locations in call order.
fn rename_stage(staging: PathBuf) -> (StageOverwrite, Rc<RefCell<Vec<Location>>>) {
    fs::create_dir_all(&staging).expect("staging dir");
    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorded = calls.clone();
    let counter = Rc::new(Cell::new(0usize));
    let stage: StageOverwrite = Rc::new(move |location: Location, _cancellable| {
        recorded.borrow_mut().push(location.clone());
        let index = counter.get();
        counter.set(index + 1);
        let staging = staging.clone();
        Box::pin(async move {
            let path = location.native_path().ok_or_else(|| {
                glib::Error::new(gio::IOErrorEnum::Failed, "no local path to stage")
            })?;
            std::fs::rename(path, staging.join(format!("staged-{index}")))
                .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))?;
            Ok(None)
        })
    });
    (stage, calls)
}

fn recorded_plans() -> (Rc<RefCell<Vec<MergePlan>>>, impl Fn(MergePlan)) {
    let plans = Rc::new(RefCell::new(Vec::new()));
    let recorded = plans.clone();
    (plans, move |plan| recorded.borrow_mut().push(plan))
}

fn placed_paths(placed: &HashMap<Location, PlacedSnapshot>) -> Vec<PathBuf> {
    let mut paths = (placed.keys())
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn staged_trash_fixture(
    root: &Path,
    name: &str,
    original: &Path,
    contents: &[u8],
) -> Result<RestoreEntry, Box<dyn Error>> {
    let uid = rustix::process::getuid().as_raw();
    let trash = root.join(format!(".Trash-{uid}"));
    fs::create_dir_all(trash.join("files"))?;
    fs::create_dir_all(trash.join("info"))?;
    let staged = trash.join("files").join(name);
    fs::write(&staged, contents)?;
    let info = trash.join("info").join(format!("{name}.trashinfo"));
    fs::write(
        &info,
        format!(
            "[Trash Info]\nPath={}\nDeletionDate=2026-01-01T00:00:00\n",
            original.display()
        ),
    )?;
    Ok(RestoreEntry {
        source: Location::local(&staged),
        display_name: name.to_owned(),
        original_target: Some(Location::local(original)),
        trash_info: Some(info),
        confirmed_destination: None,
        physical_path: Some(staged),
    })
}

fn staged_lookup(entries: HashMap<PathBuf, RestoreEntry>) -> StagedOriginalLookup {
    let entries = RefCell::new(entries);
    Rc::new(move |location, _cancellable| {
        let entry = location
            .native_path()
            .and_then(|path| entries.borrow_mut().remove(path));
        Box::pin(async move {
            entry.ok_or_else(|| {
                glib::Error::new(
                    gio::IOErrorEnum::NotFound,
                    "The original is no longer in Trash",
                )
            })
        })
    })
}

#[derive(Clone, Copy, Debug)]
enum Undone {
    Replace,
    Delete,
    Merge,
}

#[derive(Clone, Copy, Debug)]
enum Trashed {
    Original,
    Nothing,
    OriginalBesideANewerOneBehindALink,
    OnlyAnotherFromTheSamePath,
    TwoAlikeBeforeAnother,
}

type UndoParts = (
    Vec<Location>,
    Vec<Location>,
    HashMap<Location, PlacedSnapshot>,
);

fn undo_parts(plans: Vec<MergePlan>) -> UndoParts {
    let mut parts = UndoParts::default();
    for plan in plans {
        parts.0.extend(plan.created);
        parts.1.extend(plan.overwritten);
        parts.2.extend(plan.placed);
    }
    parts
}

fn run_undo(parts: UndoParts, lookup: StagedOriginalLookup) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    glib::MainContext::default().block_on(run_merge_undo(
        OperationRequestId(1616),
        parts,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
        lookup,
    ));
    events.take()
}

fn kept_in_trash(events: &[OperationEvent]) -> Vec<KeptItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.kept_in_trash.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn kept_items(events: &[OperationEvent]) -> Vec<Location> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemKept { location, .. } => Some(location.clone()),
            _ => None,
        })
        .collect()
}

fn restored(events: &[OperationEvent]) -> Option<Vec<Location>> {
    match events.last() {
        Some(OperationEvent::Restored { restored, .. }) => Some(restored.clone()),
        _ => None,
    }
}

fn changed_after_the_merge(location: &Path) -> Vec<KeptItem> {
    vec![KeptItem {
        location: Location::local(location),
        cause: KeptCause::ReplacementChanged,
    }]
}

fn trash_in(root: &Path) -> PathBuf {
    root.join(format!(".Trash-{}", rustix::process::getuid().as_raw()))
}

fn staged_trash_folder(
    root: &Path,
    name: &str,
    original: &Path,
    files: &[(&str, &str)],
) -> Result<RestoreEntry, Box<dyn Error>> {
    let entry = staged_trash_fixture(root, name, original, b"")?;
    let staged = entry.physical_path.clone().ok_or("staged path")?;
    fs::remove_file(&staged)?;
    write_files(&staged, files)?;
    Ok(entry)
}

fn without_trash(quirks: Quirks) -> Quirks {
    Quirks {
        trash: Trash::Unsupported,
        ..quirks
    }
}

fn write_files(root: &Path, files: &[(&str, &str)]) -> Result<(), io::Error> {
    for (path, contents) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)?;
    }
    Ok(())
}
