// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn nested_copy_shares_one_worker_budget_across_subdirectories() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    for folder in 0..9 {
        let directory = source.join(folder.to_string());
        fs::create_dir_all(&directory)?;
        for file in 0..9 {
            fs::write(directory.join(file.to_string()), format!("{folder}/{file}"))?;
        }
    }
    COPY_ACTIVITY.with(|activity| activity.set((0, 0)));
    glib::MainContext::default().block_on(super::super::copy_recursively_local(
        Arc::new(open_local_parent_directory(root.path())?),
        OsString::from("source"),
        gio::File::for_path(&target),
        super::super::CopyOptions {
            overwrite_existing: false,
            fat_family: false,
            workers: 2,
        },
        gio::Cancellable::new(),
        None,
        None,
    ))?;
    let (active, peak) = COPY_ACTIVITY.with(Cell::get);
    assert_eq!(active, 0, "all file copies must settle before completion");
    assert!(
        peak <= 2,
        "nested directories exceeded the copy budget: {peak}"
    );
    for folder in 0..9 {
        for file in 0..9 {
            assert_eq!(
                fs::read_to_string(target.join(format!("{folder}/{file}")))?,
                format!("{folder}/{file}")
            );
        }
    }
    Ok(())
}

#[test]
fn recursive_copy_preserves_nested_directory_contents() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-transfer-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");

    fs::write(source.join("top.txt"), b"replacement")?;
    let overwrite = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
    ));
    assert!(overwrite.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"replacement");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn copy_recursively_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-symlink-test-{unique}"));
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not copy me")?;
    fs::write(source.join("nested/visible.txt"), b"contents")?;
    std::os::unix::fs::symlink(&outside, source.join("nested/decoy"))?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("nested/visible.txt"))?, b"contents");
    let decoy_dest = target.join("nested/decoy");
    let decoy_metadata = fs::symlink_metadata(&decoy_dest)?;
    assert!(
        decoy_metadata.file_type().is_symlink(),
        "the decoy must be copied as a symlink, not followed into a real directory"
    );
    assert_eq!(fs::read_link(&decoy_dest)?, outside);
    // `is_symlink` above already rules out a real directory of copied
    // content existing under this name; confirm the thing it still points
    // at (unavoidably reachable by following the recreated symlink, same as
    // the original) was left untouched rather than overwritten.
    assert_eq!(fs::read(outside.join("secret.txt"))?, b"do not copy me");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn copy_recursively_of_a_symlink_creates_a_symlink_not_a_recursive_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-symlink-top-test-{unique}"));
    let outside = root.join("outside");
    let decoy = root.join("decoy");
    let target = root.join("target-link");
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not copy me")?;
    std::os::unix::fs::symlink(&outside, &decoy)?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&decoy),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    let target_metadata = fs::symlink_metadata(&target)?;
    assert!(
        target_metadata.file_type().is_symlink(),
        "copying a symlink must produce a symlink, not a recursive copy of its target"
    );
    assert_eq!(fs::read_link(&target)?, outside);
    assert_eq!(fs::read(outside.join("secret.txt"))?, b"do not copy me");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn copy_accepts_a_symlink_higher_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_root = root.path().join("actual");
    let linked_root = root.path().join("linked");
    fs::create_dir_all(actual_root.join("subdir"))?;
    fs::write(actual_root.join("subdir/source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_root, &linked_root)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(linked_root.join("subdir/source.txt")),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target)?, b"keep");
    assert_eq!(fs::read(actual_root.join("subdir/source.txt"))?, b"keep");
    Ok(())
}

#[test]
fn cancelling_staged_remote_file_copy_removes_only_the_incomplete_stage()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, stage, _| {
            Box::pin(async move {
                fs::write(stage.path().expect("native stage"), b"partial")
                    .map_err(super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "injected cancellation",
                ))
            })
        }),
        Rc::new(|_, _, _| Box::pin(async { panic!("cancelled copy must not commit") })),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert_eq!(fs::read(&source)?, b"source contents");
    assert_eq!(fs::read_dir(root.path())?.count(), 1);
    Ok(())
}

