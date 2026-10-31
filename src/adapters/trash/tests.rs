// SPDX-License-Identifier: MIT

use super::*;
use std::{cell::Cell, rc::Rc};

fn unique_fixture_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-trash-{label}-{unique}"))
}

#[test]
fn empty_trash_deletes_every_top_level_entry_in_bounded_batches() {
    let root = unique_fixture_root("empty-trash-streaming");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch, so this also exercises the batch-to-batch loop.
    let total_files = 150;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let last_progress = Rc::new(Cell::new(0_usize));
    let progress_tick = last_progress.clone();
    let outcome = glib::MainContext::new()
        .block_on(empty_trash(&gio::File::for_path(&root), move |processed| {
            progress_tick.set(processed);
        }))
        .expect("a plain directory tree should empty without error");
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    assert_eq!(
        outcome.deleted, total_files,
        "every top-level entry must be deleted, independent of any prior measurement budget"
    );
    assert_eq!(outcome.failed, 0);
    assert_eq!(
        last_progress.get(),
        total_files,
        "progress should account for every entry once the walk finishes"
    );
}

#[test]
fn aborting_empty_trash_stops_deletion_mid_flight() {
    let root = unique_fixture_root("abort-empty-trash-mid-flight");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    // More than one `next_files_future` batch (64 entries), so aborting after the first batch's
    // progress callback is genuinely mid-flight, not the whole walk finishing in one step.
    let total_files = 200;
    for index in 0..total_files {
        std::fs::write(root.join(format!("file-{index}.txt")), b"content")
            .expect("the trash fixture file should be written");
    }

    let context = glib::MainContext::new();
    let (progress_before_abort, progress_after_abort) = context
        .with_thread_default(|| {
            let progress = Rc::new(Cell::new(0_usize));
            let progress_tick = progress.clone();
            let trash_root = gio::File::for_path(&root);
            let task = context.spawn_local(async move {
                empty_trash(&trash_root, move |processed| progress_tick.set(processed)).await
            });

            // Drive the loop only until the first batch has reported progress, then abort
            // immediately -- with 200 files and a 64-entry batch size, that's genuinely
            // mid-flight regardless of exactly how many main-loop iterations it took to get there.
            for _ in 0..1_000 {
                if progress.get() > 0 {
                    break;
                }
                context.iteration(true);
            }
            let progress_before_abort = progress.get();

            task.abort();
            for _ in 0..20 {
                context.iteration(false);
            }

            (progress_before_abort, progress.get())
        })
        .expect("a freshly created main context should be acquirable as thread-default");
    let remaining = std::fs::read_dir(&root)
        .expect("the fixture root should still exist")
        .count();
    std::fs::remove_dir_all(&root).expect("the trash fixture should be removed");

    assert!(
        progress_before_abort > 0 && progress_before_abort < total_files,
        "the deletion should have made partial, not complete, progress before it is aborted"
    );
    assert_eq!(
        progress_after_abort, progress_before_abort,
        "aborting mid-flight should stop the deletion from making any further progress"
    );
    assert!(
        remaining > 0,
        "aborting mid-flight should leave undeleted entries behind, not finish the walk anyway"
    );
}

#[test]
fn deletion_caps_errors_and_still_processes_successful_siblings() {
    let root = tempfile::tempdir().expect("fixture");
    for index in 0..12 {
        let directory = root.path().join(format!("directory-{index}"));
        std::fs::create_dir(&directory).expect("directory");
        std::fs::write(directory.join("keep"), b"not empty").expect("child");
    }
    for index in 0..4 {
        std::fs::write(root.path().join(format!("file-{index}")), b"remove").expect("file");
    }
    let mut progress = Vec::new();
    let outcome = glib::MainContext::new()
        .block_on(empty_trash(
            &gio::File::for_path(root.path()),
            |processed| progress.push(processed),
        ))
        .expect("deletion outcome");
    assert_eq!(outcome.deleted, 4);
    assert_eq!(outcome.failed, 12);
    assert_eq!(outcome.errors.len(), 8);
    assert_eq!(progress.last(), Some(&16));
    assert!(progress.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        std::fs::read_dir(root.path())
            .expect("remaining entries")
            .count(),
        12
    );
}

#[test]
fn emptying_the_trash_releases_a_preview_of_a_trashed_file() {
    let mounts = MountTable::parse(
        "22 1 8:2 / / rw - ext4 /dev/sda2 rw\n\
         40 22 0:36 / /mnt/nas rw - nfs4 server:/export rw,vers=4.2\n\
         41 22 0:37 / /tmp rw - tmpfs tmpfs rw\n\
         75 22 8:33 / /run/media/me/USB rw - vfat /dev/sdc2 rw,flush\n\
         76 22 8:49 / /run/media/me/STICK rw - fuseblk /dev/sdd1 rw\n",
    );
    let home_trash = Path::new("/home/me/.local/share/Trash");
    assert_eq!(
        trash_file_roots(home_trash, &mounts, 1000),
        [
            "/home/me/.local/share/Trash/files",
            "/.Trash-1000/files",
            "/.Trash/1000/files",
            "/mnt/nas/.Trash-1000/files",
            "/mnt/nas/.Trash/1000/files",
            "/run/media/me/USB/.Trash-1000/files",
            "/run/media/me/USB/.Trash/1000/files",
            "/run/media/me/STICK/.Trash-1000/files",
            "/run/media/me/STICK/.Trash/1000/files",
        ]
        .map(PathBuf::from),
        "every mounted volume's Trash, network and FUSE ones included, never a virtual one"
    );

    let root = unique_fixture_root("empty-trash-release");
    std::fs::create_dir_all(&root).expect("the trash fixture should be created");
    let trashed = root.join("song.ogg");
    std::fs::write(&trashed, b"held").expect("the trashed file should be written");
    let (stop, stopped) = std::sync::mpsc::channel();
    let present = trashed.clone();
    let hold = crate::services::file_activity::hold([&trashed], move || {
        let _ = stop.send(present.exists());
    })
    .expect("a preview outside any operation");
    let reader = std::thread::spawn(move || {
        let present = stopped
            .recv_timeout(std::time::Duration::from_secs(10))
            .ok();
        drop(hold);
        present
    });

    let outcome = glib::MainContext::new()
        .block_on(empty_releasing(
            &gio::File::for_path(&root),
            vec![root.clone()],
            |_| {},
        ))
        .expect("a plain directory tree should empty without error");
    let present = reader.join().expect("the preview's worker");
    std::fs::remove_dir_all(&root).expect("the trash fixture root should be removed");

    assert_eq!(
        present,
        Some(true),
        "the preview stops before anything is deleted"
    );
    assert_eq!((outcome.deleted, outcome.failed), (1, 0));
}
