// SPDX-License-Identifier: MIT

use std::os::unix::fs::MetadataExt;

use super::fs_quirks::{Op, Racer};
use super::*;
use crate::services::{RenameCause, RenamedItem};

fn copy_renaming(
    source: &Path,
    target: &Path,
    workers: Option<usize>,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let relative = |location: &Location| {
        location
            .native_path()
            .and_then(|path| path.strip_prefix(target).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    Ok(copy_reporting(source, target, workers)?
        .renamed
        .into_iter()
        .map(
            |RenamedItem {
                 requested,
                 saved_as,
                 cause,
             }| {
                assert_eq!(cause, RenameCause::Alias);
                (relative(&requested), relative(&saved_as))
            },
        )
        .collect())
}

#[test]
fn case_aliases_inside_a_copied_folder_are_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Names are claimed in byte order where they may collide, so the later
    // one is numbered; elsewhere the one made last is.
    let names_never_collide = || Quirks {
        case_insensitive: true,
        ..Quirks::ext4()
    };
    // ciopfs folds case like FAT, and holds links, which FAT cannot.
    let claimed = || Quirks {
        mount_type: Some(("fuse.ciopfs", "rw")),
        case_insensitive: true,
        ..Quirks::default()
    };
    let rows: [(&str, Preset, &str, &str); 2] = [
        ("claimed", claimed, "makefile", "makefile (2)"),
        ("made", names_never_collide, "Makefile", "Makefile (2)"),
    ];
    for (row, preset, aliased, saved_as) in rows {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir_all(source.join("Docs"))?;
        fs::create_dir_all(source.join("docs"))?;
        fs::create_dir(&destination)?;
        fs::write(source.join("Docs/upper.txt"), b"upper")?;
        fs::write(source.join("docs/lower.txt"), b"lower")?;
        for name in ["Makefile", "makefile", "makefile (1)"] {
            fs::write(source.join(name), name)?;
        }
        std::os::unix::fs::symlink("upper", source.join("Link"))?;
        std::os::unix::fs::symlink("lower", source.join("link"))?;
        let _sources = FsQuirks::install(&source, Quirks::default());
        let _destination = FsQuirks::install(&destination, preset());
        let target = destination.join("copy");

        let renamed = copy_renaming(&source, &target, Some(1))?;

        let mut expected = vec![
            ("docs".to_owned(), "docs (1)".to_owned()),
            ("link".to_owned(), "link (1)".to_owned()),
            (aliased.to_owned(), saved_as.to_owned()),
        ];
        expected.sort();
        let mut renamed = renamed;
        renamed.sort();
        assert_eq!(renamed, expected, "{row}");
        let other = if aliased == "makefile" {
            "Makefile"
        } else {
            "makefile"
        };
        let mut expected = vec!["Docs", "Link", "docs (1)", "link (1)", "makefile (1)"];
        expected.extend([other, saved_as]);
        expected.sort_unstable();
        assert_eq!(names(&target)?, expected, "{row}");
        assert_eq!(fs::read_to_string(target.join(saved_as))?, aliased, "{row}");
        assert_eq!(fs::read_to_string(target.join(other))?, other, "{row}");
        assert_eq!(
            fs::read_to_string(target.join("makefile (1)"))?,
            "makefile (1)",
            "{row}: the sibling keeps its own name"
        );
        assert_eq!(fs::read(target.join("Docs/upper.txt"))?, b"upper", "{row}");
        assert_eq!(
            fs::read(target.join("docs (1)/lower.txt"))?,
            b"lower",
            "{row}"
        );
        assert_eq!(fs::read_link(target.join("Link"))?, Path::new("upper"));
        assert_eq!(fs::read_link(target.join("link (1)"))?, Path::new("lower"));
    }
    Ok(())
}

#[test]
fn aliases_of_long_names_are_numbered_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let names_never_collide = || Quirks {
        case_insensitive: true,
        ..Quirks::ext4()
    };
    let claimed = || Quirks {
        mount_type: Some(("fuse.ciopfs", "rw")),
        case_insensitive: true,
        ..Quirks::default()
    };
    let (upper, lower) = (
        format!("A{}.txt", "a".repeat(250)),
        format!("{}.txt", "a".repeat(251)),
    );
    let rows: [(&str, Preset, &str, String); 2] = [
        (
            "claimed",
            claimed,
            &lower,
            format!("{} (1).txt", "a".repeat(247)),
        ),
        (
            "made",
            names_never_collide,
            &upper,
            format!("A{} (1).txt", "a".repeat(246)),
        ),
    ];
    for (row, preset, aliased, saved_as) in rows {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir(&source)?;
        fs::create_dir(&destination)?;
        fs::write(source.join(&upper), "upper")?;
        fs::write(source.join(&lower), "lower")?;
        let _destination = FsQuirks::install(&destination, preset());
        let target = destination.join("copy");

        let renamed = copy_renaming(&source, &target, Some(1))?;

        assert_eq!(renamed, [(aliased.to_owned(), saved_as.clone())], "{row}");
        let kept = if aliased == upper { &lower } else { &upper };
        let mut expected = vec![kept.clone(), saved_as.clone()];
        expected.sort();
        assert_eq!(names(&target)?, expected, "{row}");
    }
    Ok(())
}