#[test]
fn staged_remote_file_copy_preserves_a_racing_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, stage, _| {
            Box::pin(async move {
                fs::copy(
                    source.path().expect("native source"),
                    stage.path().expect("native stage"),
                )
                .map(|_| ())
                .map_err(super::io_error)
            })
        }),
        Rc::new(|_, target, _| {
            Box::pin(async move {
                fs::write(target.path().expect("native target"), b"racing contents")
                    .map_err(super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "injected destination race",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Exists)));
    assert_eq!(fs::read(&target)?, b"racing contents");
    assert_eq!(fs::read(&source)?, b"source contents");
    assert_eq!(fs::read_dir(root.path())?.count(), 2);
    Ok(())
}

#[test]
fn failed_incomplete_copy_cleanup_is_reported_as_a_failure() {
    let error = copy_failure_after_cleanup(
        glib::Error::new(gio::IOErrorEnum::Cancelled, "injected cancellation"),
        Err(glib::Error::new(
            gio::IOErrorEnum::PermissionDenied,
            "injected cleanup failure",
        )),
    );

    assert!(!error.matches(gio::IOErrorEnum::Cancelled));
    assert!(
        error
            .to_string()
            .contains("incomplete copy could not be removed")
    );
    assert!(error.to_string().contains("injected cleanup failure"));
}

#[test]
fn cancelling_recursive_copy_removes_only_its_staging_output() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-copy-cancel-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("nested/item.txt"), b"contents")?;
    fs::write(root.join("pre-existing.txt"), b"keep")?;

    let cancellable = gio::Cancellable::new();
    let task = glib::MainContext::default().spawn_local(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        cancellable.clone(),
    ));
    let context = glib::MainContext::default();
    loop {
        context.iteration(true);
        if fs::read_dir(&root)?.any(|entry| {
            entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with(".strata-"))
        }) {
            break;
        }
    }
    cancellable.cancel();
    let result = context.block_on(task)?;
    settle_cancelled_io(&context);

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert_eq!(fs::read(root.join("pre-existing.txt"))?, b"keep");
    assert_eq!(fs::read_dir(&root)?.count(), 2);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cancelling_a_new_file_copy_removes_the_incomplete_stage() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::File::create(&source)?.set_len(64 * 1024 * 1024)?;
    fs::write(root.path().join("keep.txt"), b"keep")?;
    let cancellable = gio::Cancellable::new();
    let cancel_on_write = cancellable.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(88),
        Some(64 * 1024 * 1024),
        Some(1),
        Rc::new(move |event| {
            if matches!(
                event,
                OperationEvent::TransferProgress {
                    transferred_bytes: 1..,
                    ..
                }
            ) {
                cancel_on_write.cancel();
            }
        }),
    );
    let result = glib::MainContext::default().block_on(copy_new_recursively_with_progress(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        cancellable,
        Some(progress),
    ));
    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert_eq!(fs::read_dir(root.path())?.count(), 2);
    assert_eq!(fs::read(root.path().join("keep.txt"))?, b"keep");
    Ok(())
}

#[test]
fn copying_and_replacing_symlinks_accepts_an_aliased_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    let source = root.path().join("source");
    fs::create_dir(&actual)?;
    fs::write(root.path().join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink("../sentinel.txt", &source)?;
    let context = glib::MainContext::default();

    for overwrite in [false, true] {
        context.block_on(copy_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(alias.join("link")),
            overwrite,
            gio::Cancellable::new(),
            None,
        ))?;
        assert_eq!(
            fs::read_link(actual.join("link"))?,
            Path::new("../sentinel.txt")
        );
    }
    context.block_on(copy_new_recursively(
        gio::File::for_path(root.path().join("sentinel.txt")),
        gio::File::for_path(alias.join("new.txt")),
        gio::Cancellable::new(),
    ))?;
    assert_eq!(fs::read(actual.join("new.txt"))?, b"keep");
    assert_eq!(fs::read(root.path().join("sentinel.txt"))?, b"keep");
    Ok(())
}

