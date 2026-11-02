// SPDX-License-Identifier: MIT

//! A replacement is staged beside its target and published over it only whole.

use super::*;

fn cancelled(result: &Result<(), glib::Error>) -> bool {
    result
        .as_ref()
        .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled))
}

#[test]
fn replacement_publication_preserves_concurrent_arrivals() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for directory in [false, true] {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        let staged = if directory {
            staged(root.path(), StageContent::Directory)?
        } else {
            file_stage(root.path())?
        };
        let staged_path = root.path().join(&staged.name);
        if directory {
            fs::write(staged_path.join("incoming"), b"replacement")?;
            fs::create_dir(&target)?;
        } else {
            fs::write(&staged_path, b"replacement")?;
            fs::write(&target, b"new arrival")?;
        }
        let result = glib::MainContext::default().block_on(publish_staged_replacement(
            staged,
            target.clone(),
            false,
        ));

        assert!(
            result.is_err(),
            "publication must not replace a concurrent arrival"
        );
        if directory {
            assert!(target.is_dir());
            assert_eq!(fs::read_dir(&target)?.count(), 0);
        } else {
            assert_eq!(fs::read(&target)?, b"new arrival");
        }
        assert!(!staged_path.exists());
    }
    Ok(())
}

#[test]
fn a_replacement_reports_what_it_left_out_only_once_it_is_published() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for published in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source)?;
        fs::create_dir(&target)?;
        fs::write(source.join("a.txt"), b"incoming")?;
        fs::write(target.join("old.txt"), b"old")?;
        let _socket = std::os::unix::net::UnixListener::bind(source.join("socket"))?;
        let fail = if published {
            Vec::new()
        } else {
            vec![(Op::Rename, ".strata-copy-*", 0, Errno::IO)]
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                fail,
                ..Quirks::default()
            },
        );

        let (result, skipped) =
            replace_collecting(&source, &target, false, |report| report.skipped);

        assert_eq!(result.is_ok(), published, "{result:?}");
        let expected = if published {
            vec![crate::services::SkippedItem {
                location: Location::local(target.join("socket")),
                kind: crate::services::SpecialKind::Socket,
            }]
        } else {
            Vec::new()
        };
        assert_eq!(skipped, expected, "published: {published}");
    }
    Ok(())
}

#[test]
fn replacement_publication_reports_a_stage_it_cannot_remove() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::write(&target, b"existing")?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            reused_inodes: Some(".strata-copy-*"),
            no_birth_time: true,
            ..Quirks::default()
        },
    );
    let staged = file_stage(root.path())?;
    let staged_path = root.path().join(&staged.name);
    fs::remove_file(&staged_path)?;
    fs::create_dir(&staged_path)?;
    let error = glib::MainContext::default()
        .block_on(publish_staged_replacement(staged, target.clone(), false))
        .expect_err("concurrent destination prevents publication");
    assert!(
        error
            .message()
            .contains("incomplete copy could not be removed")
    );
    assert!(staged_path.is_dir());
    assert_eq!(fs::read(target)?, b"existing");
    fs::remove_dir(staged_path)?;
    Ok(())
}

#[test]
fn staged_file_replacement_preserves_the_destination_on_disk_full() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            fail: vec![
                (Op::CopyRange, "*", 0, Errno::NOSPC),
                (Op::Write, "*", 0, Errno::NOSPC),
            ],
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        },
    );

    let result = replace(&source, &target, false);

    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::NoSpace)),
        "{result:?}"
    );
    expect_left(&target, false, b"original", &["source.txt", "target.txt"]);
    Ok(())
}

#[test]
fn cancelling_staging_preserves_the_destination_and_cleans_the_partial_copy()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for directory in [false, true] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_item(&source, directory, &vec![0x5a; 4 * 1024 * 1024])?;
        write_item(&target, directory, b"original")?;
        let cancellable = gio::Cancellable::new();
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![
                    (Op::CopyRange, "*", 0, cancelling(&cancellable)),
                    (Op::Write, "*", 0, cancelling(&cancellable)),
                ],
                reflink: Support::Rejected(Errno::OPNOTSUPP),
                ..Quirks::default()
            },
        );

        let result = glib::MainContext::default().block_on(replace_local(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            cancellable,
            None,
        ));

        assert!(cancelled(&result), "{directory}: {result:?}");
        assert_eq!(read_item(&target, directory)?, b"original");
        expect_names(root.path(), &["source", "target"], &directory.to_string())?;
    }
    Ok(())
}

