// SPDX-License-Identifier: MIT

use std::os::unix::fs::MetadataExt;

use rustix::{fs::FsWord, io::Errno};

use super::super::{Copying, StageContent, publish_staged_replacement};
use super::fs_quirks::{Exchange, Op, Racer, Support, Trash};
use super::*;
use crate::{adapters::volume::mount_capabilities, services::ReplacedOriginal};

const NFS_SUPER_MAGIC: FsWord = 0x6969;
const FUSE_SUPER_MAGIC: FsWord = 0x6573_5546;
const TMPFS_MAGIC: FsWord = 0x0102_1994;

fn without_trash(fs_type: FsWord, quirks: Quirks) -> Quirks {
    Quirks {
        trash: Trash::Unsupported,
        fs_type: Some(fs_type),
        ..quirks
    }
}

fn inode(path: &Path) -> Result<u64, io::Error> {
    Ok(fs::symlink_metadata(path)?.ino())
}

fn replace_reporting(
    source: &Path,
    target: &Path,
    move_source: bool,
) -> (Result<(), glib::Error>, Vec<ReplacedOriginal>) {
    let replaced = RefCell::new(Vec::new());
    let result = glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
        Copying::default(),
        &|original, _| replaced.borrow_mut().push(original),
    ));
    (result, replaced.into_inner())
}

fn write_item(path: &Path, directory: bool, contents: &[u8]) -> Result<(), io::Error> {
    if directory {
        fs::create_dir(path)?;
        fs::write(path.join("item"), contents)
    } else {
        fs::write(path, contents)
    }
}

fn read_item(path: &Path, directory: bool) -> Result<Vec<u8>, io::Error> {
    fs::read(if directory {
        path.join("item")
    } else {
        path.to_path_buf()
    })
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
            vec![(Op::Rename, ".strata-replacement-*", 0, Errno::IO)]
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                fail,
                ..Quirks::default()
            },
        );
        let skipped = Rc::new(RefCell::new(Vec::new()));
        let reported = skipped.clone();
        let progress = TransferProgressTracker::new(
            OperationRequestId(1),
            None,
            None,
            Rc::new(move |event| {
                if let OperationEvent::ItemsReported { report, .. } = event {
                    reported.borrow_mut().extend(report.skipped);
                }
            }),
        );

        let result = glib::MainContext::default().block_on(replace_local_with(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            false,
            gio::Cancellable::new(),
            None,
            Copying {
                progress: Some(progress),
                names: NamePolicy::default(),
                ..Default::default()
            },
            &|_, _| {},
        ));

        assert_eq!(result.is_ok(), published, "{result:?}");
        let expected = if published {
            vec![crate::services::SkippedItem {
                location: Location::local(target.join("socket")),
                kind: crate::services::SpecialKind::Socket,
            }]
        } else {
            Vec::new()
        };
        assert_eq!(*skipped.borrow(), expected, "published: {published}");
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
            reused_inodes: Some(".strata-replacement-*"),
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

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::NoSpace)),
        "{result:?}"
    );
    assert_eq!(fs::read(&target)?, b"original");
    assert_eq!(names(root.path())?, ["source.txt", "target.txt"]);
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

        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
            "{directory}: {result:?}"
        );
        assert_eq!(read_item(&target, directory)?, b"original");
        assert_eq!(names(root.path())?, ["source", "target"], "{directory}");
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
    let cancel =
        |cancellable: gio::Cancellable| -> Racer { Box::new(move || cancellable.cancel()) };
    // The staged file's last step is its close: once linked, an unnamed
    // file's close names it by its inode. Without Trash, nothing after
    // staging checks the cancellation.
    let _scope = FsQuirks::install(
        root.path(),
        without_trash(
            TMPFS_MAGIC,
            Quirks {
                before: vec![
                    (Op::Close, "#*", 0, cancel(cancellable.clone())),
                    (
                        Op::Close,
                        ".strata-replacement-*",
                        0,
                        cancel(cancellable.clone()),
                    ),
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

    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)),
        "{result:?}"
    );
    assert_eq!(fs::read(&target)?, b"original");
    assert_eq!(names(root.path())?, ["source.txt", "target.txt"]);
    Ok(())
}

