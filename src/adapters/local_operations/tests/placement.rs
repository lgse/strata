// SPDX-License-Identifier: MIT

use std::os::{fd::AsFd, unix::fs::MetadataExt};

use rustix::io::Errno;

use super::super::{
    LocalFileIdentity,
    fs_ops::{Entry, fs},
    placement::{
        Feasibility, PlaceError, Placed, Probed, RenameFailure, Renamed, can_set_aside, place,
        probe, rename_in_place, set_aside,
    },
};
use super::fs_quirks::{Op, OverEmptyDirectory, Racer, Support};
use super::*;
use crate::{
    adapters::volume::{MountCapabilities, mount_capabilities},
    services::InodeNumbers,
};

fn without_flags(error: Errno) -> Quirks {
    Quirks {
        rename_flags: Support::Rejected(error),
        ..Quirks::default()
    }
}

fn identity(path: &Path) -> LocalFileIdentity {
    LocalFileIdentity::from_stat(&rustix::fs::lstat(path).expect("entry exists"))
}

fn place_in(
    dir: &Path,
    from: &str,
    to: &str,
    expected: Option<LocalFileIdentity>,
) -> Result<Placed, PlaceError> {
    let parent = open_local_parent_directory(dir).expect("open parent");
    place(
        fs(),
        Entry::new(&parent, OsStr::new(from)),
        Entry::new(&parent, OsStr::new(to)),
        expected,
    )
}

fn fingerprint(path: &Path) -> String {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return "absent".to_owned();
    };
    if metadata.file_type().is_symlink() {
        format!("link {:?}", fs::read_link(path).expect("read link"))
    } else if metadata.is_dir() {
        format!("dir {} {:?}", metadata.ino(), names(path).expect("list"))
    } else {
        format!(
            "file {} {:?}",
            metadata.ino(),
            fs::read(path).expect("read")
        )
    }
}

#[test]
fn noreplace_success_needs_no_fallback_stat_or_mount_key() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(root.path(), Quirks::default());
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::Renamed)
    );

    let calls = scope.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].op, Op::Rename);
    assert_eq!(calls[0].names, ["source", "target"]);
    Ok(())
}

#[test]
fn rejected_rename_flags_link_files_without_copying() -> Result<(), Box<dyn Error>> {
    for error in [Errno::INVAL, Errno::NOSYS, Errno::OPNOTSUPP] {
        for kind in ["file", "symlink", "fifo"] {
            let root = tempfile::tempdir()?;
            let scope = FsQuirks::install(root.path(), without_flags(error));
            let source = root.path().join("source");
            match kind {
                "file" => fs::write(&source, b"data")?,
                "symlink" => std::os::unix::fs::symlink("elsewhere", &source)?,
                _ => rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &source,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_raw_mode(0o600),
                    0,
                )?,
            }
            let original = identity(&source);

            assert_eq!(
                place_in(root.path(), "source", "target", None),
                Ok(Placed::Linked),
                "{error:?} {kind}"
            );

            assert_eq!(identity(&root.path().join("target")), original);
            assert_eq!(names(root.path())?, ["target"]);
            assert_eq!(mount_capabilities(scope.key()).no_replace, Some(false));
            // Only an ambiguous EINVAL needs the probe to learn this.
            let probed = scope.count(Op::Create) > 0;
            assert_eq!(probed, error == Errno::INVAL, "{error:?} {kind}");
        }
    }
    Ok(())
}

#[test]
fn rejected_rename_flags_move_folders_over_a_placeholder() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), without_flags(Errno::NOSYS));
    let source = root.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("inside.txt"), b"inside")?;
    let original = identity(&source);

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::ReservedDirectory)
    );

    let target = root.path().join("target");
    assert_eq!(identity(&target), original);
    assert_eq!(fs::read(target.join("inside.txt"))?, b"inside");
    assert_eq!(names(root.path())?, ["target"]);
    Ok(())
}

