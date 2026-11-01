// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    ops::ControlFlow,
    os::fd::AsFd,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
    time::Duration,
};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, StatxFlags};

use super::{Entry, FsOps, OffMain, System};
use crate::util::ui_thread::{self, Exception};

struct ReportsDrop(mpsc::Sender<thread::ThreadId>);

impl Drop for ReportsDrop {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}

#[test]
fn what_the_ui_thread_drops_outside_dispatch_is_dropped_on_a_worker() {
    thread::spawn(|| {
        crate::util::ui_thread::mark();
        let (sender, dropped) = mpsc::channel();
        drop(OffMain::new(ReportsDrop(sender)));
        let dropped_on = dropped
            .recv_timeout(Duration::from_secs(10))
            .expect("dropped");
        assert_ne!(dropped_on, thread::current().id());
    })
    .join()
    .expect("marked thread");
}

fn refused(call: impl FnOnce()) -> bool {
    catch_unwind(AssertUnwindSafe(call)).is_err()
}

#[test]
#[cfg(debug_assertions)]
fn path_walks_through_the_seam_are_refused_on_the_ui_thread() {
    let ran = thread::spawn(|| {
        ui_thread::mark();
        let folder = OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let root = {
            let _setup = ui_thread::allow(Exception::TestSetup);
            System
                .open_path(OsStr::new("/"), folder, Mode::empty())
                .expect("root")
        };
        [
            (
                "open_path",
                refused(|| drop(System.open_path(OsStr::new("/"), folder, Mode::empty()))),
            ),
            (
                "open_at",
                refused(|| {
                    drop(System.open_at(
                        Entry::new(&root, OsStr::new(".")),
                        folder,
                        Mode::empty(),
                        None,
                    ));
                }),
            ),
            ("stat_vfs", refused(|| drop(System.stat_vfs(root.as_fd())))),
        ]
        .into_iter()
        .filter(|(_, refused)| !refused)
        .map(|(call, _)| call)
        .collect::<Vec<_>>()
    })
    .join()
    .expect("marked thread");
    assert_eq!(ran, Vec::<&str>::new());
}

#[test]
fn a_folder_stamp_follows_a_symlink_to_the_folder_and_names_its_mount() {
    let root = tempfile::tempdir().expect("fixture");
    let real = root.path().join("real");
    std::fs::create_dir(&real).expect("fixture folder");
    std::os::unix::fs::symlink(&real, root.path().join("link")).expect("fixture link");
    let mount_id = rustix::fs::statx(rustix::fs::CWD, &real, AtFlags::empty(), StatxFlags::MNT_ID)
        .expect("mount id")
        .stx_mnt_id;

    let stamp = |path: &std::path::Path, force_sync| {
        let stamp = System.folder_stamp(path, force_sync).expect("stamp");
        (
            stamp.kind,
            (stamp.device, stamp.inode, stamp.birth),
            (stamp.modified, stamp.changed),
            stamp.mount_id,
        )
    };

    let through_link = stamp(&root.path().join("link"), false);
    assert_eq!(through_link, stamp(&real, false));
    assert_eq!(through_link, stamp(&real, true));
    assert_eq!(through_link.0, FileType::Directory);
    assert_eq!(through_link.3, Some(mount_id));
}

#[test]
fn a_listing_arrives_one_getdents_buffer_at_a_time() {
    let root = tempfile::tempdir().expect("fixture");
    let names: BTreeSet<OsString> = (0..300)
        .map(|index| OsString::from(format!("{index:03}-{}", "x".repeat(196))))
        .collect();
    for name in &names {
        crate::test_support::create_empty_file(&root.path().join(name)).expect("fixture file");
    }
    std::fs::create_dir(root.path().join("folder")).expect("fixture folder");
    let folder = rustix::fs::open(
        root.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("fixture folder");

    let mut chunks = 0;
    let mut listed = BTreeSet::new();
    let mut folders = Vec::new();
    System
        .list_chunks(folder.as_fd(), &mut |chunk| {
            chunks += 1;
            for (name, kind) in chunk {
                if kind == FileType::Directory {
                    folders.push(name);
                } else {
                    assert_eq!(kind, FileType::RegularFile);
                    listed.insert(name);
                }
            }
            ControlFlow::Continue(())
        })
        .expect("listing");
    assert!(chunks > 1, "{chunks} chunks");
    assert_eq!(listed, names);
    assert_eq!(folders, [OsString::from("folder")]);

    let folder = rustix::fs::open(
        root.path(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("fixture folder");
    let mut stopped_after = 0;
    System
        .list_chunks(folder.as_fd(), &mut |_| {
            stopped_after += 1;
            ControlFlow::Break(())
        })
        .expect("listing");
    assert_eq!(stopped_after, 1);
}
