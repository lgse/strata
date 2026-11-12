// SPDX-License-Identifier: MIT

use super::*;

/// Each `(name, contents)` in `root` reads as its contents.
fn assert_files(root: &Path, files: &[(&str, &str)]) -> io::Result<()> {
    for (name, contents) in files {
        assert_eq!(fs::read(root.join(name))?, contents.as_bytes(), "{name}");
    }
    Ok(())
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

/// A `source` file placed at `target` in a fresh folder whose mount has
/// `quirks` installed before the file is written.
struct DataPlaced {
    root: tempfile::TempDir,
    _scope: FsQuirks,
    original: LocalFileIdentity,
    result: Result<Placed, PlaceError>,
}

fn place_data(quirks: Quirks) -> Result<DataPlaced, Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(root.path(), quirks);
    fs::write(root.path().join("source"), b"data")?;
    let original = identity(&root.path().join("source"));
    let result = place_in(root.path(), "source", "target", None);
    Ok(DataPlaced {
        root,
        _scope: scope,
        original,
        result,
    })
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

            assert_only(root.path(), "target", original, "")?;
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

fn check_ladder(
    index: usize,
    quirks: Preset,
    source_is_folder: bool,
    kind: &'static str,
) -> Result<(), Box<dyn Error>> {
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
    Ok(())
}

#[test]
fn the_ladder_never_replaces_an_existing_destination() -> Result<(), Box<dyn Error>> {
    let quirk_sets: [Preset; 5] = [
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
        || Quirks {
            links: Support::Rejected(Errno::ACCESS),
            ..without_flags(Errno::NOSYS)
        },
    ];
    let kinds = [
        "file",
        "empty folder",
        "full folder",
        "symlink",
        "dangling symlink",
    ];
    for (index, quirks) in quirk_sets.into_iter().enumerate() {
        for source_is_folder in [false, true] {
            for kind in kinds {
                check_ladder(index, quirks, source_is_folder, kind)?;
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
fn a_mount_refusing_links_with_eacces_still_places_an_item() -> Result<(), Box<dyn Error>> {
    // As an SFTP server without link extensions refuses every link and symlink.
    for kind in ["file", "symlink"] {
        let root = tempfile::tempdir()?;
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                links: Support::Rejected(Errno::ACCESS),
                symlinks: Support::Rejected(Errno::ACCESS),
                ..Quirks::sshfs()
            },
        );
        let source = root.path().join("source");
        if kind == "file" {
            fs::write(&source, b"data")?;
        } else {
            std::os::unix::fs::symlink("elsewhere", &source)?;
        }
        let original = identity(&source);

        assert_eq!(
            place_in(root.path(), "source", "target", None),
            Ok(Placed::OverReservedFile),
            "{kind}"
        );
        assert_eq!(
            mount_capabilities(scope.key()).hard_links,
            Some(false),
            "{kind}"
        );
        assert_only(root.path(), "target", original, kind)?;
    }
    Ok(())
}

#[test]
fn eacces_from_a_folder_that_refuses_the_probe_is_reported() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            links: Support::Rejected(Errno::ACCESS),
            fail: vec![(Op::Create, ".strata-probe-*", 0, Errno::ACCESS)],
            ..without_flags(Errno::NOSYS)
        },
    );
    fs::write(root.path().join("source"), b"data")?;

    assert_eq!(
        place_in(root.path(), "source", "target", None),
        Err(PlaceError::Io(Errno::ACCESS))
    );
    assert_eq!(mount_capabilities(scope.key()).hard_links, None);
    assert_eq!(names(root.path())?, ["source"]);
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
    assert_only(root.path(), "target", original, "")?;
    Ok(())
}

#[test]
fn a_lost_link_reply_is_recognized() -> Result<(), Box<dyn Error>> {
    let placed = place_data(Quirks {
        lost_reply: vec![(Op::Link, "target", Errno::EXIST)],
        ..without_flags(Errno::NOSYS)
    })?;

    assert_eq!(placed.result, Ok(Placed::Linked));
    assert_only(placed.root.path(), "target", placed.original, "")?;
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
    let placed = place_data(Quirks {
        lost_reply: vec![(Op::Unlink, "source", Errno::NOENT)],
        ..without_flags(Errno::NOSYS)
    })?;

    let root = placed.root.path();
    assert_eq!(placed.result, Ok(Placed::Linked));
    assert_eq!(fs::read(root.join("target"))?, b"data");
    assert_eq!(names(root)?, ["target"]);
    Ok(())
}

#[test]
fn a_failed_source_unlink_rolls_back_only_our_link() -> Result<(), Box<dyn Error>> {
    let placed = place_data(Quirks {
        fail: vec![(Op::Unlink, "source", 0, Errno::ACCESS)],
        ..without_flags(Errno::NOSYS)
    })?;

    let root = placed.root.path();
    assert_eq!(placed.result, Err(PlaceError::Io(Errno::ACCESS)));
    assert_eq!(names(root)?, ["source"]);
    assert_eq!(fs::metadata(root.join("source"))?.nlink(), 1);
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
    assert_files(
        root.path(),
        &[("target", "original"), ("source", "intruder")],
    )?;
    Ok(())
}
