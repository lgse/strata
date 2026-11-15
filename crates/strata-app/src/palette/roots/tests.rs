// SPDX-License-Identifier: MIT

//! GTK's cases for the search roots, from `src/ui/window/devices/tests.rs`.

use strata_fs::adapters::{MountTable, mount_watch};

use super::*;
use crate::test_support::in_own_process;

#[test]
fn global_search_always_includes_home_and_all_mounted_drives() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            ["/run/media/me/USB", "/mnt/Backup"].map(PathBuf::from)
        ),
        ["/home/me", "/mnt/Backup", "/run/media/me/USB"].map(PathBuf::from)
    );
    assert_eq!(
        search_roots(Path::new("/home/me"), []),
        [PathBuf::from("/home/me")]
    );
}

#[test]
fn global_search_deduplicates_roots_without_dropping_nested_mounts() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            [
                "/home/me",
                "/home/me/USB",
                "/home/me/USB",
                "/home/me/USB/nested"
            ]
            .map(PathBuf::from)
        ),
        ["/home/me", "/home/me/USB", "/home/me/USB/nested"].map(PathBuf::from)
    );
}

#[test]
fn global_search_does_not_expand_into_system_mounts_or_gvfs_mirrors() {
    assert_eq!(
        search_roots(
            Path::new("/home/me"),
            [
                "/",
                "/home",
                "/boot",
                "/proc",
                "/sys",
                "relative",
                "/run/user/1000/gvfs"
            ]
            .map(PathBuf::from)
        ),
        [PathBuf::from("/home/me")]
    );
}

#[test]
fn a_non_system_drive_containing_home_is_still_included() {
    assert_eq!(
        search_roots(
            Path::new("/mnt/Storage/Home"),
            [PathBuf::from("/mnt/Storage")]
        ),
        ["/mnt/Storage/Home", "/mnt/Storage"].map(PathBuf::from)
    );
}

fn native_root(path: &str) -> MountRoot {
    MountRoot {
        native: true,
        path: Some(PathBuf::from(path)),
    }
}

/// Name, volume mounts, other mounts, expected roots.
type RootsCase = (
    &'static str,
    Vec<MountRoot>,
    Vec<(MountRoot, MountListing)>,
    &'static [&'static str],
);

const OWN_ROW: MountListing = MountListing {
    shadowed: false,
    volume_listed: false,
};

#[test]
fn search_takes_the_local_roots_of_the_mounts_devices_lists() {
    const USB: &str = "/run/media/me/USB";
    let gvfs_share = MountRoot {
        native: false,
        path: Some(PathBuf::from(
            "/run/user/1000/gvfs/smb-share:server=nas,share=media",
        )),
    };
    let pathless = MountRoot {
        native: true,
        path: None,
    };
    let volume_listed = MountListing {
        shadowed: false,
        volume_listed: true,
    };
    let shadowed = MountListing {
        shadowed: true,
        volume_listed: false,
    };
    let cases: [RootsCase; 7] = [
        ("volume mount", vec![native_root(USB)], vec![], &[USB]),
        (
            "own-row mount",
            vec![],
            vec![(native_root("/mnt/nfs"), OWN_ROW)],
            &["/mnt/nfs"],
        ),
        ("pathless volume mount", vec![pathless.clone()], vec![], &[]),
        ("pathless mount", vec![], vec![(pathless, OWN_ROW)], &[]),
        ("GVfs share mount", vec![], vec![(gvfs_share, OWN_ROW)], &[]),
        (
            "shadowed mount",
            vec![],
            vec![(native_root("/media/shadowed"), shadowed)],
            &[],
        ),
        (
            "volume mount listed again",
            vec![native_root(USB)],
            vec![(native_root(USB), volume_listed)],
            &[USB],
        ),
    ];
    for (case, volume_mounts, other_mounts, expected) in cases {
        assert_eq!(
            local_device_roots(volume_mounts, other_mounts),
            expected.iter().map(PathBuf::from).collect::<Vec<_>>(),
            "{case}"
        );
    }
}

/// In a process of its own: the published mount table is the process's.
#[test]
fn global_search_skips_mount_table_entries_the_volume_monitor_does_not_list() {
    in_own_process(
        "palette::roots::tests::global_search_skips_mount_table_entries_the_volume_monitor_does_not_list",
        &[],
        || {
            mount_watch::publish_for_isolated_test(Some(MountTable::parse(
                br"25 1 0:31 /@ / rw - btrfs /dev/sda1 rw,subvol=/@
26 1 0:31 /@pkg /var/cache/strata-fixture/pkg rw - btrfs /dev/sda1 rw,subvol=/@pkg
27 1 8:4 / /mnt/strata-fixture-archive ro - fuseblk /dev/sdc1 ro
",
            )));
            let home = crate::launch::home_directory();
            let roots = search_roots(&home, device_mount_roots());
            assert_eq!(roots.first(), Some(&home));
            for hidden in [
                "/var/cache/strata-fixture/pkg",
                "/mnt/strata-fixture-archive",
            ] {
                assert!(!roots.contains(&PathBuf::from(hidden)), "{hidden}");
            }
        },
    );
}
