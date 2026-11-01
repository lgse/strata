// SPDX-License-Identifier: MIT

use std::{error::Error, fs, os::fd::AsFd, path::Path};

use super::super::{
    fs_ops::fs,
    open_local_parent_directory,
    tests::fs_quirks::{FsQuirks, Preset, Quirks},
};
use super::{MountProfile, Profiles, folder_name_limit, learn_mount_type};
use crate::adapters::volume::{
    FAT32_MAX_FILE_SIZE, Family, FuseFormat, MountNames, MountTable, NameEncoding, NameLimit,
    NameProfile, NameRules, Reserved, Strictness, Strip, family, gvfs_bridge_profile, gvfs_profile,
    learn_mount_capability, mount_capabilities,
};

fn profile_of(profiles: &Profiles, folder: &Path) -> Result<MountProfile, Box<dyn Error>> {
    let folder = open_local_parent_directory(folder)?;
    Ok(profiles.of(fs(), folder.as_fd()))
}

fn posix(bytes: Option<u16>, learns: bool) -> NameProfile {
    NameProfile {
        rules: NameRules::Posix,
        strip: Strip::None,
        reserved: Reserved::None,
        encoding: NameEncoding::AnyBytes,
        mapped: None,
        limit: NameLimit {
            bytes,
            ..NameLimit::default()
        },
        folds: false,
        learns,
    }
}

fn windows(reserved: Reserved, learns: bool) -> NameProfile {
    NameProfile {
        rules: NameRules::Windows,
        strip: Strip::Trailing,
        reserved,
        encoding: NameEncoding::Utf8,
        mapped: None,
        limit: NameLimit {
            units: Some(255),
            ..NameLimit::default()
        },
        folds: false,
        learns,
    }
}

