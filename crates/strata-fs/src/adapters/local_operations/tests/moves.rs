// SPDX-License-Identifier: MIT

use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    sync::atomic::Ordering,
};

use rustix::io::Errno;

use super::fs_quirks::{NameOpens, Op, Racer, Support, hold};
use super::*;
use crate::adapters::volume::{Family, mount_capabilities};
use crate::services::{Holder, KeptCause, KeptItem, SpecialKind, TransferReport};

mod across_volumes;
mod changed;
mod coarse_ticks;
mod copy_gone;
mod deletion;
mod flushing;
mod identity;
mod inode_numbers;
mod inodes;
mod left_out;
mod local_moves;
mod renumbered;
mod still_open;
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

/// A racer run before or after a call on a name.
type Before = (Op, &'static str, usize, Racer);

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

/// Each `(path, contents)` beneath `root` reads as its contents.
fn assert_files<C: AsRef<[u8]>>(root: &Path, files: &[(&str, C)], row: &str) -> io::Result<()> {
    for (path, contents) in files {
        assert_eq!(
            fs::read(root.join(path))?,
            contents.as_ref(),
            "{row}: {path}"
        );
    }
    Ok(())
}

fn assert_pasted(events: &RefCell<Vec<OperationEvent>>, row: &str) {
    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{row}: {:?}",
        events.borrow()
    );
}

/// An hour before now, in seconds since the epoch: settled times a rewrite
/// shows in on any kernel.
fn an_hour_ago() -> Result<u64, Box<dyn Error>> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600)
}

/// Sets the access and modification times of `path` back to the
/// modification time `before` recorded.
fn put_times_back(path: &Path, before: &fs::Metadata) -> rustix::io::Result<()> {
    let modified = rustix::fs::Timespec {
        tv_sec: before.mtime(),
        tv_nsec: before.mtime_nsec(),
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        path,
        &rustix::fs::Timestamps {
            last_access: modified,
            last_modification: modified,
        },
        rustix::fs::AtFlags::empty(),
    )
}

/// What the move reported kept at the source, in path order.
fn kept_in_order(mut kept: Vec<KeptItem>) -> Vec<KeptItem> {
    kept.sort_by_key(|item| item.location.native_path().map(Path::to_path_buf));
    kept
}
