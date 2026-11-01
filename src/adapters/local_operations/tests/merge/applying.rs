// SPDX-License-Identifier: MIT

use super::*;

type MakeCopy = fn() -> CopyStep;

fn gio_merge_copy(names: NamePolicy) -> CopyStep {
    Rc::new(
        move |source, target, _: super::super::super::MergeCopy, cancellable| {
            let names = super::super::super::GioNames::new(names.clone(), Default::default());
            Box::pin(async move {
                super::super::super::copy_recursively_with_progress(
                    source,
                    target,
                    super::super::super::CopyInto::Existing,
                    cancellable,
                    super::super::super::CopyRecord::Nothing,
                    None,
                    names,
                )
                .await
                .map(|()| MergeCopied::default())
            })
        },
    )
}

#[test]
fn a_merge_replaces_nothing_with_a_link_the_destination_cannot_hold() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (row, quirks, links) in [
        ("vfat", Quirks::vfat as Preset, false),
        ("ext4", Quirks::ext4, true),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(&source)?;
        fs::create_dir_all(&target)?;
        fs::write(source.join("a.txt"), b"a")?;
        std::os::unix::fs::symlink("a.txt", source.join("link"))?;
        fs::write(target.join("link"), b"earlier")?;
        let _target = FsQuirks::install(&target, quirks());
        let (stage, staged) = rename_stage(root.path().join("staging"));
        let (_plans, on_merged) = recorded_plans();

        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: fixing(),
                copy_into_target: super::super::super::merge_copy(None, fixing()),
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;

        assert_eq!(fs::read(target.join("a.txt"))?, b"a", "{row}");
        if links {
            assert_eq!(*staged.borrow(), [Location::local(target.join("link"))]);
            assert_eq!(fs::read_link(target.join("link"))?, Path::new("a.txt"));
        } else {
            assert_eq!(*staged.borrow(), Vec::<Location>::new());
            assert_eq!(fs::read(target.join("link"))?, b"earlier");
        }
    }
    Ok(())
}