fn create_destination(path: &Path, kind: &str) {
    match kind {
        "file" => fs::write(path, b"theirs"),
        "empty folder" => fs::create_dir(path),
        "full folder" => fs::create_dir(path).and_then(|()| fs::write(path.join("child"), b"x")),
        "symlink" => std::os::unix::fs::symlink("other", path),
        _ => std::os::unix::fs::symlink("missing", path),
    }
    .expect("create destination");
}

#[test]
fn the_ladder_never_replaces_an_existing_destination() -> Result<(), Box<dyn Error>> {
    let quirk_sets: [Preset; 4] = [
        Quirks::default,
        || without_flags(Errno::NOSYS),
        || Quirks {
            collisions_as_eperm: true,
            ..without_flags(Errno::NOSYS)
        },
        || Quirks {
            links: Support::Rejected(Errno::PERM),
            ..without_flags(Errno::NOSYS)
        },
    ];
    for (index, quirks) in quirk_sets.into_iter().enumerate() {
        for source_is_folder in [false, true] {
            for kind in [
                "file",
                "empty folder",
                "full folder",
                "symlink",
                "dangling symlink",
            ] {
                let root = tempfile::tempdir()?;
                fs::write(root.path().join("other"), b"other")?;
                let source = root.path().join("source");
                if source_is_folder {
                    fs::create_dir(&source)?;
                    fs::write(source.join("mine"), b"mine")?;
                } else {
                    fs::write(&source, b"mine")?;
                }
                let target = root.path().join("target");
                let mut quirks = quirks();
                if index == 0 {
                    create_destination(&target, kind);
                } else {
                    // Rung 1 sees no destination; it appears in front of the fallback rung.
                    let raced = target.clone();
                    let fallback = if source_is_folder {
                        Op::Mkdir
                    } else {
                        Op::Link
                    };
                    let racer: Racer = Box::new(move || create_destination(&raced, kind));
                    quirks.before.push((fallback, "target", 0, racer));
                }
                let _scope = FsQuirks::install(root.path(), quirks);
                let source_before = fingerprint(&source);
                let target_before = (index == 0).then(|| fingerprint(&target));

                let result = place_in(root.path(), "source", "target", None);

                assert_eq!(result, Err(PlaceError::Exists), "{index} {kind}");
                assert_eq!(fingerprint(&source), source_before, "{index} {kind}");
                if let Some(target_before) = target_before {
                    assert_eq!(fingerprint(&target), target_before);
                }
                assert_eq!(
                    names(root.path())?,
                    ["other", "source", "target"],
                    "{index} {kind}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn an_invalid_name_on_a_fat_like_mount_is_reported_and_learns_nothing() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            invalid_name: Some(has_colon),
            links: Support::Rejected(Errno::PERM),
            ..Quirks::default()
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "a:b", None),
        Err(PlaceError::InvalidName)
    );
    assert_eq!(
        mount_capabilities(scope.key()),
        MountCapabilities {
            no_replace: Some(true),
            // Learned by the probe on Strata's own names, not from "a:b".
            hard_links: Some(false),
            ..MountCapabilities::default()
        }
    );
    assert_eq!(names(root.path())?, ["source"]);

    scope.clear_calls();
    assert_eq!(
        place_in(root.path(), "source", "valid", None),
        Ok(Placed::Renamed)
    );
    assert_eq!(scope.calls().len(), 1);
    Ok(())
}

#[test]
fn a_folder_into_itself_is_refused_and_learns_nothing() -> Result<(), Box<dyn Error>> {
    for quirks in [
        Quirks::default(),
        without_flags(Errno::INVAL),
        without_flags(Errno::NOSYS),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::create_dir_all(source.join("child"))?;
        let rejected_definitively = matches!(quirks.rename_flags, Support::Rejected(Errno::NOSYS));
        let scope = FsQuirks::install(root.path(), quirks);
        let parent = open_local_parent_directory(root.path())?;
        let child = open_local_parent_directory(&source.join("child"))?;

        let result = place(
            fs(),
            Entry::new(&parent, OsStr::new("source")),
            Entry::new(&child, OsStr::new("moved")),
            None,
        );

        assert_eq!(result, Err(PlaceError::IntoItself));
        assert!(names(&source.join("child"))?.is_empty());
        let learned = mount_capabilities(scope.key());
        let expected = MountCapabilities {
            no_replace: rejected_definitively.then_some(false),
            ..MountCapabilities::default()
        };
        assert_eq!(learned, expected);
    }
    Ok(())
}

#[test]
fn item_level_eperm_does_not_disable_links_for_the_mount() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Link, "target", 0, Errno::PERM)],
            ..without_flags(Errno::NOSYS)
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    let original = identity(&root.path().join("source"));

    // As protected_hardlinks refuses another user's file.
    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::OverReservedFile)
    );
    assert_eq!(mount_capabilities(scope.key()).hard_links, Some(true));
    assert_eq!(names(root.path())?, ["target"]);
    assert_eq!(identity(&root.path().join("target")), original);
    Ok(())
}

