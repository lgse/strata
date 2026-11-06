// SPDX-License-Identifier: MIT

use crate::services::RenameCause;

use super::*;

/// The visible names of `live` each time a racer before `op` runs.
fn names_seen_before(op: Op, live: PathBuf) -> (Arc<std::sync::Mutex<Vec<String>>>, Before) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_then = seen.clone();
    let racer: Racer = Box::new(move || {
        let names = visible_names(&live).expect("list the merged folder");
        seen_then.lock().expect("record").extend(names);
    });
    (seen, (op, "*", 0, racer))
}

type Before = (Op, &'static str, usize, Racer);

/// Installs `before` on `target`, on a mount known to make hard links.
fn linking_mount(target: &Path, before: Before) -> FsQuirks {
    let scope = FsQuirks::install(
        target,
        Quirks {
            before: vec![before],
            ..Quirks::default()
        },
    );
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.hard_links = Some(true);
    });
    scope
}

#[test]
fn a_merge_keeps_hard_links_only_within_one_merge_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(source.join("sub"))?;
    fs::create_dir_all(target.join("sub"))?;
    fs::write(source.join("a"), b"linked")?;
    for name in ["a2", "new/c", "sub/b"] {
        fs::hard_link(source.join("a"), source.join(name))?;
    }
    let (seen, at_link) = names_seen_before(Op::Link, target.clone());
    let _scope = linking_mount(&target, at_link);

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        *seen.lock().map_err(|error| error.to_string())?,
        ["sub"],
        "names are linked before their stage is placed"
    );
    let [a, a2, c, b] = ["a", "a2", "new/c", "sub/b"].map(|name| {
        fs::metadata(target.join(name)).map(|metadata| (metadata.ino(), metadata.nlink()))
    });
    let (a, b) = (a?, b?);
    assert_eq!((a2?, c?), (a, a));
    assert_eq!((a.1, b.1), (3, 1));
    assert_ne!(b.0, a.0);
    let linked = fs::read(target.join("sub/b"))?;
    assert_eq!(linked, b"linked");
    Ok(())
}

/// Replaces each copy of `a` or `a2` in the copy stage of `live` with
/// another item.
fn replace_staged_copies(live: PathBuf) -> Racer {
    Box::new(move || {
        let stage = names(&live)
            .expect("list the merged folder")
            .into_iter()
            .find(|name| name.starts_with(".strata-copy-"))
            .map(|name| live.join(name));
        let copies = stage
            .iter()
            .flat_map(|stage| [stage.join("a"), stage.join("a2")]);
        for copied in copies.filter(|copied| copied.exists()) {
            let other = live.join("other");
            fs::write(&other, b"other").expect("write the other item");
            fs::rename(&other, &copied).expect("replace the copy");
        }
    })
}

#[test]
fn a_merged_copy_replaced_before_linking_is_not_linked() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    fs::create_dir(&target)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("a2"))?;
    let racer = replace_staged_copies(target.clone());
    let _scope = linking_mount(&target, (Op::Link, "*", 0, racer));

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_err(), "{result:?}");
    assert_eq!(names(&target)?, Vec::<String>::new());
    Ok(())
}

type Renamed = &'static [(&'static str, &'static str, RenameCause)];

/// A name another program takes just before the merge places there, what
/// the merge saves its item as instead, and the renames it reports.
struct Taken {
    taken: &'static str,
    saved_as: &'static str,
    preset: Preset,
    reported: Renamed,
}