#[test]
fn cancelling_as_the_replacement_is_staged_keeps_the_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;
    let cancellable = gio::Cancellable::new();
    // The staged file's last step is its close: once linked, an unnamed
    // file's close names it by its inode. Without Trash, nothing after
    // staging checks the cancellation.
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(
            TMPFS_MAGIC,
            Quirks {
                before: vec![
                    (Op::Close, "#*", 0, cancelling(&cancellable)),
                    (Op::Close, ".strata-copy-*", 0, cancelling(&cancellable)),
                ],
                ..Quirks::default()
            },
        ),
    );

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        cancellable,
        None,
    ));

    assert!(cancelled(&result), "{result:?}");
    expect_left(&target, false, b"original", &["source.txt", "target.txt"]);
    Ok(())
}

#[test]
fn staged_file_replacement_commits_then_removes_a_moved_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = unique_root("strata-replacement-success-test")?;
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root)?;
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;

    let result = replace(&source, &target, true);

    assert!(result.is_ok(), "{result:?}");
    expect_left(&target, false, b"replacement", &["target.txt"]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn replacing_a_symlink_preserves_link_semantics() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source-link");
    let target = root.path().join("target-link");
    std::os::unix::fs::symlink("new-target", &source)?;
    std::os::unix::fs::symlink("old-target", &target)?;

    let result = replace(&source, &target, false);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read_link(&target)?, Path::new("new-target"));
    assert_eq!(fs::read_link(&source)?, Path::new("new-target"));
    Ok(())
}

#[test]
fn replace_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"new")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");
    fs::write(&target, b"old")?;

    let mut affected_locations = HashSet::new();
    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        Some(&mut affected_locations),
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(&target)?, b"new");
    assert_eq!(fs::read(actual_parent.join("source.txt"))?, b"new");
    Ok(())
}

#[test]
fn cancelled_replacement_move_tracks_the_modified_source_and_target_roots()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = unique_root("strata-replacement-move-cancel-test")?;
    let source = root.join("sources/source");
    let target = root.join("destination/target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(target.join("old"))?;
    let _volumes = separate_volumes(&root.join("sources"), &root.join("destination"));
    fs::write(source.join("new/item.txt"), b"replacement")?;
    for index in 0..16 {
        fs::write(target.join(format!("old/item-{index}.txt")), b"old")?;
    }

    let cancellable = gio::Cancellable::new();
    let cancel_after_commit = cancellable.clone();
    let committed_marker = target.join("new/item.txt");
    let context = glib::MainContext::default();
    let watcher = context.spawn_local(async move {
        while !committed_marker.exists() {
            glib::timeout_future(Duration::ZERO).await;
        }
        cancel_after_commit.cancel();
    });
    let mut affected_locations = HashSet::new();
    let result = context.block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        cancellable,
        Some(&mut affected_locations),
    ));
    context.block_on(watcher)?;
    settle_cancelled_io(&context);

    assert!(cancelled(&result));
    let roots = [&source, &target].map(Location::local);
    assert!(roots.iter().all(|root| affected_locations.contains(root)));
    expect_contents(&[(target.join("new/item.txt"), b"replacement")]);
    assert!(source.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_directory_replacement_does_not_merge_old_contents() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = unique_root("strata-directory-replacement-test")?;
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(target.join("old"))?;
    fs::write(source.join("new/item.txt"), b"new")?;
    fs::write(target.join("old/item.txt"), b"old")?;

    let result = replace(&source, &target, false);

    assert!(result.is_ok(), "{result:?}");
    expect_contents(&[(target.join("new/item.txt"), b"new")]);
    assert!(!target.join("old").exists());
    assert!(source.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn replacing_a_directory_cleans_up_a_symlink_in_the_old_contents_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = unique_root("strata-replace-symlink-cleanup-test")?;
    let source = root.join("source");
    let target = root.join("target");
    let outside = root.join("outside");
    fs::create_dir_all(&source)?;
    fs::write(source.join("item.txt"), b"new")?;
    fs::create_dir_all(&target)?;
    fs::write(target.join("keep.txt"), b"old")?;
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("secret.txt"), b"do not delete me")?;
    std::os::unix::fs::symlink(&outside, target.join("decoy"))?;

    let result = replace(&source, &target, false);

    assert!(result.is_ok(), "{result:?}");
    expect_contents(&[(target.join("item.txt"), b"new")]);
    assert!(
        !target.join("keep.txt").exists(),
        "the replaced directory's old contents must be gone"
    );
    let decoy = target.join("decoy");
    assert!(
        !decoy.exists() && !decoy.is_symlink(),
        "the old decoy symlink itself must be gone from the replaced directory"
    );
    assert_eq!(
        fs::read(outside.join("secret.txt"))?,
        b"do not delete me",
        "cleaning up the old target must never follow a symlink it contained"
    );

    fs::remove_dir_all(root)?;
    Ok(())
}
