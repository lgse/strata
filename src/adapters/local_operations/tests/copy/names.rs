// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_paste_fixes_the_names_it_brings_onto_fat_and_keeps_those_already_there()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (one, two, stick) = (
        root.path().join("one"),
        root.path().join("two"),
        root.path().join("stick"),
    );
    let earlier = stick.join("earlier");
    for folder in [&one, &two, &earlier.join("kept")] {
        fs::create_dir_all(folder)?;
    }
    for (path, contents) in [
        (one.join("a:b"), "colon"),
        (two.join("a_b"), "kept"),
        (one.join("x"), "lower"),
        (two.join("X"), "upper"),
        (earlier.join("CON"), "device"),
        (earlier.join("kept/CON"), "nested device"),
    ] {
        fs::write(path, contents)?;
    }
    // More than eight files are copied in parallel.
    let bulk = root.path().join("bulk");
    fs::create_dir_all(stick.join("bulk"))?;
    let mut bulk_items = vec![bulk.join("b?c")];
    bulk_items.extend((0..8).map(|index| bulk.join(format!("file {index}"))));
    fs::create_dir(&bulk)?;
    for item in &bulk_items {
        fs::write(item, "bulk")?;
    }
    let _stick = FsQuirks::install(&stick, Quirks::vfat());
    let paste = |destination: &Path, items: &[PathBuf]| PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(destination),
        items: items
            .iter()
            .map(|source| PasteItem {
                source: Location::local(source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: false,
    };

    let events = run_paste(paste(
        &stick,
        &[
            one.join("a:b"),
            two.join("a_b"),
            one.join("x"),
            two.join("X"),
            earlier.join("CON"),
            earlier.join("kept"),
        ],
    ));
    let bulk_events = run_paste(paste(&stick.join("bulk"), &bulk_items));

    for (name, contents) in [
        ("a_b (1)", "colon"),
        ("a_b", "kept"),
        ("x (1)", "lower"),
        ("X", "upper"),
        ("CON", "device"),
        ("kept/CON", "nested device"),
        ("bulk/b_c", "bulk"),
    ] {
        assert_eq!(fs::read(stick.join(name))?, contents.as_bytes(), "{name}");
    }
    assert_eq!(names(&stick)?.len(), 8);
    let renamed = |folder: &Path, events: &[OperationEvent]| {
        renamed_beneath(folder, &followed_report(events))
    };
    use crate::services::RenameCause::{Alias, Invalid};
    assert_eq!(
        renamed(&stick, &events),
        [("a:b", "a_b (1)", Invalid), ("x", "x (1)", Alias)].map(|(name, saved_as, cause)| (
            name.to_owned(),
            saved_as.to_owned(),
            cause
        ))
    );
    assert_eq!(
        renamed(&stick.join("bulk"), &bulk_events),
        [("b?c".to_owned(), "b_c".to_owned(), Invalid)]
    );
    Ok(())
}

#[test]
fn a_fat_copy_fixes_an_invalid_name_instead_of_discarding_the_whole_tree()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let stick = root.path().join("stick");
    let target = stick.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir(&stick)?;
    fs::write(source.join("ok.txt"), b"fine")?;
    fs::write(
        source.join("nested/sc_macroorganizer?tabid:short=1.png"),
        b"cached icon",
    )?;
    let _stick = FsQuirks::install(&stick, Quirks::vfat());

    let result = glib::MainContext::default().block_on(copy_recursively_fixing(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("ok.txt"))?, b"fine");
    assert_eq!(
        fs::read(target.join("nested/sc_macroorganizer_tabid_short=1.png"))?,
        b"cached icon"
    );
    assert_eq!(names(&target.join("nested"))?.len(), 1);
    Ok(())
}

