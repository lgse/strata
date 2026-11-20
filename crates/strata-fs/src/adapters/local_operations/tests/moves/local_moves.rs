// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn moving_a_directory_falls_back_to_a_safe_copy_when_the_move_needs_one()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-fallback-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        always_copy(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_files(
        &target,
        &[("top.txt", "top"), ("nested/child.txt", "child")],
        "",
    )?;
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_failed_move_attempt_is_returned_without_falling_back() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-real-failure-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, _, _, _| {
            Box::pin(async {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "injected permission failure",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::PermissionDenied)));
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    assert!(!target.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_successful_move_attempt_is_used_without_falling_back_to_copy() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-success-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, target, _, _| {
            Box::pin(async move {
                fs::rename(
                    source.path().expect("native source"),
                    target.path().expect("native target"),
                )
                .map(|()| super::super::super::Moved::Done(target))
                .map_err(super::super::io_error)
            })
        }),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_plain_move_relocates_the_entry_via_the_hardened_rename_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"payload")?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target)?, b"payload");
    Ok(())
}

fn move_native(source: &Path, target: &Path) -> Result<(), glib::Error> {
    glib::MainContext::default()
        .block_on(move_local(
            gio::File::for_path(source),
            gio::File::for_path(target),
            Taken::Fail,
            gio::Cancellable::new(),
            None,
            NamePolicy::default(),
        ))
        .map(drop)
}

fn assert_refused_into_itself(result: Result<(), glib::Error>) {
    assert_eq!(
        result.map_err(|error| error.message().to_owned()),
        Err(crate::i18n::tr("Can\u{2019}t put a folder inside itself"))
    );
}

#[test]
fn moving_a_directory_into_its_own_child_fails_instead_of_deleting_it() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for quirks in [Quirks::default(), Quirks::vboxsf()] {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks);
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested"))?;
        fs::write(source.join("top.txt"), b"top")?;
        let target = source.join("nested").join("moved-source");

        assert_refused_into_itself(move_native(&source, &target));
        assert_eq!(fs::read(source.join("top.txt"))?, b"top");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(source.join("nested"))?.count(), 0);
    }
    Ok(())
}

#[test]
fn a_move_never_copies_a_folder_into_itself_across_a_nested_mount() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let docs = root.path().join("docs");
    fs::create_dir_all(docs.join("Backup"))?;
    fs::write(docs.join("top.txt"), b"top")?;
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&docs, &link)?;
    let _outer = FsQuirks::install(root.path(), Quirks::default());
    let _nested = FsQuirks::install(&docs.join("Backup"), Quirks::default());

    for target in [docs.join("Backup/docs"), link.join("Backup/docs")] {
        assert_refused_into_itself(move_native(&docs, &target));
        assert_eq!(fs::read(docs.join("top.txt"))?, b"top");
        assert_eq!(fs::read_dir(docs.join("Backup"))?.count(), 0);
    }
    Ok(())
}

#[test]
fn same_volume_move_on_a_flag_rejecting_filesystem_never_copies() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for rejection in [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                rename_flags: Support::Rejected(rejection),
                ..Quirks::default()
            },
        );
        let file = root.path().join("notes.txt");
        let folder = root.path().join("folder");
        fs::write(&file, b"notes")?;
        fs::create_dir(&folder)?;
        fs::write(folder.join("inside.txt"), b"inside")?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let file_inode = fs::metadata(&file)?.ino();
        let folder_inode = fs::metadata(&folder)?.ino();

        move_native(&file, &destination.join("notes.txt"))?;
        move_native(&folder, &destination.join("folder"))?;

        let inodes = (
            fs::metadata(destination.join("notes.txt"))?.ino(),
            fs::metadata(destination.join("folder"))?.ino(),
        );
        assert_eq!(inodes, (file_inode, folder_inode));
        assert_eq!(fs::read(destination.join("folder/inside.txt"))?, b"inside");
        assert!(!file.exists() && !folder.exists());
        assert_eq!(
            scope.count(Op::Create),
            usize::from(rejection == Errno::INVAL)
        );
    }
    Ok(())
}

