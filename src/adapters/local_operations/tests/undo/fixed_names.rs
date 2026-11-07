// SPDX-License-Identifier: MIT

//! Names a mount fixed on the way out are given back by an undo, or reported.

use super::*;

/// An `origin/project` folder and a `usb` folder under a root.
fn project_and_usb() -> io::Result<(tempfile::TempDir, PathBuf, PathBuf, PathBuf)> {
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let usb = root.path().join("usb");
    let project = origin.join("project");
    fs::create_dir_all(&project)?;
    fs::create_dir(&usb)?;
    Ok((root, origin, usb, project))
}

/// Writes each name's path as its contents, under `project`.
fn write_named(project: &Path, names: &[&str]) -> io::Result<()> {
    for name in names {
        fs::write(project.join(name), name)?;
    }
    Ok(())
}

fn undo_moved_project(id: u64, mode: NameMode, disk: &Path, moved: &Path) -> Vec<OperationEvent> {
    undo_move(UndoMoveRequest {
        names: mode,
        ..undo_request(
            id,
            vec![fail_if_exists(record(&disk.join("project"), moved))],
        )
    })
}

/// A move of two names onto a mount, undone or redone with the names it leaves.
struct NamesBack {
    row: &'static str,
    preset: Preset,
    mode: NameMode,
    moved_names: Vec<OsString>,
    expected: [&'static str; 2],
}

impl NamesBack {
    fn check(&self) -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        if self
            .moved_names
            .iter()
            .any(|name| skips_malformed_name_on_fuse(root.path(), name))
        {
            return Ok(());
        }
        let disk = root.path().join("disk");
        let moved = root.path().join("moved").join("project");
        fs::create_dir(&disk)?;
        fs::create_dir_all(&moved)?;
        for name in &self.moved_names {
            fs::write(moved.join(name), b"contents")?;
        }
        let _disk = FsQuirks::install(&disk, (self.preset)());

        let events = undo_moved_project(1530, self.mode, &disk, &moved);

        assert_pasted(&events, self.row);
        assert_eq!(names(&disk.join("project"))?, self.expected, "{}", self.row);
        assert!(!moved.exists(), "{}", self.row);
        Ok(())
    }
}

#[test]
fn an_undone_move_gives_names_back_as_they_were_and_a_redone_one_fixes_them()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let malformed = || OsString::from_vec(b"x\xc0\xafy".to_vec());
    let rows = [
        (
            "ntfs3 undo",
            Quirks::ntfs3 as Preset,
            NameMode::Keep,
            vec!["a:b".into(), "CON".into()],
            ["CON", "a:b"],
        ),
        (
            "ntfs3 redo",
            Quirks::ntfs3,
            NameMode::Fix,
            vec!["a:b".into(), "CON".into()],
            ["CON_", "a_b"],
        ),
        // Whatever the mode, exfat-fuse never gets a name it would corrupt.
        (
            "exfat-fuse undo",
            Quirks::exfat_fuse,
            NameMode::Keep,
            vec![malformed(), "CON".into()],
            ["CON", "x__y"],
        ),
        (
            "exfat-fuse redo",
            Quirks::exfat_fuse,
            NameMode::Fix,
            vec![malformed(), "CON".into()],
            ["CON_", "x__y"],
        ),
    ];
    for (row, preset, mode, moved_names, expected) in rows {
        NamesBack {
            row,
            preset,
            mode,
            moved_names,
            expected,
        }
        .check()?;
    }
    Ok(())
}

fn refuses_a_colon_b(name: &OsStr) -> Option<Errno> {
    (name == "a:b").then_some(Errno::INVAL)
}