#[test]
fn a_gio_copy_lists_a_whole_folder_first_only_where_the_mount_takes_two_names_for_one()
-> Result<(), Box<dyn Error>> {
    use super::super::super::{NamedChildren, mount_profile::FolderNames};
    use crate::adapters::volume::{NameEncoding, NameProfile, Reserved};
    use crate::services::RenameCause;
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    for name in ["a:b", "a_b"] {
        fs::write(folder.join(name), name)?;
    }
    // More than one batch of 64 children.
    for index in 0..100 {
        fs::write(folder.join(format!("pad {index:03}")), b"")?;
    }
    let ntfs3 = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8);
    let fat = NameProfile {
        folds: true,
        ..ntfs3
    };
    for (rules, listed_first) in [(ntfs3, false), (fat, true)] {
        let batches = glib::MainContext::default().block_on(async {
            let source = gio::File::for_path(&folder);
            let cancellable = gio::Cancellable::new();
            let enumerator =
                await_cancellable(&source, &cancellable, |source, cancellable, result| {
                    source.enumerate_children_async(
                        "standard::name,standard::type",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                        Some(cancellable),
                        move |output| result.resolve(output),
                    );
                })
                .await?;
            let names = FolderNames {
                rules,
                fixing: true,
                links: true,
            };
            let mut listed = NamedChildren::new(&enumerator, names);
            let mut batches = Vec::new();
            loop {
                let batch = listed.next(&cancellable).await?;
                if batch.is_empty() {
                    return Ok::<_, glib::Error>(batches);
                }
                batches.push(batch);
            }
        })?;

        let fixed = batches
            .iter()
            .flatten()
            .filter(|(_, _, planned)| planned.cause.is_some())
            .map(|(name, _, planned)| (name.clone(), planned.target.clone(), planned.cause))
            .collect::<Vec<_>>();
        assert_eq!(
            fixed,
            [("a:b".into(), "a_b (1)".into(), Some(RenameCause::Invalid))],
            "{rules:?}"
        );
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 102);
        if listed_first {
            assert_eq!(batches.len(), 1, "{rules:?}");
        } else {
            assert!(batches[0].len() <= 64, "{rules:?}");
            assert_eq!(batches.last().map(Vec::len), Some(1), "{rules:?}");
        }
    }
    Ok(())
}

#[test]
fn a_gio_copy_that_fails_reports_the_fixed_name_of_a_folder_it_made() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let target = destination.join("target");
    fs::create_dir_all(source.join("x:y"))?;
    fs::create_dir(&destination)?;
    // GIO copies no named pipe.
    make_fifo(&source.join("x:y/pipe"), 0o644)?;
    let _destination = FsQuirks::install(&destination, Quirks::vfat());
    let (progress, events) = recording_progress();

    let result =
        glib::MainContext::default().block_on(super::super::super::copy_recursively_with_progress(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            super::super::super::CopyInto::New,
            gio::Cancellable::new(),
            super::super::super::CopyRecord::Nothing,
            Some(progress),
            super::super::super::GioNames::new(fixing(), Default::default()),
        ));

    assert!(result.is_err());
    assert_eq!(names(&target)?, ["x_y"]);
    assert_eq!(
        renamed_beneath(&target, &followed_report(&events.borrow())),
        [(
            "x:y".to_owned(),
            "x_y".to_owned(),
            crate::services::RenameCause::Invalid
        )]
    );
    Ok(())
}