#[test]
fn items_without_noreplace_or_a_usable_link_are_renamed_without_copying()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As protected_hardlinks refuses another user's file that a rename could move.
    let refused = || Quirks {
        fail: vec![(Op::Link, "moved", 0, Errno::PERM)],
        ..Quirks::without_rename_flags()
    };
    let linked_before_a_probe = || Quirks {
        rename_flags: Support::Rejected(Errno::NOSYS),
        ..Quirks::sshfs()
    };
    let ntfs_3g = || Quirks {
        rename_flags: Support::Rejected(Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let cut = "m".repeat(237);
    let cases: [(&str, Preset, bool, &str); 7] = [
        ("refused", refused, false, "moved"),
        ("sshfs", Quirks::sshfs, false, "moved"),
        ("sshfs", Quirks::sshfs, true, "moved"),
        (
            "linked before a probe",
            linked_before_a_probe,
            false,
            "moved",
        ),
        ("vboxsf", Quirks::vboxsf, false, "moved"),
        ("vboxsf", Quirks::vboxsf, true, "moved"),
        ("ntfs-3g, a name it would cut", ntfs_3g, true, &cut),
    ];
    for (case, quirks, folder, name) in cases {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let source = root.path().join("item");
        let moved = root.path().join(name);
        if folder {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"inside")?;
        } else {
            fs::write(&source, b"notes")?;
        }
        let inode = fs::metadata(&source)?.ino();
        let progress =
            TransferProgressTracker::new(OperationRequestId(1), None, None, Rc::new(|_| {}));

        glib::MainContext::default()
            .block_on(move_local(
                gio::File::for_path(&source),
                gio::File::for_path(&moved),
                Taken::Fail,
                gio::Cancellable::new(),
                Some(progress.clone()),
                NamePolicy::default(),
            ))
            .map_err(|error| format!("{case}: {error}"))?;

        let case = format!("{case}, folder: {folder}");
        assert_eq!(fs::metadata(&moved)?.ino(), inode, "{case}");
        let progressed = (
            progress.completed_files.get(),
            progress.transferred_bytes.get(),
        );
        assert_eq!(progressed, (0, 0), "{case}");
        assert_eq!(names(root.path())?, [name], "{case}");
    }
    Ok(())
}

#[test]
fn items_no_step_can_place_are_moved_by_copying() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As SFTP without `posix-rename` refuses to rename over the reserved file.
    let rename_over_refused = || Quirks {
        fail: vec![(Op::Rename, "moved", 1, Errno::PERM)],
        ..Quirks::sshfs()
    };
    let cases: [(&str, Preset, bool); 3] = [
        ("links unsettled", Quirks::without_placement, false),
        ("rename over refused", rename_over_refused, false),
        (
            "folders unplaceable",
            Quirks::without_folder_placement,
            true,
        ),
    ];
    for (case, quirks, folder) in cases {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks());
        let source = root.path().join("item");
        let moved = root.path().join("moved");
        if folder {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"inside")?;
        } else {
            fs::write(&source, b"inside")?;
        }
        let inode = fs::metadata(&source)?.ino();

        move_native(&source, &moved).map_err(|error| format!("{case}: {error}"))?;

        assert_ne!(fs::metadata(&moved)?.ino(), inode, "{case}");
        let contents = if folder {
            moved.join("inside.txt")
        } else {
            moved.clone()
        };
        assert_eq!(fs::read(contents)?, b"inside", "{case}");
        assert_eq!(names(root.path())?, ["moved"], "{case}");
    }
    Ok(())
}

#[test]
fn einval_from_every_rung_reports_an_invalid_name_and_keeps_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let flags_supported = || Quirks {
        invalid_name: Some(has_colon),
        ..Quirks::default()
    };
    let without_flags_or_links = || Quirks {
        invalid_name: Some(has_colon),
        rename_flags: Support::Rejected(Errno::INVAL),
        links: Support::Rejected(Errno::PERM),
        ..Quirks::default()
    };
    for (quirks, kind) in [
        (flags_supported(), "file"),
        (flags_supported(), "folder"),
        (without_flags_or_links(), "folder"),
        (without_flags_or_links(), "symlink"),
    ] {
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), quirks);
        let source = root.path().join("source");
        match kind {
            "file" => fs::write(&source, b"kept")?,
            "folder" => fs::create_dir(&source)?,
            _ => std::os::unix::fs::symlink("elsewhere", &source)?,
        }
        let target = root.path().join("a:b");

        let error = move_native(&source, &target).expect_err("invalid name");

        assert_eq!(
            error.message(),
            rust_i18n::t!(
                "Could not move %{name}: %{error}",
                name = "source",
                error = crate::services::error_detail(crate::i18n::tr(
                    crate::services::INVALID_NAME_REASON
                ))
            ),
            "{kind}"
        );
        let present = (
            fs::symlink_metadata(&source).is_ok(),
            fs::symlink_metadata(&target).is_ok(),
        );
        assert_eq!(present, (true, false), "{kind}");
        assert_eq!(fs::read_dir(root.path())?.count(), 1, "{kind}");
    }
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!actual_parent.join("source.txt").exists());
    assert_eq!(fs::read(target)?, b"keep");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_destinations_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let actual_destination = root.path().join("actual");
    let linked_destination = root.path().join("linked");
    fs::create_dir(&actual_destination)?;
    fs::write(&source, b"keep")?;
    std::os::unix::fs::symlink(&actual_destination, &linked_destination)?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(linked_destination.join("target.txt")),
        Taken::Fail,
        gio::Cancellable::new(),
        None,
        NamePolicy::default(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(actual_destination.join("target.txt"))?, b"keep");
    Ok(())
}
