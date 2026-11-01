// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn nested_copy_shares_one_worker_budget_across_subdirectories() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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
    *copy_activity() = (0, 0);
    glib::MainContext::default().block_on(copy_local(
        source,
        Destination::New(target.clone(), Taken::Fail),
        Options {
            workers: Some(2),
            ..Options::default()
        },
        None,
        &gio::Cancellable::new(),
    ))?;
    let (active, peak) = *copy_activity();
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
    let _serial = serial()?;
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
    ));

    assert!(result.is_ok());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");

    fs::write(source.join("top.txt"), b"replacement")?;
    let merged = glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
    ));
    assert!(merged.is_ok(), "{merged:?}");
    // A merge fills only names it moved out of the way, and numbers the rest.
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("top (1).txt"))?, b"replacement");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");
    assert_eq!(fs::read(target.join("nested/child (1).txt"))?, b"child");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn copy_recursively_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
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
    let _serial = serial()?;
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
    let _serial = serial()?;
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
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target)?, b"keep");
    assert_eq!(fs::read(actual_root.join("subdir/source.txt"))?, b"keep");
    Ok(())
}

#[test]
fn cancelling_staged_remote_file_copy_removes_only_the_incomplete_stage()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        Rc::new(|_, stage, _| {
            Box::pin(async move {
                fs::write(stage.path().expect("native stage"), b"partial")
                    .map_err(super::super::io_error)?;
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
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;

    let result = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Fail,
        gio::Cancellable::new(),
        Rc::new(|source, stage, _| {
            Box::pin(async move {
                fs::copy(
                    source.path().expect("native source"),
                    stage.path().expect("native stage"),
                )
                .map(|_| ())
                .map_err(super::super::io_error)
            })
        }),
        Rc::new(|_, target, _| {
            Box::pin(async move {
                fs::write(target.path().expect("native target"), b"racing contents")
                    .map_err(super::super::io_error)?;
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
fn a_remote_keep_both_copy_takes_the_next_name_when_one_is_taken_meanwhile()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"source contents")?;
    fs::write(&target, b"existing")?;

    let copied = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Number,
        gio::Cancellable::new(),
        Rc::new(|source, stage, _| {
            Box::pin(async move {
                fs::copy(
                    source.path().expect("native source"),
                    stage.path().expect("native stage"),
                )
                .map(drop)
                .map_err(super::super::io_error)
            })
        }),
        Rc::new(|stage, target, _| {
            Box::pin(async move {
                let target = target.path().expect("native target");
                if target.ends_with("target (1).bin") {
                    fs::write(&target, b"racing contents").map_err(super::super::io_error)?;
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::Exists,
                        "injected destination race",
                    ));
                }
                fs::rename(stage.path().expect("native stage"), target)
                    .map_err(super::super::io_error)
            })
        }),
    ))?;

    assert_eq!(copied.path(), Some(root.path().join("target (2).bin")));
    assert_eq!(
        fs::read(root.path().join("target (2).bin"))?,
        b"source contents"
    );
    assert_eq!(
        fs::read(root.path().join("target (1).bin"))?,
        b"racing contents"
    );
    assert_eq!(fs::read(&target)?, b"existing");
    assert_eq!(
        names(root.path())?,
        [
            "source.bin",
            "target (1).bin",
            "target (2).bin",
            "target.bin"
        ]
    );
    Ok(())
}

#[test]
fn a_remote_keep_both_copy_numbers_a_long_name_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join(format!("{}.bin", "t".repeat(251)));
    let numbered = root.path().join(format!("{} (1).bin", "t".repeat(247)));
    fs::write(&source, b"source contents")?;
    fs::write(&target, b"existing")?;

    let copied = glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        Taken::Number,
        gio::Cancellable::new(),
        Rc::new(|source, stage, _| {
            Box::pin(async move {
                fs::copy(
                    source.path().expect("native source"),
                    stage.path().expect("native stage"),
                )
                .map(drop)
                .map_err(super::super::io_error)
            })
        }),
        Rc::new(|stage, target, _| {
            Box::pin(async move {
                fs::rename(
                    stage.path().expect("native stage"),
                    target.path().expect("native target"),
                )
                .map_err(super::super::io_error)
            })
        }),
    ))?;

    assert_eq!(copied.path(), Some(numbered.clone()));
    assert_eq!(fs::read(numbered)?, b"source contents");
    assert_eq!(fs::read(&target)?, b"existing");
    Ok(())
}