#[test]
fn staged_file_replacement_commits_then_removes_a_moved_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-success-test-{unique}"));
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root)?;
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original")?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(&target)?, b"replacement");
    assert!(!source.exists());
    assert_eq!(fs::read_dir(&root)?.count(), 1);
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

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read_link(&target)?, Path::new("new-target"));
    assert_eq!(fs::read_link(&source)?, Path::new("new-target"));
    Ok(())
}

fn substitute(path: &Path, kept: &Path) -> Racer {
    let (path, kept) = (path.to_path_buf(), kept.to_path_buf());
    Box::new(move || {
        fs::rename(&path, &kept).expect("move the item away");
        fs::write(&path, b"new arrival").expect("save a new item");
    })
}

#[test]
fn replacement_move_does_not_delete_a_substituted_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir(&sources)?;
    fs::create_dir(&destination)?;
    let source = sources.join("source.txt");
    let original_source = sources.join("original-source.txt");
    let target = destination.join("target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original target")?;
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![(Op::Write, "*", 0, substitute(&source, &original_source))],
            ..Quirks::default()
        },
    );

    let reports = Rc::new(RefCell::new(Vec::new()));
    let reported = reports.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| {
            if let OperationEvent::ItemsReported { report, .. } = event {
                reported.borrow_mut().extend(report.kept);
            }
        }),
    );

    glib::MainContext::default().block_on(replace_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        true,
        gio::Cancellable::new(),
        None,
        Copying {
            progress: Some(progress),
            names: NamePolicy::default(),
            ..Default::default()
        },
        &|_, _| {},
    ))?;

    assert_eq!(fs::read(target)?, b"replacement");
    assert_eq!(fs::read(&source)?, b"new arrival");
    assert_eq!(fs::read(original_source)?, b"replacement");
    assert_eq!(
        *reports.borrow(),
        [crate::services::KeptItem {
            location: Location::local(&source),
            cause: crate::services::KeptCause::Changed,
        }]
    );
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
fn replacement_stops_before_exchanging_a_substituted_target() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    let original_target = root.path().join("original-target.txt");
    fs::write(&source, b"replacement")?;
    fs::write(&target, b"original target")?;
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(Op::Clone, "*", 0, substitute(&target, &original_target))],
            ..Quirks::default()
        },
    );

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    let error = result.expect_err("a substituted target must fail identity validation");
    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(target)?, b"new arrival");
    assert_eq!(fs::read(original_target)?, b"original target");
    assert_eq!(
        names(root.path())?,
        ["original-target.txt", "source.txt", "target.txt"]
    );
    Ok(())
}