#[test]
fn a_lost_link_reply_is_recognized() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            lost_reply: vec![(Op::Link, "target", Errno::EXIST)],
            ..without_flags(Errno::NOSYS)
        },
    );
    fs::write(root.path().join("source"), b"data")?;
    let original = identity(&root.path().join("source"));

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::Linked)
    );
    assert_eq!(identity(&root.path().join("target")), original);
    assert_eq!(names(root.path())?, ["target"]);
    Ok(())
}

#[test]
fn a_case_alias_or_existing_hard_link_is_never_taken_for_a_lost_reply() -> Result<(), Box<dyn Error>>
{
    for case in ["alias", "older link", "alias of a new link"] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::write(&source, b"data")?;
        let mut quirks = Quirks {
            case_insensitive: true,
            fail: vec![(Op::Rename, "*", 0, Errno::NOSYS)],
            ..Quirks::default()
        };
        let target = match case {
            "alias" => "SOURCE",
            "older link" => {
                fs::hard_link(&source, root.path().join("target"))?;
                "target"
            }
            _ => {
                let (linked, from) = (root.path().join("TARGET"), source.clone());
                let racer: Racer = Box::new(move || fs::hard_link(&from, &linked).expect("link"));
                quirks.before.push((Op::Link, "target", 0, racer));
                "target"
            }
        };
        let _scope = FsQuirks::install(root.path(), quirks);
        let before = names(root.path())?;

        let result = place_in(root.path(), "source", target, None);

        assert_eq!(result, Err(PlaceError::Exists), "{case}");
        assert_eq!(fs::read(&source)?, b"data", "{case}");
        if case != "alias of a new link" {
            assert_eq!(names(root.path())?, before, "{case}");
        }
    }
    Ok(())
}

#[test]
fn a_lost_unlink_reply_completes_the_move() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            lost_reply: vec![(Op::Unlink, "source", Errno::NOENT)],
            ..without_flags(Errno::NOSYS)
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Ok(Placed::Linked)
    );
    assert_eq!(fs::read(root.path().join("target"))?, b"data");
    assert_eq!(names(root.path())?, ["target"]);
    Ok(())
}

#[test]
fn a_failed_source_unlink_rolls_back_only_our_link() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![(Op::Unlink, "source", 0, Errno::ACCESS)],
            ..without_flags(Errno::NOSYS)
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Err(PlaceError::Io(Errno::ACCESS))
    );
    assert_eq!(names(root.path())?, ["source"]);
    assert_eq!(fs::metadata(root.path().join("source"))?.nlink(), 1);
    Ok(())
}

