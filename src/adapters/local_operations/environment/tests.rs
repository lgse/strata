// SPDX-License-Identifier: MIT

use std::os::fd::AsFd;

use rustix::fs::{AtFlags, Mode, OFlags, StatxFlags};

use super::{FsEnvironment, System};
use crate::adapters::volume::{MountTable, MountType, mount_watch};

#[test]
fn mount_types_come_from_the_table_the_watcher_read() {
    crate::test_support::gtk_test(
        "adapters::local_operations::environment::tests::mount_types_come_from_the_table_the_watcher_read",
        || {
            std::thread::spawn(|| {
                let root = tempfile::tempdir().expect("fixture");
                let folder = rustix::fs::open(
                    root.path(),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .expect("fixture folder");
                let id = rustix::fs::statx(&folder, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
                    .expect("mount id")
                    .stx_mnt_id;
                let fs_type = || {
                    System
                        .mount_type(folder.as_fd())
                        .map(|(kind, _)| kind.fs_type)
                };

                mount_watch::publish_for_isolated_test(Some(MountTable::parse(format!(
                    "{id} 1 0:90 / /published rw - nfs4 server:/export rw\n"
                ))));
                assert_eq!(fs_type().as_deref(), Some("nfs4"));

                mount_watch::publish_for_isolated_test(Some(MountTable::parse(
                    "1 0 8:1 / / rw - ext4 /dev/root rw\n",
                )));
                let current = MountTable::current()
                    .mount_of(id)
                    .map(|(kind, _)| kind.fs_type.clone());
                assert!(current.is_some());
                assert_eq!(fs_type(), current, "a mount the watcher has not read yet");

                let options = |options: &str| {
                    format!("{id} 1 0:90 / /share rw - cifs //server/share rw,{options}\n")
                };
                mount_watch::publish_for_isolated_test(Some(MountTable::parse(options(
                    "vers=3.1.1,serverino,mapposix",
                ))));
                crate::adapters::volume::set_mountinfo_for_isolated_test(Some(options(
                    "vers=3.1.1,mapposix",
                )));
                let options_of = |mount: Option<MountType>| mount.map(|mount| mount.super_options);
                assert_eq!(
                    options_of(System.mount_type(folder.as_fd()).map(|(mount, _)| mount))
                        .as_deref(),
                    Some("rw,vers=3.1.1,serverino,mapposix")
                );
                assert_eq!(
                    options_of(System.current_mount_type(folder.as_fd())).as_deref(),
                    Some("rw,vers=3.1.1,mapposix"),
                    "options the kernel changed without a mount-table event"
                );
                crate::adapters::volume::set_mountinfo_for_isolated_test(None);
            })
            .join()
            .expect("worker");
        },
    );
}