#[test]
fn rules_a_mount_learned_fix_names_a_redo_brings_but_not_those_an_undo_gives_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (mode, expected) in [(NameMode::Keep, "c:d"), (NameMode::Fix, "c_d")] {
        let root = tempfile::tempdir()?;
        let disk = root.path().join("disk");
        let source = root.path().join("source");
        let moved = root.path().join("moved").join("project");
        for folder in [&disk, &source, &moved] {
            fs::create_dir_all(folder)?;
        }
        fs::write(source.join("a:b"), b"refused")?;
        fs::write(moved.join("c:d"), b"held before the move")?;
        let _disk = FsQuirks::install(
            &disk,
            Quirks {
                mount_type: Some(("fuse.strictfs", "rw,user_id=0,group_id=0")),
                invalid_name: Some(refuses_a_colon_b),
                ..Quirks::default()
            },
        );
        run_paste(PasteRequest {
            id: OperationRequestId(1590),
            destination: Location::local(&disk),
            items: vec![PasteItem {
                source: Location::local(source.join("a:b")),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: false,
        });
        assert_eq!(names(&disk)?, ["a_b"], "{mode:?}");

        let events = undo_moved_project(1591, mode, &disk, &moved);

        assert_pasted(&events, &format!("{mode:?}"));
        assert_eq!(names(&disk.join("project"))?, [expected], "{mode:?}");
    }
    Ok(())
}

fn takes_name(path: &Path) -> Racer {
    let taken = path.to_path_buf();
    Box::new(move || fs::write(&taken, b"taken").expect("take the name"))
}

fn move_onto_usb(id: u64, project: &Path, usb: &Path) -> Vec<OperationEvent> {
    run_paste(PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(usb),
        items: vec![PasteItem {
            source: Location::local(project),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        move_sources: true,
    })
}

/// A project whose names differ only in case, in its folder and a subfolder.
fn seed_case_aliases(project: &Path) -> io::Result<()> {
    fs::write(project.join("Makefile"), b"upper")?;
    fs::write(project.join("makefile"), b"lower")?;
    fs::create_dir(project.join("Docs"))?;
    fs::create_dir(project.join("docs"))?;
    fs::write(project.join("docs/Makefile"), b"inner upper")?;
    fs::write(project.join("docs/makefile"), b"inner lower")
}

#[test]
fn undoing_a_move_gives_back_names_numbered_as_aliases() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for taken in [false, true] {
        let (_root, origin, usb, project) = project_and_usb()?;
        seed_case_aliases(&project)?;
        let restored = project.join("makefile");
        let _origin = FsQuirks::install(
            &origin,
            Quirks {
                before: taken
                    .then(|| (Op::Rename, "makefile (1)", 0, takes_name(&restored)))
                    .into_iter()
                    .collect(),
                ..Quirks::default()
            },
        );
        let _usb = FsQuirks::install(&usb, Quirks::vfat());
        let moved = move_onto_usb(44, &project, &usb);
        let renamed = reported_renames(&moved);
        expect_names(&[(
            &usb.join("project"),
            &["Docs", "Makefile", "docs (1)", "makefile (1)"],
        )])?;
        let item = fail_if_exists(MoveRecord {
            renamed,
            ..record(&project, &usb.join("project"))
        });

        let events = undo_move(undo_request(45, vec![item]));

        assert_pasted(&events, "");
        expect_contents(&[
            (project.join("Makefile"), b"upper"),
            (project.join("docs/makefile"), b"inner lower"),
            (project.join("docs/Makefile"), b"inner upper"),
        ]);
        let reported = reported_renames(&events);
        if taken {
            expect_names(&[(
                &project,
                &["Docs", "Makefile", "docs", "makefile", "makefile (1)"],
            )])?;
            expect_contents(&[
                (restored.clone(), b"taken"),
                (project.join("makefile (1)"), b"lower"),
            ]);
            assert_eq!(
                reported,
                [RenamedItem {
                    requested: Location::local(&restored),
                    saved_as: Location::local(project.join("makefile (1)")),
                    cause: RenameCause::Taken,
                }]
            );
        } else {
            expect_names(&[(&project, &["Docs", "Makefile", "docs", "makefile"])])?;
            expect_contents(&[(restored.clone(), b"lower")]);
            assert!(reported.is_empty());
        }
    }
    Ok(())
}

fn moved_onto_fat(project: &Path, usb: &Path) -> MoveRecord {
    let moved = move_onto_usb(1590, project, usb);
    let current = Location::local(usb.join("project"));
    MoveRecord {
        original: Location::local(project),
        renamed: followed_report(&moved)
            .renamed
            .into_iter()
            .filter(|item| item.saved_as != current && item.saved_as.is_within(&current))
            .collect(),
        current,
        replaced: None,
    }
}

fn undone(record: MoveRecord) -> Vec<OperationEvent> {
    let events = undo_move(undo_request(1591, vec![fail_if_exists(record)]));
    assert_pasted(&events, "");
    events
}

fn refuses_refused(name: &OsStr) -> Option<Errno> {
    let name = name.as_encoded_bytes();
    if name.starts_with(b"refused:") {
        Some(Errno::INVAL)
    } else {
        name.starts_with(b"illegal:").then_some(Errno::ILSEQ)
    }
}

#[test]
fn undoing_a_move_gives_fixed_names_back_and_reports_those_it_cannot() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let (_root, origin, usb, project) = project_and_usb()?;
    fs::create_dir(project.join("x:y"))?;
    write_named(
        &project,
        &[
            "x:y/a:b",
            "CON",
            "taken:1",
            "refused:2",
            "failed:3",
            "illegal:4",
            "plain.txt",
            "long",
        ],
    )?;
    let _origin = FsQuirks::install(
        &origin,
        Quirks {
            invalid_name: Some(refuses_refused),
            fail: vec![(Op::Rename, "failed:3", 0, Errno::ACCESS)],
            before: vec![(
                Op::Rename,
                "taken:1",
                0,
                takes_name(&project.join("taken:1")),
            )],
            ..Quirks::default()
        },
    );
    let _usb = FsQuirks::install(&usb, Quirks::vfat());
    let mut record = moved_onto_fat(&project, &usb);
    expect_names(&[(
        &usb.join("project"),
        &[
            "CON_",
            "failed_3",
            "illegal_4",
            "long",
            "plain.txt",
            "refused_2",
            "taken_1",
            "x_y",
        ],
    )])?;
    // As a name too long for the original, cut on a mount that takes more.
    let long = "l".repeat(300);
    record.renamed.push(RenamedItem {
        requested: Location::local(usb.join("project").join(&long)),
        saved_as: Location::local(usb.join("project/long")),
        cause: RenameCause::TooLong,
    });

    let report = followed_report(&undone(record));

    expect_names(&[(
        &project,
        &[
            "CON",
            "failed_3",
            "illegal_4",
            "long",
            "plain.txt",
            "refused_2",
            "taken:1",
            "taken_1",
            "x:y",
        ],
    )])?;
    expect_contents(&[
        (project.join("x:y/a:b"), b"x:y/a:b"),
        (project.join("taken_1"), b"taken:1"),
        (project.join("taken:1"), b"taken"),
    ]);
    assert_eq!(
        renamed_beneath(&project, &report),
        [
            ("illegal:4", "illegal_4", RenameCause::Invalid),
            (long.as_str(), "long", RenameCause::TooLong),
            ("refused:2", "refused_2", RenameCause::Invalid),
            ("taken:1", "taken_1", RenameCause::Taken),
        ]
        .map(|(name, saved_as, cause)| (name.to_owned(), saved_as.to_owned(), cause))
    );
    let denied = crate::services::error_detail(super::super::super::os_error_text(Errno::ACCESS));
    assert_eq!(
        report.kept,
        [KeptItem {
            location: Location::local(project.join("failed:3")),
            cause: KeptCause::NameNotRestored {
                saved_as: Location::local(project.join("failed_3")),
                error: denied,
            },
        }]
    );
    Ok(())
}

