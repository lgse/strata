// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Item {
    File,
    Folder,
    ReadOnlyFolder,
}

fn mode(path: &Path) -> io::Result<u32> {
    fs::metadata(path).map(|metadata| metadata.mode() & 0o7777)
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// The modes of what `root` holds, each opened again for the cleanup.
fn modes_reopened(root: &Path, left: &[String]) -> io::Result<Vec<Option<u32>>> {
    left.iter()
        .map(|name| {
            let path = root.join(name);
            let kept = mode(&path).ok();
            set_mode(&path, 0o755).map(|()| kept)
        })
        .collect()
}

fn check_case_only_rename(quirks: Preset, item: Item) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), case_insensitive(quirks()));
    let source = root.path().join("report");
    make(&source, !matches!(item, Item::File))?;
    if let Item::ReadOnlyFolder = item {
        set_mode(&source, 0o555)?;
    }
    let original = identity(&source);
    let original_mode = forced_mode(&source)? & 0o7777;

    assert_eq!(
        rename_in(root.path(), "report", "report"),
        Ok(Renamed::Unchanged)
    );
    let renamed = rename_in(root.path(), "report", "Report");

    let left = names(root.path())?;
    let modes = modes_reopened(root.path(), &left)?;
    assert_eq!(renamed, Ok(Renamed::CaseOnly), "{item:?}");
    assert_eq!(left, ["Report"], "{item:?}");
    assert_eq!(
        (identity(&root.path().join("Report")), modes),
        (original, vec![Some(original_mode)]),
        "{item:?}"
    );
    Ok(())
}

#[test]
fn a_case_only_rename_goes_through_a_temporary_name() -> Result<(), Box<dyn Error>> {
    let smb = || Quirks {
        stale_aliases: true,
        ..Quirks::cifs()
    };
    let exfat_fuse = || Quirks {
        alias_inodes: true,
        ..Quirks::exfat_fuse()
    };
    let plain: &[Item] = &[Item::File, Item::Folder];
    let shapes: [(Preset, &[Item]); 4] = [
        (
            Quirks::default,
            &[Item::File, Item::Folder, Item::ReadOnlyFolder],
        ),
        (Quirks::vboxsf, plain),
        (smb, plain),
        (exfat_fuse, plain),
    ];
    for (quirks, items) in shapes {
        for &item in items {
            check_case_only_rename(quirks, item)?;
        }
    }
    Ok(())
}

#[test]
fn a_case_only_rename_is_not_refused_as_taken_where_each_spelling_has_its_own_number()
-> Result<(), Box<dyn Error>> {
    let exfat_fuse = || Quirks {
        alias_inodes: true,
        ..Quirks::exfat_fuse()
    };
    let cifs_nocase = || Quirks {
        alias_inodes: true,
        ..Quirks::cifs_nocase()
    };
    let rows: [(Preset, bool, bool, bool); 6] = [
        (Quirks::default, false, false, false),
        (Quirks::default, true, false, true),
        (exfat_fuse, false, false, false),
        (exfat_fuse, true, false, true),
        (cifs_nocase, false, true, false),
        (cifs_nocase, true, true, true),
    ];
    for (quirks, another_item, serverino_off, taken) in rows {
        for folder in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), case_insensitive(quirks()));
            let (current, wanted) = (root.path().join("report"), root.path().join("Report"));
            make(&current, folder)?;
            if another_item {
                fs::write(&wanted, b"another item")?;
            }
            if serverino_off {
                assert!(super::super::fs_quirks::autodisable_serverino(root.path()));
            }

            assert_eq!(
                super::super::super::typed_names::rename_taken(&current, &wanted),
                taken,
                "{:?}, folder: {folder}, another item: {another_item}",
                quirks().mount_type
            );
        }
    }
    Ok(())
}

#[test]
fn a_file_handle_follows_the_item_and_not_its_name() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let dir = open_local_parent_directory(root.path())?;
    let handle =
        |name: &str| super::super::super::native_fs::file_handle(dir.as_fd(), OsStr::new(name));
    if !base_offers(&FILE_HANDLES)? {
        return Ok(());
    }
    fs::write(root.path().join("report"), b"first")?;
    let first = handle("report")?;
    fs::rename(root.path().join("report"), root.path().join("renamed"))?;
    assert_eq!(handle("renamed"), Ok(first));
    let held = fs::File::open(root.path().join("renamed"))?;
    assert_eq!(
        super::super::super::native_fs::file_handle(held.as_fd(), OsStr::new("")),
        Ok(first)
    );
    fs::remove_file(root.path().join("renamed"))?;
    fs::write(root.path().join("renamed"), b"first")?;
    assert_ne!(handle("renamed"), Ok(first));
    assert_eq!(handle("missing"), Err(Errno::NOENT));
    Ok(())
}

#[test]
fn a_hard_link_is_never_taken_for_a_case_alias() -> Result<(), Box<dyn Error>> {
    for (link, wanted) in [("Report", "Report"), ("Notes", "notes")] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(root.path(), case_insensitive(Quirks::default()));
        fs::write(root.path().join("report"), b"data")?;
        fs::hard_link(root.path().join("report"), root.path().join(link))?;

        assert!(
            super::super::super::typed_names::rename_taken(
                &root.path().join("report"),
                &root.path().join(wanted)
            ),
            "{wanted}"
        );
        assert_eq!(
            (
                rename_in(root.path(), "report", wanted),
                scope.count(Op::Rename)
            ),
            (Err(RenameFailure::Place(PlaceError::Exists)), 1),
            "{wanted}"
        );
        assert_eq!(names(root.path())?, [link, "report"]);
        assert_eq!(fs::metadata(root.path().join("report"))?.nlink(), 2);
    }
    Ok(())
}