#[test]
fn fat_merge_overwrites_the_names_it_plans_and_keeps_the_names_that_need_no_fix()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (row, copy) in [
        ("engine", super::super::super::merge_copy(None, fixing())),
        ("GIO", gio_merge_copy(fixing())),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(source.join("nested?"))?;
        fs::create_dir_all(target.join("nested_"))?;
        for (name, contents) in [
            ("a?.txt", "question"),
            ("a:.txt", "colon"),
            ("b:.txt", "b colon"),
            ("b_.txt", "b kept"),
            ("C.txt", "upper c"),
            ("c.txt", "lower c"),
        ] {
            fs::write(source.join("nested?").join(name), contents)?;
        }
        // `b_ (1).txt` is what an earlier merge of `b:.txt` saved.
        for (name, contents) in [
            ("a_.txt", "original a"),
            ("b_.txt", "original b"),
            ("b_ (1).txt", "earlier b colon"),
        ] {
            fs::write(target.join("nested_").join(name), contents)?;
        }
        let _sources = FsQuirks::install(&source, Quirks::default());
        let _target = FsQuirks::install(&target, Quirks::vfat());
        let staging = root.path().join("staging");
        let (stage, staged) = rename_stage(staging.clone());
        let (plans, on_merged) = recorded_plans();
        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: fixing(),
                copy_into_target: copy,
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;
        let nested = target.join("nested_");
        for (name, contents) in [
            ("a_.txt", "colon"),
            ("a_ (1).txt", "question"),
            ("b_.txt", "b kept"),
            ("b_ (1).txt", "b colon"),
            ("C.txt", "upper c"),
            ("c (1).txt", "lower c"),
        ] {
            assert_eq!(
                fs::read(nested.join(name))?,
                contents.as_bytes(),
                "{row}: {name}"
            );
        }
        assert_eq!(names(&nested)?.len(), 6, "{row}");
        let mut staged_contents = (0..staged.borrow().len())
            .map(|index| fs::read_to_string(staging.join(format!("staged-{index}"))))
            .collect::<io::Result<Vec<_>>>()?;
        staged_contents.sort();
        assert_eq!(
            staged_contents,
            ["earlier b colon", "original a", "original b"],
            "{row}"
        );
        let plans = plans.borrow();
        assert_eq!(
            plans[0].created.iter().collect::<HashSet<_>>(),
            ["a_ (1).txt", "C.txt", "c (1).txt"]
                .map(|name| Location::local(nested.join(name)))
                .iter()
                .collect(),
            "{row}"
        );
        assert_eq!(plans[0].overwritten, *staged.borrow(), "{row}");
        // A GIO copy proves no creation, so it records nothing for undo to delete.
        let (count, proven) = match row {
            "engine" => (2, plans[0].overwritten.iter().collect()),
            _ => (1, HashSet::new()),
        };
        assert_eq!(plans.len(), count, "{row}");
        assert_eq!(
            plans[1..]
                .iter()
                .flat_map(|plan| plan.placed.keys())
                .collect::<HashSet<_>>(),
            proven,
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn a_merge_off_fat_keeps_the_names_fat_would_fix() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (row, copy) in [
        ("engine", super::super::super::merge_copy(None, fixing())),
        ("GIO", gio_merge_copy(fixing())),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(source.join("nested"))?;
        fs::create_dir_all(target.join("nested"))?;
        for (name, contents) in [("a:b", "colon"), ("C.txt", "upper"), ("c.txt", "lower")] {
            fs::write(source.join("nested").join(name), contents)?;
        }
        fs::write(target.join("nested/a_b"), "original")?;
        let _target = FsQuirks::install(&target, Quirks::ext4());
        let (stage, staged) = rename_stage(root.path().join("staging"));
        let (plans, on_merged) = recorded_plans();

        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: fixing(),
                copy_into_target: copy,
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;

        let nested = target.join("nested");
        for (name, contents) in [
            ("a:b", "colon"),
            ("a_b", "original"),
            ("C.txt", "upper"),
            ("c.txt", "lower"),
        ] {
            assert_eq!(
                fs::read(nested.join(name))?,
                contents.as_bytes(),
                "{row}: {name}"
            );
        }
        assert!(staged.borrow().is_empty(), "{row}");
        assert_eq!(
            plans.borrow()[0].created.iter().collect::<HashSet<_>>(),
            ["a:b", "C.txt", "c.txt"]
                .map(|name| Location::local(nested.join(name)))
                .iter()
                .collect(),
            "{row}"
        );
    }
    Ok(())
}

fn no_plan(_: MergePlan) {}

#[test]
fn a_merge_onto_fat_takes_the_rules_of_a_folder_inside_it_only_from_another_device_mounted_there()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let copies: [(&str, MakeCopy); 2] = [
        ("engine", || super::super::super::merge_copy(None, fixing())),
        ("GIO", || gio_merge_copy(fixing())),
    ];
    for (copy_row, copy) in copies {
        // On the stick's device, the folder's ext4 rules are never looked up.
        for (own_device, disk_holds, overwritten) in [
            (true, &[("a:b", "colon"), ("a_b", "original")][..], None),
            (false, &[("a_b", "colon")][..], Some("a_b")),
        ] {
            let row = format!("{copy_row}, own device {own_device}");
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            let stick = root.path().join("stick");
            let disk = stick.join("disk");
            fs::create_dir_all(source.join("disk"))?;
            fs::create_dir_all(&disk)?;
            fs::write(source.join("x:y"), "top")?;
            fs::write(source.join("disk/a:b"), "colon")?;
            fs::write(disk.join("a_b"), "original")?;
            let _stick = FsQuirks::install(&stick, Quirks::vfat());
            let _disk = FsQuirks::install(
                &disk,
                Quirks {
                    own_device,
                    ..Quirks::ext4()
                },
            );
            let (stage, staged) = rename_stage(root.path().join("staging"));
            let (plans, on_merged) = recorded_plans();

            glib::MainContext::default().block_on(merge_local_with(
                gio::File::for_path(&source),
                gio::File::for_path(&stick),
                false,
                gio::Cancellable::new(),
                None,
                MergeHooks {
                    names: fixing(),
                    copy_into_target: copy(),
                    stage_overwrite: stage,
                    on_merged: &on_merged,
                    progress: None,
                },
            ))?;

            assert_eq!(fs::read(stick.join("x_y"))?, b"top", "{row}");
            assert_eq!(
                names(&disk)?,
                disk_holds.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                "{row}"
            );
            for (name, contents) in disk_holds {
                assert_eq!(fs::read(disk.join(name))?, contents.as_bytes(), "{row}");
            }
            let overwritten =
                Vec::from_iter(overwritten.map(|name| Location::local(disk.join(name))));
            assert_eq!(*staged.borrow(), overwritten, "{row}");
            let plan = &plans.borrow()[0];
            assert_eq!(plan.overwritten, overwritten, "{row}");
            let created = [stick.join("x_y")]
                .into_iter()
                .chain(own_device.then(|| disk.join("a:b")))
                .map(Location::local)
                .collect::<HashSet<_>>();
            assert_eq!(
                plan.created.iter().cloned().collect::<HashSet<_>>(),
                created,
                "{row}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_merge_within_one_disk_fixes_the_names_another_filesystem_mounted_in_its_source_brings()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let copies: [(&str, MakeCopy); 2] = [
        ("engine", || super::super::super::merge_copy(None, fixing())),
        ("GIO", || gio_merge_copy(fixing())),
    ];
    for (copy_row, copy) in copies {
        for (own_device, landed) in [(true, "a_b"), (false, "a:b")] {
            let row = format!("{copy_row}, own device {own_device}");
            let root = tempfile::tempdir()?;
            let stick = root.path().join("stick");
            let source = stick.join("a");
            let target = stick.join("b");
            fs::create_dir_all(source.join("inner"))?;
            fs::create_dir_all(target.join("inner"))?;
            fs::write(source.join("inner/a:b"), "colon")?;
            let _stick = FsQuirks::install(
                &stick,
                Quirks {
                    own_device: true,
                    ..Quirks::ntfs3()
                },
            );
            let _inner = FsQuirks::install(
                &source.join("inner"),
                Quirks {
                    own_device,
                    ..Quirks::ext4()
                },
            );
            let (stage, _) = rename_stage(root.path().join("staging"));
            let (plans, on_merged) = recorded_plans();

            glib::MainContext::default().block_on(merge_local_with(
                gio::File::for_path(&source),
                gio::File::for_path(&target),
                false,
                gio::Cancellable::new(),
                None,
                MergeHooks {
                    names: fixing(),
                    copy_into_target: copy(),
                    stage_overwrite: stage,
                    on_merged: &on_merged,
                    progress: None,
                },
            ))?;

            assert_eq!(names(&target.join("inner"))?, [landed], "{row}");
            assert_eq!(
                plans.borrow()[0].created,
                [Location::local(target.join("inner").join(landed))],
                "{row}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_merge_records_the_names_its_copy_gives_a_cut_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (row, copy) in [
        ("engine", super::super::super::merge_copy(None, fixing())),
        ("GIO", gio_merge_copy(fixing())),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let disk = root.path().join("disk");
        fs::create_dir_all(&source)?;
        fs::create_dir(&disk)?;
        // Cut to 64 bytes, the long name differs from the short one in case only.
        fs::write(source.join(format!("A{}", "b".repeat(70))), "long")?;
        fs::write(source.join(format!("a{}", "b".repeat(63))), "short")?;
        let _disk = FsQuirks::install(
            &disk,
            Quirks {
                name_max: Some(64),
                ..Quirks::ext4()
            },
        );
        let (stage, _) = rename_stage(root.path().join("staging"));
        let (plans, on_merged) = recorded_plans();

        glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&disk),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: fixing(),
                copy_into_target: copy,
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ))?;

        let made = names(&disk)?
            .into_iter()
            .map(|name| Location::local(disk.join(name)))
            .collect::<HashSet<_>>();
        assert_eq!(made.len(), 2, "{row}");
        assert_eq!(
            plans.borrow()[0]
                .created
                .iter()
                .cloned()
                .collect::<HashSet<_>>(),
            made,
            "{row}"
        );
    }
    Ok(())
}

#[test]
fn merging_folders_unites_contents_and_overwrites_shared_names() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(source.join("shared.txt"), b"incoming wins")?;
    fs::write(target.join("stays.txt"), b"keep me")?;
    fs::write(target.join("shared.txt"), b"old")?;
    let staging = root.path().join("staging");
    let (stage, staged) = rename_stage(staging.clone());
    let (plans, on_merged) = recorded_plans();

    let result = glib::MainContext::default().block_on(merge_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target: real_copy(),
            stage_overwrite: stage,
            on_merged: &on_merged,
            progress: None,
        },
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("incoming.txt"))?, b"new");
    assert_eq!(fs::read(target.join("shared.txt"))?, b"incoming wins");
    assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    assert!(source.exists(), "a copy merge keeps the source");
    assert_eq!(
        staged.borrow().as_slice(),
        &[Location::local(target.join("shared.txt"))],
        "the overwritten original is staged before the copy"
    );
    assert_eq!(fs::read(staging.join("staged-0"))?, b"old");
    let plans = plans.borrow();
    assert_eq!(plans.len(), 2);
    assert_eq!(
        plans[0].created,
        vec![Location::local(target.join("incoming.txt"))]
    );
    assert_eq!(
        plans[0].overwritten,
        vec![Location::local(target.join("shared.txt"))]
    );
    assert_eq!(
        placed_paths(&plans[1].placed),
        [target.join("shared.txt")],
        "undo trashes what was created, so only what the copy put over an original is recorded"
    );
    Ok(())
}

