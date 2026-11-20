// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn items_without_noreplace_or_a_usable_hard_link_are_placed_over_a_reserved_name()
-> Result<(), Box<dyn Error>> {
    // The folder placeholder places folders on sshfs before links are ever probed.
    let cases: [(Preset, bool, Option<bool>, Placed); 5] = [
        (Quirks::vboxsf, false, Some(false), Placed::OverReservedFile),
        (Quirks::vboxsf, true, Some(false), Placed::AfterCheck),
        (
            links_refused_for_the_item,
            false,
            Some(true),
            Placed::OverReservedFile,
        ),
        (Quirks::sshfs, false, Some(false), Placed::OverReservedFile),
        (Quirks::sshfs, true, None, Placed::ReservedDirectory),
    ];
    for (quirks, folder, links, placed) in cases {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(root.path(), quirks());
        let source = root.path().join("source");
        make(&source, folder)?;
        let original = identity(&source);

        assert_eq!(
            place_in(root.path(), "source", "target", Some(original)),
            Ok(placed)
        );

        assert_eq!(names(root.path())?, ["target"], "{placed:?}");
        assert_eq!(identity(&root.path().join("target")), original);
        let capabilities = mount_capabilities(scope.key());
        assert_eq!(capabilities.no_replace, Some(false));
        assert_eq!(capabilities.hard_links, links, "{placed:?}");
    }
    Ok(())
}

#[test]
fn the_reserved_name_rung_never_clobbers_a_plain_creator() -> Result<(), Box<dyn Error>> {
    let cases: [(Preset, bool); 4] = [
        (Quirks::vboxsf, false),
        (Quirks::vboxsf, true),
        (links_refused_for_the_item, false),
        (Quirks::sshfs, false),
    ];
    for (index, (quirks, folder)) in cases.into_iter().enumerate() {
        for early in [true, false] {
            let root = tempfile::tempdir()?;
            let target = root.path().join("target");
            let raced = target.clone();
            let blocked = Arc::new(std::sync::Mutex::new(None));
            let outcome = blocked.clone();
            let racer: Racer = Box::new(move || {
                let created = if folder {
                    fs::create_dir(&raced).and_then(|()| fs::write(raced.join("theirs"), b"theirs"))
                } else {
                    fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&raced)
                        .and_then(|mut file| io::Write::write_all(&mut file, b"theirs"))
                };
                *outcome.lock().expect("racer outcome") = Some(created.is_err());
            });
            // Files: before the reserving create, or before the rename over it.
            // Folders: before the name check, or before the plain rename.
            let before = match (folder, early) {
                (false, true) => (Op::Create, "target", 0, racer),
                (false, false) => (Op::Rename, "target", 1, racer),
                (true, true) => (Op::Stat, "target", 2, racer),
                (true, false) => (Op::Rename, "target", 2, racer),
            };
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    before: vec![before],
                    ..quirks()
                },
            );
            let source = root.path().join("source");
            make(&source, folder)?;
            let original = identity(&source);

            let result = place_in(root.path(), "source", "target", None);

            let racer_blocked = blocked.lock().expect("racer outcome").expect("racer ran");
            let case = format!("case {index}, early: {early}");
            if racer_blocked {
                assert!(!folder && !early, "{case}");
                assert!(result.is_ok(), "{case}");
                assert_eq!(identity(&target), original, "{case}");
            } else {
                assert_eq!(result, Err(PlaceError::Exists), "{case}");
                assert_eq!(identity(&source), original, "{case}");
                let theirs = if folder {
                    target.join("theirs")
                } else {
                    target.clone()
                };
                assert_eq!(fs::read(theirs)?, b"theirs", "{case}");
            }
        }
    }
    Ok(())
}

#[test]
fn a_reservation_swapped_before_its_check_keeps_the_swapped_in_item() -> Result<(), Box<dyn Error>>
{
    for quirks in [Quirks::vboxsf, Quirks::sshfs] {
        let root = tempfile::tempdir()?;
        let dir = root.path().to_path_buf();
        let racer: Racer = Box::new(move || {
            fs::write(dir.join("theirs"), b"theirs").expect("write");
            fs::rename(dir.join("theirs"), dir.join("target")).expect("swap the reservation");
        });
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                // The first stat of `target` is the check that it still holds the reservation.
                before: vec![(Op::Stat, "target", 0, racer)],
                ..quirks()
            },
        );
        fs::write(root.path().join("source"), b"data")?;
        let original = identity(&root.path().join("source"));

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(PlaceError::Exists)
        );
        assert_eq!(fs::read(root.path().join("target"))?, b"theirs");
        assert_eq!(identity(&root.path().join("source")), original);
        assert_eq!(names(root.path())?, ["source", "target"]);
    }
    Ok(())
}

#[test]
fn the_reserved_name_rung_needs_the_missing_primitives_confirmed() -> Result<(), Box<dyn Error>> {
    let noreplace_unsettled = || Quirks {
        fail: vec![(Op::Rename, ".strata-probe-*", 0, Errno::IO)],
        ..Quirks::vboxsf()
    };
    let links_unsettled: Preset = Quirks::without_placement;
    for quirks in [links_unsettled, noreplace_unsettled] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(root.path(), quirks());
        make(&root.path().join("source"), false)?;

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(PlaceError::Unsupported)
        );
        assert_eq!(names(root.path())?, ["source"]);
        // The probe runs once, however many rungs consult it.
        assert_eq!(scope.count(Op::Create), 1);
    }
    Ok(())
}