#[test]
fn copying_a_tree_with_a_named_pipe_fails_instead_of_blocking() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        source.join("00-pipe"),
        rustix::fs::Mode::from_bits_truncate(0o600),
    )?;
    for index in 1..16 {
        fs::write(
            source.join(format!("{index:02}-file.txt")),
            index.to_string(),
        )?;
    }

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    let error = result.expect_err("a named pipe cannot be copied as a regular file");
    assert!(
        error.to_string().contains("pipe"),
        "the error should name the entry: {error}"
    );
    assert!(!target.join("00-pipe").exists());
    let entries_after_error = fs::read_dir(&target)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<HashSet<_>, _>>()?;
    glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(20)));
    let entries_after_settling = fs::read_dir(&target)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<HashSet<_>, _>>()?;
    assert_eq!(entries_after_settling, entries_after_error);
    fs::remove_dir_all(&target)?;
    let staged = glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
    ));
    assert!(staged.is_err());
    assert!(
        !target.exists(),
        "failed new copies must not expose partial trees"
    );
    assert_eq!(
        fs::read_dir(root.path())?.count(),
        1,
        "failed stages are removed"
    );
    Ok(())
}

#[test]
fn fat_sanitized_name_replaces_invalid_characters_and_trims_trailing_dots_and_spaces() {
    assert_eq!(
        fat_sanitized_name(OsStr::new("sc_macroorganizer?tabid:short=1.png")),
        OsStr::new("sc_macroorganizer_tabid_short=1.png")
    );
    assert_eq!(
        fat_sanitized_name(OsStr::new(r#"a"*/:<>?\|b"#)),
        OsStr::new("a_________b")
    );
    assert_eq!(
        fat_sanitized_name(OsStr::new("trailing dots.. ")),
        OsStr::new("trailing dots")
    );
    assert_eq!(fat_sanitized_name(OsStr::new("...")), OsStr::new("_"));
    assert_eq!(
        fat_sanitized_name(OsStr::new("plain.txt")),
        OsStr::new("plain.txt")
    );
}

#[test]
fn unique_fat_sibling_name_numbers_a_collision_instead_of_overwriting() {
    let mut used = HashSet::new();
    let first = unique_fat_sibling_name(OsString::from("a_b.txt"), &mut used);
    let second = unique_fat_sibling_name(OsString::from("a_b.txt"), &mut used);
    let third = unique_fat_sibling_name(OsString::from("a_b.txt"), &mut used);
    assert_eq!(first, OsString::from("a_b.txt"));
    assert_eq!(second, OsString::from("a_b (1).txt"));
    assert_eq!(third, OsString::from("a_b (2).txt"));
    assert_eq!(
        unique_fat_sibling_name(OsString::from("A_B.txt"), &mut used),
        OsString::from("A_B (3).txt")
    );
    let maximum = OsString::from("a (18446744073709551615).txt");
    assert_eq!(unique_fat_sibling_name(maximum.clone(), &mut used), maximum);
    let almost_maximum = OsString::from("a (18446744073709551614).txt");
    assert_eq!(
        unique_fat_sibling_name(almost_maximum.clone(), &mut used),
        almost_maximum
    );
    assert_eq!(
        unique_fat_sibling_name(almost_maximum, &mut used),
        OsString::from("a (1).txt")
    );
}

#[test]
fn target_is_fat_family_reads_the_reported_filesystem_type() {
    let mount_point = std::env::temp_dir().join("strata-fat-family-mount-probe");
    let mounts = MountTable::parse(format!(
        "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n22 1 8:2 / {} rw - exfat /dev/sdb1 rw\n",
        mount_point.display()
    ));
    assert!(target_is_fat_family(
        &gio::File::for_path(mount_point.join("photo.jpg")),
        &mounts
    ));
    assert!(!target_is_fat_family(
        &gio::File::for_path("/some/other/path"),
        &mounts
    ));
    assert!(!target_is_fat_family(
        &gio::File::for_uri("sftp://example.com/remote"),
        &mounts
    ));
}

#[test]
fn fat_family_copy_sanitizes_an_invalid_name_instead_of_discarding_the_whole_tree()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("ok.txt"), b"fine")?;
    fs::write(
        source.join("nested/sc_macroorganizer?tabid:short=1.png"),
        b"cached icon",
    )?;

    let result = glib::MainContext::default().block_on(copy_recursively_fat_family(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("ok.txt"))?, b"fine");
    assert_eq!(
        fs::read(target.join("nested/sc_macroorganizer_tabid_short=1.png"))?,
        b"cached icon"
    );
    assert!(
        !target
            .join("nested/sc_macroorganizer?tabid:short=1.png")
            .exists()
    );
    Ok(())
}