#[test]
fn failed_incomplete_copy_cleanup_is_reported_as_a_failure() {
    let error = copy_failure_after_cleanup(
        glib::Error::new(gio::IOErrorEnum::Cancelled, "injected cancellation"),
        Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
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
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("nested/item.txt"), b"contents")?;
    fs::write(root.path().join("pre-existing.txt"), b"keep")?;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(
                Op::OpenSource,
                "item.txt",
                0,
                Box::new(move || cancel.cancel()),
            )],
            ..Quirks::default()
        },
    );

    let result = glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        cancellable,
    ));

    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
        "{result:?}"
    );
    assert_eq!(names(root.path())?, ["pre-existing.txt", "source"]);
    assert_eq!(fs::read(root.path().join("pre-existing.txt"))?, b"keep");
    Ok(())
}

#[test]
fn cancelling_a_new_file_copy_removes_the_incomplete_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (case, preset) in [
        ("unnamed file", Quirks::default as Preset),
        ("named stage", || Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::ext4()
        }),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source.bin");
        let target = root.path().join("target.bin");
        fs::write(&source, vec![0x5a; 4 * 1024 * 1024])?;
        fs::write(root.path().join("keep.txt"), b"keep")?;
        let cancellable = gio::Cancellable::new();
        let cancel =
            |cancellable: gio::Cancellable| -> Racer { Box::new(move || cancellable.cancel()) };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![
                    (Op::CopyRange, "*", 0, cancel(cancellable.clone())),
                    (Op::Write, "*", 0, cancel(cancellable.clone())),
                ],
                reflink: Support::Rejected(Errno::OPNOTSUPP),
                ..preset()
            },
        );

        let result = glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            cancellable,
        ));

        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
            "{case}: {result:?}"
        );
        assert_eq!(names(root.path())?, ["keep.txt", "source.bin"], "{case}");
    }
    Ok(())
}

#[test]
fn a_stage_name_left_by_a_lost_reply_is_adopted_and_a_taken_one_is_not()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let is_stage = |name: &str| name.starts_with(".strata-copy-");
    for (op, folder) in [(Op::Create, false), (Op::Mkdir, true)] {
        for taken in [false, true] {
            let row = format!("{op:?}, taken: {taken}");
            let root = tempfile::tempdir()?;
            let (source, target) = (root.path().join("source"), root.path().join("target"));
            let copied = if folder {
                fs::create_dir(&source)?;
                source.join("inner.txt")
            } else {
                source.clone()
            };
            fs::write(&copied, b"copied")?;
            let parent = root.path().to_path_buf();
            let racer: Racer = Box::new(move || {
                let stage = names(&parent)
                    .expect("list")
                    .into_iter()
                    .find(|name| is_stage(name))
                    .map(|name| parent.join(name))
                    .expect("the stage");
                let theirs = if folder { stage.join("theirs") } else { stage };
                fs::write(theirs, b"theirs").expect("fill the stage");
            });
            let scope = FsQuirks::install(
                root.path(),
                Quirks {
                    tmpfile: Support::Rejected(Errno::OPNOTSUPP),
                    lost_reply: vec![(op, ".strata-copy-*", Errno::EXIST)],
                    before: if taken {
                        vec![(Op::Stat, ".strata-copy-*", 0, racer)]
                    } else {
                        Vec::new()
                    },
                    ..Quirks::ext4()
                },
            );

            glib::MainContext::default().block_on(copy_new_recursively(
                gio::File::for_path(&source),
                gio::File::for_path(&target),
                gio::Cancellable::new(),
            ))?;

            let stages = scope
                .calls()
                .into_iter()
                .filter(|call| call.op == op)
                .flat_map(|call| call.names)
                .filter(|name| is_stage(&name.to_string_lossy()))
                .collect::<HashSet<_>>();
            assert_eq!(stages.len(), if taken { 2 } else { 1 }, "{row}");
            let copy = if folder {
                target.join("inner.txt")
            } else {
                target
            };
            assert_eq!(fs::read(copy)?, b"copied", "{row}");
            let left = names(root.path())?
                .into_iter()
                .filter(|name| is_stage(name))
                .map(|name| root.path().join(name))
                .collect::<Vec<_>>();
            match left.as_slice() {
                [] => assert!(!taken, "{row}"),
                [theirs] => {
                    let theirs = if folder {
                        theirs.join("theirs")
                    } else {
                        theirs.clone()
                    };
                    assert!(taken, "{row}");
                    assert_eq!(fs::read(theirs)?, b"theirs", "{row}");
                }
                _ => panic!("{row}: {left:?}"),
            }
        }
    }
    Ok(())
}

