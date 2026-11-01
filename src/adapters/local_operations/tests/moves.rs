// SPDX-License-Identifier: MIT

use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    sync::atomic::Ordering,
};

use rustix::io::Errno;

use super::fs_quirks::{NameOpens, Op, Support, hold};
use super::*;
use crate::adapters::volume::{Family, mount_capabilities};
use crate::services::{Holder, KeptCause, KeptItem, SpecialKind, TransferReport};

mod changed;
mod deletion;
mod flushing;
mod identity;
mod inodes;
mod left_out;
mod local_moves;
mod ticks;

fn paste_move(
    id: u64,
    destination: &Path,
    source: &Path,
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    paste_moves(id, destination, &[source], conflict)
}

fn paste_moves(
    id: u64,
    destination: &Path,
    sources: &[&Path],
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(id),
            destination: Location::local(destination),
            items: sources
                .iter()
                .map(|source| PasteItem {
                    source: Location::local(source),
                    conflict,
                    target_name: None,
                })
                .collect(),
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (operation, events)
}

fn reported(events: &RefCell<Vec<OperationEvent>>) -> TransferReport {
    let mut reported = TransferReport::default();
    for event in events.borrow().iter() {
        if let OperationEvent::ItemsReported { report, .. } = event {
            reported.extend(report.clone());
        }
    }
    reported
}

fn kept(location: PathBuf, cause: KeptCause) -> KeptItem {
    KeptItem {
        location: Location::local(location),
        cause,
    }
}

fn destination_for(
    root: &Path,
    conflict: TransferConflict,
) -> Result<(PathBuf, FsQuirks), Box<dyn Error>> {
    let destination = root.join("destination");
    fs::create_dir_all(&destination)?;
    match conflict {
        TransferConflict::Merge => fs::create_dir(destination.join("folder"))?,
        TransferConflict::ReplaceExisting => {
            fs::create_dir(destination.join("folder"))?;
            fs::write(destination.join("folder/old.txt"), b"old")?;
        }
        _ => {}
    }
    let quirks = Quirks {
        trash: super::fs_quirks::Trash::Unsupported,
        ..Quirks::default()
    };
    let scope = FsQuirks::install(&destination, quirks);
    Ok((destination, scope))
}

const MOVES: [TransferConflict; 3] = [
    TransferConflict::FailIfExists,
    TransferConflict::Merge,
    TransferConflict::ReplaceExisting,
];

fn move_across_volumes(
    folder: &Path,
    quirks: Quirks,
) -> Result<(PathBuf, TransferReport, Vec<OperationEvent>), Box<dyn Error>> {
    move_across_volumes_logged(folder, quirks)
        .map(|(moved, report, events, _)| (moved, report, events))
}

type LoggedMove = (
    PathBuf,
    TransferReport,
    Vec<OperationEvent>,
    Vec<super::fs_quirks::Call>,
);

fn move_across_volumes_logged(folder: &Path, quirks: Quirks) -> Result<LoggedMove, Box<dyn Error>> {
    let source_dir = folder.parent().ok_or("a folder in a source volume")?;
    let destination = source_dir.with_file_name("destination");
    fs::create_dir_all(&destination)?;
    let source = FsQuirks::install(source_dir, quirks);
    let _destination = FsQuirks::install(&destination, Quirks::default());

    let (_operation, events) = paste_move(97, &destination, folder, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    let report = reported(&events);
    let name = folder.file_name().ok_or("a named item")?;
    Ok((
        destination.join(name),
        report,
        events.take(),
        source.calls(),
    ))
}

fn rewrite(path: &Path, contents: &'static [u8]) -> super::fs_quirks::Racer {
    let path = path.to_path_buf();
    Box::new(move || fs::write(&path, contents).expect("rewrite the source"))
}

/// A rewrite within one change-time tick of `than` (10 ms on exFAT) would keep
/// its change time.
fn wait_for_a_later_change_time(probe: &Path, than: &fs::Metadata) {
    loop {
        fs::write(probe, b"probe").expect("write the probe");
        let stamped = fs::metadata(probe).expect("stat the probe");
        if (stamped.ctime(), stamped.ctime_nsec()) > (than.ctime(), than.ctime_nsec()) {
            break;
        }
    }
    fs::remove_file(probe).expect("remove the probe");
}