#[test]
fn merging_recurses_into_subdirectories_shared_by_both_sides() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir_all(target.join("nested"))?;
    fs::write(source.join("nested/incoming.txt"), b"new")?;
    fs::write(source.join("nested/shared.txt"), b"incoming wins")?;
    fs::write(target.join("nested/stays.txt"), b"keep me")?;
    fs::write(target.join("nested/shared.txt"), b"old")?;
    let (stage, _) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();
    let listed_on = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listing = |folder: &'static str| -> (Op, &'static str, usize, Racer) {
        let listed_on = listed_on.clone();
        let racer: Racer = Box::new(move || {
            listed_on
                .lock()
                .expect("listings")
                .push((folder, thread::current().id()));
        });
        (Op::ListChunks, folder, 0, racer)
    };
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![listing("source"), listing("nested")],
            ..Quirks::default()
        },
    );

    let result = glib::MainContext::default().block_on(merge_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target: real_copy(),
            stage_overwrite: stage,
            on_merged: &on_merged,
            progress: None,
        },
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("nested/incoming.txt"))?, b"new");
    assert_eq!(
        fs::read(target.join("nested/shared.txt"))?,
        b"incoming wins"
    );
    assert_eq!(fs::read(target.join("nested/stays.txt"))?, b"keep me");
    let ui = thread::current().id();
    let listed_on = listed_on.lock().expect("listings");
    assert_eq!(
        listed_on
            .iter()
            .map(|(folder, _)| *folder)
            .collect::<Vec<_>>(),
        ["source", "nested"],
        "each local source folder is listed through the filesystem seam"
    );
    assert!(
        listed_on.iter().all(|(_, thread)| *thread != ui),
        "and never on the UI thread"
    );
    let plans = plans.borrow();
    assert_eq!(plans.len(), 2);
    assert_eq!(
        plans[0].created,
        vec![Location::local(target.join("nested/incoming.txt"))]
    );
    assert_eq!(
        plans[0].overwritten,
        vec![Location::local(target.join("nested/shared.txt"))]
    );
    assert_eq!(
        placed_paths(&plans[1].placed),
        [target.join("nested/shared.txt")],
        "undo trashes what was created, so only what the copy put over an original is recorded"
    );
    Ok(())
}

