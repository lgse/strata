// SPDX-License-Identifier: MIT

use super::*;

fn publish(staged: StagedSibling, target: &Path) -> Result<(), glib::Error> {
    glib::MainContext::default()
        .block_on(publish_staged_without_replace(
            staged,
            target.to_path_buf(),
            false,
            gio::Cancellable::new(),
        ))
        .map(drop)
}

#[test]
fn staged_publication_without_rename_flags_places_the_stage_itself() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::MetadataExt;
    let _serial = serial()?;
    let presets: [(&str, Preset); 5] = [
        ("EINVAL", Quirks::without_rename_flags),
        ("ENOSYS", || Quirks {
            rename_flags: Support::Rejected(Errno::NOSYS),
            ..Quirks::default()
        }),
        ("EOPNOTSUPP", || Quirks {
            rename_flags: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        }),
        ("sshfs", Quirks::sshfs),
        ("vboxsf", Quirks::vboxsf),
    ];
    for (case, quirks) in presets {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), quirks());
            let staged = stage(root.path(), directory)?;
            let staged_path = root.path().join(&staged.name);
            let target = root.path().join("target");
            if directory {
                fs::create_dir_all(staged_path.join("nested"))?;
                fs::write(staged_path.join("nested/file.txt"), b"directory data")?;
            } else {
                fs::write(&staged_path, b"file data")?;
            }
            let inode = fs::metadata(&staged_path)?.ino();

            publish(staged, &target)?;

            assert_eq!(names(root.path())?, ["target"], "{case}");
            assert_eq!(fs::metadata(&target)?.ino(), inode, "{case}");
            if directory {
                assert_eq!(fs::read(target.join("nested/file.txt"))?, b"directory data");
            } else {
                assert_eq!(fs::read(&target)?, b"file data");
            }
        }
    }
    Ok(())
}

#[test]
fn staged_publication_refuses_to_overwrite_an_existing_target() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [
        Quirks::default,
        Quirks::without_rename_flags,
        Quirks::vboxsf,
    ] {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let _scope = FsQuirks::install(root.path(), quirks());
            let staged = stage(root.path(), directory)?;
            let staged_path = root.path().join(&staged.name);
            let target = root.path().join("target");
            if directory {
                fs::create_dir(&target)?;
                fs::write(target.join("existing.txt"), b"target data")?;
            } else {
                fs::write(&staged_path, b"new data")?;
                fs::write(&target, b"target data")?;
            }

            let error = publish(staged, &target).expect_err("must refuse an existing target");

            assert!(error.matches(gio::IOErrorEnum::Exists), "{error}");
            assert!(
                !staged_path.exists(),
                "failed publication must clean its stage"
            );
            let kept = if directory {
                target.join("existing.txt")
            } else {
                target.clone()
            };
            assert_eq!(fs::read(kept)?, b"target data");
        }
    }
    Ok(())
}

#[test]
fn staged_publication_preserves_racing_destinations() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for source_directory in [false, true] {
        for destination_kind in ["file", "directory", "symlink"] {
            let root = tempfile::tempdir()?;
            let target = root.path().join("target");
            let raced = target.clone();
            let racer: Racer = Box::new(move || match destination_kind {
                "file" => fs::write(&raced, b"racing data").expect("create competing file"),
                "directory" => fs::create_dir(&raced).expect("create competing directory"),
                _ => std::os::unix::fs::symlink("protected", &raced)
                    .expect("create competing symlink"),
            });
            // The race lands after the first rung, in front of the fallback rung.
            let fallback = if source_directory {
                Op::Mkdir
            } else {
                Op::Link
            };
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    rename_flags: Support::Rejected(Errno::OPNOTSUPP),
                    before: vec![(fallback, "target", 0, racer)],
                    ..Quirks::default()
                },
            );
            let staged = stage(root.path(), source_directory)?;
            let stage_path = root.path().join(&staged.name);
            if !source_directory {
                fs::write(&stage_path, b"new data")?;
            }
            fs::write(root.path().join("protected"), b"protected data")?;

            assert!(publish(staged, &target).is_err());

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
fn staged_symlinks_are_recreated_where_links_are_missing() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), Quirks::vboxsf());
    let staged = staged(root.path(), StageContent::Symlink("missing".into()))?;
    let staged_path = root.path().join(&staged.name);
    let target = root.path().join("target");

    publish(staged, &target)?;

    assert_eq!(fs::read_link(target)?, Path::new("missing"));
    assert!(fs::symlink_metadata(staged_path).is_err());
    Ok(())
}

#[test]
fn staged_fallback_removes_reserved_directory_on_copy_error() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let _scope = FsQuirks::install(root.path(), Quirks::without_placement());
    let staged = stage(root.path(), true)?;
    let staged_path = root.path().join(&staged.name);
    let _socket = std::os::unix::net::UnixListener::bind(staged_path.join("unsupported.sock"))?;
    let target = root.path().join("target");

    assert!(publish(staged, &target).is_err());

    assert!(!target.exists());
    assert!(!staged_path.exists());
    Ok(())
}

fn only_stage(root: &Path) -> PathBuf {
    only_entry(root, ".strata-copy-")
}