#[test]
fn cancelled_replacement_move_tracks_the_modified_source_and_target_roots()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replacement-move-cancel-test-{unique}"));
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

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(affected_locations.contains(&Location::local(&source)));
    assert!(affected_locations.contains(&Location::local(&target)));
    assert_eq!(fs::read(target.join("new/item.txt"))?, b"replacement");
    assert!(source.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn staged_directory_replacement_does_not_merge_old_contents() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-directory-replacement-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("new"))?;
    fs::create_dir_all(target.join("old"))?;
    fs::write(source.join("new/item.txt"), b"new")?;
    fs::write(target.join("old/item.txt"), b"old")?;

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("new/item.txt"))?, b"new");
    assert!(!target.join("old").exists());
    assert!(source.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn replacing_a_directory_cleans_up_a_symlink_in_the_old_contents_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-replace-symlink-cleanup-test-{unique}"));
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

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(fs::read(target.join("item.txt"))?, b"new");
    assert!(
        !target.join("keep.txt").exists(),
        "the replaced directory's old contents must be gone"
    );
    assert!(
        !target.join("decoy").exists() && !target.join("decoy").is_symlink(),
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

#[test]
fn replace_without_trash_or_exchange_sets_the_original_aside() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let mounts: [(FsWord, Preset); 3] = [
        (NFS_SUPER_MAGIC, Quirks::without_rename_flags),
        (FUSE_SUPER_MAGIC, Quirks::sshfs),
        (FUSE_SUPER_MAGIC, Quirks::vboxsf),
    ];
    for (fs_type, quirks) in mounts {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let source = root.path().join("source");
            let target = root.path().join("target");
            write_item(&source, directory, b"new")?;
            write_item(&target, directory, b"old")?;
            let quirks = FsQuirks::install(root.path(), without_trash(fs_type, quirks()));

            let (result, replaced) = replace_reporting(&source, &target, false);

            assert!(result.is_ok(), "{result:?}");
            assert_eq!(replaced, [ReplacedOriginal::Deleted]);
            assert_eq!(read_item(&target, directory)?, b"new");
            assert_eq!(read_item(&source, directory)?, b"new");
            assert_eq!(names(root.path())?, ["source", "target"]);
            assert!(quirks.used_name(".strata-replaced-*"));
            assert_eq!(quirks.count(Op::Exchange), 0);
        }
    }
    Ok(())
}

#[test]
fn replace_falls_back_to_aside_when_trash_lies() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (trash, move_source) in [
        (Trash::FalseWithoutError, false),
        (Trash::SucceedsWithoutMoving, true),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash,
                fs_type: Some(NFS_SUPER_MAGIC),
                ..Quirks::default()
            },
        );

        let (result, replaced) = replace_reporting(&source, &target, move_source);

        assert!(result.is_ok(), "{trash:?}: {result:?}");
        assert_eq!(quirks.count(Op::Trash), 1);
        assert_eq!(replaced, [ReplacedOriginal::Deleted]);
        assert_eq!(fs::read(&target)?, b"new");
        assert!(quirks.used_name(".strata-replaced-*"));
        assert_eq!(
            names(root.path())?,
            if move_source {
                vec!["target"]
            } else {
                vec!["source", "target"]
            }
        );
    }
    Ok(())
}

#[test]
fn replace_never_uses_exchange_on_fuse() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            FUSE_SUPER_MAGIC,
            Quirks {
                exchange: Exchange::IgnoredOverwrites,
                ..Quirks::default()
            },
        ),
    );

    let (result, _) = replace_reporting(&source, &target, false);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(quirks.count(Op::Exchange), 0);
    assert_eq!(fs::read(&target)?, b"new");
    assert_eq!(names(root.path())?, ["source", "target"]);
    Ok(())
}

#[test]
fn rejected_exchange_falls_back_to_aside_and_is_remembered() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            TMPFS_MAGIC,
            Quirks {
                exchange: Exchange::Rejected,
                ..Quirks::default()
            },
        ),
    );

    for exchanges in [1, 0] {
        fs::write(&target, b"old")?;
        quirks.clear_calls();

        let (result, replaced) = replace_reporting(&source, &target, false);

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(replaced, [ReplacedOriginal::Deleted]);
        assert_eq!(quirks.count(Op::Exchange), exchanges);
        assert_eq!(fs::read(&target)?, b"new");
        assert_eq!(names(root.path())?, ["source", "target"]);
    }
    assert_eq!(mount_capabilities(quirks.key()).exchange, Some(false));
    Ok(())
}