#[test]
fn a_merge_from_a_location_without_paths_does_not_claim_provenance_for_undo()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    gio::resources_register_include!("strata.gresource")?;
    let source = gio::File::for_uri("resource:///io/github/lgse/Strata/brand");
    assert!(!source.is_native());
    let root = tempfile::tempdir()?;
    let target = root.path().join("brand");
    let overwritten = target.join("strata-logo-white.svg");
    write_files(
        &target,
        &[("strata-logo-white.svg", "old"), ("stays.txt", "keep me")],
    )?;
    let (stage, _) = rename_stage(root.path().join("staging"));
    let (plans, on_merged) = recorded_plans();

    let result = glib::MainContext::default().block_on(merge_local_with(
        source,
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target: real_copy(),
            stage_overwrite: stage,
            on_merged: &on_merged,
            progress: None,
        },
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_ne!(fs::read(&overwritten)?, b"old");
    let (_, staged, placed) = undo_parts(plans.take());
    assert_eq!(staged, [Location::local(&overwritten)]);
    assert!(placed.is_empty());
    Ok(())
}

#[test]
fn a_merge_from_a_location_without_paths_keeps_unproven_copies_during_no_trash_undo()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    gio::resources_register_include!("strata.gresource")?;
    let source = gio::File::for_uri("resource:///io/github/lgse/Strata/icons/scalable");
    let root = tempfile::tempdir()?;
    let target = root.path().join("scalable");
    write_files(&target, &[("stays.txt", "keep me")])?;
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));
    let (plans, on_merged) = recorded_plans();

    let result = glib::MainContext::default().block_on(merge_local(
        source,
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &on_merged,
    ));

    assert!(result.is_ok(), "{result:?}");
    let [actions, apps] = ["actions", "apps"].map(|name| target.join(name));
    let parts = undo_parts(plans.take());
    assert!(parts.2.is_empty());
    let edited = fs::read_dir(&apps)?.next().ok_or("an app icon")??.path();
    fs::write(&edited, b"edited after the merge")?;

    let events = run_undo(parts, staged_lookup(HashMap::new()));

    assert!(actions.exists(), "{events:?}");
    assert_eq!(fs::read(&edited)?, b"edited after the merge");
    assert_eq!(names(&target)?, ["actions", "apps", "stays.txt"]);
    let mut kept = kept_items(&events);
    kept.sort_by_key(Location::display_name);
    assert_eq!(kept, [Location::local(&actions), Location::local(&apps)]);
    Ok(())
}

#[test]
fn a_new_subdirectory_is_reported_as_a_single_created_entry() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("sub/deep"))?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("sub/deep/incoming.txt"), b"new")?;
    let _scope = FsQuirks::install(root.path(), without_trash(Quirks::default()));
    let (plans, on_merged) = recorded_plans();

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &on_merged,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("sub/deep/incoming.txt"))?, b"new");
    let plans = plans.borrow();
    assert_eq!(plans.len(), 2);
    assert_eq!(
        plans[0].created,
        vec![Location::local(target.join("sub"))],
        "undo removes the topmost created directory, not each nested child"
    );
    assert!(plans[0].overwritten.is_empty());
    assert_eq!(
        placed_paths(&plans[1].placed),
        [target.join("sub")],
        "without Trash, it is recorded whole"
    );
    Ok(())
}