#[test]
fn a_linked_name_that_aliases_another_is_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(&source)?;
    fs::create_dir(&destination)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("B"))?;
    fs::write(source.join("b"), b"other")?;
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            case_insensitive: true,
            ..Quirks::ext4()
        },
    );
    let target = destination.join("copy");

    // One worker takes the folder's files last first, so `B` is linked
    // after `b` is made.
    let renamed = copy_renaming(&source, &target, Some(1))?;

    assert_eq!(renamed, [("B".to_owned(), "B (1)".to_owned())]);
    assert_eq!(names(&target)?, ["B (1)", "a", "b"]);
    assert_eq!(
        fs::metadata(target.join("B (1)"))?.ino(),
        fs::metadata(target.join("a"))?.ino()
    );
    assert_eq!(fs::read(target.join("b"))?, b"other");
    Ok(())
}

#[test]
fn two_names_are_never_taken_for_one_where_the_listing_changes_their_case()
-> Result<(), Box<dyn Error>> {
    use super::fs_quirks::{Op, Racer, renumbered};
    use crate::services::{PasteItem, PasteRequest};
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // ciopfs lists `Makefile` as `makefile` once it is made, as Samba with
    // `preserve case = no` does; a racer renames it so. Where inode numbers
    // are not kept, the entry then reports another number. FAT and SMB plan
    // such names apart before.
    let rows = [
        ("copy", false, false),
        ("move", true, false),
        ("unstable inodes", false, true),
    ];
    for (row, moving, renumber) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let folder = sources.join("folder");
        let destination = root.path().join("destination");
        fs::create_dir_all(&folder)?;
        fs::create_dir(&destination)?;
        fs::write(folder.join("Makefile"), b"upper")?;
        fs::write(folder.join("makefile"), b"lower")?;
        let live = destination.clone();
        let racer: Racer = Box::new(move || {
            let stage = names(&live)
                .expect("destination")
                .into_iter()
                .find(|name| name.starts_with(".strata-copy-"))
                .expect("stage");
            let lowered = live.join(stage).join("makefile");
            fs::rename(lowered.with_file_name("Makefile"), &lowered).expect("lower the name");
            if renumber {
                renumbered(&lowered);
            }
        });
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                mount_type: Some(("fuse.ciopfs", "rw")),
                case_insensitive: true,
                before: vec![(Op::CreateNode, "makefile", 0, racer)],
                ..Quirks::default()
            },
        );

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&folder),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            move_sources: moving,
        });

        let copied = destination.join("folder");
        let renamed = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemsReported { report, .. } => Some(report.renamed.clone()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(
            renamed,
            [RenamedItem {
                requested: Location::local(copied.join("makefile")),
                saved_as: Location::local(copied.join("makefile (1)")),
                cause: RenameCause::Alias,
            }],
            "{row}"
        );
        assert_eq!(names(&copied)?, ["makefile", "makefile (1)"], "{row}");
        assert_eq!(fs::read(copied.join("makefile"))?, b"upper", "{row}");
        assert_eq!(fs::read(copied.join("makefile (1)"))?, b"lower", "{row}");
        assert_eq!(folder.exists(), !moving, "{row}");
    }
    Ok(())
}

