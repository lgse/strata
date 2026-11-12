// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_racer_filling_or_swapping_the_placeholder_wins() -> Result<(), Box<dyn Error>> {
    for swap in [false, true] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("mine"), b"mine")?;
        let target = root.path().join("target");
        let raced = target.clone();
        let racer: Racer = Box::new(move || {
            if swap {
                fs::remove_dir(&raced).expect("remove placeholder");
                fs::write(&raced, b"theirs").expect("swap placeholder");
            } else {
                fs::write(raced.join("theirs"), b"theirs").expect("fill placeholder");
            }
        });
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                // The second rename into `target` is the one over the placeholder.
                before: vec![(Op::Rename, "target", 1, racer)],
                ..without_flags(Errno::NOSYS)
            },
        );

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(PlaceError::Exists)
        );
        assert_eq!(fs::read(source.join("mine"))?, b"mine");
        if swap {
            assert_eq!(fs::read(&target)?, b"theirs");
        } else {
            assert_eq!(fs::read(target.join("theirs"))?, b"theirs");
        }
    }
    Ok(())
}

#[test]
fn an_empty_own_placeholder_rejected_means_unsupported() -> Result<(), Box<dyn Error>> {
    for (over_empty, learned) in [
        (OverEmptyDirectory::NotEmpty, Some(false)),
        (OverEmptyDirectory::Eperm, None),
    ] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                rename_over_empty_dir: over_empty,
                ..without_flags(Errno::NOSYS)
            },
        );
        fs::create_dir(root.path().join("source"))?;

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(PlaceError::Unsupported)
        );
        assert_eq!(names(root.path())?, ["source"]);
        assert_eq!(
            mount_capabilities(scope.key()).rename_over_empty_directory,
            learned
        );
    }
    Ok(())
}

#[test]
fn an_ambiguous_rename_failure_is_resolved_by_identity() -> Result<(), Box<dyn Error>> {
    for (quirks, folder, placed) in [
        (Quirks::default(), true, Placed::Renamed),
        (without_flags(Errno::NOSYS), true, Placed::ReservedDirectory),
        (Quirks::vboxsf(), false, Placed::OverReservedFile),
        (Quirks::vboxsf(), true, Placed::AfterCheck),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        make(&source, folder)?;
        let original = identity(&source);
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                lost_reply: vec![(Op::Rename, "target", Errno::NOENT)],
                ..quirks
            },
        );

        assert_eq!(
            place_in(root.path(), "source", "target", Some(original)),
            Ok(placed)
        );
        assert_eq!(identity(&root.path().join("target")), original);
        assert_eq!(names(root.path())?, ["target"]);
    }
    Ok(())
}

/// What a placement over a placeholder ends as when the reply to the
/// rename over it is lost, if it was `performed`, or it failed.
fn lost_reply_outcome(
    numbers: InodeNumbers,
    performed: bool,
    folder: bool,
) -> Result<Placed, PlaceError> {
    let placed = if folder {
        Placed::ReservedDirectory
    } else {
        Placed::OverReservedFile
    };
    match (numbers, performed) {
        (InodeNumbers::Renewed, _) | (InodeNumbers::Unknown, false) => Err(PlaceError::Unconfirmed),
        (_, true) => Ok(placed),
        (InodeNumbers::Kept, false) => Err(PlaceError::Io(Errno::IO)),
    }
}

fn check_lost_placeholder_reply(
    preset: Preset,
    folder: bool,
    numbers: InodeNumbers,
    performed: bool,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    if folder {
        fs::create_dir(&source)?;
    } else {
        // Empty, so the moved file passes for the placeholder at `target`.
        fs::write(&source, b"")?;
    }
    let original = identity(&source);
    let mut quirks = preset();
    if performed {
        quirks.lost_reply.push((Op::Rename, "target", Errno::IO));
    } else {
        // The second rename into `target` is the one over the placeholder.
        quirks.fail.push((Op::Rename, "target", 1, Errno::IO));
    }
    let case = format!(
        "{:?}, folder: {folder}, performed: {performed}",
        quirks.mount_type
    );
    let _scope = FsQuirks::install(root.path(), quirks);

    assert_eq!(
        place_in(root.path(), "source", "target", Some(original)),
        lost_reply_outcome(numbers, performed, folder),
        "{case}"
    );
    if performed {
        return Ok(assert_only(root.path(), "target", original, &case)?);
    }
    let left: &[&str] = if numbers == InodeNumbers::Kept {
        &["source"]
    } else {
        &["source", "target"]
    };
    assert_eq!(names(root.path())?, left, "{case}");
    assert_eq!(identity(&source), original, "{case}");
    Ok(())
}