#[test]
fn a_merge_stage_goes_once_another_program_closes_a_file_it_kept_there()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("new.txt"), b"new")?;
    let holds = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (held, live) = (holds.clone(), target.clone());
    let hold_staged: Racer = Box::new(move || {
        for stage in fs::read_dir(&live).expect("list the target").flatten() {
            if stage
                .file_name()
                .to_string_lossy()
                .starts_with(".strata-copy-")
            {
                let staged = hold(&stage.path().join("new.txt"));
                held.lock().expect("the holds").push(staged);
            }
        }
    });
    let release: Racer = Box::new(move || holds.lock().expect("the holds").clear());
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(Quirks {
            before: vec![(Op::Unlink, "new.txt", 0, hold_staged)],
            after: vec![(Op::Names, ".strata-copy-*", 0, release)],
            links: Support::Rejected(Errno::MLINK),
            rename_over_empty_dir: OverEmptyDirectory::NotEmpty,
            ..Quirks::nfs3()
        }),
    );
    let (_plans, on_merged) = recorded_plans();

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &on_merged,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(names(&target)?, ["new.txt"]);
    assert_eq!(fs::read(target.join("new.txt"))?, b"new");
    Ok(())
}

#[test]
fn a_staging_failure_aborts_the_merge_and_reports_only_what_was_staged()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (code, expected_message) in [
        (
            gio::IOErrorEnum::Failed,
            "Could not move b.txt to Trash before merging: injected staging failure",
        ),
        (
            gio::IOErrorEnum::ReadOnly,
            "Could not move b.txt to Trash before merging: The file system is read-only",
        ),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(&source)?;
        fs::create_dir_all(&target)?;
        fs::write(source.join("a.txt"), b"new a")?;
        fs::write(source.join("b.txt"), b"new b")?;
        fs::write(target.join("a.txt"), b"old a")?;
        fs::write(target.join("b.txt"), b"old b")?;
        let staging = root.path().join("staging");
        fs::create_dir_all(&staging)?;
        let staging_into = staging.clone();
        let (plans, on_merged) = recorded_plans();
        let stage: StageOverwrite = Rc::new(move |location: Location, _cancellable| {
            let staging = staging_into.clone();
            Box::pin(async move {
                if location.display_name() == "b.txt" {
                    return Err(glib::Error::new(code, "injected staging failure").into());
                }
                let path = location
                    .native_path()
                    .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::Failed, "no path"))?;
                std::fs::rename(path, staging.join("staged")).map_err(|error| {
                    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
                })?;
                Ok(None)
            })
        });

        let result = glib::MainContext::default().block_on(merge_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            MergeHooks {
                names: NamePolicy::default(),
                copy_into_target: real_copy(),
                stage_overwrite: stage,
                on_merged: &on_merged,
                progress: None,
            },
        ));

        let error = result.expect_err("staging failure aborts the merge");
        assert_eq!(error.message(), expected_message, "{code:?}");
        assert_eq!(fs::read(source.join("a.txt"))?, b"new a");
        assert_eq!(fs::read(source.join("b.txt"))?, b"new b");
        assert_eq!(fs::read(target.join("b.txt"))?, b"old b", "{code:?}");
        let staged = fs::exists(staging.join("staged"))?;
        assert_eq!(
            !target.join("a.txt").exists(),
            staged,
            "enumeration order is unspecified: a.txt is staged only if it came first"
        );
        if staged {
            assert_eq!(fs::read(staging.join("staged"))?, b"old a");
        }
        let plans = plans.borrow();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].created.is_empty());
        assert_eq!(
            plans[0].overwritten.len(),
            usize::from(staged),
            "only the staged original"
        );
    }
    Ok(())
}

#[test]
fn merge_places_a_read_only_incoming_folder() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("incoming"))?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming/file.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;
    let incoming = source.join("incoming");
    fs::set_permissions(&incoming, fs::Permissions::from_mode(0o555))?;

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

    let placed = fs::metadata(target.join("incoming")).map(|metadata| metadata.mode() & 0o7777);
    for folder in [&incoming, &target.join("incoming")] {
        if folder.is_dir() {
            fs::set_permissions(folder, fs::Permissions::from_mode(0o755))?;
        }
    }
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(placed?, 0o555);
    assert_eq!(fs::read(target.join("incoming/file.txt"))?, b"new");
    assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    Ok(())
}