#[test]
fn a_failed_publication_puts_the_original_back() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for (directory, quirks, op) in [
        (false, Quirks::default(), Op::Rename),
        (true, Quirks::default(), Op::Rename),
        (false, Quirks::without_rename_flags(), Op::Link),
    ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        write_item(&source, directory, b"new")?;
        write_item(&target, directory, b"old")?;
        let original = inode(&target)?;
        // The first match sets the original aside; the second publishes.
        let quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    fail: vec![(op, "target", 1, Errno::IO)],
                    ..quirks
                },
            ),
        );

        let (result, replaced) = replace_reporting(&source, &target, false);

        assert!(result.is_err(), "{directory} {op:?}");
        assert!(replaced.is_empty());
        assert!(quirks.used_name(".strata-replaced-*"));
        assert_eq!(inode(&target)?, original);
        assert_eq!(read_item(&target, directory)?, b"old");
        assert_eq!(names(root.path())?, ["source", "target"]);
    }
    Ok(())
}

#[test]
fn a_taken_name_keeps_the_original_aside_and_names_it() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The racer writes just before the new item is published at `target`, or
    // saves over it just as the original is set aside, which sets the save aside.
    let cases: [(Preset, Op, usize, bool, bool); 4] = [
        (Quirks::default, Op::Rename, 1, false, false),
        (Quirks::vboxsf, Op::Create, 0, false, false),
        (Quirks::default, Op::Rename, 0, true, false),
        (Quirks::default, Op::Rename, 0, true, true),
    ];
    for (quirks, op, skip, saves_over, move_source) in cases {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let (racer_target, saved) = (target.clone(), root.path().join("saved"));
        let racer: Box<dyn FnMut() + Send> = Box::new(move || {
            if saves_over {
                let _ = fs::write(&saved, b"racer");
                let _ = fs::rename(&saved, &racer_target);
            } else {
                let _ = fs::write(&racer_target, b"racer");
            }
        });
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    before: vec![(op, "target", skip, racer)],
                    ..quirks()
                },
            ),
        );

        let (result, _) = replace_reporting(&source, &target, move_source);

        let row = format!("{op:?}, saves over: {saves_over}, moving: {move_source}");
        let error = result.expect_err("the racer keeps its item");
        let kept = names(root.path())?
            .into_iter()
            .find(|name| name.starts_with(".strata-replaced-"))
            .ok_or("the original was not kept")?;
        assert!(error.message().contains(&kept), "{row}: {error}");
        if saves_over {
            assert_eq!(fs::read(root.path().join(&kept))?, b"racer", "{row}");
            assert_eq!(names(root.path())?, [kept.as_str(), "source"], "{row}");
        } else {
            assert_eq!(fs::read(root.path().join(&kept))?, b"old", "{row}");
            assert_eq!(fs::read(&target)?, b"racer", "{row}");
            assert_eq!(
                names(root.path())?,
                [kept.as_str(), "source", "target"],
                "{row}"
            );
        }
        assert_eq!(fs::read(&source)?, b"new", "{row}");
    }
    Ok(())
}

#[test]
fn replace_fails_before_staging_when_no_strategy_is_safe() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"new")?;
    fs::write(&target, b"old")?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(FUSE_SUPER_MAGIC, Quirks::without_placement()),
    );

    let result = glib::MainContext::default().block_on(replace_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        false,
        gio::Cancellable::new(),
        None,
    ));

    let error = result.expect_err("no safe way to replace");
    assert!(error.message().contains("replaced safely"), "{error}");
    assert_eq!(quirks.count(Op::OpenSource), 0);
    assert_eq!(fs::read(&target)?, b"old");
    assert_eq!(names(root.path())?, ["source", "target"]);
    Ok(())
}