#[test]
fn a_twin_spelling_is_never_taken_for_a_case_alias() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    for name in ["report", "Report"] {
        fs::write(root.path().join(name), b"data")?;
        fs::File::options()
            .write(true)
            .open(root.path().join(name))?
            .set_modified(modified)?;
    }
    // Separate nodes alike in type, size and time, as libfuse without `use_ino`
    // numbers each spelling; the first listed would pass for the item itself.
    let listed = fs::read_dir(root.path())?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let scope = FsQuirks::install(root.path(), Quirks::default());

    assert_eq!(
        rename_in(root.path(), &listed[0], &listed[1]),
        Err(RenameFailure::Place(PlaceError::Exists))
    );
    assert_eq!(scope.count(Op::Rename), 1);
    assert_eq!(names(root.path())?, ["Report", "report"]);
    Ok(())
}

#[test]
fn a_failed_case_only_rename_moves_back_or_names_the_temporary() -> Result<(), Box<dyn Error>> {
    for moves_back in [true, false] {
        let root = tempfile::tempdir()?;
        let mut fail = vec![(Op::Rename, "Report", 1, Errno::IO)];
        if !moves_back {
            // Skips the direct attempt and the move to the temporary name.
            fail.push((Op::Rename, "report", 1, Errno::IO));
        }
        let _scope = FsQuirks::install(
            root.path(),
            case_insensitive(Quirks {
                fail,
                ..Quirks::default()
            }),
        );
        let source = root.path().join("report");
        fs::write(&source, b"data")?;
        let original = identity(&source);

        let result = rename_in(root.path(), "report", "Report");

        let left = names(root.path())?;
        assert_eq!(left.len(), 1);
        if moves_back {
            assert_eq!(result, Err(RenameFailure::Place(PlaceError::Io(Errno::IO))));
            assert_eq!(left, ["report"]);
            assert_eq!(identity(&source), original);
        } else {
            assert!(left[0].starts_with(".strata-originals-"), "{left:?}");
            assert_only(&root.path().join(&left[0]), "report", original, "")?;
            assert_eq!(
                result,
                Err(RenameFailure::Stranded {
                    temporary: Path::new(&left[0]).join("report").into_os_string(),
                    error: PlaceError::Io(Errno::IO),
                })
            );
        }
    }
    Ok(())
}

#[test]
fn a_hard_link_reporting_another_inode_is_rolled_back_and_never_used_again()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            // A definitive errno, so the ladder links before any probe.
            rename_flags: Support::Rejected(Errno::NOSYS),
            ..Quirks::sshfs()
        },
    );
    fs::write(root.path().join("source"), b"data")?;
    let original = identity(&root.path().join("source"));

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::OverReservedFile)
    );
    assert_only(root.path(), "target", original, "")?;
    assert_eq!(mount_capabilities(scope.key()).hard_links, Some(false));

    scope.clear_calls();
    assert_eq!(
        place_in(root.path(), "target", "moved", None),
        Ok(Placed::OverReservedFile)
    );
    assert_eq!(scope.count(Op::Link), 0);
    assert_only(root.path(), "moved", original, "")?;
    Ok(())
}

fn check_replaced_new_link(
    case: &str,
    quirks: Preset,
    links_work: bool,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let dir = root.path().to_path_buf();
    let racer: Racer = Box::new(move || {
        fs::write(dir.join("theirs"), b"theirs").expect("write");
        fs::rename(dir.join("theirs"), dir.join("target")).expect("replace");
    });
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::Stat, "target", 0, racer)],
            ..quirks()
        },
    );
    fs::write(root.path().join("source"), b"data")?;
    let original = identity(&root.path().join("source"));

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Err(PlaceError::Unconfirmed),
        "{case}"
    );
    assert_eq!(
        (
            fs::read(root.path().join("target"))?,
            identity(&root.path().join("source"))
        ),
        (b"theirs".to_vec(), original)
    );
    assert_eq!(mount_capabilities(scope.key()).hard_links, Some(links_work));
    if links_work {
        assert_eq!(
            place_in(root.path(), "source", "moved", None),
            Ok(Placed::Linked)
        );
        assert_eq!(names(root.path())?, ["moved", "target"]);
    } else {
        assert_eq!(names(root.path())?, ["source", "target"]);
    }
    Ok(())
}

#[test]
fn a_racer_replacing_a_new_link_keeps_its_item() -> Result<(), Box<dyn Error>> {
    // NOSYS links before any probe; EINVAL probes first. On sshfs the probe
    // after the link finds that links renumber.
    let cases: [(&str, Preset, bool); 3] = [
        ("NOSYS", || without_flags(Errno::NOSYS), true),
        ("INVAL", || without_flags(Errno::INVAL), true),
        (
            "sshfs",
            || Quirks {
                rename_flags: Support::Rejected(Errno::NOSYS),
                ..Quirks::sshfs()
            },
            false,
        ),
    ];
    for (case, quirks, links_work) in cases {
        check_replaced_new_link(case, quirks, links_work)?;
    }
    Ok(())
}