fn check_taken_name(row: &Taken) -> Result<(), Box<dyn Error>> {
    let Taken {
        taken,
        saved_as,
        preset,
        reported,
    } = *row;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("photos"))?;
    fs::create_dir(&target)?;
    fs::write(source.join("notes.txt"), b"incoming")?;
    for name in ["Makefile", "makefile"] {
        fs::write(source.join("photos").join(name), name)?;
    }
    let live = target.clone();
    let racer: Racer = Box::new(move || {
        fs::write(live.join(taken), b"racer").expect("take the name");
    });
    let _scope = FsQuirks::install(
        &target,
        Quirks {
            before: vec![(Op::Rename, taken, 0, racer)],
            ..preset()
        },
    );

    let (merged, report) = merge_reporting(&source, &target, None);

    assert!(merged.is_ok(), "{taken}: {merged:?}");
    assert_eq!(fs::read(target.join(taken))?, b"racer", "{taken}");
    let mut expected = vec!["notes.txt", "photos", saved_as];
    expected.sort_unstable();
    assert_eq!(names(&target)?, expected, "{taken}");
    let ours = target.join(saved_as);
    if taken == "photos" {
        assert_files(
            &ours,
            &[("Makefile", "Makefile"), ("makefile (1)", "makefile")],
            "",
        )?;
    } else {
        assert_eq!(fs::read(ours)?, b"incoming");
    }
    let reported = reported
        .iter()
        .map(|(requested, saved_as, cause)| (requested.to_string(), saved_as.to_string(), *cause))
        .collect::<Vec<_>>();
    assert_eq!(renamed_beneath(&target, &report), reported, "{taken}");
    Ok(())
}

#[test]
fn merge_never_overwrites_an_entry_created_during_the_merge() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = [
        Taken {
            taken: "notes.txt",
            saved_as: "notes (1).txt",
            preset: Quirks::default,
            reported: &[("notes.txt", "notes (1).txt", RenameCause::Taken)],
        },
        Taken {
            taken: "photos",
            saved_as: "photos (1)",
            preset: Quirks::vfat,
            reported: &[
                ("photos", "photos (1)", RenameCause::Taken),
                (
                    "photos (1)/makefile",
                    "photos (1)/makefile (1)",
                    RenameCause::Alias,
                ),
            ],
        },
    ];
    for row in &rows {
        check_taken_name(row)?;
    }
    Ok(())
}

fn unstable_inodes() -> Quirks {
    Quirks {
        rename_flags: Support::Supported,
        ..Quirks::sshfs()
    }
}

/// How another program saves over a name the merge just placed.
struct SavedOver {
    row: &'static str,
    preset: Preset,
    /// The call that places the item, after which the program saves.
    placed: Op,
    /// The child of a placed folder it saves over, or the placed item itself.
    descendant: Option<&'static str>,
}

impl SavedOver {
    fn in_place(&self) -> bool {
        self.row.starts_with("edited")
    }

    fn same_size(&self) -> bool {
        self.row.starts_with("edited same-size")
    }

    fn contents(&self) -> &'static [u8] {
        if self.same_size() {
            b"modified"
        } else {
            b"saved by another program"
        }
    }

    fn over(&self, target: &Path) -> PathBuf {
        self.descendant.map_or_else(
            || target.join("a.txt"),
            |child| target.join("a.txt").join(child),
        )
    }

    /// Edits `over` in place, or renames `saved` over it.
    fn racer(&self, saved: PathBuf, over: PathBuf) -> Racer {
        let (in_place, same_size, contents) = (self.in_place(), self.same_size(), self.contents());
        Box::new(move || {
            if !in_place {
                fs::rename(&saved, &over).expect("rename over the placed name");
                return;
            }
            fs::write(&over, contents).expect("edit the placed entry");
            if same_size {
                fs::File::open(&over)
                    .expect("edited file")
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1))
                    .expect("detectable edit time");
            }
        })
    }
}

fn check_saved_over(case: &SavedOver) -> Result<(), Box<dyn Error>> {
    let row = case.row;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    fs::create_dir(&target)?;
    if case.descendant.is_some() {
        fs::create_dir(source.join("a.txt"))?;
        fs::write(source.join("a.txt/original"), b"incoming")?;
    } else {
        fs::write(source.join("a.txt"), b"incoming")?;
    }
    let saved = root.path().join("saved.txt");
    fs::write(&saved, case.contents())?;
    let racer = case.racer(saved, case.over(&target));
    let _scope = FsQuirks::install(
        &target,
        without_trash(Quirks {
            after: vec![(case.placed, "a.txt", 0, racer)],
            ..(case.preset)()
        }),
    );
    let (plans, on_merged) = recorded_plans();
    let merged = merge_into(&source, &target, &on_merged);
    assert!(merged.is_ok(), "{row}: {merged:?}");
    let events = run_undo(undo_parts(plans.take()), staged_lookup(HashMap::new()));
    assert!(
        restored(&events).is_none_or(|items| items.is_empty()),
        "{row}: {events:?}"
    );
    assert_eq!(fs::read(case.over(&target))?, case.contents(), "{row}");
    assert_eq!(names(&target)?, ["a.txt"], "{row}");
    Ok(())
}