/// A folder put at a file stage's name gets the stage's inode number back.
fn reusing_stage_inodes(before: (Op, &'static str, usize, Racer)) -> Quirks {
    Quirks {
        reused_inodes: Some(".strata-copy-*"),
        no_birth_time: true,
        before: vec![before],
        ..Quirks::without_placement()
    }
}

/// Replaces the only file stage beneath `stage_root` with a folder,
/// holding `copied.txt` with `foreign` contents when given.
fn stage_swapped_for_a_folder(stage_root: PathBuf, foreign: Option<&'static [u8]>) -> Racer {
    Box::new(move || {
        let stage = only_stage(&stage_root);
        fs::remove_file(&stage).expect("remove file stage");
        fs::create_dir(&stage).expect("replace stage with directory");
        if let Some(foreign) = foreign {
            fs::write(stage.join("copied.txt"), foreign).expect("populate");
        }
    })
}

fn file_stage_at(root: &Path) -> Result<(StagedSibling, PathBuf), Box<dyn Error>> {
    let staged = stage(root, false)?;
    let staged_path = root.join(&staged.name);
    Ok((staged, staged_path))
}

#[test]
fn a_stage_left_after_the_exclusive_copy_is_reported() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let racer = stage_swapped_for_a_folder(root.path().to_path_buf(), None);
    // Two stats decide publication; the third is the discard.
    let _scope = FsQuirks::install(
        root.path(),
        reusing_stage_inodes((Op::Stat, ".strata-copy-*", 2, racer)),
    );
    let (staged, staged_path) = file_stage_at(root.path())?;
    fs::write(&staged_path, b"copied")?;
    let target = root.path().join("target");

    let error = publish(staged, &target).expect_err("the stage cannot be removed");

    assert!(
        error
            .message()
            .contains("staging copy could not be removed"),
        "{error}"
    );
    assert_eq!(fs::read(&target)?, b"copied");
    assert!(staged_path.is_dir());
    Ok(())
}

#[test]
fn a_stage_swapped_before_publication_is_neither_published_nor_deleted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let racer = stage_swapped_for_a_folder(root.path().to_path_buf(), Some(b"foreign data"));
    let _scope = FsQuirks::install(
        root.path(),
        reusing_stage_inodes((Op::Rename, ".strata-copy-*", 0, racer)),
    );
    let (staged, staged_path) = file_stage_at(root.path())?;
    let target = root.path().join("target");

    let error = publish(staged, &target).expect_err("a swapped stage is refused");

    assert!(
        error
            .message()
            .contains("incomplete copy could not be removed"),
        "{error}"
    );
    assert!(fs::symlink_metadata(&target).is_err());
    assert_eq!(fs::read(staged_path.join("copied.txt"))?, b"foreign data");
    Ok(())
}

#[test]
fn cancelling_staged_fallback_removes_its_reserved_directory() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    let cancellable = gio::Cancellable::new();
    let reserved = Arc::new(AtomicBool::new(false));
    let (seen, made, cancel) = (reserved.clone(), target.clone(), cancellable.clone());
    let racer: Racer = Box::new(move || {
        seen.store(made.is_dir(), std::sync::atomic::Ordering::Relaxed);
        cancel.cancel();
    });
    let scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::OpenSource, "large.bin", 0, racer)],
            ..Quirks::without_placement()
        },
    );
    // No placeholder, so the target appears only when the copy makes it.
    learn_mount_capability(scope.key(), |capabilities| {
        capabilities.rename_over_empty_directory = Some(false);
    });
    let staged = stage(root.path(), true)?;
    let staged_path = root.path().join(&staged.name);
    fs::File::create(staged_path.join("large.bin"))?.set_len(64 * 1024 * 1024)?;

    let result = glib::MainContext::default().block_on(publish_staged_without_replace(
        staged,
        target.clone(),
        false,
        cancellable,
    ));

    assert!(
        reserved.load(std::sync::atomic::Ordering::Relaxed),
        "fallback must reserve the destination before cancellation"
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
        "{result:?}"
    );
    assert!(!target.exists());
    assert!(!staged_path.exists());
    Ok(())
}

#[test]
fn a_symlink_child_never_overwrites_an_existing_entry() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source)?;
    fs::create_dir(&target)?;
    std::os::unix::fs::symlink("elsewhere", source.join("Link"))?;
    fs::write(target.join("Link"), b"existing")?;

    glib::MainContext::default().block_on(copy_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
    ))?;

    assert_eq!(fs::read(target.join("Link"))?, b"existing");
    assert_eq!(
        fs::read_link(target.join("Link (1)"))?,
        Path::new("elsewhere")
    );
    Ok(())
}

fn check_read_only_xattrs(directory: bool, mode: u32) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    if directory {
        fs::create_dir(&source)?;
        fs::write(source.join("inside.txt"), b"contents")?;
    } else {
        fs::write(&source, b"contents")?;
    }
    rustix::fs::setxattr(
        &source,
        "user.strata-test",
        b"metadata",
        rustix::fs::XattrFlags::empty(),
    )?;
    fs::set_permissions(&source, fs::Permissions::from_mode(mode))?;

    let copied = glib::MainContext::default().block_on(copy_new_recursively(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
    ));

    let copied_mode = fs::metadata(&target).map(|metadata| metadata.mode() & 0o7777);
    let mut value = [0u8; 32];
    let xattr = rustix::fs::getxattr(&target, "user.strata-test", &mut value[..]);
    for path in [&source, &target].into_iter().filter(|path| path.is_dir()) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    copied?;
    assert_eq!(copied_mode?, mode, "{directory}");
    assert_eq!(&value[..xattr?], b"metadata", "{directory}");
    Ok(())
}

#[test]
fn read_only_items_keep_their_xattrs() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (directory, mode) in [(false, 0o444), (true, 0o555)] {
        check_read_only_xattrs(directory, mode)?;
    }
    Ok(())
}