#[test]
fn undoing_a_merge_without_trash_opens_only_its_own_read_only_folders_to_remove_them()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;
    #[derive(Clone, Copy, Debug)]
    enum Undo {
        Whole,
        Refilled,
        Cancelled,
        IntoLockedTarget,
    }
    let _serial = serial()?;
    let mode = |path: &Path| {
        fs::metadata(path)
            .ok()
            .map(|metadata| metadata.mode() & 0o7777)
    };
    let set_mode = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode));
    for undo in [
        Undo::Whole,
        Undo::Refilled,
        Undo::Cancelled,
        Undo::IntoLockedTarget,
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(source.join("incoming/inner"))?;
        fs::create_dir_all(&target)?;
        fs::write(source.join("incoming/inner/file.txt"), b"new")?;
        fs::write(target.join("stays.txt"), b"keep me")?;
        for folder in ["incoming/inner", "incoming"] {
            set_mode(&source.join(folder), 0o555)?;
        }
        let merging = FsQuirks::install(root.path(), without_trash(Quirks::default()));
        let (plans, on_merged) = recorded_plans();
        let merged = glib::MainContext::default().block_on(merge_local(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            &on_merged,
        ));
        drop(merging);
        let (incoming, inner) = (target.join("incoming"), target.join("incoming/inner"));
        let cancellable = gio::Cancellable::new();
        let (refill, cancel) = (inner.join("theirs.txt"), cancellable.clone());
        let racer: Racer = Box::new(move || match undo {
            Undo::Refilled => drop(fs::write(&refill, b"theirs")),
            _ => cancel.cancel(),
        });
        let mut quirks = without_trash(Quirks::default());
        if let Undo::Refilled | Undo::Cancelled = undo {
            quirks.before = vec![(Op::Unlink, "file.txt", 0, racer)];
        }
        let _undoing = FsQuirks::install(root.path(), quirks);
        if let Undo::IntoLockedTarget = undo {
            set_mode(&target, 0o555)?;
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        glib::MainContext::default().block_on(run_merge_undo(
            OperationRequestId(93),
            undo_parts(plans.take()),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            cancellable,
            staged_lookup(HashMap::new()),
        ));

        let modes = [&target, &incoming, &inner].map(|folder| mode(folder));
        let left = [&incoming, &inner].map(|folder| names(folder).ok());
        for folder in [&source.join("incoming"), &source.join("incoming/inner")] {
            set_mode(folder, 0o755)?;
        }
        for folder in [&target, &incoming, &inner] {
            if folder.is_dir() {
                set_mode(folder, 0o755)?;
            }
        }
        let row = format!("{undo:?}: {:?}", events.borrow());
        assert!(merged.is_ok(), "{row}: {merged:?}");
        let theirs = || Some(vec!["theirs.txt".to_owned()]);
        let (expected_modes, expected_left) = match undo {
            Undo::Whole => ([Some(0o755), None, None], [None, None]),
            Undo::Refilled => (
                [Some(0o755), Some(0o555), Some(0o555)],
                [Some(vec!["inner".to_owned()]), theirs()],
            ),
            Undo::Cancelled => (
                [Some(0o755), Some(0o555), Some(0o555)],
                [Some(vec!["inner".to_owned()]), Some(Vec::new())],
            ),
            Undo::IntoLockedTarget => ([Some(0o555), Some(0o555), None], [Some(Vec::new()), None]),
        };
        assert_eq!(modes, expected_modes, "{row}");
        assert_eq!(left, expected_left, "{row}");
        if let Undo::Whole = undo {
            assert!(restored(&events.borrow()).is_some(), "{row}");
        }
        assert!(names(&target)?.contains(&"stays.txt".to_owned()), "{row}");
    }
    Ok(())
}

fn visible_names(folder: &Path) -> io::Result<Vec<String>> {
    Ok(names(folder)?
        .into_iter()
        .filter(|name| !name.starts_with(".strata-"))
        .collect())
}

#[test]
fn a_merge_keeps_hard_links_only_within_one_merge_stage() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    use std::os::unix::fs::MetadataExt;
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
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (live, seen_at_link) = (target.clone(), seen.clone());
    let racer: Racer = Box::new(move || {
        let names = visible_names(&live).expect("list the merged folder");
        seen_at_link.lock().expect("record").extend(names);
    });
    let scope = FsQuirks::install(
        &target,
        Quirks {
            before: vec![(Op::Link, "*", 0, racer)],
            ..Quirks::default()
        },
    );
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.hard_links = Some(true);
    });

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

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
    assert_eq!(a.1, 3);
    assert_ne!(b.0, a.0);
    assert_eq!(b.1, 1);
    assert_eq!(fs::read(target.join("sub/b"))?, b"linked");
    Ok(())
}