#[test]
fn the_original_is_deleted_only_once_its_replacement_is_flushed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for flushed in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"old")?;
        let aside_during_flush = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (record, watched) = (aside_during_flush.clone(), root.path().to_path_buf());
        let racer: super::fs_quirks::Racer = Box::new(move || {
            let kept = names(&watched).is_ok_and(|names| {
                names
                    .iter()
                    .any(|name| name.starts_with(".strata-replaced-"))
            });
            record
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(kept);
        });
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    before: vec![(Op::SyncFs, "*", 0, racer)],
                    fail: if flushed {
                        Vec::new()
                    } else {
                        vec![(Op::SyncFs, "*", 0, Errno::IO)]
                    },
                    ..Quirks::default()
                },
            ),
        );

        let (result, _) = replace_reporting(&source, &target, false);

        assert_eq!(
            *aside_during_flush
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            [true],
            "flushed: {flushed}"
        );
        assert_eq!(fs::read(&target)?, b"new");
        if flushed {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(names(root.path())?, ["source", "target"]);
            continue;
        }
        let aside = names(root.path())?
            .into_iter()
            .find(|name| name.starts_with(".strata-replaced-"))
            .ok_or("the original stays aside")?;
        assert_eq!(fs::read(root.path().join(&aside))?, b"old");
        let error = result.expect_err("the original is kept");
        assert!(
            error.message().contains(&aside)
                && error.message().contains("writes may still be pending"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_places_the_source_without_a_stage() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let mounts: [(FsWord, Preset); 3] = [
        (NFS_SUPER_MAGIC, Quirks::default),
        (FUSE_SUPER_MAGIC, Quirks::sshfs),
        (FUSE_SUPER_MAGIC, Quirks::vboxsf),
    ];
    for (fs_type, quirks) in mounts {
        for directory in [false, true] {
            let root = tempfile::tempdir()?;
            let sources = root.path().join("sources");
            let destination = root.path().join("destination");
            fs::create_dir(&sources)?;
            fs::create_dir(&destination)?;
            let source = sources.join("item");
            let target = destination.join("item");
            write_item(&source, directory, b"new")?;
            write_item(&target, directory, b"old")?;
            let moved = inode(&source)?;
            let quirks = FsQuirks::install(root.path(), without_trash(fs_type, quirks()));

            let (result, replaced) = replace_reporting(&source, &target, true);

            assert!(result.is_ok(), "{result:?}");
            assert_eq!(replaced, [ReplacedOriginal::Deleted]);
            assert_eq!(inode(&target)?, moved);
            assert_eq!(read_item(&target, directory)?, b"new");
            assert!(!source.exists());
            assert_eq!(names(&destination)?, ["item"]);
            assert!(!quirks.used_name(".strata-replacement-*"));
        }
    }
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_refuses_a_substituted_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for quirks in [Quirks::default, Quirks::without_rename_flags] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let preserved = root.path().join("preserved");
        let target = root.path().join("target");
        fs::write(&source, b"incoming")?;
        fs::write(&target, b"old")?;
        let original = inode(&target)?;
        let (racer_source, racer_preserved) = (source.clone(), preserved.clone());
        let racer: Box<dyn FnMut() + Send> = Box::new(move || {
            let _ = fs::rename(&racer_source, &racer_preserved);
            let _ = fs::write(&racer_source, b"new arrival");
        });
        let _quirks = FsQuirks::install(
            root.path(),
            without_trash(
                NFS_SUPER_MAGIC,
                Quirks {
                    // As the original is set aside, before the source moves.
                    before: vec![(Op::Rename, "target", 0, racer)],
                    ..quirks()
                },
            ),
        );

        let (result, replaced) = replace_reporting(&source, &target, true);

        let error = result.expect_err("a substituted source is refused");
        assert!(error.message().contains("changed"), "{error}");
        assert!(replaced.is_empty());
        assert_eq!(inode(&target)?, original);
        assert_eq!(fs::read(&target)?, b"old");
        assert_eq!(fs::read(&source)?, b"new arrival");
        assert_eq!(fs::read(&preserved)?, b"incoming");
        assert_eq!(names(root.path())?, ["preserved", "source", "target"]);
    }
    Ok(())
}