#[test]
fn names_a_fat_target_takes_for_one_are_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(&source)?;
    fs::create_dir(&destination)?;
    // `a:b` would be saved as `a_b`, which the real `a_b` keeps.
    for name in ["Makefile", "a:b", "a_b", "makefile"] {
        fs::write(source.join(name), name)?;
    }
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _destination = FsQuirks::install(&destination, Quirks::vfat());
    let target = destination.join("copy");
    let options = super::super::copy_engine::Options {
        names: NamePolicy::new(NameMode::Fix),
        ..super::super::copy_engine::Options::default()
    };

    let (copied, report) = reporting(
        &source,
        super::super::copy_engine::Destination::New(target.clone(), Taken::Fail),
        options,
    );

    copied?;
    assert_eq!(
        renamed_beneath(&target, &report),
        [
            ("a:b", "a_b (1)", RenameCause::Invalid),
            ("makefile", "makefile (1)", RenameCause::Alias),
        ]
        .map(|(name, saved_as, cause)| (name.to_owned(), saved_as.to_owned(), cause))
    );
    assert_eq!(
        names(&target)?,
        ["Makefile", "a_b", "a_b (1)", "makefile (1)"]
    );
    assert_eq!(fs::read(target.join("a_b"))?, b"a_b");
    assert_eq!(fs::read(target.join("a_b (1)"))?, b"a:b");
    assert_eq!(fs::read(target.join("makefile (1)"))?, b"makefile");
    Ok(())
}

#[test]
fn top_level_items_a_destination_takes_for_an_earlier_one_are_numbered_and_reported()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Multigrain times, so a move reads its just-written sources only to
    // copy them, not again to compare them.
    let _kernel = KernelGuard::set(Some((6, 13, 0)));
    let casefolded = || Quirks {
        case_insensitive: true,
        ..Quirks::ext4()
    };
    let case_insensitive_sftp_server = || Quirks {
        case_insensitive: true,
        alias_inodes: true,
        ..Quirks::sshfs()
    };
    // Past eight files a paste copies several at once.
    let padding = (0..8)
        .map(|index| format!("pad {index}"))
        .collect::<Vec<_>>();
    for (mount, preset) in [
        ("casefolded ext4", casefolded as Preset),
        ("cifs", Quirks::cifs),
        (
            "sshfs to a case-insensitive server",
            case_insensitive_sftp_server,
        ),
    ] {
        for moving in [false, true] {
            for parallel in [false, true] {
                let context = format!("{mount}, moving: {moving}, parallel: {parallel}");
                let root = tempfile::tempdir()?;
                let source = root.path().join("source");
                let destination = root.path().join("destination");
                fs::create_dir_all(&source)?;
                fs::create_dir(&destination)?;
                let mut items = vec!["Makefile", "makefile"];
                if parallel {
                    items.extend(padding.iter().map(String::as_str));
                } else {
                    items.extend(["Docs", "docs"]);
                }
                for name in &items {
                    if name.ends_with("ocs") {
                        fs::create_dir(source.join(name))?;
                        fs::write(source.join(name).join("inside"), name)?;
                    } else {
                        fs::write(source.join(name), name)?;
                    }
                }
                let sources = FsQuirks::install(&source, Quirks::ext4());
                let _destination = FsQuirks::install(&destination, preset());

                let events = run_paste(PasteRequest {
                    id: OperationRequestId(1530),
                    destination: Location::local(&destination),
                    items: items
                        .iter()
                        .map(|name| PasteItem {
                            source: Location::local(source.join(name)),
                            conflict: TransferConflict::FailIfExists,
                            target_name: None,
                        })
                        .collect(),
                    move_sources: moving,
                });

                let numbered = if parallel {
                    vec!["makefile"]
                } else {
                    vec!["docs", "makefile"]
                };
                let expected = numbered
                    .iter()
                    .map(|name| (name.to_string(), format!("{name} (1)"), RenameCause::Alias))
                    .collect::<Vec<_>>();
                assert_eq!(
                    renamed_beneath(&destination, &followed_report(&events)),
                    expected,
                    "{context}"
                );
                for name in numbered {
                    let saved_as = destination.join(format!("{name} (1)"));
                    let read = if name.ends_with("ocs") {
                        fs::read_to_string(saved_as.join("inside"))?
                    } else {
                        fs::read_to_string(saved_as)?
                    };
                    assert_eq!(read, name, "{context}");
                }
                assert_eq!(names(&destination)?.len(), items.len(), "{context}");
                assert_eq!(names(&source)?.is_empty(), moving, "{context}");
                assert_eq!(
                    sources.count(Op::OpenSource),
                    items.len(),
                    "{context}: each file is read once"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn smb_without_posix_extensions_numbers_case_pairs_without_looking_up_the_later_spelling()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The cifs client turns server inode numbers off for the whole mount once
    // it looks a folder up under a second spelling.
    let later = ["docs", "makefile", "project/notes"];
    for (preset, numbered) in [(Quirks::cifs as Preset, true), (Quirks::cifs_posix, false)] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir_all(source.join("project"))?;
        fs::create_dir(&destination)?;
        for folder in ["Docs", "docs"] {
            fs::create_dir(source.join(folder))?;
            fs::write(source.join(folder).join("inside"), folder)?;
        }
        for file in ["Makefile", "makefile", "project/Notes", "project/notes"] {
            fs::write(source.join(file), file)?;
        }
        let scope = FsQuirks::install(&destination, preset());

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1530),
            destination: Location::local(&destination),
            items: ["Docs", "Makefile", "docs", "makefile", "project"]
                .map(|item| PasteItem {
                    source: Location::local(source.join(item)),
                    conflict: TransferConflict::FailIfExists,
                    target_name: None,
                })
                .into(),
            move_sources: false,
        });

        let context = format!("{:?}", preset().mount_type);
        let expected = if numbered {
            later
                .map(|name| (name.to_owned(), format!("{name} (1)"), RenameCause::Alias))
                .into()
        } else {
            Vec::new()
        };
        assert_eq!(
            renamed_beneath(&destination, &followed_report(&events)),
            expected,
            "{context}"
        );
        for name in later {
            let saved_as = destination.join(if numbered {
                format!("{name} (1)")
            } else {
                name.to_owned()
            });
            let read = if name == "docs" {
                fs::read_to_string(saved_as.join("inside"))?
            } else {
                fs::read_to_string(saved_as)?
            };
            assert_eq!(read, name, "{context}");
            let spelling = Path::new(name).file_name().ok_or("no name")?;
            assert_eq!(
                scope.used_name(&spelling.to_string_lossy()),
                !numbered,
                "{context}: {name}"
            );
        }
    }
    Ok(())
}