#[test]
fn each_mount_is_profiled_from_what_its_folder_reports() -> Result<(), Box<dyn Error>> {
    let profile = |names, symlinks, max_file_size, family| MountProfile {
        names,
        strictness: Strictness::default(),
        symlinks,
        max_file_size,
        family,
        fuse_format: None,
    };
    let fat = NameProfile {
        folds: true,
        ..windows(Reserved::Bare, false)
    };
    let unknown_udev = || Quirks {
        device_format: None,
        ..Quirks::exfat_fuse()
    };
    let zfs = || Quirks {
        mount_type: Some(("zfs", "rw,xattr,posixacl")),
        name_max: Some(1023),
        ..Quirks::default()
    };
    let silent_daemon = || Quirks {
        name_max: Some(0),
        ..Quirks::sshfs()
    };
    let no_symlinks = Some(false);
    let fat32 = Some(FAT32_MAX_FILE_SIZE);
    let cases: [(&str, Preset, MountProfile); 5] = [
        (
            "vfat",
            Quirks::vfat,
            profile(fat, no_symlinks, fat32, Family::Fat),
        ),
        (
            "exfat-fuse",
            Quirks::exfat_fuse,
            MountProfile {
                fuse_format: Some(FuseFormat::Exfat),
                ..profile(fat, no_symlinks, None, Family::Fuse)
            },
        ),
        (
            "fuseblk without udev",
            unknown_udev,
            profile(windows(Reserved::Bare, true), None, None, Family::Fuse),
        ),
        (
            "zfs",
            zfs,
            profile(posix(Some(1023), false), None, None, Family::Zfs),
        ),
        (
            "sshfs reporting no name length",
            silent_daemon,
            profile(posix(None, true), None, None, Family::Fuse),
        ),
    ];
    for (case, quirks, expected) in cases {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());

        assert_eq!(
            profile_of(&Profiles::default(), root.path())?,
            expected,
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn a_scope_reports_the_filesystem_below_it_unless_its_preset_names_another()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let host = MountTable::current()
        .mount_at(root.path())
        .map(|(mount, _)| mount.clone())
        .ok_or("the temporary folder's mount")?;
    let host_kind = rustix::fs::statfs(root.path())?.f_type;
    let cases: [(&str, Preset, Family, rustix::fs::FsWord); 2] = [
        (
            "no mount type",
            Quirks::default,
            family(&host.fs_type),
            host_kind,
        ),
        ("vboxsf", Quirks::vboxsf, Family::Other, 0x786f_4256),
    ];
    for (case, quirks, expected_family, expected_kind) in cases {
        let folder = root.path().join(case);
        fs::create_dir(&folder)?;
        let _scope = FsQuirks::install(&folder, quirks());
        let opened = open_local_parent_directory(&folder)?;

        assert_eq!(
            (
                profile_of(&Profiles::default(), &folder)?.family,
                fs().statfs(opened.as_fd()).map(|stat| stat.kind),
            ),
            (expected_family, Some(expected_kind)),
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn only_disks_in_a_windows_format_are_taken_for_slow_ones_where_their_device_does_not_say()
-> Result<(), Box<dyn Error>> {
    let bridge = || Quirks {
        mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
        ..Quirks::default()
    };
    let cases: [(&str, Preset, bool); 8] = [
        ("vfat", Quirks::vfat, true),
        ("exfat-fuse", Quirks::exfat_fuse, true),
        ("ntfs3", Quirks::ntfs3, true),
        ("ntfs-3g", Quirks::ntfs_3g, true),
        ("cifs", Quirks::cifs, false),
        ("ext4", Quirks::ext4, false),
        ("sshfs", Quirks::sshfs, false),
        ("a share through the GVfs bridge", bridge, false),
    ];
    for (case, quirks, windows_disk) in cases {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("smb-share:server=nas.local,share=media");
        fs::create_dir(&folder)?;
        let _scope = FsQuirks::install(root.path(), quirks());

        let profile = profile_of(&Profiles::default(), &folder)?;

        assert_eq!(profile.windows_disk(), windows_disk, "{case}: {profile:?}");
    }
    Ok(())
}

#[test]
fn the_gvfs_bridge_takes_the_rules_of_each_backend_from_its_folder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _bridge = FsQuirks::install(
        root.path(),
        Quirks {
            mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
            ..Quirks::default()
        },
    );
    let phone = root
        .path()
        .join("mtp:host=Google_Pixel_8_Pro_3A101FDJG0012B");
    let share = root.path().join("smb-share:server=nas.local,share=media");
    let server = root.path().join("sftp:host=example.org,user=me");
    let cloud = root.path().join("dav:host=cloud.example.org,ssl=true");
    // A folder named like a backend deeper inside one changes nothing.
    let nested = phone.join("Internal storage/smb-share:server=x,share=y");
    for folder in [&share, &server, &cloud, &nested] {
        fs::create_dir_all(folder)?;
    }
    let unnamed = root.path().join("folder without a scheme");
    fs::create_dir(&unnamed)?;
    let profiles = Profiles::default();

    for (folder, profile) in [
        (phone.join("Internal storage"), gvfs_profile("mtp")),
        (nested, gvfs_profile("mtp")),
        (share, gvfs_profile("smb")),
        (server, gvfs_profile("sftp")),
        (cloud, gvfs_profile("dav")),
        (unnamed, gvfs_bridge_profile(None)),
        (root.path().to_path_buf(), gvfs_bridge_profile(None)),
    ] {
        assert_eq!(
            profile_of(&profiles, &folder)?.names,
            profile,
            "{}",
            folder.display()
        );
    }
    Ok(())
}

#[test]
fn a_bind_mount_inside_the_gvfs_bridge_takes_the_rules_of_its_backend() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let _phone = FsQuirks::install(
        root.path(),
        Quirks {
            mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
            mount_root: Some("/mtp:host=Google_Pixel_8_Pro_3A101FDJG0012B/Internal storage"),
            ..Quirks::default()
        },
    );
    let camera = root.path().join("DCIM");
    fs::create_dir(&camera)?;

    for folder in [root.path(), &camera] {
        assert_eq!(
            profile_of(&Profiles::default(), folder)?.names,
            gvfs_profile("mtp"),
            "{}",
            folder.display()
        );
    }
    Ok(())
}

#[test]
fn an_operation_keeps_the_profile_it_read_first_when_its_mount_changes()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(root.path(), Quirks::ntfs3());
    let operation = Profiles::default();
    let before = profile_of(&operation, root.path())?;
    let remounted = windows(Reserved::Full, false);

    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.names = Some(MountNames::Profile(remounted));
    });

    assert_eq!(profile_of(&operation, root.path())?, before);
    assert_eq!(profile_of(&operation.clone(), root.path())?, before);
    assert_eq!(
        profile_of(&Profiles::default(), root.path())?.names,
        remounted
    );
    Ok(())
}