const fn saved_over(row: &'static str, descendant: Option<&'static str>) -> SavedOver {
    SavedOver {
        row,
        preset: Quirks::default,
        placed: Op::Rename,
        descendant,
    }
}

#[test]
fn an_item_put_at_a_placed_name_right_after_placing_stays_there() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // A program that saves by renaming over the name does so as soon as the
    // child is placed: by a rename, or by a link and the stage's unlink.
    let rows = [
        saved_over("rename", None),
        SavedOver {
            preset: Quirks::without_rename_flags,
            placed: Op::Unlink,
            ..saved_over("link", None)
        },
        SavedOver {
            preset: unstable_inodes,
            ..saved_over("unstable inode without birth time", None)
        },
        saved_over("replaced descendant", Some("original")),
        saved_over("new descendant", Some("arrival")),
        saved_over("edited file", None),
        saved_over("edited descendant", Some("original")),
        saved_over("edited same-size file", None),
        saved_over("edited same-size descendant", Some("original")),
    ];
    for case in &rows {
        check_saved_over(case)?;
    }
    Ok(())
}

#[test]
fn a_stopped_merge_reports_only_what_it_placed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("z"))?;
    fs::create_dir(&target)?;
    fs::write(source.join("a.txt"), b"incoming")?;
    for name in ["Makefile", "makefile"] {
        fs::write(source.join("z").join(name), name)?;
    }
    let live = target.clone();
    let racer: Racer = Box::new(move || {
        fs::write(live.join("a.txt"), b"racer").expect("take the name");
    });
    let _scope = FsQuirks::install(
        &target,
        Quirks {
            before: vec![(Op::Rename, "a.txt", 0, racer)],
            fail: vec![(Op::Rename, "z", 0, Errno::IO)],
            ..Quirks::vfat()
        },
    );

    let (merged, report) = merge_reporting(&source, &target, None);

    assert!(merged.is_err(), "{merged:?}");
    assert_eq!(names(&target)?, ["a (1).txt", "a.txt"]);
    assert_files(&target, &[("a (1).txt", "incoming")], "")?;
    assert_eq!(
        renamed_beneath(&target, &report),
        [("a.txt".into(), "a (1).txt".into(), RenameCause::Taken)]
    );
    Ok(())
}

#[test]
fn an_unconfirmed_placement_keeps_its_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    fs::create_dir(&target)?;
    fs::write(source.join("leaf"), b"incoming")?;
    // The placing rename's reply is lost, and the leaf is found at neither
    // name: it went back into the stage under another one.
    let live = target.clone();
    let racer: Racer = Box::new(move || {
        let stage = names(&live)
            .expect("live folder")
            .into_iter()
            .find(|name| name.starts_with(".strata-copy-"))
            .expect("stage");
        fs::rename(live.join("leaf"), live.join(stage).join("moved")).expect("move the leaf");
    });
    let _scope = FsQuirks::install(
        &target,
        Quirks {
            lost_reply: vec![(Op::Rename, "leaf", Errno::IO)],
            after: vec![(Op::Rename, "leaf", 0, racer)],
            ..Quirks::default()
        },
    );

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_err(), "{result:?}");
    let names = names(&target)?;
    let [stage] = names.as_slice() else {
        panic!("only the stage that holds the leaf stays: {names:?}");
    };
    assert_eq!(fs::read(target.join(stage).join("moved"))?, b"incoming");
    Ok(())
}