#[test]
fn undo_says_nothing_of_fixed_entries_gone_from_the_names_they_were_saved_as()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // As a share's location, which GIO renames.
    for through_gio in [false, true] {
        let (_root, _origin, usb, project) = project_and_usb()?;
        fs::create_dir(project.join("gone:4"))?;
        write_named(
            &project,
            &["deleted:1", "renamed:2", "kept:3", "gone:4/a:b"],
        )?;
        let _usb = FsQuirks::install(&usb, Quirks::vfat());
        let mut record = moved_onto_fat(&project, &usb);
        if through_gio {
            record.original = Location::uri(gio::File::for_path(&project).uri());
        }
        let moved = usb.join("project");
        fs::remove_file(moved.join("deleted_1"))?;
        fs::rename(moved.join("renamed_2"), moved.join("mine.txt"))?;
        fs::remove_dir_all(moved.join("gone_4"))?;

        let report = followed_report(&undone(record));

        assert_eq!(names(&project)?, ["kept:3", "mine.txt"], "{through_gio}");
        assert!(report.is_empty(), "{through_gio}: {report:?}");
    }
    Ok(())
}

#[test]
fn undoing_a_move_back_to_another_kind_of_location_gives_fixed_names_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (_root, _origin, usb, project) = project_and_usb()?;
    fs::create_dir(project.join("x:y"))?;
    write_named(
        &project,
        &["x:y/a:b", "CON", "taken", "plain", "long", "bytes"],
    )?;
    let _usb = FsQuirks::install(&usb, Quirks::vfat());
    let mut record = moved_onto_fat(&project, &usb);
    let moved = usb.join("project");
    let entry = |requested: OsString, saved_as: &str, cause| RenamedItem {
        requested: Location::local(moved.join(requested)),
        saved_as: Location::local(moved.join(saved_as)),
        cause,
    };
    let long = OsString::from("l".repeat(300));
    let bytes = OsString::from_vec(b"b\xffytes".to_vec());
    record.renamed.extend([
        entry("taken".into(), "plain", RenameCause::Invalid),
        entry(long.clone(), "long", RenameCause::TooLong),
        entry(bytes.clone(), "bytes", RenameCause::Invalid),
    ]);
    // As a share's location, which GIO renames.
    record.original = Location::uri(gio::File::for_path(&project).uri());

    let report = followed_report(&undone(record));

    assert_eq!(
        names(&project)?,
        ["CON", "bytes", "long", "plain", "taken", "x:y"]
    );
    expect_contents(&[(project.join("x:y/a:b"), b"x:y/a:b")]);
    let mut renamed = report
        .renamed
        .iter()
        .map(|item| {
            (
                item.requested.file_name(),
                item.saved_as.file_name(),
                item.cause,
            )
        })
        .collect::<Vec<_>>();
    renamed.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(
        renamed,
        [
            (Some(bytes), Some("bytes".into()), RenameCause::Invalid),
            (Some(long), Some("long".into()), RenameCause::TooLong),
            (
                Some("taken".into()),
                Some("plain".into()),
                RenameCause::Taken
            ),
        ]
    );
    assert!(report.kept.is_empty(), "{report:?}");
    Ok(())
}
