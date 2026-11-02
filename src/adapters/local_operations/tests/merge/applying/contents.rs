// SPDX-License-Identifier: MIT

use super::*;

/// The merge recorded two plans: the first created `created` and overwrote
/// `overwritten`, the second holds only what the copy put over it.
fn assert_overwrite_plans(plans: &[MergePlan], created: &Path, overwritten: &Path) {
    assert_eq!(plans.len(), 2);
    assert_eq!(
        (&plans[0].created, &plans[0].overwritten),
        (
            &vec![Location::local(created)],
            &vec![Location::local(overwritten)]
        )
    );
    assert_eq!(
        placed_paths(&plans[1].placed),
        [overwritten],
        "undo trashes what was created, so only what the copy put over an original is recorded"
    );
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

    let result = merge_staged(&source, &target, stage, &on_merged);

    assert!(result.is_ok(), "{result:?}");
    assert_files(
        &target,
        &[
            ("incoming.txt", "new"),
            ("shared.txt", "incoming wins"),
            ("stays.txt", "keep me"),
        ],
        "",
    )?;
    assert!(source.exists(), "a copy merge keeps the source");
    assert_eq!(
        staged.borrow().as_slice(),
        &[Location::local(target.join("shared.txt"))],
        "the overwritten original is staged before the copy"
    );
    assert_eq!(fs::read(staging.join("staged-0"))?, b"old");
    assert_overwrite_plans(
        &plans.borrow(),
        &target.join("incoming.txt"),
        &target.join("shared.txt"),
    );
    Ok(())
}

type Listings = Arc<std::sync::Mutex<Vec<(&'static str, thread::ThreadId)>>>;

/// Records the thread each `folder` is listed on.
fn listing(listed_on: &Listings, folder: &'static str) -> (Op, &'static str, usize, Racer) {
    let listed_on = listed_on.clone();
    let racer: Racer = Box::new(move || {
        listed_on
            .lock()
            .expect("listings")
            .push((folder, thread::current().id()));
    });
    (Op::ListChunks, folder, 0, racer)
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
    let listed_on = Listings::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![listing(&listed_on, "source"), listing(&listed_on, "nested")],
            ..Quirks::default()
        },
    );

    let result = merge_staged(&source, &target, stage, &on_merged);

    assert!(result.is_ok(), "{result:?}");
    assert_files(
        &target.join("nested"),
        &[
            ("incoming.txt", "new"),
            ("shared.txt", "incoming wins"),
            ("stays.txt", "keep me"),
        ],
        "",
    )?;
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
    assert_overwrite_plans(
        &plans.borrow(),
        &target.join("nested/incoming.txt"),
        &target.join("nested/shared.txt"),
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

    let hooks = hooks(NamePolicy::default(), real_copy(), stage, &on_merged);
    let result = merge_hooked(source, &target, hooks);

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

    let result = merge_file_into(source, &target, &on_merged);

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

    let result = merge_into(&source, &target, &on_merged);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("sub/deep/incoming.txt"))?, b"new");
    let plans = plans.borrow();
    assert_eq!(plans.len(), 2);
    assert_eq!(
        (&plans[0].created, plans[0].overwritten.is_empty()),
        (&vec![Location::local(target.join("sub"))], true),
        "undo removes the topmost created directory, not each nested child"
    );
    assert_eq!(
        placed_paths(&plans[1].placed),
        [target.join("sub")],
        "without Trash, it is recorded whole"
    );
    Ok(())
}

/// Before `new.txt` is unlinked, holds it open in each copy stage of `live`
/// until `release` runs after the stage is listed.
fn holding_staged_copies(live: PathBuf) -> (Racer, Racer) {
    let holds = Arc::new(std::sync::Mutex::new(Vec::new()));
    let held = holds.clone();
    let hold_staged: Racer = Box::new(move || {
        let stages = fs::read_dir(&live).expect("list the target").flatten();
        for stage in stages.filter(|stage| {
            stage
                .file_name()
                .to_string_lossy()
                .starts_with(".strata-copy-")
        }) {
            let staged = hold(&stage.path().join("new.txt"));
            held.lock().expect("the holds").push(staged);
        }
    });
    let release: Racer = Box::new(move || holds.lock().expect("the holds").clear());
    (hold_staged, release)
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
    let (hold_staged, release) = holding_staged_copies(target.clone());
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

    let result = merge_into(&source, &target, &on_merged);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(names(&target)?, ["new.txt"]);
    assert_eq!(fs::read(target.join("new.txt"))?, b"new");
    Ok(())
}

/// Stages by renaming into `staging`, failing with `code` for `b.txt`.
fn stage_failing_on_b(staging: PathBuf, code: gio::IOErrorEnum) -> StageOverwrite {
    Rc::new(move |location: Location, _cancellable| {
        let staging = staging.clone();
        Box::pin(async move {
            if location.display_name() == "b.txt" {
                return Err(glib::Error::new(code, "injected staging failure").into());
            }
            let path = location
                .native_path()
                .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::Failed, "no path"))?;
            std::fs::rename(path, staging.join("staged"))
                .map_err(|error| glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string()))?;
            Ok(None)
        })
    })
}

fn check_staging_failure(
    code: gio::IOErrorEnum,
    expected_message: &str,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    write_files(&source, &[("a.txt", "new a"), ("b.txt", "new b")])?;
    write_files(&target, &[("a.txt", "old a"), ("b.txt", "old b")])?;
    let staging = root.path().join("staging");
    fs::create_dir_all(&staging)?;
    let (plans, on_merged) = recorded_plans();
    let stage = stage_failing_on_b(staging.clone(), code);

    let result = merge_staged(&source, &target, stage, &on_merged);

    let error = result.expect_err("staging failure aborts the merge");
    let row = format!("{code:?}");
    assert_eq!(error.message(), expected_message, "{row}");
    assert_files(&source, &[("a.txt", "new a"), ("b.txt", "new b")], "")?;
    assert_eq!(fs::read(target.join("b.txt"))?, b"old b", "{row}");
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
        check_staging_failure(code, expected_message)?;
    }
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
    assert_files(
        &target,
        &[("incoming.txt", "new"), ("stays.txt", "keep me")],
        "",
    )?;
    assert!(!source.exists());
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