#[test]
fn a_merge_copies_folders_in_place_where_the_mount_cannot_place_them() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir(&target)?;
    for name in ["Makefile", "makefile"] {
        fs::write(source.join("new").join(name), name)?;
    }
    fs::write(target.join("new"), b"taken")?;
    let scope = FsQuirks::install(
        &target,
        Quirks {
            mount_type: Some(("ext4", "rw")),
            case_insensitive: true,
            ..Quirks::without_folder_placement()
        },
    );
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.no_replace = Some(false);
        capabilities.hard_links = Some(true);
        capabilities.rename_over_empty_directory = Some(false);
    });

    // Without claims, one worker makes the folder's files last first, so
    // `Makefile` is the alias.
    let (merged, report) = merge_reporting(&source, &target, Some(1));

    assert!(merged.is_ok(), "{merged:?}");
    assert_eq!(names(&target)?, ["new", "new (1)"]);
    assert_eq!(
        names(&target.join("new (1)"))?,
        ["Makefile (1)", "makefile"]
    );
    assert_files(
        &target,
        &[("new", "taken"), ("new (1)/Makefile (1)", "Makefile")],
        "",
    )?;
    assert!(!scope.used_name(".strata-*"), "{:?}", scope.calls());
    assert_eq!(
        renamed_beneath(&target, &report),
        [
            ("new".into(), "new (1)".into(), RenameCause::Taken),
            (
                "new (1)/Makefile".into(),
                "new (1)/Makefile (1)".into(),
                RenameCause::Alias
            ),
        ]
    );
    Ok(())
}

fn check_invisible_until_complete(row: usize, preset: Preset) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let source = sources.join("folder");
    let target = destination.join("folder");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(&target)?;
    let (incoming, inner) = (vec![0x5a; 256 * 1024], vec![0xa5; 256 * 1024]);
    fs::write(source.join("incoming.bin"), &incoming)?;
    fs::write(source.join("new/inner.bin"), &inner)?;
    fs::write(target.join("stays.txt"), b"keep me")?;
    let (seen, at_write) = names_seen_before(Op::Write, target.clone());
    // Separate mounts, so the data is written rather than cloned.
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![at_write],
            ..preset()
        },
    );

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_ok(), "row {row}: {result:?}");
    assert_eq!(
        *seen.lock().map_err(|error| error.to_string())?,
        ["stays.txt"],
        "row {row}"
    );
    assert_eq!(
        names(&target)?,
        ["incoming.bin", "new", "stays.txt"],
        "row {row}"
    );
    let written: [(&str, &[u8]); 3] = [
        ("incoming.bin", &incoming),
        ("new/inner.bin", &inner),
        ("stays.txt", b"keep me"),
    ];
    assert_files(&target, &written, "")?;
    Ok(())
}

#[test]
fn a_merged_item_is_invisible_until_complete() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Placed by rename, by the rungs that cannot replace, and copied out of
    // the stage where nothing can place it.
    let presets: [Preset; 3] = [Quirks::default, Quirks::vboxsf, Quirks::without_placement];
    for (row, preset) in presets.into_iter().enumerate() {
        check_invisible_until_complete(row, preset)?;
    }
    Ok(())
}

#[test]
fn merge_does_not_follow_a_target_folder_swapped_for_a_link() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    let outside = root.path().join("outside");
    fs::create_dir_all(source.join("sub"))?;
    fs::create_dir_all(target.join("sub"))?;
    fs::create_dir(&outside)?;
    fs::write(source.join("sub/new.txt"), b"new")?;
    fs::write(target.join("sub/old.txt"), b"old")?;
    let (live, elsewhere) = (target.clone(), outside.clone());
    let racer: Racer = Box::new(move || {
        fs::rename(live.join("sub"), live.join("moved")).expect("move the folder away");
        std::os::unix::fs::symlink(&elsewhere, live.join("sub")).expect("link elsewhere");
    });
    let _scope = FsQuirks::install(
        &target,
        Quirks {
            before: vec![(Op::OpenDir, "sub", 0, racer)],
            ..Quirks::default()
        },
    );

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_err(), "{result:?}");
    assert_eq!(names(&outside)?, Vec::<String>::new());
    assert_eq!(names(&target.join("moved"))?, ["old.txt"]);
    Ok(())
}