#[test]
fn a_mount_learned_again_keeps_what_was_learned_of_it_meanwhile() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(root.path(), Quirks::sshfs());
    let folder = open_local_parent_directory(root.path())?;
    learn_mount_type(fs(), folder.as_fd(), scope.key());
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.symlinks = Some(false);
    });
    let learned = mount_capabilities(scope.key());

    // As a thread that read the mount before the refusal finishes.
    let seen = learn_mount_type(fs(), folder.as_fd(), scope.key());

    assert_eq!(seen, learned);
    assert_eq!(mount_capabilities(scope.key()), learned);
    Ok(())
}

#[test]
fn a_gvfs_location_is_numbered_within_the_limit_of_its_scheme() {
    use gtk::{gio, glib};
    for (uri, scheme) in [
        ("mtp://Google_Pixel_8/Internal%20storage/DCIM", "mtp"),
        ("smb://nas.local/media/films", "smb"),
        ("sftp://me@example.org/home/me", "sftp"),
    ] {
        let folder = gio::File::for_uri(uri);
        assert_eq!(
            glib::MainContext::new().block_on(folder_name_limit(&folder)),
            gvfs_profile(scheme).limit,
            "{uri}"
        );
    }
}

#[test]
fn what_an_operation_learns_after_its_mount_changed_stays_with_the_operation()
-> Result<(), Box<dyn Error>> {
    use crate::adapters::volume::{MountCapabilities, StrictRule};
    let root = tempfile::tempdir()?;
    let bridge = root.path().join("bridge");
    let server = bridge.join("sftp:host=example.org");
    let disk = root.path().join("disk");
    for folder in [&server, &disk] {
        fs::create_dir_all(folder)?;
    }
    let _bridge = FsQuirks::install(
        &bridge,
        Quirks {
            mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
            ..Quirks::default()
        },
    );
    let _disk = FsQuirks::install(&disk, Quirks::sshfs());
    let characters = Strictness::of([StrictRule::Characters]);
    for folder in [&disk, &server] {
        let operation = Profiles::default();
        let opened = open_local_parent_directory(folder)?;
        let (key, seen) = operation.mount(fs(), opened.as_fd());
        let site = operation.site(key, seen.names, opened.as_fd());
        // As a mount change drops what was learned of the mount.
        learn_mount_capability(key, |capabilities| {
            *capabilities = MountCapabilities::default();
        });

        operation.learn(&site, characters);
        operation.refuse_links(&site);

        assert_eq!(operation.taught(&site).strictness, characters, "{folder:?}");
        assert!(!operation.holds_links(&site), "{folder:?}");
        let learned = mount_capabilities(site.key());
        assert_eq!(learned.strictness, Strictness::default(), "{folder:?}");
        assert_eq!(learned.symlinks, None, "{folder:?}");
    }
    Ok(())
}

#[test]
fn items_on_the_gvfs_filesystem_of_a_destination_keep_their_names() {
    use gtk::glib::GString;
    let share = || Some(GString::from("smb-share:server=nas,share=media"));
    let destination = super::PasteDestination {
        profile: MountProfile::gvfs("smb"),
        crossing: vec![None, None, None, Some(true)],
        filesystem: share(),
    };

    let fixing = destination.fixing(&[share(), Some(GString::from("mtp:host=x")), None, share()]);

    assert_eq!(fixing, [false, true, true, true]);
}