#[test]
fn a_lost_reply_over_a_placeholder_removes_only_one_proven_untouched() -> Result<(), Box<dyn Error>>
{
    // Stable inode numbers tell an untouched placeholder from the moved item,
    // which libfuse without `use_ino` and vboxsf number as the placeholder it
    // replaced.
    let vboxsf = || Quirks {
        lost_renames_keep_inodes: true,
        ..Quirks::vboxsf()
    };
    let links_refused = || links_refused_for_the_item().on_ext4();
    let folders = || without_flags(Errno::NOSYS).on_ext4();
    let unknown_type = || Quirks {
        mount_type: Some(("jfs", "rw")),
        ..links_refused_for_the_item()
    };
    let unread_type = || Quirks {
        fail: vec![(Op::MountType, "*", 0, Errno::ACCESS)],
        ..without_flags(Errno::NOSYS)
    };
    let share = || Quirks {
        mount_type: Quirks::cifs().mount_type,
        ..without_flags(Errno::NOSYS)
    };
    let share_without_server_numbers = || Quirks {
        mount_type: Quirks::cifs_noserverino().mount_type,
        lost_renames_keep_inodes: true,
        ..without_flags(Errno::NOSYS)
    };
    let rows: [(Preset, bool, InodeNumbers); 9] = [
        (Quirks::sshfs, false, InodeNumbers::Renewed),
        (Quirks::sshfs, true, InodeNumbers::Renewed),
        (vboxsf, false, InodeNumbers::Renewed),
        (links_refused, false, InodeNumbers::Kept),
        (folders, true, InodeNumbers::Kept),
        (unknown_type, false, InodeNumbers::Unknown),
        (unread_type, true, InodeNumbers::Unknown),
        (share, true, InodeNumbers::Kept),
        (share_without_server_numbers, true, InodeNumbers::Renewed),
    ];
    for (preset, folder, numbers) in rows {
        for performed in [true, false] {
            check_lost_placeholder_reply(preset, folder, numbers, performed)?;
        }
    }
    Ok(())
}

#[test]
fn another_item_at_either_name_is_reported_and_never_moved() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Swap {
        SourceBeforePlacing,
        SourceAsItMoves,
        TargetOnceMoved,
    }
    // `skip` passes the renames into `target` that precede the one that moves it.
    let presets: [(Preset, bool, usize); 4] = [
        (Quirks::default, false, 0),
        (|| without_flags(Errno::NOSYS), true, 1),
        (Quirks::vboxsf, false, 1),
        (Quirks::vboxsf, true, 2),
    ];
    for (quirks, folder, skip) in presets {
        for swap in [
            Swap::SourceBeforePlacing,
            Swap::SourceAsItMoves,
            Swap::TargetOnceMoved,
        ] {
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            make(&source, folder)?;
            let original = identity(&source);
            let swapped = match swap {
                Swap::TargetOnceMoved => root.path().join("target"),
                _ => source.clone(),
            };
            let (raced, moved) = (swapped, root.path().join("moved"));
            let mut racer: Racer = Box::new(move || {
                fs::rename(&raced, &moved).expect("move the item away");
                make(&raced, folder).expect("substitute");
            });
            let mut quirks = quirks();
            match swap {
                Swap::SourceBeforePlacing => racer(),
                Swap::SourceAsItMoves => quirks.before = vec![(Op::Rename, "target", skip, racer)],
                Swap::TargetOnceMoved => quirks.after = vec![(Op::Rename, "target", skip, racer)],
            }
            let scope = FsQuirks::install(root.path(), quirks);
            let case = format!("folder: {folder}, {swap:?}");

            let result = place_in(root.path(), "source", "target", Some(original));

            assert_eq!(identity(&root.path().join("moved")), original, "{case}");
            if let Swap::SourceBeforePlacing = swap {
                assert_eq!(result, Err(PlaceError::Changed), "{case}");
                assert_eq!(names(root.path())?, ["moved", "source"], "{case}");
                assert_eq!(scope.count(Op::Rename), 0, "{case}");
            } else {
                assert_eq!(result, Err(PlaceError::Unconfirmed), "{case}");
                assert_eq!(names(root.path())?, ["moved", "target"], "{case}");
            }
        }
    }
    Ok(())
}

#[test]
fn an_ambiguous_mkdir_failure_is_not_taken_for_a_taken_name() -> Result<(), Box<dyn Error>> {
    for (lost, expected, left) in [
        (false, PlaceError::Io(Errno::IO), vec!["source"]),
        // A lost reply leaves the documented empty folder behind.
        (true, PlaceError::Exists, vec!["source", "target"]),
    ] {
        let root = tempfile::tempdir()?;
        let mut quirks = without_flags(Errno::NOSYS);
        if lost {
            quirks.lost_reply = vec![(Op::Mkdir, "target", Errno::IO)];
        } else {
            quirks.fail = vec![(Op::Mkdir, "target", 0, Errno::IO)];
        }
        let _scope = FsQuirks::install(root.path(), quirks);
        fs::create_dir(root.path().join("source"))?;

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Err(expected),
            "lost: {lost}"
        );
        assert_eq!(names(root.path())?, left, "lost: {lost}");
    }
    Ok(())
}

#[test]
fn cross_device_is_reported_without_copying() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let (left, right) = (root.path().join("left"), root.path().join("right"));
    fs::create_dir(&left)?;
    fs::create_dir(&right)?;
    fs::write(left.join("source"), b"data")?;
    let _volumes = separate_volumes(&left, &right);
    let from = open_local_parent_directory(&left)?;
    let to = open_local_parent_directory(&right)?;

    let result = place(
        fs(),
        Entry::new(&from, OsStr::new("source")),
        Entry::new(&to, OsStr::new("target")),
        None,
    );

    assert_eq!(result, Err(PlaceError::CrossDevice));
    assert_eq!(names(&left)?, ["source"]);
    assert!(names(&right)?.is_empty());
    Ok(())
}