#[test]
fn a_stage_replaced_during_the_copy_is_kept() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (sources, destination) = (root.path().join("sources"), root.path().join("destination"));
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    fs::write(sources.join("source.bin"), vec![1; 64 * 1024])?;
    let stage_folder = destination.clone();
    let racer: Racer = Box::new(move || {
        let stage = only_entry(&stage_folder, ".strata-copy-");
        fs::remove_file(&stage).expect("remove the stage");
        fs::write(&stage, b"theirs").expect("take the stage name");
    });
    let _source = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            before: vec![(Op::Write, "*", 0, racer)],
            fail: vec![(Op::Write, "*", 0, Errno::IO)],
            ..Quirks::ext4()
        },
    );

    let error = glib::MainContext::default()
        .block_on(copy_new_recursively(
            gio::File::for_path(sources.join("source.bin")),
            gio::File::for_path(destination.join("target.bin")),
            gio::Cancellable::new(),
        ))
        .expect_err("the write fails");

    assert!(
        error
            .message()
            .contains("incomplete copy could not be removed"),
        "{error}"
    );
    assert_eq!(
        fs::read(only_entry(&destination, ".strata-copy-"))?,
        b"theirs"
    );
    assert_eq!(names(&destination)?.len(), 1);
    Ok(())
}

#[test]
fn a_folder_swapped_for_a_link_during_the_copy_is_not_followed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let outside = root.path().join("outside");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("nested/inside.txt"), b"inside")?;
    fs::create_dir(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not copy me")?;
    let (nested, moved) = (source.join("nested"), source.join("moved"));
    let racer: Racer = Box::new(move || {
        fs::rename(&nested, &moved).expect("move the folder away");
        std::os::unix::fs::symlink("../outside", &nested).expect("put a link in its place");
    });
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::OpenDir, "nested", 0, racer)],
            ..Quirks::default()
        },
    );

    let error = glib::MainContext::default()
        .block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(root.path().join("target")),
            gio::Cancellable::new(),
        ))
        .expect_err("the swapped folder is refused");

    assert!(error.message().contains("changed"), "{error}");
    assert_eq!(names(root.path())?, ["outside", "source"]);
    assert_eq!(names(&outside)?, ["secret.txt"]);
    Ok(())
}

#[test]
fn copying_symlinks_accepts_an_aliased_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    let source = root.path().join("source");
    fs::create_dir(&actual)?;
    fs::write(root.path().join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink("../sentinel.txt", &source)?;
    let context = glib::MainContext::default();

    context.block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(alias.join("link")),
        false,
        gio::Cancellable::new(),
    ))?;
    assert_eq!(
        fs::read_link(actual.join("link"))?,
        Path::new("../sentinel.txt")
    );
    context.block_on(copy_new_recursively(
        gio::File::for_path(root.path().join("sentinel.txt")),
        gio::File::for_path(alias.join("new.txt")),
        gio::Cancellable::new(),
    ))?;
    assert_eq!(fs::read(actual.join("new.txt"))?, b"keep");
    assert_eq!(fs::read(root.path().join("sentinel.txt"))?, b"keep");
    Ok(())
}