#[test]
fn same_filesystem_replacing_move_restores_the_original_when_placement_fails()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"incoming")?;
    fs::write(&target, b"old")?;
    let original = inode(&target)?;
    let quirks = FsQuirks::install(
        root.path(),
        without_trash(
            NFS_SUPER_MAGIC,
            Quirks {
                fail: vec![(Op::Rename, "source", 0, Errno::IO)],
                ..Quirks::default()
            },
        ),
    );

    let (result, replaced) = replace_reporting(&source, &target, true);

    assert!(result.is_err());
    assert!(replaced.is_empty());
    assert!(quirks.used_name(".strata-replaced-*"));
    assert_eq!(inode(&target)?, original);
    assert_eq!(fs::read(&target)?, b"old");
    assert_eq!(fs::read(&source)?, b"incoming");
    assert_eq!(names(root.path())?, ["source", "target"]);
    Ok(())
}

#[test]
fn a_replacing_move_on_ntfs_3g_renames_names_it_would_cut_or_puts_the_original_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (incoming, name) = ("y".repeat(237), "x".repeat(237));
    for (directory, placing_fails) in [(true, false), (false, false), (true, true)] {
        let case = format!("folder: {directory}, placing fails: {placing_fails}");
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir(&sources)?;
        fs::create_dir(&destination)?;
        let source = sources.join(&incoming);
        let target = destination.join(&name);
        write_item(&source, directory, b"new")?;
        write_item(&target, directory, b"old")?;
        let (moved, original) = (inode(&source)?, inode(&target)?);
        // Skips the refused RENAME_NOREPLACE, failing the rename once the name is found free.
        let fail = if placing_fails {
            vec![(Op::Rename, "y*", 1, Errno::ACCESS)]
        } else {
            Vec::new()
        };
        let quirks = FsQuirks::install(
            root.path(),
            without_trash(
                FUSE_SUPER_MAGIC,
                Quirks {
                    rename_flags: Support::Rejected(Errno::INVAL),
                    fail,
                    ..Quirks::ntfs_3g()
                },
            ),
        );

        let (result, replaced) = replace_reporting(&source, &target, true);

        assert_eq!(result.is_err(), placing_fails, "{case}: {result:?}");
        let (kept, contents, reported) = if placing_fails {
            (original, b"old", Vec::new())
        } else {
            (moved, b"new", vec![ReplacedOriginal::Deleted])
        };
        assert_eq!(replaced, reported, "{case}");
        assert_eq!(inode(&target)?, kept, "{case}");
        assert_eq!(read_item(&target, directory)?, contents, "{case}");
        assert_eq!(source.exists(), placing_fails, "{case}");
        assert_eq!(names(&destination)?, [name.as_str()], "{case}");
        assert!(!quirks.used_name(".strata-replacement-*"), "{case}");
    }
    Ok(())
}

#[test]
fn a_true_alias_is_a_no_op_but_a_hard_link_elsewhere_is_replaced() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let photo = root.path().join("Photo.txt");
    let link = root.path().join("link.txt");
    fs::write(&photo, b"photo")?;
    fs::hard_link(&photo, &link)?;
    let original = inode(&photo)?;
    let _quirks = FsQuirks::install(
        root.path(),
        without_trash(
            NFS_SUPER_MAGIC,
            Quirks {
                case_insensitive: true,
                ..Quirks::default()
            },
        ),
    );

    let (aliased, aliased_replaced) =
        replace_reporting(&photo, &root.path().join("photo.txt"), false);
    let (linked, link_replaced) = replace_reporting(&photo, &link, false);

    assert!(aliased.is_ok(), "{aliased:?}");
    assert!(aliased_replaced.is_empty());
    assert!(linked.is_ok(), "{linked:?}");
    assert_eq!(link_replaced, [ReplacedOriginal::Deleted]);
    assert_eq!(inode(&photo)?, original);
    assert_eq!(fs::symlink_metadata(&photo)?.nlink(), 1);
    assert_ne!(inode(&link)?, original);
    assert_eq!(fs::read(&link)?, b"photo");
    assert_eq!(names(root.path())?, ["Photo.txt", "link.txt"]);
    Ok(())
}