#[test]
fn identical_top_level_names_from_different_folders_are_numbered_and_reported()
-> Result<(), Box<dyn Error>> {
    use super::super::transfer_targets::transfer_targets;
    let _serial = serial()?;
    // Past eight files a paste copies several at once.
    let padding = (0..8)
        .map(|index| format!("one/pad {index}"))
        .collect::<Vec<_>>();
    let rows: [(&str, Preset, bool); 3] = [
        ("one filesystem", Quirks::ext4, true),
        ("another filesystem", Quirks::ext4, false),
        ("vfat", Quirks::vfat, false),
    ];
    // A prompt's names reach the paste as the names it showed.
    let cases = [false, true]
        .into_iter()
        .flat_map(|moving| [false, true].map(|parallel| (moving, parallel)))
        .flat_map(|case| [false, true].map(|prompted| (case, prompted)));
    for (mount, preset, one_filesystem) in rows {
        for ((moving, parallel), prompted) in cases.clone() {
            let context =
                format!("{mount}, moving: {moving}, parallel: {parallel}, prompted: {prompted}");
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            let destination = root.path().join("destination");
            fs::create_dir_all(source.join("one"))?;
            fs::create_dir_all(source.join("two"))?;
            fs::create_dir(&destination)?;
            let mut items = vec!["one/notes.txt", "two/notes.txt"];
            if parallel {
                items.extend(padding.iter().map(String::as_str));
            } else {
                items.extend(["one/docs", "two/docs"]);
            }
            for item in &items {
                if item.ends_with("docs") {
                    fs::create_dir(source.join(item))?;
                    fs::write(source.join(item).join("inside"), item)?;
                } else {
                    fs::write(source.join(item), item)?;
                }
            }
            let mount_root = if one_filesystem {
                root.path()
            } else {
                destination.as_path()
            };
            let _scope = FsQuirks::install(mount_root, preset());
            let sources = items
                .iter()
                .map(|item| Location::local(source.join(item)))
                .collect::<Vec<_>>();
            let shown = if prompted {
                glib::MainContext::default()
                    .block_on(transfer_targets(
                        &Location::local(&destination),
                        &sources,
                        moving,
                        &gio::Cancellable::new(),
                    ))?
                    .into_iter()
                    .map(|target| target.name)
                    .collect()
            } else {
                vec![None; sources.len()]
            };

            let events = run_paste(PasteRequest {
                id: OperationRequestId(1530),
                destination: Location::local(&destination),
                items: sources
                    .into_iter()
                    .zip(shown)
                    .map(|(source, target_name)| PasteItem {
                        source,
                        conflict: TransferConflict::FailIfExists,
                        target_name,
                    })
                    .collect(),
                move_sources: moving,
            });

            let mut numbered = vec![("notes.txt", "notes (1).txt")];
            if !parallel {
                numbered.insert(0, ("docs", "docs (1)"));
            }
            let expected = numbered
                .iter()
                .map(|(name, saved_as)| {
                    (name.to_string(), saved_as.to_string(), RenameCause::Taken)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                renamed_beneath(&destination, &followed_report(&events)),
                expected,
                "{context}"
            );
            for (name, saved_as) in numbered {
                let (first, second) = (destination.join(name), destination.join(saved_as));
                let read = |path: PathBuf| {
                    fs::read_to_string(if name == "docs" {
                        path.join("inside")
                    } else {
                        path
                    })
                };
                assert_eq!(read(first)?, format!("one/{name}"), "{context}");
                assert_eq!(read(second)?, format!("two/{name}"), "{context}");
            }
            assert_eq!(names(&destination)?.len(), items.len(), "{context}");
            for folder in ["one", "two"] {
                assert_eq!(names(&source.join(folder))?.is_empty(), moving, "{context}");
            }
        }
    }
    Ok(())
}

#[test]
fn an_item_from_another_folder_meets_the_original_of_a_duplicate() -> Result<(), Box<dyn Error>> {
    use super::super::transfer_targets::{Collision, transfer_targets};
    let _serial = serial()?;
    let meets = Some(Collision { mergeable: false });
    // The mount, the original's name, whether two/'s item comes first, whether it is moved, what
    // the prompt shows for one/'s and two/'s items, and one/ once two/'s is kept beside it.
    let rows = [
        (
            Quirks::ext4(),
            "notes.txt",
            false,
            false,
            [("notes.txt", None), ("notes.txt", meets)],
            vec![
                ("notes (1).txt", "one"),
                ("notes (2).txt", "two"),
                ("notes.txt", "one"),
            ],
        ),
        (
            Quirks::ext4(),
            "notes.txt",
            true,
            false,
            [("notes.txt", None), ("notes.txt", meets)],
            vec![
                ("notes (1).txt", "one"),
                ("notes (2).txt", "two"),
                ("notes.txt", "one"),
            ],
        ),
        (
            Quirks::vfat(),
            "NOTES.TXT",
            false,
            false,
            [("NOTES.TXT", None), ("notes.txt", meets)],
            vec![
                ("NOTES (1).TXT", "one"),
                ("NOTES.TXT", "one"),
                ("notes (2).txt", "two"),
            ],
        ),
        // A moved item stays where it is and keeps its name.
        (
            Quirks::ext4(),
            "notes.txt",
            false,
            true,
            [("notes.txt", None), ("notes (1).txt", None)],
            vec![("notes (1).txt", "two"), ("notes.txt", "one")],
        ),
    ];
    for (quirks, original, two_first, moving, shown, listed) in rows {
        let context = format!("{original}, two first: {two_first}, moving: {moving}");
        let root = tempfile::tempdir()?;
        let (one, two) = (root.path().join("one"), root.path().join("two"));
        fs::create_dir(&one)?;
        fs::create_dir(&two)?;
        fs::write(one.join(original), "one")?;
        fs::write(two.join("notes.txt"), "two")?;
        let _scope = FsQuirks::install(root.path(), quirks);
        let mut sources = vec![
            Location::local(one.join(original)),
            Location::local(two.join("notes.txt")),
        ];
        let mut expected = shown
            .map(|(name, collision)| (Some(OsString::from(name)), collision))
            .to_vec();
        if two_first {
            sources.reverse();
            expected.reverse();
        }

        let targets = glib::MainContext::default().block_on(transfer_targets(
            &Location::local(&one),
            &sources,
            moving,
            &gio::Cancellable::new(),
        ))?;
        assert_eq!(
            targets
                .iter()
                .map(|target| {
                    let name = target.name.as_ref().map(|shown| shown.name.clone());
                    (name, target.collision)
                })
                .collect::<Vec<_>>(),
            expected,
            "{context}"
        );

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1530),
            destination: Location::local(&one),
            items: sources
                .into_iter()
                .zip(targets)
                .map(|(source, target)| PasteItem {
                    source,
                    conflict: if target.collision.is_some() {
                        TransferConflict::KeepBoth
                    } else {
                        TransferConflict::FailIfExists
                    },
                    target_name: target.name,
                })
                .collect(),
            move_sources: moving,
        });

        let held = names(&one)?
            .into_iter()
            .map(|name| {
                let content = fs::read_to_string(one.join(&name))?;
                Ok((name, content))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let listed = listed
            .into_iter()
            .map(|(name, content)| (name.to_owned(), content.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(held, listed, "{context}");
        let numbered = moving.then(|| {
            (
                "notes.txt".to_owned(),
                "notes (1).txt".to_owned(),
                RenameCause::Taken,
            )
        });
        assert_eq!(
            renamed_beneath(&one, &followed_report(&events)),
            Vec::from_iter(numbered),
            "{context}"
        );
        assert_eq!(names(&two)?.is_empty(), moving, "{context}");
    }
    Ok(())
}

#[test]
fn same_named_items_from_three_folders_all_land_whatever_their_order_and_choice()
-> Result<(), Box<dyn Error>> {
    use super::super::transfer_targets::transfer_targets;
    let _serial = serial()?;
    let (keep_both, replace) = (
        TransferConflict::KeepBoth,
        TransferConflict::ReplaceExisting,
    );
    let three_one_two = vec![
        ("notes (1).txt", "three"),
        ("notes (2).txt", "one"),
        ("notes (3).txt", "two"),
        ("notes.txt", "one"),
    ];
    // The folders whose notes.txt are pasted into one/, as selected; the choice for the item that
    // meets one/notes.txt, None skipping it; whether the request lists the items as selected
    // instead of as the browser does, the ones asked about last; and one/ after the paste.
    let rows = [
        (
            vec!["two", "three"],
            Some(keep_both),
            false,
            vec![
                ("notes (1).txt", "three"),
                ("notes (2).txt", "two"),
                ("notes.txt", "one"),
            ],
        ),
        (
            vec!["two", "three"],
            None,
            false,
            vec![("notes (1).txt", "three"), ("notes.txt", "one")],
        ),
        (
            vec!["one", "two", "three"],
            Some(keep_both),
            false,
            three_one_two.clone(),
        ),
        (
            vec!["one", "two", "three"],
            None,
            false,
            vec![
                ("notes (1).txt", "three"),
                ("notes (2).txt", "one"),
                ("notes.txt", "one"),
            ],
        ),
        (
            vec!["two", "three", "one"],
            Some(keep_both),
            false,
            three_one_two.clone(),
        ),
        (
            vec!["one", "two", "three"],
            Some(replace),
            false,
            vec![
                ("notes (1).txt", "three"),
                ("notes (2).txt", "one"),
                ("notes.txt", "two"),
            ],
        ),
        (
            vec!["two", "one", "three"],
            Some(keep_both),
            false,
            three_one_two.clone(),
        ),
        (
            vec!["one", "three", "two"],
            Some(keep_both),
            false,
            vec![
                ("notes (1).txt", "two"),
                ("notes (2).txt", "one"),
                ("notes (3).txt", "three"),
                ("notes.txt", "one"),
            ],
        ),
        (
            vec!["two", "three"],
            Some(keep_both),
            true,
            vec![
                ("notes (1).txt", "three"),
                ("notes (2).txt", "two"),
                ("notes.txt", "one"),
            ],
        ),
        (
            vec!["one", "two", "three"],
            Some(keep_both),
            true,
            three_one_two.clone(),
        ),
        (
            vec!["two", "three", "one"],
            Some(keep_both),
            true,
            three_one_two.clone(),
        ),
    ];
    for (selected, choice, as_selected, listed) in rows {
        let context = format!("{selected:?}, {choice:?}, as selected: {as_selected}");
        let root = tempfile::tempdir()?;
        for folder in ["one", "two", "three"] {
            fs::create_dir(root.path().join(folder))?;
            fs::write(root.path().join(folder).join("notes.txt"), folder)?;
        }
        let one = root.path().join("one");
        let sources = selected
            .iter()
            .map(|folder| Location::local(root.path().join(folder).join("notes.txt")))
            .collect::<Vec<_>>();
        let targets = glib::MainContext::default().block_on(transfer_targets(
            &Location::local(&one),
            &sources,
            false,
            &gio::Cancellable::new(),
        ))?;
        let mut items = sources
            .into_iter()
            .zip(targets)
            .filter_map(|(source, target)| {
                let conflict = match target.collision {
                    Some(_) => choice?,
                    None => TransferConflict::FailIfExists,
                };
                Some(PasteItem {
                    source,
                    conflict,
                    target_name: target.name,
                })
            })
            .collect::<Vec<_>>();
        if !as_selected {
            items.sort_by_key(|item| item.conflict != TransferConflict::FailIfExists);
        }

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(1530),
                destination: Location::local(&one),
                items,
                move_sources: false,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);

        let held = names(&one)?
            .into_iter()
            .map(|name| {
                let content = fs::read_to_string(one.join(&name))?;
                Ok((name, content))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let listed = listed
            .iter()
            .map(|(name, content)| (name.to_string(), content.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(held, listed, "{context}");
        assert_eq!(
            renamed_beneath(&one, &followed_report(&events.borrow())),
            [(
                "notes.txt".to_owned(),
                "notes (1).txt".to_owned(),
                RenameCause::Taken
            )],
            "{context}"
        );
    }
    Ok(())
}

#[test]
fn an_item_already_at_a_top_level_name_is_never_taken_for_one_the_paste_saved()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let padding = (0..8).map(|index| format!("item {index}"));
    let spellings = ["Makefile", "makefile"].map(String::from);
    // Past eight files a paste copies several at once.
    let rows = [
        (
            "theirs at the same spelling",
            "item 3",
            padding
                .clone()
                .chain(["item 8", "item 9"].map(String::from))
                .collect(),
            false,
        ),
        (
            "theirs at another spelling, at once",
            "MAKEFILE",
            spellings.iter().cloned().chain(padding.clone()).collect(),
            false,
        ),
        (
            "theirs at another spelling, one at a time",
            "MAKEFILE",
            spellings.to_vec(),
            false,
        ),
        (
            "theirs once the earlier spelling was saved",
            "MAKEFILE",
            spellings.to_vec(),
            true,
        ),
        (
            "theirs once the earlier spelling was saved, at once",
            "MAKEFILE",
            spellings.iter().cloned().chain(padding).collect(),
            true,
        ),
    ];
    for (row, theirs, items, after_saved) in rows {
        let items: Vec<String> = items;
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        for item in &items {
            let path = source.join(item);
            fs::create_dir_all(path.parent().ok_or("no parent")?)?;
            fs::write(path, item)?;
        }
        let moved_out = root.path().join("moved out");
        let mut before = Vec::new();
        if after_saved {
            let (saved, taken, moved_out) = (
                destination.join("Makefile"),
                destination.join(theirs),
                moved_out.clone(),
            );
            let racer: Racer = Box::new(move || {
                fs::rename(&saved, &moved_out).expect("move the saved item out");
                fs::write(&taken, b"theirs").expect("take its name");
            });
            before.push((Op::Stat, "makefile", 0, racer));
        } else {
            fs::write(destination.join(theirs), b"theirs")?;
        }
        let _source = FsQuirks::install(&source, Quirks::ext4());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before,
                case_insensitive: true,
                ..Quirks::ext4()
            },
        );

        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        let _operation = LocalOperationProvider.paste(
            PasteRequest {
                id: OperationRequestId(1530),
                destination: Location::local(&destination),
                items: items
                    .iter()
                    .map(|item| PasteItem {
                        source: Location::local(source.join(item)),
                        conflict: TransferConflict::FailIfExists,
                        target_name: None,
                    })
                    .collect(),
                move_sources: false,
            },
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_transfer_settles(&events);

        let events = events.take();
        assert!(
            matches!(
                terminal_transfer(&events),
                Some(OperationEvent::TransferFailed { .. })
            ),
            "{row}: {events:?}"
        );
        assert_eq!(fs::read(destination.join(theirs))?, b"theirs", "{row}");
        let saved = names(&destination)?;
        assert!(
            saved.iter().all(|name| !name.ends_with(" (1)")),
            "{row}: {saved:?}"
        );
        assert_eq!(moved_out.exists(), after_saved, "{row}");
    }
    Ok(())
}