#[test]
fn a_source_replaced_after_linking_keeps_the_original_at_the_target() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::write(&source, b"original")?;
    let original = identity(&source);
    let (intruder, replaced) = (root.path().join("intruder"), source.clone());
    let racer: Racer = Box::new(move || {
        fs::write(&intruder, b"intruder").expect("write intruder");
        fs::rename(&intruder, &replaced).expect("replace source");
    });
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            // The first stat of the source decides the ladder; the second confirms the link.
            before: vec![(Op::Stat, "source", 1, racer)],
            ..without_flags(Errno::NOSYS)
        },
    );

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Err(PlaceError::Changed)
    );
    assert_eq!(identity(&root.path().join("target")), original);
    assert_eq!(fs::read(root.path().join("target"))?, b"original");
    assert_eq!(fs::read(&source)?, b"intruder");
    Ok(())
}

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
    let folders = || without_flags(Errno::NOSYS);
    let unknown_type = || Quirks {
        mount_type: Some(("jfs", "rw")),
        ..links_refused_for_the_item()
    };
    let unread_type = || Quirks {
        fail: vec![(Op::MountType, "*", 0, Errno::ACCESS)],
        ..without_flags(Errno::NOSYS)
    };
    let rows: [(Preset, bool, InodeNumbers); 7] = [
        (Quirks::sshfs, false, InodeNumbers::Renewed),
        (Quirks::sshfs, true, InodeNumbers::Renewed),
        (vboxsf, false, InodeNumbers::Renewed),
        (links_refused_for_the_item, false, InodeNumbers::Kept),
        (folders, true, InodeNumbers::Kept),
        (unknown_type, false, InodeNumbers::Unknown),
        (unread_type, true, InodeNumbers::Unknown),
    ];
    for (preset, folder, numbers) in rows {
        for performed in [true, false] {
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

            let placed = if folder {
                Placed::ReservedDirectory
            } else {
                Placed::OverReservedFile
            };
            let expected = match (numbers, performed) {
                (InodeNumbers::Renewed, _) | (InodeNumbers::Unknown, false) => {
                    Err(PlaceError::Unconfirmed)
                }
                (_, true) => Ok(placed),
                (InodeNumbers::Kept, false) => Err(PlaceError::Io(Errno::IO)),
            };
            assert_eq!(
                place_in(root.path(), "source", "target", Some(original)),
                expected,
                "{case}"
            );
            if performed {
                assert_eq!(names(root.path())?, ["target"], "{case}");
                assert_eq!(identity(&root.path().join("target")), original, "{case}");
            } else {
                let left: &[&str] = if numbers == InodeNumbers::Kept {
                    &["source"]
                } else {
                    &["source", "target"]
                };
                assert_eq!(names(root.path())?, left, "{case}");
                assert_eq!(identity(&source), original, "{case}");
            }
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

#[test]
fn set_aside_refuses_a_substituted_item_or_names_where_it_went() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Swap {
        BeforeSettingAside,
        AsItIsSetAside,
        AsACaseOnlyRenameParksIt,
    }
    for swap in [
        Swap::BeforeSettingAside,
        Swap::AsItIsSetAside,
        Swap::AsACaseOnlyRenameParksIt,
    ] {
        let root = tempfile::tempdir()?;
        let (name, kind, parking, mut quirks) = match swap {
            Swap::AsACaseOnlyRenameParksIt => (
                "report",
                HiddenName::Rename,
                ".strata-rename-*",
                case_insensitive(Quirks::default()),
            ),
            _ => (
                "target",
                HiddenName::Replaced,
                ".strata-replaced-*",
                Quirks::default(),
            ),
        };
        let item = root.path().join(name);
        fs::write(&item, b"original")?;
        let expected = identity(&item);
        let (moved, stranger) = (root.path().join("moved"), item.clone());
        let mut racer: Racer = Box::new(move || {
            fs::rename(&stranger, &moved).expect("move the original away");
            fs::write(&stranger, b"stranger").expect("substitute");
        });
        match swap {
            Swap::BeforeSettingAside => racer(),
            _ => quirks.before = vec![(Op::Rename, parking, 0, racer)],
        }
        let scope = FsQuirks::install(root.path(), quirks);
        let parent = open_local_parent_directory(root.path())?;

        let result = match swap {
            Swap::AsACaseOnlyRenameParksIt => rename_in(root.path(), "report", "Report").map(drop),
            _ => set_aside(fs(), Entry::new(&parent, OsStr::new(name)), kind, expected).map(drop),
        };

        assert_eq!(
            fs::read(root.path().join("moved"))?,
            b"original",
            "{swap:?}"
        );
        if let Swap::BeforeSettingAside = swap {
            assert_eq!(result, Err(RenameFailure::Place(PlaceError::Changed)));
            assert_eq!(scope.count(Op::Rename), 0);
            assert_eq!(fs::read(&item)?, b"stranger");
            assert_eq!(names(root.path())?, ["moved", name]);
            continue;
        }
        let left = names(root.path())?;
        let [hidden, _] = &left[..] else {
            return Err(format!("{swap:?}: expected two names, found {left:?}").into());
        };
        assert!(hidden.starts_with(kind.prefix()), "{swap:?}: {left:?}");
        assert_eq!(
            result,
            Err(RenameFailure::Stranded {
                temporary: OsString::from(hidden),
                error: PlaceError::Unconfirmed,
            }),
            "{swap:?}"
        );
        assert_eq!(fs::read(root.path().join(hidden))?, b"stranger", "{swap:?}");
    }
    Ok(())
}

type ProbeCase = (Preset, Probed, Feasibility, Feasibility);

#[test]
fn the_probe_learns_each_quirk_and_leaves_nothing_behind() -> Result<(), Box<dyn Error>> {
    let cases: [ProbeCase; 10] = [
        (
            Quirks::default,
            Probed {
                no_replace: Some(true),
                hard_links: Some(true),
                rename_over_empty_directory: Some(true),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            Quirks::without_rename_flags,
            Probed {
                no_replace: Some(false),
                hard_links: Some(true),
                rename_over_empty_directory: Some(true),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            Quirks::vboxsf,
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(false),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            || Quirks {
                rename_over_empty_dir: OverEmptyDirectory::Eperm,
                ..Quirks::vboxsf()
            },
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: None,
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            || Quirks {
                links: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::without_rename_flags()
            },
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            Quirks::without_folder_placement,
            Probed {
                no_replace: Some(false),
                hard_links: Some(true),
                rename_over_empty_directory: Some(false),
            },
            Feasibility::Yes,
            Feasibility::No,
        ),
        (
            Quirks::without_placement,
            Probed {
                no_replace: Some(false),
                hard_links: None,
                rename_over_empty_directory: Some(false),
            },
            Feasibility::No,
            Feasibility::No,
        ),
        (
            // SMB with POSIX extensions: EISDIR from an exclusive file create is
            // a taken name, but from mkdir it is not.
            || Quirks {
                fail: vec![
                    (Op::Create, ".strata-probe-*", 0, Errno::ISDIR),
                    (Op::Mkdir, ".strata-probe-*", 0, Errno::ISDIR),
                ],
                ..Quirks::default()
            },
            Probed {
                no_replace: Some(true),
                hard_links: Some(true),
                rename_over_empty_directory: None,
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            // cifs with server inode numbers turned off.
            || Quirks {
                link_inodes: true,
                ..Quirks::default()
            },
            Probed {
                no_replace: Some(true),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
        (
            Quirks::sshfs,
            Probed {
                no_replace: Some(false),
                hard_links: Some(false),
                rename_over_empty_directory: Some(true),
            },
            Feasibility::Yes,
            Feasibility::Yes,
        ),
    ];
    for (quirks, expected, files, folders) in cases {
        let root = tempfile::tempdir()?;
        let parent = open_local_parent_directory(root.path())?;
        {
            let _scope = FsQuirks::install(root.path(), quirks());
            assert_eq!(probe(fs(), parent.as_fd(), true, true, true), Ok(expected));
        }
        let _scope = FsQuirks::install(root.path(), quirks());
        assert_eq!(can_set_aside(fs(), parent.as_fd(), false), Ok(files));
        assert_eq!(can_set_aside(fs(), parent.as_fd(), true), Ok(folders));
        assert!(names(root.path())?.is_empty(), "{:?}", names(root.path())?);
    }
    Ok(())
}

#[test]
fn the_probe_keeps_files_it_cannot_prove_are_its_own() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let parent = open_local_parent_directory(root.path())?;
    let dir = root.path().to_path_buf();
    let racer: Racer = Box::new(move || {
        for name in names(&dir).expect("list") {
            fs::write(dir.join("theirs"), b"theirs").expect("write");
            fs::rename(dir.join("theirs"), dir.join(name)).expect("replace");
        }
    });
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            // After the probe saw its link, before it removes it.
            before: vec![(Op::Stat, ".strata-probe-*", 2, racer)],
            ..Quirks::sshfs()
        },
    );

    let probed = probe(fs(), parent.as_fd(), true, true, false)?;

    assert_eq!(probed.hard_links, Some(false));
    let left = names(root.path())?;
    assert_eq!(left.len(), 2, "{left:?}");
    for name in left {
        assert_eq!(fs::read(root.path().join(name))?, b"theirs");
    }
    Ok(())
}

fn rename_in(dir: &Path, from: &str, to: &str) -> Result<Renamed, RenameFailure> {
    let parent = open_local_parent_directory(dir).expect("open parent");
    rename_in_place(
        fs(),
        Entry::new(&parent, OsStr::new(from)),
        Entry::new(&parent, OsStr::new(to)),
    )
}

fn case_insensitive(quirks: Quirks) -> Quirks {
    Quirks {
        case_insensitive: true,
        ..quirks
    }
}

fn make(path: &Path, folder: bool) -> io::Result<()> {
    if folder {
        fs::create_dir(path)?;
        fs::write(path.join("inside.txt"), b"inside")
    } else {
        fs::write(path, b"data")
    }
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
    let shapes: [Preset; 4] = [Quirks::default, Quirks::vboxsf, smb, exfat_fuse];
    for quirks in shapes {
        for folder in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), case_insensitive(quirks()));
            let source = root.path().join("report");
            make(&source, folder)?;
            let original = identity(&source);

            assert_eq!(
                rename_in(root.path(), "report", "report"),
                Ok(Renamed::Unchanged)
            );
            assert_eq!(
                rename_in(root.path(), "report", "Report"),
                Ok(Renamed::CaseOnly)
            );

            assert_eq!(names(root.path())?, ["Report"], "folder: {folder}");
            assert_eq!(identity(&root.path().join("Report")), original);
        }
    }
    Ok(())
}