#[test]
fn a_failed_rename_over_the_reserved_file_removes_only_an_untouched_placeholder()
-> Result<(), Box<dyn Error>> {
    // EPERM is how SFTP without `posix-rename` refuses any existing target.
    for (error, written, expected) in [
        (Errno::ACCESS, false, PlaceError::Io(Errno::ACCESS)),
        (Errno::ACCESS, true, PlaceError::Io(Errno::ACCESS)),
        (Errno::PERM, false, PlaceError::Unsupported),
        (Errno::PERM, true, PlaceError::Io(Errno::PERM)),
    ] {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        let raced = target.clone();
        let mut quirks = Quirks {
            // The second rename into `target` is the one over the reserved file.
            fail: vec![(Op::Rename, "target", 1, error)],
            ..Quirks::vboxsf()
        };
        if written {
            let racer: Racer = Box::new(move || {
                fs::write(&raced, b"theirs").expect("write into the placeholder");
            });
            quirks.before = vec![(Op::Rename, "target", 1, racer)];
        }
        let _scope = FsQuirks::install(root.path(), quirks);
        fs::write(root.path().join("source"), b"data")?;

        let case = format!("{error:?}, written: {written}");
        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(expected),
            "{case}"
        );
        assert_eq!(fs::read(root.path().join("source"))?, b"data", "{case}");
        if written {
            assert_eq!(fs::read(&target)?, b"theirs", "{case}");
        } else {
            assert_eq!(names(root.path())?, ["source"], "{case}");
        }
    }
    Ok(())
}

#[test]
fn a_reserved_name_reported_as_a_folder_is_taken() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Create, "target", 0, Errno::ISDIR)],
            ..Quirks::vboxsf()
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Err(PlaceError::Exists)
    );
    assert_eq!(names(root.path())?, ["source"]);
    Ok(())
}

#[test]
fn names_ntfs_3g_would_cut_are_never_renamed_over() -> Result<(), Box<dyn Error>> {
    use {
        PlaceError::{Exists, Unsupported},
        Placed::{AfterCheck, OverReservedFile, ReservedDirectory},
    };
    fn ntfs_3g() -> Quirks {
        Quirks {
            rename_flags: Support::Rejected(Errno::INVAL),
            ..Quirks::ntfs_3g()
        }
    }
    let unknown_fuseblk = || Quirks {
        device_format: None,
        ..ntfs_3g()
    };
    let exfat_fuseblk = || Quirks {
        device_format: Some("exfat"),
        cut_rename_over_names: false,
        ..ntfs_3g()
    };
    let links_refused = || Quirks {
        fail: vec![(Op::Link, "x*", 0, Errno::PERM)],
        ..ntfs_3g()
    };
    let probe_unsettled = || Quirks {
        fail: vec![(Op::Create, ".strata-probe-*", 0, Errno::IO)],
        ..ntfs_3g()
    };
    let (fits, cut, accented) = ("x".repeat(236), "x".repeat(237), "é".repeat(120));
    type Row<'a> = (Preset, bool, &'a str, bool, Result<Placed, PlaceError>);
    let rows: [Row<'_>; 9] = [
        (ntfs_3g, true, &fits, false, Ok(ReservedDirectory)),
        (ntfs_3g, true, &cut, false, Ok(AfterCheck)),
        (ntfs_3g, true, &cut, true, Err(Exists)),
        (probe_unsettled, true, &cut, false, Err(Unsupported)),
        (ntfs_3g, true, &accented, false, Ok(ReservedDirectory)),
        (unknown_fuseblk, true, &cut, false, Ok(AfterCheck)),
        (exfat_fuseblk, true, &cut, false, Ok(ReservedDirectory)),
        (links_refused, false, &fits, false, Ok(OverReservedFile)),
        (links_refused, false, &cut, false, Err(Unsupported)),
    ];
    for (row, (preset, folder, name, arrives, expected)) in rows.into_iter().enumerate() {
        let context = format!("row {row}, {} units", name.chars().count());
        let root = tempfile::tempdir()?;
        let mut quirks = preset();
        if arrives {
            let arrival = root.path().join(name);
            let racer: Racer = Box::new(move || fs::create_dir(&arrival).expect("arrival"));
            quirks.after.push((Op::Rename, "x*", 0, racer));
        }
        let _scope = FsQuirks::install(root.path(), quirks);
        make(&root.path().join("source"), folder)?;

        assert_eq!(
            place_in(root.path(), "source", name, None),
            expected,
            "{context}"
        );

        let left = match (expected.is_ok(), arrives) {
            (true, _) => vec![name],
            (false, false) => vec!["source"],
            (false, true) => vec!["source", name],
        };
        assert_eq!(names(root.path())?, left, "{context}");
        if arrives {
            assert!(names(&root.path().join(name))?.is_empty(), "{context}");
        }
    }
    Ok(())
}