#[test]
fn a_merged_copy_replaced_before_linking_is_not_linked() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    fs::create_dir(&target)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("a2"))?;
    let live = target.clone();
    let racer: Racer = Box::new(move || {
        let stage = names(&live)
            .expect("list the merged folder")
            .into_iter()
            .find(|name| name.starts_with(".strata-copy-"))
            .map(|name| live.join(name));
        for copied in stage
            .iter()
            .flat_map(|stage| [stage.join("a"), stage.join("a2")])
        {
            if copied.exists() {
                let other = live.join("other");
                fs::write(&other, b"other").expect("write the other item");
                fs::rename(&other, &copied).expect("replace the copy");
            }
        }
    });
    let scope = FsQuirks::install(
        &target,
        Quirks {
            before: vec![(Op::Link, "*", 0, racer)],
            ..Quirks::default()
        },
    );
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.hard_links = Some(true);
    });

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

    assert!(result.is_err(), "{result:?}");
    assert_eq!(names(&target)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn merge_never_overwrites_an_entry_created_during_the_merge() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    use crate::services::RenameCause;
    let _serial = serial()?;
    type Renamed = &'static [(&'static str, &'static str, RenameCause)];
    let rows: [(&str, &str, Preset, Renamed); 2] = [
        (
            "notes.txt",
            "notes (1).txt",
            Quirks::default,
            &[("notes.txt", "notes (1).txt", RenameCause::Taken)],
        ),
        (
            "photos",
            "photos (1)",
            Quirks::vfat,
            &[
                ("photos", "photos (1)", RenameCause::Taken),
                (
                    "photos (1)/makefile",
                    "photos (1)/makefile (1)",
                    RenameCause::Alias,
                ),
            ],
        ),
    ];
    for (taken, saved_as, preset, reported) in rows {
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
            assert_eq!(fs::read(ours.join("Makefile"))?, b"Makefile");
            assert_eq!(fs::read(ours.join("makefile (1)"))?, b"makefile");
        } else {
            assert_eq!(fs::read(ours)?, b"incoming");
        }
        let reported = reported
            .iter()
            .map(|(requested, saved_as, cause)| {
                (requested.to_string(), saved_as.to_string(), *cause)
            })
            .collect::<Vec<_>>();
        assert_eq!(renamed_beneath(&target, &report), reported, "{taken}");
    }
    Ok(())
}

#[test]
fn an_item_put_at_a_placed_name_right_after_placing_stays_there() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    let _serial = serial()?;
    // A program that saves by renaming over the name does so as soon as the
    // child is placed: by a rename, or by a link and the stage's unlink.
    fn unstable_inodes() -> Quirks {
        Quirks {
            rename_flags: super::super::fs_quirks::Support::Supported,
            ..Quirks::sshfs()
        }
    }
    let rows: [(&str, Preset, Op, Option<&str>); 9] = [
        ("rename", Quirks::default, Op::Rename, None),
        ("link", Quirks::without_rename_flags, Op::Unlink, None),
        (
            "unstable inode without birth time",
            unstable_inodes,
            Op::Rename,
            None,
        ),
        (
            "replaced descendant",
            Quirks::default,
            Op::Rename,
            Some("original"),
        ),
        (
            "new descendant",
            Quirks::default,
            Op::Rename,
            Some("arrival"),
        ),
        ("edited file", Quirks::default, Op::Rename, None),
        (
            "edited descendant",
            Quirks::default,
            Op::Rename,
            Some("original"),
        ),
        ("edited same-size file", Quirks::default, Op::Rename, None),
        (
            "edited same-size descendant",
            Quirks::default,
            Op::Rename,
            Some("original"),
        ),
    ];
    for (row, preset, placed, descendant) in rows {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source)?;
        fs::create_dir(&target)?;
        if descendant.is_some() {
            fs::create_dir(source.join("a.txt"))?;
            fs::write(source.join("a.txt/original"), b"incoming")?;
        } else {
            fs::write(source.join("a.txt"), b"incoming")?;
        }
        let saved = root.path().join("saved.txt");
        let same_size = row.starts_with("edited same-size");
        let contents: &'static [u8] = if same_size {
            b"modified"
        } else {
            b"saved by another program"
        };
        fs::write(&saved, contents)?;
        let over = descendant.map_or_else(
            || target.join("a.txt"),
            |child| target.join("a.txt").join(child),
        );
        let in_place = row.starts_with("edited");
        let racer: Racer = Box::new(move || {
            if in_place {
                fs::write(&over, contents).expect("edit the placed entry");
                if same_size {
                    fs::File::open(&over)
                        .expect("edited file")
                        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1))
                        .expect("detectable edit time");
                }
            } else {
                fs::rename(&saved, &over).expect("rename over the placed name");
            }
        });
        let _scope = FsQuirks::install(
            &target,
            without_trash(Quirks {
                after: vec![(placed, "a.txt", 0, racer)],
                ..preset()
            }),
        );
        let (plans, on_merged) = recorded_plans();
        let merged = glib::MainContext::default().block_on(merge_local(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            &on_merged,
        ));
        assert!(merged.is_ok(), "{row}: {merged:?}");
        let events = run_undo(undo_parts(plans.take()), staged_lookup(HashMap::new()));
        assert!(
            restored(&events).is_none_or(|items| items.is_empty()),
            "{row}: {events:?}"
        );
        let surviving = descendant.map_or_else(
            || target.join("a.txt"),
            |child| target.join("a.txt").join(child),
        );
        assert_eq!(fs::read(surviving)?, contents, "{row}");
        assert_eq!(names(&target)?, ["a.txt"], "{row}");
    }
    Ok(())
}