#[test]
fn fat_family_copies_number_fixed_names_past_the_names_their_siblings_keep()
-> Result<(), Box<dyn Error>> {
    use crate::services::RenameCause::{Alias, Invalid};
    let _serial = serial()?;
    type CopyTree = fn(
        gio::File,
        gio::File,
        gio::Cancellable,
        Rc<TransferProgressTracker>,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>;
    let copies: [(&str, CopyTree); 2] = [
        ("engine", |source, target, cancellable, progress| {
            Box::pin(async move {
                copy_new_recursively_on_filesystem(
                    source,
                    target,
                    Taken::Fail,
                    cancellable,
                    Some(progress),
                    fixing(),
                    None,
                )
                .await
                .map(drop)
            })
        }),
        ("GIO", |source, target, cancellable, progress| {
            super::super::super::copy_recursively_with_progress(
                source,
                target,
                super::super::super::CopyInto::New,
                cancellable,
                super::super::super::CopyRecord::Nothing,
                Some(progress),
                super::super::super::GioNames::new(fixing(), Default::default()),
            )
        }),
    ];
    for (row, copy) in copies {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        let target = destination.join("target");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        fs::write(source.join("a?.txt"), b"question")?;
        // Enough siblings that GIO lists them in several batches.
        for index in 0..130 {
            fs::write(source.join(format!("pad {index:03}")), b"")?;
        }
        fs::write(source.join("a:.txt"), b"colon")?;
        fs::write(source.join("A*.txt"), b"star")?;
        fs::write(source.join("a_.txt"), b"kept")?;
        fs::write(source.join("README.txt"), b"upper")?;
        fs::write(source.join("Readme.txt"), b"mixed")?;
        fs::write(source.join("CON"), b"device")?;
        for folder in ["Photos.2024", "photos.2024"] {
            fs::create_dir(source.join(folder))?;
            fs::write(source.join(folder).join("photo"), folder)?;
        }
        let _destination = FsQuirks::install(&destination, Quirks::vfat());
        let (progress, events) = recording_progress();

        let result = glib::MainContext::default().block_on(copy(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            gio::Cancellable::new(),
            progress,
        ));

        assert!(result.is_ok(), "{row}: {result:?}");
        assert_eq!(
            renamed_beneath(&target, &followed_report(&events.borrow())),
            [
                ("A*.txt", "A_ (1).txt", Invalid),
                ("CON", "CON_", Invalid),
                ("Readme.txt", "Readme (1).txt", Alias),
                ("a:.txt", "a_ (2).txt", Invalid),
                ("a?.txt", "a_ (3).txt", Invalid),
                ("photos.2024", "photos.2024 (1)", Alias),
            ]
            .map(|(name, saved_as, cause)| (
                name.to_owned(),
                saved_as.to_owned(),
                cause
            )),
            "{row}"
        );
        for (name, contents) in [
            ("a_.txt", "kept"),
            ("A_ (1).txt", "star"),
            ("a_ (2).txt", "colon"),
            ("a_ (3).txt", "question"),
            ("README.txt", "upper"),
            ("Readme (1).txt", "mixed"),
            ("CON_", "device"),
            ("Photos.2024/photo", "Photos.2024"),
            ("photos.2024 (1)/photo", "photos.2024"),
        ] {
            assert_eq!(
                fs::read(target.join(name))?,
                contents.as_bytes(),
                "{row}: {name}"
            );
        }
        assert_eq!(names(&target)?.len(), 139, "{row}");
    }
    Ok(())
}

#[test]
fn a_copy_into_a_case_sensitive_folder_keeps_a_cut_name_apart_by_case_only()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let disk = root.path().join("disk");
    let target = disk.join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir(&disk)?;
    let (long, short) = (
        format!("A{}", "b".repeat(70)),
        format!("a{}", "b".repeat(63)),
    );
    for (name, contents) in [
        (long.as_str(), "long"),
        (short.as_str(), "short"),
        ("README", "upper"),
        ("Readme", "mixed"),
    ] {
        fs::write(source.join(name), contents)?;
    }
    let _disk = FsQuirks::install(
        &disk,
        Quirks {
            name_max: Some(64),
            ..Quirks::ext4()
        },
    );
    let (progress, events) = recording_progress();

    glib::MainContext::default().block_on(copy_new_recursively_on_filesystem(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        Some(progress),
        fixing(),
        None,
    ))?;

    assert_eq!(
        renamed_beneath(&target, &followed_report(&events.borrow())),
        [(
            long.clone(),
            format!("A{}", "b".repeat(63)),
            crate::services::RenameCause::TooLong
        )]
    );
    for (name, contents) in [
        (format!("A{}", "b".repeat(63)), "long"),
        (short, "short"),
        ("README".to_owned(), "upper"),
        ("Readme".to_owned(), "mixed"),
    ] {
        assert_eq!(fs::read(target.join(&name))?, contents.as_bytes(), "{name}");
    }
    assert_eq!(names(&target)?.len(), 4);
    Ok(())
}