#[test]
fn fat_family_copy_disambiguates_sibling_names_that_collide_after_sanitizing()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("a?.txt"), b"first")?;
    fs::write(source.join("a:.txt"), b"second")?;
    fs::write(source.join("A*.txt"), b"third")?;

    let result = glib::MainContext::default().block_on(copy_recursively_fat_family(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    let mut contents = [
        fs::read(target.join("A_.txt"))?,
        fs::read(target.join("a_ (1).txt"))?,
        fs::read(target.join("a_ (2).txt"))?,
    ];
    contents.sort();
    assert_eq!(
        contents,
        [b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
    );
    Ok(())
}

#[test]
fn non_fat_copy_leaves_invalid_characters_untouched() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("a?b:c.txt"), b"unchanged")?;

    let result = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("a?b:c.txt"))?, b"unchanged");
    Ok(())
}

#[test]
fn commit_staged_falls_back_when_noreplace_is_unsupported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for error in [
        rustix::io::Errno::INVAL,
        rustix::io::Errno::NOSYS,
        rustix::io::Errno::OPNOTSUPP,
    ] {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let staged = super::super::StagedSibling::create(root.path(), directory)?;
            let staged_path = staged.path().to_owned();
            let target = root.path().join("target");
            if directory {
                fs::create_dir_all(staged_path.join("nested"))?;
                fs::write(staged_path.join("nested/file.txt"), b"directory data")?;
            } else {
                fs::write(&staged_path, b"file data")?;
            }

            let result = glib::MainContext::default().block_on(
                super::super::publish_staged_without_replace_with(
                    staged,
                    target.clone(),
                    gio::Cancellable::new(),
                    move |_, _| Err(error),
                ),
            );
            assert!(result.is_ok(), "fallback commit must succeed for {error:?}");
            assert!(
                !staged_path.exists(),
                "staged source should no longer exist"
            );
            assert!(target.exists(), "target destination must exist");
            if directory {
                assert_eq!(
                    fs::read_to_string(target.join("nested/file.txt"))?,
                    "directory data"
                );
            } else {
                assert_eq!(fs::read_to_string(&target)?, "file data");
            }
        }
    }
    Ok(())
}

#[test]
fn commit_staged_fallback_refuses_to_overwrite_existing_target() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for error in [
        rustix::io::Errno::INVAL,
        rustix::io::Errno::NOSYS,
        rustix::io::Errno::OPNOTSUPP,
    ] {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let staged = super::super::StagedSibling::create(root.path(), directory)?;
            let staged_path = staged.path().to_owned();
            let target = root.path().join("target");
            if directory {
                fs::create_dir(&target)?;
                fs::write(target.join("existing.txt"), b"target data")?;
            } else {
                fs::write(&staged_path, b"new data")?;
                fs::write(&target, b"target data")?;
            }

            let result = glib::MainContext::default().block_on(
                super::super::publish_staged_without_replace_with(
                    staged,
                    target.clone(),
                    gio::Cancellable::new(),
                    move |_, _| Err(error),
                ),
            );
            assert!(
                result
                    .expect_err("must refuse to overwrite existing target")
                    .matches(gio::IOErrorEnum::Exists)
            );
            assert!(
                !staged_path.exists(),
                "failed publication must clean its stage"
            );
            if directory {
                assert_eq!(
                    fs::read_to_string(target.join("existing.txt"))?,
                    "target data"
                );
            } else {
                assert_eq!(fs::read_to_string(&target)?, "target data");
            }
        }
    }
    Ok(())
}