#[test]
fn a_stopped_merge_reports_only_what_it_placed() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    use crate::services::RenameCause;
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
    assert_eq!(fs::read(target.join("a (1).txt"))?, b"incoming");
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
    let racer: super::super::fs_quirks::Racer = Box::new(move || {
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

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

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
    use crate::services::RenameCause;
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
    assert_eq!(fs::read(target.join("new"))?, b"taken");
    assert_eq!(
        names(&target.join("new (1)"))?,
        ["Makefile (1)", "makefile"]
    );
    assert_eq!(fs::read(target.join("new (1)/Makefile (1)"))?, b"Makefile");
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

#[test]
fn a_merged_item_is_invisible_until_complete() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
    let _serial = serial()?;
    // Placed by rename, by the rungs that cannot replace, and copied out of
    // the stage where nothing can place it.
    let presets: [Preset; 3] = [Quirks::default, Quirks::vboxsf, Quirks::without_placement];
    for (row, preset) in presets.into_iter().enumerate() {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        let source = sources.join("folder");
        let target = destination.join("folder");
        fs::create_dir_all(source.join("new"))?;
        fs::create_dir_all(&target)?;
        fs::write(source.join("incoming.bin"), vec![0x5a; 256 * 1024])?;
        fs::write(source.join("new/inner.bin"), vec![0xa5; 256 * 1024])?;
        fs::write(target.join("stays.txt"), b"keep me")?;
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (live, seen_at_write) = (target.clone(), seen.clone());
        let racer: Racer = Box::new(move || {
            let names = visible_names(&live).expect("list the merged folder");
            seen_at_write.lock().expect("record").extend(names);
        });
        // Separate mounts, so the data is written rather than cloned.
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before: vec![(Op::Write, "*", 0, racer)],
                ..preset()
            },
        );

        let result = glib::MainContext::default().block_on(merge_local(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            &no_plan,
        ));

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
        assert_eq!(
            fs::read(target.join("incoming.bin"))?,
            vec![0x5a; 256 * 1024]
        );
        assert_eq!(
            fs::read(target.join("new/inner.bin"))?,
            vec![0xa5; 256 * 1024]
        );
        assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    }
    Ok(())
}

#[test]
fn merge_does_not_follow_a_target_folder_swapped_for_a_link() -> Result<(), Box<dyn Error>> {
    use super::super::fs_quirks::Racer;
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

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

    assert!(result.is_err(), "{result:?}");
    assert_eq!(names(&outside)?, Vec::<String>::new());
    assert_eq!(names(&target.join("moved"))?, ["old.txt"]);
    Ok(())
}

#[test]
fn a_merge_move_removes_the_emptied_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        &no_plan,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("incoming.txt"))?, b"new");
    assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn merging_a_file_is_rejected_without_touching_either_side() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

    assert!(result.is_err());
    assert_eq!(fs::read(&source)?, b"new");
    assert_eq!(fs::read(&target)?, b"old");
    Ok(())
}

#[test]
fn merging_onto_a_file_is_rejected_without_touching_either_side() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(&target, b"old")?;

    let result = glib::MainContext::default().block_on(merge_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        &no_plan,
    ));

    assert!(result.is_err());
    assert!(source.join("incoming.txt").exists());
    assert_eq!(fs::read(&target)?, b"old");
    Ok(())
}

#[test]
fn a_failed_merge_preserves_the_destinations_existing_contents() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;

    let result = glib::MainContext::default().block_on(merge_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target: Rc::new(|_, target, _, _| {
                Box::pin(async move {
                    fs::write(
                        target
                            .path()
                            .ok_or_else(|| super::super::io_error("missing target path"))?
                            .join("partial.txt"),
                        b"partial",
                    )
                    .map_err(super::super::io_error)?;
                    Err(glib::Error::new(
                        gio::IOErrorEnum::Cancelled,
                        "injected cancellation",
                    ))
                })
            }),
            stage_overwrite: Rc::new(trash_stage_overwrite),
            on_merged: &no_plan,
            progress: None,
        },
    ));

    assert!(result.is_err());
    assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    assert!(source.join("incoming.txt").exists());
    Ok(())
}

#[test]
fn a_merged_paste_reports_no_created_location() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(sources.join("folder"))?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(sources.join("folder/incoming.txt"), b"new")?;
    fs::write(destination.join("folder/stays.txt"), b"keep me")?;

    // No created location: copy undo trashes recorded locations, and the
    // merged destination held pre-existing contents.
    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(81),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(sources.join("folder")),
            conflict: TransferConflict::Merge,
            target_name: None,
        }],
        move_sources: false,
    })?;

    assert!(created.into_iter().flatten().next().is_none());
    assert_eq!(fs::read(destination.join("folder/incoming.txt"))?, b"new");
    assert_eq!(fs::read(destination.join("folder/stays.txt"))?, b"keep me");
    Ok(())
}