#[test]
fn copies_onto_mounts_with_windows_rules_fix_names_and_report_them() -> Result<(), Box<dyn Error>> {
    use crate::services::RenameCause::{Invalid, TooLong};
    let _serial = serial()?;
    type CopyTree = fn(
        gio::File,
        gio::File,
        Rc<TransferProgressTracker>,
    ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>>;
    let copies: [(&str, CopyTree); 2] = [
        ("engine", |source, target, progress| {
            Box::pin(async move {
                copy_new_recursively_on_filesystem(
                    source,
                    target,
                    Taken::Fail,
                    gio::Cancellable::new(),
                    Some(progress),
                    fixing(),
                    None,
                )
                .await
                .map(drop)
            })
        }),
        ("GIO", |source, target, progress| {
            super::super::super::copy_recursively_with_progress(
                source,
                target,
                super::super::super::CopyInto::New,
                gio::Cancellable::new(),
                super::super::super::CopyRecord::Nothing,
                Some(progress),
                super::super::super::GioNames::new(fixing(), Default::default()),
            )
        }),
    ];
    let (folder, cut) = ("d".repeat(220), "d".repeat(213));
    let rows: [(&str, Preset, &str, bool, &str, _); 7] = [
        ("ext4", Quirks::ext4, "a?b:c.txt", false, "a?b:c.txt", None),
        (
            "exfat-fuse",
            Quirks::exfat_fuse,
            "a:b",
            false,
            "a_b",
            Some(Invalid),
        ),
        ("ntfs3", Quirks::ntfs3, "CON", false, "CON_", Some(Invalid)),
        (
            "ntfs-3g",
            Quirks::ntfs_3g,
            "con.txt",
            false,
            "con_.txt",
            Some(Invalid),
        ),
        ("cifs", Quirks::cifs, "x:y", false, "x_y", Some(Invalid)),
        // Samba makes a folder under a longer name first.
        ("cifs", Quirks::cifs, &folder, true, &cut, Some(TooLong)),
        (
            "cifs nocase",
            Quirks::cifs_nocase,
            "😀.txt",
            false,
            "_.txt",
            Some(Invalid),
        ),
    ];
    for (copy_label, copy) in copies {
        for (preset, quirks, name, is_folder, saved, cause) in &rows {
            let row = format!("{copy_label}, {preset}, {name}");
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            let disk = root.path().join("disk");
            let target = disk.join("target");
            fs::create_dir_all(&source)?;
            fs::create_dir(&disk)?;
            if *is_folder {
                fs::create_dir(source.join(name))?;
                fs::write(source.join(name).join("inside"), name)?;
            } else {
                fs::write(source.join(name), name)?;
            }
            let _disk = FsQuirks::install(&disk, quirks());
            let (progress, events) = recording_progress();

            let result = glib::MainContext::default().block_on(copy(
                gio::File::for_path(&source),
                gio::File::for_path(&target),
                progress,
            ));

            assert!(result.is_ok(), "{row}: {result:?}");
            assert_eq!(names(&target)?, [*saved], "{row}");
            let contents = if *is_folder {
                target.join(saved).join("inside")
            } else {
                target.join(saved)
            };
            assert_eq!(fs::read(contents)?, name.as_bytes(), "{row}");
            let expected = cause
                .map(|cause| (name.to_string(), saved.to_string(), cause))
                .into_iter()
                .collect::<Vec<_>>();
            assert_eq!(
                renamed_beneath(&target, &followed_report(&events.borrow())),
                expected,
                "{row}"
            );
        }
    }
    Ok(())
}