#[test]
fn a_hard_link_is_never_taken_for_a_case_alias() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), case_insensitive(Quirks::default()));
    fs::write(root.path().join("report"), b"data")?;
    fs::hard_link(root.path().join("report"), root.path().join("Report"))?;

    assert_eq!(
        rename_in(root.path(), "report", "Report"),
        Err(RenameFailure::Place(PlaceError::Exists))
    );
    assert_eq!(names(root.path())?, ["Report", "report"]);
    assert_eq!(fs::metadata(root.path().join("report"))?.nlink(), 2);
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
        assert_eq!(identity(&root.path().join(&left[0])), original);
        if moves_back {
            assert_eq!(result, Err(RenameFailure::Place(PlaceError::Io(Errno::IO))));
            assert_eq!(left, ["report"]);
        } else {
            assert!(left[0].starts_with(".strata-rename-"), "{left:?}");
            assert_eq!(
                result,
                Err(RenameFailure::Stranded {
                    temporary: OsString::from(&left[0]),
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
    assert_eq!(names(root.path())?, ["target"]);
    assert_eq!(identity(&root.path().join("target")), original);
    assert_eq!(mount_capabilities(scope.key()).hard_links, Some(false));

    scope.clear_calls();
    assert_eq!(
        place_in(root.path(), "target", "moved", None),
        Ok(Placed::OverReservedFile)
    );
    assert_eq!(scope.count(Op::Link), 0);
    assert_eq!(names(root.path())?, ["moved"]);
    assert_eq!(identity(&root.path().join("moved")), original);
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
        assert_eq!(fs::read(root.path().join("target"))?, b"theirs");
        assert_eq!(identity(&root.path().join("source")), original);
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
    }
    Ok(())
}

/// No `RENAME_NOREPLACE`, and a link to `target` refused for that item alone,
/// as `protected_hardlinks` does for another user's file.
fn links_refused_for_the_item() -> Quirks {
    Quirks {
        fail: vec![(Op::Link, "target", 0, Errno::PERM)],
        ..Quirks::without_rename_flags()
    }
}

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