#[test]
fn staged_publication_preserves_racing_destinations() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for source_directory in [false, true] {
        for destination_kind in ["file", "directory", "symlink"] {
            let root = tempfile::tempdir()?;
            let staged = super::super::StagedSibling::create(root.path(), source_directory)?;
            let stage_path = staged.path().to_owned();
            if !source_directory {
                fs::write(&stage_path, b"new data")?;
            }
            let target = root.path().join("target");
            let protected = root.path().join("protected");
            fs::write(&protected, b"protected data")?;
            let result = glib::MainContext::default().block_on(
                super::super::publish_staged_without_replace_with(
                    staged,
                    target.clone(),
                    gio::Cancellable::new(),
                    move |_, to| {
                        match destination_kind {
                            "file" => fs::write(to, b"racing data").expect("create competing file"),
                            "directory" => fs::create_dir(to).expect("create competing directory"),
                            _ => std::os::unix::fs::symlink("protected", to)
                                .expect("create competing symlink"),
                        }
                        Err(rustix::io::Errno::OPNOTSUPP)
                    },
                ),
            );
            assert!(result.is_err());
            assert!(!stage_path.exists());
            match destination_kind {
                "file" => assert_eq!(fs::read(&target)?, b"racing data"),
                "directory" => assert_eq!(fs::read_dir(&target)?.count(), 0),
                _ => assert_eq!(fs::read_link(&target)?, Path::new("protected")),
            }
            assert_eq!(fs::read(root.path().join("protected"))?, b"protected data");
        }
    }
    Ok(())
}

#[test]
fn staged_fallback_preserves_dangling_symlinks() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let staged = super::super::StagedSibling::create(root.path(), false)?;
    let staged_path = staged.path().to_owned();
    fs::remove_file(&staged_path)?;
    std::os::unix::fs::symlink("missing", &staged_path)?;
    let target = root.path().join("target");
    glib::MainContext::default().block_on(super::super::publish_staged_without_replace_with(
        staged,
        target.clone(),
        gio::Cancellable::new(),
        |_, _| Err(rustix::io::Errno::INVAL),
    ))?;
    assert_eq!(fs::read_link(target)?, Path::new("missing"));
    assert!(fs::symlink_metadata(staged_path).is_err());
    Ok(())
}

#[test]
fn staged_fallback_removes_reserved_directory_on_copy_error() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let staged = super::super::StagedSibling::create(root.path(), true)?;
    let staged_path = staged.path().to_owned();
    let _socket = std::os::unix::net::UnixListener::bind(staged_path.join("unsupported.sock"))?;
    let target = root.path().join("target");
    let result =
        glib::MainContext::default().block_on(super::super::publish_staged_without_replace_with(
            staged,
            target.clone(),
            gio::Cancellable::new(),
            |_, _| Err(rustix::io::Errno::NOSYS),
        ));
    assert!(result.is_err());
    assert!(!target.exists());
    assert!(!staged_path.exists());
    Ok(())
}

#[test]
fn staged_fallback_reports_cleanup_failure_after_publication() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let staged = super::super::StagedSibling::create(root.path(), false)?;
    let target = root.path().join("target");
    let result =
        glib::MainContext::default().block_on(super::super::publish_staged_without_replace_with(
            staged,
            target.clone(),
            gio::Cancellable::new(),
            |from, _| {
                fs::remove_file(from).expect("remove file stage");
                fs::create_dir(from).expect("replace stage with directory");
                fs::write(from.join("copied.txt"), b"published data").expect("populate stage");
                Err(rustix::io::Errno::INVAL)
            },
        ));
    assert!(
        result
            .expect_err("cleanup must fail")
            .to_string()
            .contains("The item was copied")
    );
    assert_eq!(fs::read(target.join("copied.txt"))?, b"published data");
    Ok(())
}

#[test]
fn cancelling_staged_fallback_removes_its_reserved_directory() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let staged = super::super::StagedSibling::create(root.path(), true)?;
    let staged_path = staged.path().to_owned();
    fs::File::create(staged_path.join("large.bin"))?.set_len(64 * 1024 * 1024)?;
    let target = root.path().join("target");
    let cancellable = gio::Cancellable::new();
    let finished = Rc::new(Cell::new(false));
    let completed = finished.clone();
    let publish_target = target.clone();
    let publish_cancellable = cancellable.clone();
    let context = glib::MainContext::default();
    let task = context.spawn_local(async move {
        let result = super::super::publish_staged_without_replace_with(
            staged,
            publish_target,
            publish_cancellable,
            |_, _| Err(rustix::io::Errno::INVAL),
        )
        .await;
        completed.set(true);
        result
    });
    while !target.exists() && !finished.get() {
        context.iteration(true);
    }
    assert!(
        target.exists(),
        "fallback must reserve the destination before cancellation"
    );
    cancellable.cancel();
    let result = context.block_on(task)?;
    settle_cancelled_io(&context);
    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    assert!(!staged_path.exists());
    Ok(())
}
