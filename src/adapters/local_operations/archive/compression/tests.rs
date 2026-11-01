// SPDX-License-Identifier: MIT

mod cancellation;
mod policy;

use super::super::fixtures::{
    COMPRESSION_STAGE, HomeTrashGuard, compression_stage_mode, names, never_cancelled,
    set_times_without_following, stages, tempdir_on_home_device, write_compression_fixture,
};
use super::{ArchiveError, inspect_archive_sources, process_umask, write_staged_archive};
use crate::{
    adapters::local_operations::tests::fs_quirks::{
        FsQuirks, Op, Preset, Quirks, Trash, evict, recreated,
    },
    services::{ArchiveFormat, ReplacedOriginal, TransferConflict},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};
use gtk::glib;
use policy::assert_seven_z_methods;
use rustix::io::Errno;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    fmt::Debug,
    fs,
    io::{Read, Write},
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

const HOLD_LIMIT: Duration = Duration::from_secs(30);

/// Holds a staged compression's writer on its worker until the test releases it, which
/// dropping this does too, so a failed assertion never leaves the worker spinning.
#[derive(Default)]
struct HeldWriter {
    held: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    spawned: std::cell::Cell<bool>,
}

impl HeldWriter {
    fn hold(&self) -> impl Fn() -> Result<(), String> + Send + 'static {
        let (held, released) = (self.held.clone(), self.released.clone());
        move || {
            held.store(true, Ordering::Release);
            let deadline = Instant::now() + HOLD_LIMIT;
            while !released.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err("the test never released the writer".to_owned());
                }
                std::thread::yield_now();
            }
            Ok(())
        }
    }

    fn spawn<T: 'static>(
        &self,
        compression: impl Future<Output = T> + 'static,
    ) -> glib::JoinHandle<T> {
        let ended = self.ended.clone();
        self.spawned.set(true);
        glib::MainContext::default().spawn_local(async move {
            let compressed = compression.await;
            ended.store(true, Ordering::Release);
            compressed
        })
    }

    /// Fails fast if the compression ends before its writer ran.
    fn wait_until_held<T: Debug + 'static>(
        &self,
        task: &mut glib::JoinHandle<T>,
    ) -> Result<(), String> {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + HOLD_LIMIT;
        while !self.held.load(Ordering::Acquire) {
            if self.ended.load(Ordering::Acquire) {
                return Err(format!(
                    "the compression ended before its writer ran: {:?}",
                    context.block_on(task)
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!("the writer was not reached within {HOLD_LIMIT:?}"));
            }
            context.iteration(false);
            std::thread::yield_now();
        }
        Ok(())
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
    }
}

impl Drop for HeldWriter {
    fn drop(&mut self) {
        self.release();
        // The spawned task belongs to this thread: run it to its end here, or the
        // next test to iterate the default context finalizes it on another thread.
        let context = glib::MainContext::default();
        let deadline = Instant::now() + HOLD_LIMIT;
        while self.spawned.get() && !self.ended.load(Ordering::Acquire) && Instant::now() < deadline
        {
            context.iteration(false);
            std::thread::yield_now();
        }
    }
}

#[test]
fn an_archive_replaced_immediately_after_publication_is_not_owned_for_undo()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for edited in [false, true] {
        let root = tempdir_on_home_device()?;
        let archive = root.path().join("existing.zip");
        let _trash = HomeTrashGuard::new(&archive);
        fs::write(&archive, b"original")?;
        let arrival = root.path().join("arrival");
        fs::write(&arrival, b"other program")?;
        let over = archive.clone();
        let racer = Box::new(move || {
            if edited {
                fs::write(&over, b"other program").expect("edit published archive");
            } else {
                fs::rename(&arrival, &over).expect("replace published archive");
            }
        });
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                after: vec![(Op::Rename, "existing.zip", 0, racer)],
                ..Quirks::default()
            },
        );
        let result = glib::MainContext::default().block_on(write_staged_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            |mut file| {
                file.write_all(b"archive")
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        ));
        let (_, replaced, placed) = result?;
        assert!(
            matches!(replaced, Some(ReplacedOriginal::Trashed(_))),
            "{replaced:?}"
        );
        assert!(placed.is_none());
        assert_eq!(fs::read(&archive)?, b"other program");
    }
    Ok(())
}

#[test]
fn compression_staging_stays_private_while_encoding() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempdir_on_home_device()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    let _trash = HomeTrashGuard::new(&archive);
    fs::write(&archive, b"original")?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
    let original = {
        let archive = archive.clone();
        std::thread::spawn(move || {
            crate::adapters::local_operations::native_fs::identity_at_path(&archive)
        })
        .join()
        .ok()
        .flatten()
        .ok_or("identity")?
    };
    let writer = HeldWriter::default();
    let hold = writer.hold();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let mut task = writer.spawn(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            move |mut file| {
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                Ok(hold()?)
            },
        )
        .await
    });
    writer.wait_until_held(&mut task)?;
    assert_eq!(compression_stage_mode(&destination)?, 0o600);

    writer.release();
    let (name, replaced, placed) = glib::MainContext::default().block_on(task)??;
    assert_eq!(name, "existing.zip");
    assert!(
        matches!(replaced, Some(ReplacedOriginal::Trashed(Some(trashed))) if trashed.same(&original)),
        "{replaced:?}"
    );
    assert!(placed.is_some(), "the archive is recorded for undo");
    assert_eq!(fs::read(&archive)?, b"replacement");
    assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn compression_new_archive_staging_stays_private_until_publish() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("created.zip");
    let writer = HeldWriter::default();
    let hold = writer.hold();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let mut task = writer.spawn(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            move |mut file| {
                file.write_all(b"created")
                    .map_err(|error| error.to_string())?;
                Ok(hold()?)
            },
        )
        .await
    });
    writer.wait_until_held(&mut task)?;
    assert_eq!(compression_stage_mode(&destination)?, 0o600);

    writer.release();
    assert_eq!(
        glib::MainContext::default().block_on(task)?,
        Ok(("created.zip".to_owned(), None, None))
    );
    assert_eq!(fs::read(&archive)?, b"created");
    assert_eq!(
        fs::metadata(&archive)?.permissions().mode() & 0o777,
        0o666 & !process_umask()
    );
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn compression_replacement_preserves_directory_destinations() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for populated in [false, true] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::create_dir(&archive)?;
        if populated {
            fs::write(archive.join("contents"), b"original")?;
        }
        let result = glib::MainContext::default().block_on(write_staged_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            |mut file| {
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        ));

        assert!(matches!(result, Err(ArchiveError::Failed(_))));
        assert!(archive.is_dir());
        if populated {
            assert_eq!(fs::read(archive.join("contents"))?, b"original");
        }
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[test]
fn keep_both_retries_publication_collisions_without_encoding_again() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    for extension in ["zip", "7z", "tar", "tar.gz"] {
        let destination = root.path().join(extension);
        fs::create_dir(&destination)?;
        let archive = destination.join(format!("archive.part.{extension}"));
        let first = destination.join(format!("archive.part (1).{extension}"));
        let directory = destination.join(format!("archive.part (2).{extension}"));
        let symlink = destination.join(format!("archive.part (3).{extension}"));
        fs::create_dir(&directory)?;
        std::os::unix::fs::symlink("missing", &symlink)?;
        let encoded = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_encoded = encoded.clone();
        let late_archive = archive.clone();
        let late_first = first.clone();
        let (published, original, _) =
            glib::MainContext::default().block_on(write_staged_archive(
                &destination,
                &archive,
                TransferConflict::KeepBoth,
                never_cancelled(),
                None,
                move |mut file| {
                    worker_encoded.fetch_add(1, Ordering::Relaxed);
                    file.write_all(b"new archive")
                        .map_err(|error| error.to_string())?;
                    fs::write(late_archive, b"late original").map_err(|error| error.to_string())?;
                    fs::write(late_first, b"late numbered").map_err(|error| error.to_string())?;
                    Ok(())
                },
            ))?;
        assert_eq!(published, format!("archive.part (4).{extension}"));
        assert!(original.is_none());
        assert_eq!(encoded.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read(destination.join(published))?, b"new archive");
        assert_eq!(fs::read(&archive)?, b"late original");
        assert_eq!(fs::read(&first)?, b"late numbered");
        assert!(directory.is_dir());
        assert_eq!(fs::read_link(symlink)?, Path::new("missing"));
        assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[test]
fn keep_both_numbers_a_long_archive_name_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    for (name, numbered) in [
        (
            format!("{}.zip", "c".repeat(251)),
            format!("{} (1).zip", "c".repeat(247)),
        ),
        (
            format!("{}.tar.gz", "c".repeat(248)),
            format!("{} (1).tar.gz", "c".repeat(244)),
        ),
    ] {
        let archive = root.path().join(&name);
        fs::write(&archive, b"existing")?;

        let (published, _, _) = glib::MainContext::default().block_on(write_staged_archive(
            root.path(),
            &archive,
            TransferConflict::KeepBoth,
            never_cancelled(),
            None,
            |mut file| {
                file.write_all(b"new archive")
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        ))?;

        assert_eq!(published, numbered);
        assert_eq!(fs::read(root.path().join(&numbered))?, b"new archive");
        assert_eq!(fs::read(&archive)?, b"existing");
    }
    Ok(())
}

#[test]
fn keep_both_tries_every_numbered_name_up_to_its_bound() -> Result<(), Box<dyn Error>> {
    use crate::adapters::local_operations::placement::NUMBERED_NAME_ATTEMPTS;
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("archive.zip");
    fs::write(&archive, b"existing")?;
    let last = NUMBERED_NAME_ATTEMPTS - 1;
    let _taken = FsQuirks::install(
        root.path(),
        Quirks {
            invalid_name: Some(|name| {
                let number: u64 = name
                    .to_str()?
                    .strip_prefix("archive (")?
                    .strip_suffix(").zip")?
                    .parse()
                    .ok()?;
                (number < NUMBERED_NAME_ATTEMPTS - 1).then_some(Errno::EXIST)
            }),
            ..Quirks::default()
        },
    );

    let (published, _, _) = glib::MainContext::default().block_on(write_staged_archive(
        root.path(),
        &archive,
        TransferConflict::KeepBoth,
        never_cancelled(),
        None,
        |mut file| {
            file.write_all(b"new archive")
                .map_err(|error| error.to_string())?;
            Ok(())
        },
    ))?;

    assert_eq!(published, format!("archive ({last}).zip"));
    assert_eq!(fs::read(root.path().join(&published))?, b"new archive");
    assert_eq!(fs::read(&archive)?, b"existing");
    Ok(())
}

const NFS_SUPER_MAGIC: rustix::fs::FsWord = 0x6969;
const TMPFS_MAGIC: rustix::fs::FsWord = 0x0102_1994;
const FUSE_SUPER_MAGIC: rustix::fs::FsWord = 0x6573_5546;

type Compressed = Result<(String, Option<ReplacedOriginal>), ArchiveError>;

fn publish_replacement(destination: &Path, conflict: TransferConflict) -> (Compressed, bool) {
    let encoded = Arc::new(AtomicBool::new(false));
    let worker_encoded = encoded.clone();
    let result = glib::MainContext::default().block_on(write_staged_archive(
        destination,
        &destination.join("existing.zip"),
        conflict,
        never_cancelled(),
        None,
        move |mut file| {
            worker_encoded.store(true, Ordering::Relaxed);
            file.write_all(b"replacement")
                .map_err(|error| error.to_string())?;
            Ok(())
        },
    ));
    (
        result.map(|(name, original, _)| (name, original)),
        encoded.load(Ordering::Relaxed),
    )
}

#[test]
fn an_archive_replaced_without_trash_deletes_its_original() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let cases: [(Trash, rustix::fs::FsWord, Preset, bool); 5] = [
        (Trash::Unsupported, NFS_SUPER_MAGIC, Quirks::default, true),
        (Trash::Unsupported, TMPFS_MAGIC, Quirks::default, false),
        (
            Trash::SucceedsWithoutMoving,
            NFS_SUPER_MAGIC,
            Quirks::default,
            true,
        ),
        (Trash::Unsupported, FUSE_SUPER_MAGIC, Quirks::sshfs, true),
        (Trash::Unsupported, FUSE_SUPER_MAGIC, Quirks::vboxsf, true),
    ];
    for (trash, fs_type, quirks, sets_aside) in cases {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o640))?;
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash,
                fs_type: Some(fs_type),
                ..quirks()
            },
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        assert_eq!(
            result,
            Ok(("existing.zip".to_owned(), Some(ReplacedOriginal::Deleted))),
            "{trash:?} {fs_type:#x}"
        );
        assert_eq!(fs::read(&archive)?, b"replacement");
        assert_eq!(fs::metadata(&archive)?.permissions().mode() & 0o777, 0o640);
        assert_eq!(names(root.path())?, ["existing.zip"]);
        assert!(quirks.used_name(".strata-originals-*"));
        assert_eq!(
            quirks.count(Op::Exchange),
            usize::from(!sets_aside),
            "{trash:?} {fs_type:#x}"
        );
    }
    Ok(())
}

#[test]
fn an_unplug_after_an_exchange_leaves_no_original_at_a_stage() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("existing.zip");
    fs::write(&archive, b"original")?;
    let quirks = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            fs_type: Some(TMPFS_MAGIC),
            unplugged_after: Some((Op::Exchange, "existing.zip", 0)),
            ..Quirks::default()
        },
    );

    let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

    assert!(result.is_err());
    quirks.replug();
    let names = names(root.path())?;
    let [originals, published] = names.as_slice() else {
        return Err(format!("the original and the archive: {names:?}").into());
    };
    assert!(originals.starts_with(".strata-originals-"), "{names:?}");
    assert_eq!(published, "existing.zip");
    assert_eq!(
        fs::read(root.path().join(originals).join("existing.zip"))?,
        b"original"
    );
    assert_eq!(fs::read(&archive)?, b"replacement");
    Ok(())
}

#[test]
fn an_archive_replaced_without_trash_is_on_disk_before_its_original_is_deleted()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    // The family, whether the barrier fails, and whether the archive is
    // synced itself, as the barrier cannot reach it.
    for (fs_type, fails, each_file) in [
        ("ext4", false, false),
        ("ext4", true, false),
        ("cifs", false, true),
    ] {
        let row = format!("{fs_type} fails: {fails}");
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let during = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (record, watched) = (during.clone(), root.path().to_path_buf());
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                fs_type: Some(TMPFS_MAGIC),
                mount_type: Some((fs_type, "rw")),
                before: vec![(
                    Op::SyncFs,
                    "*",
                    0,
                    Box::new(move || {
                        record
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push(names(&watched).map_or(0, |names| names.len()));
                    }),
                )],
                fail: if fails {
                    vec![(Op::SyncFs, "*", 0, Errno::IO)]
                } else {
                    Vec::new()
                },
                ..Quirks::default()
            },
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        assert_eq!(fs::read(&archive)?, b"replacement", "{row}");
        assert_eq!(quirks.count(Op::SyncData) == 1, each_file, "{row}");
        if each_file {
            assert!(
                during
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_empty()
            );
        } else {
            assert_eq!(
                *during.lock().unwrap_or_else(|error| error.into_inner()),
                [2],
                "{row}: the original stays until the barrier"
            );
        }
        if !fails {
            assert_eq!(
                result,
                Ok(("existing.zip".to_owned(), Some(ReplacedOriginal::Deleted))),
                "{row}"
            );
            assert_eq!(names(root.path())?, ["existing.zip"], "{row}");
            continue;
        }
        let kept = names(root.path())?
            .into_iter()
            .find(|name| name != "existing.zip")
            .ok_or("the original is kept")?;
        assert_eq!(
            fs::read(root.path().join(&kept).join("existing.zip"))?,
            b"original"
        );
        let Err(ArchiveError::Failed(message)) = result else {
            return Err(format!("{row}: {result:?}").into());
        };
        assert!(
            message.contains(&kept) && message.contains("writes may still be pending"),
            "{message}"
        );
    }
    Ok(())
}

#[test]
fn archives_are_created_without_noreplace_or_usable_links() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for quirks in [Quirks::sshfs, Quirks::vboxsf] {
        let root = tempfile::tempdir()?;
        let _quirks = FsQuirks::install(root.path(), quirks());

        let (result, encoded) = publish_replacement(root.path(), TransferConflict::FailIfExists);

        assert_eq!(result, Ok(("existing.zip".to_owned(), None)));
        assert!(encoded);
        assert_eq!(fs::read(root.path().join("existing.zip"))?, b"replacement");
        assert_eq!(names(root.path())?, ["existing.zip"]);
    }
    Ok(())
}

#[test]
fn replacement_fails_before_encoding_when_no_strategy_is_safe() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let archive = root.path().join("existing.zip");
    fs::write(&archive, b"original")?;
    let _quirks = FsQuirks::install(
        root.path(),
        Quirks {
            trash: Trash::Unsupported,
            fs_type: Some(NFS_SUPER_MAGIC),
            ..Quirks::without_placement()
        },
    );

    let (result, encoded) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

    let Err(ArchiveError::Failed(message)) = result else {
        return Err(format!("expected a refusal, got {result:?}").into());
    };
    assert!(message.contains("replaced safely"), "{message}");
    assert!(!encoded);
    assert_eq!(fs::read(&archive)?, b"original");
    assert_eq!(names(root.path())?, ["existing.zip"]);
    Ok(())
}

#[test]
fn an_archive_replaces_only_the_item_its_mount_still_knows() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let reused = |fs_type| {
        move || Quirks {
            fs_type: Some(fs_type),
            reused_inodes: Some("existing.zip"),
            ..Quirks::default()
        }
    };
    // NFS sets the original aside; tmpfs exchanges it.
    type MakeQuirks = Box<dyn Fn() -> Quirks>;
    let rows: [(MakeQuirks, bool); 3] = [
        (Box::new(reused(NFS_SUPER_MAGIC)), true),
        (Box::new(reused(TMPFS_MAGIC)), true),
        (Box::new(Quirks::vfat), false),
    ];
    for (quirks, swapped) in rows {
        let row = format!("{:?}, swapped: {swapped}", quirks().fs_type);
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                ..quirks()
            },
        );
        let changed = archive.clone();

        let result = glib::MainContext::default().block_on(write_staged_archive(
            root.path(),
            &archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            move |mut file| {
                if swapped {
                    fs::remove_file(&changed).map_err(|error| error.to_string())?;
                    fs::write(&changed, b"theirs").map_err(|error| error.to_string())?;
                    recreated(&changed);
                } else if !evict(&changed) {
                    return Err("the original was not evicted".to_owned().into());
                }
                file.write_all(b"replacement")
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        ));

        if swapped {
            let Err(ArchiveError::Failed(message)) = result else {
                return Err(format!("{row}: expected a refusal, got {result:?}").into());
            };
            assert!(message.contains("changed"), "{row}: {message}");
            assert_eq!(fs::read(&archive)?, b"theirs", "{row}");
        } else {
            assert!(result.is_ok(), "{row}: {result:?}");
            assert_eq!(fs::read(&archive)?, b"replacement", "{row}");
        }
        assert_eq!(names(root.path())?, ["existing.zip"], "{row}");
    }
    Ok(())
}

#[test]
fn a_name_taken_during_replacement_keeps_the_original_aside() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    // The first rename of the name sets the original aside; the second publishes.
    for (skip, saves_over) in [(1, false), (0, true)] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("existing.zip");
        fs::write(&archive, b"original")?;
        let (racer_archive, saved) = (archive.clone(), root.path().join("saved"));
        let _quirks = FsQuirks::install(
            root.path(),
            Quirks {
                trash: Trash::Unsupported,
                fs_type: Some(NFS_SUPER_MAGIC),
                before: vec![(
                    Op::Rename,
                    "existing.zip",
                    skip,
                    Box::new(move || {
                        if saves_over {
                            let _ = fs::write(&saved, b"racer");
                            let _ = fs::rename(&saved, &racer_archive);
                        } else {
                            let _ = fs::write(&racer_archive, b"racer");
                        }
                    }),
                )],
                ..Quirks::default()
            },
        );

        let (result, _) = publish_replacement(root.path(), TransferConflict::ReplaceExisting);

        let Err(ArchiveError::Failed(message)) = result else {
            return Err(format!("expected the original to be kept aside, got {result:?}").into());
        };
        let names = names(root.path())?;
        let [kept] = names
            .iter()
            .filter(|name| name.starts_with(".strata-originals-"))
            .collect::<Vec<_>>()[..]
        else {
            return Err(format!("expected one kept original, found {names:?}").into());
        };
        assert!(
            message.contains(&format!("{kept}/existing.zip")),
            "{saves_over}: {message}"
        );
        let original = root.path().join(kept).join("existing.zip");
        if saves_over {
            assert_eq!(fs::read(original)?, b"racer");
            assert_eq!(names, [kept.as_str()]);
        } else {
            assert_eq!(fs::read(original)?, b"original");
            assert_eq!(fs::read(&archive)?, b"racer");
        }
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[test]
fn keep_both_numbering_works_with_network_collision_errnos() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for sshfs in [true, false] {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("existing.zip"), b"original")?;
        fs::write(root.path().join("existing (1).zip"), b"numbered")?;
        let quirks = if sshfs {
            Quirks {
                collisions_as_eperm: true,
                ..Quirks::without_rename_flags()
            }
        } else {
            // SMB with POSIX extensions: the first stage name is held by a folder.
            Quirks {
                fail: vec![(Op::Create, ".strata-copy-*", 0, Errno::ISDIR)],
                ..Quirks::default()
            }
        };
        let _quirks = FsQuirks::install(root.path(), quirks);

        let (result, _) = publish_replacement(root.path(), TransferConflict::KeepBoth);

        assert_eq!(
            result,
            Ok(("existing (2).zip".to_owned(), None)),
            "sshfs: {sshfs}"
        );
        assert_eq!(
            fs::read(root.path().join("existing (2).zip"))?,
            b"replacement"
        );
        assert_eq!(fs::read(root.path().join("existing.zip"))?, b"original");
        assert_eq!(fs::read(root.path().join("existing (1).zip"))?, b"numbered");
        assert!(stages(root.path(), COMPRESSION_STAGE)?.is_empty());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum CompressedEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn read_compressed_entries(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<BTreeMap<PathBuf, CompressedEntry>, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    let mut result = BTreeMap::new();
    match format {
        ArchiveFormat::Zip => {
            let mut archive = zip::ZipArchive::new(file)?;
            for index in 0..archive.len() {
                let options =
                    zip::read::ZipReadOptions::new().password(password.map(str::as_bytes));
                let mut entry = archive.by_index_with_options(index, options)?;
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                let value = if entry.is_dir() {
                    CompressedEntry::Directory
                } else if entry.is_symlink() {
                    CompressedEntry::Symlink(PathBuf::from(OsString::from_vec(bytes)))
                } else {
                    CompressedEntry::File(bytes)
                };
                assert!(
                    result
                        .insert(PathBuf::from(&*entry.name().expect("member name")), value)
                        .is_none()
                );
            }
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            let reader: Box<dyn Read> = if format == ArchiveFormat::TarGz {
                Box::new(flate2::read::GzDecoder::new(file))
            } else {
                Box::new(file)
            };
            for entry in tar::Archive::new(reader).entries()? {
                let mut entry = entry?;
                let value = if entry.header().entry_type().is_dir() {
                    CompressedEntry::Directory
                } else if entry.header().entry_type().is_symlink() {
                    CompressedEntry::Symlink(
                        entry
                            .link_name()?
                            .ok_or("Missing link target")?
                            .into_owned(),
                    )
                } else {
                    assert!(entry.header().entry_type().is_file());
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes)?;
                    CompressedEntry::File(bytes)
                };
                assert!(result.insert(entry.path()?.into_owned(), value).is_none());
            }
        }
        ArchiveFormat::SevenZ => {
            let mut archive = sevenz_rust2::ArchiveReader::new(
                file,
                password
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default(),
            )?;
            archive.for_each_entries(|entry, reader| {
                let value = if entry.is_directory() {
                    CompressedEntry::Directory
                } else {
                    let mut bytes = Vec::new();
                    reader.read_to_end(&mut bytes)?;
                    CompressedEntry::File(bytes)
                };
                assert!(result.insert(PathBuf::from(entry.name()), value).is_none());
                Ok(true)
            })?;
        }
        ArchiveFormat::Rar => return Err("RAR compression is not supported".into()),
    }
    Ok(result)
}

#[test]
fn compression_preserves_links_in_zip_and_tar() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir(source.join("empty"))?;
    fs::write(source.join("file.txt"), b"file contents")?;
    fs::write(source.join("nested/child.txt"), b"child contents")?;
    let mut expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/nested"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/file.txt"),
            CompressedEntry::File(b"file contents".to_vec()),
        ),
        (
            PathBuf::from("source/nested/child.txt"),
            CompressedEntry::File(b"child contents".to_vec()),
        ),
    ]);
    for (name, target) in [
        ("file-link", "file.txt"),
        ("directory-link", "nested"),
        ("broken-link", "missing.txt"),
        ("current-directory-link", "."),
    ] {
        std::os::unix::fs::symlink(target, source.join(name))?;
        expected.insert(
            PathBuf::from("source").join(name),
            CompressedEntry::Symlink(PathBuf::from(target)),
        );
    }
    let selected_link = root.path().join("selected-link");
    std::os::unix::fs::symlink("source/nested", &selected_link)?;
    expected.insert(
        PathBuf::from("selected-link"),
        CompressedEntry::Symlink(PathBuf::from("source/nested")),
    );
    let entries = [source, selected_link];
    assert_eq!(
        inspect_archive_sources(&entries, &never_cancelled())?.files,
        7
    );
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
    ] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, password)?,
            7
        );
        assert_eq!(
            read_compressed_entries(&archive, format, password)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn seven_z_compression_preserves_mixed_methods_and_empty_directories() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("empty"))?;
    fs::write(source.join("one.txt"), b"one")?;
    fs::write(source.join("two.png"), b"two")?;
    let expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/one.txt"),
            CompressedEntry::File(b"one".to_vec()),
        ),
        (
            PathBuf::from("source/two.png"),
            CompressedEntry::File(b"two".to_vec()),
        ),
    ]);
    for password in [None, Some("test-password")] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(
                &archive,
                std::slice::from_ref(&source),
                ArchiveFormat::SevenZ,
                password
            )?,
            2
        );
        assert_eq!(
            read_compressed_entries(&archive, ArchiveFormat::SevenZ, password)?,
            expected,
        );
        assert_seven_z_methods(&archive, password, "source/one.txt", false)?;
        assert_seven_z_methods(&archive, password, "source/two.png", true)?;
        if password.is_some() {
            for wrong_password in [None, Some("wrong-password")] {
                assert!(
                    read_compressed_entries(&archive, ArchiveFormat::SevenZ, wrong_password)
                        .is_err()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn compression_handles_non_utf8_link_targets_without_loss() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let link = root.path().join("link");
    let target = PathBuf::from(OsString::from_vec(b"target-\xff".to_vec()));
    std::os::unix::fs::symlink(&target, &link)?;
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, std::slice::from_ref(&link), format, None)?;
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            BTreeMap::from([(
                PathBuf::from("link"),
                CompressedEntry::Symlink(target.clone())
            )])
        );
    }
    let error = write_compression_fixture(
        &root.path().join("archive.zip"),
        &[link],
        ArchiveFormat::Zip,
        None,
    )
    .expect_err("ZIP must reject a link target it cannot encode");
    assert!(error.contains("non-UTF-8 link target"));
    Ok(())
}

#[test]
fn cancelling_staged_compression_waits_for_worker_exit_before_cleanup() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("existing.zip");
    fs::write(&archive, b"original")?;
    let writer = HeldWriter::default();
    let hold = writer.hold();
    let finished = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let worker_cancelled = cancelled.clone();
    let worker_finished = finished.clone();
    let worker_destination = destination.clone();
    let worker_archive = archive.clone();
    let mut task = writer.spawn(async move {
        write_staged_archive(
            &worker_destination,
            &worker_archive,
            TransferConflict::ReplaceExisting,
            task_cancelled,
            None,
            move |mut file| {
                file.write_all(b"partial")
                    .map_err(|error| error.to_string())?;
                hold()?;
                worker_finished.store(true, Ordering::Release);
                super::check_archive_cancelled(&worker_cancelled)
            },
        )
        .await
    });
    writer.wait_until_held(&mut task)?;
    assert_eq!(stages(&destination, COMPRESSION_STAGE)?.len(), 1);

    cancelled.store(true, Ordering::Release);
    assert!(!finished.load(Ordering::Acquire));
    assert_eq!(stages(&destination, COMPRESSION_STAGE)?.len(), 1);
    writer.release();
    let result = glib::MainContext::default().block_on(task)?;
    assert!(matches!(result, Err(ArchiveError::Cancelled)));
    assert!(finished.load(Ordering::Acquire));
    assert!(stages(&destination, COMPRESSION_STAGE)?.is_empty());
    assert_eq!(fs::read(&archive)?, b"original");
    Ok(())
}

#[test]
fn staged_archives_are_not_published_after_cancellation_or_close_failure()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    for replacing in [false, true] {
        for (boundary, preset) in [
            ("encoding", Quirks::default as Preset),
            ("close", Quirks::default),
            ("close, then evicted", Quirks::vfat),
            ("close on SMB without serverino", Quirks::cifs_noserverino),
            (
                "close on SMB without serverino, actimeo=0",
                cifs_noserverino_without_attribute_cache,
            ),
            ("close error", Quirks::default),
        ] {
            let root = tempfile::tempdir()?;
            let evicted = boundary == "close, then evicted";
            if evicted && !base_keeps_written_times(root.path())? {
                eprintln!(
                    "skipped on a base that moves a written file's time after its descriptor \
                     showed it: {boundary}"
                );
                continue;
            }
            let archive = root.path().join("archive.zip");
            if replacing {
                fs::write(&archive, b"original")?;
            }
            let cancelled = Arc::new(AtomicBool::new(false));
            let on_close = cancelled.clone();
            let server_stamped = boundary.starts_with("close on SMB");
            let mut quirks = Quirks {
                trash: Trash::Unsupported,
                ..preset()
            };
            if boundary == "close" || evicted || server_stamped {
                let folder = root.path().to_path_buf();
                quirks.after.push((
                    Op::Close,
                    ".strata-copy-*",
                    0,
                    Box::new(move || {
                        if evicted {
                            let stage = stages(&folder, COMPRESSION_STAGE)
                                .ok()
                                .and_then(|mut found| found.pop())
                                .expect("the stage");
                            assert!(evict(&folder.join(stage)), "evict the stage");
                        }
                        on_close.store(true, Ordering::Release);
                    }),
                ));
            } else if boundary == "close error" {
                quirks
                    .fail
                    .push((Op::Close, ".strata-copy-*", 0, Errno::IO));
            }
            let _scope = FsQuirks::install(root.path(), quirks);
            let on_write = cancelled.clone();
            let result = glib::MainContext::default().block_on(write_staged_archive(
                root.path(),
                &archive,
                if replacing {
                    TransferConflict::ReplaceExisting
                } else {
                    TransferConflict::FailIfExists
                },
                cancelled,
                None,
                move |mut file| {
                    file.write_all(b"replacement")
                        .map_err(|error| error.to_string())?;
                    if server_stamped {
                        file.set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000))
                            .map_err(|error| error.to_string())?;
                    }
                    if boundary == "encoding" {
                        on_write.store(true, Ordering::Release);
                    }
                    Ok(())
                },
            ));
            if boundary == "close error" {
                assert!(
                    matches!(result, Err(ArchiveError::Failed(_))),
                    "{boundary}, replacing {replacing}: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(ArchiveError::Cancelled)),
                    "{boundary}, replacing {replacing}: {result:?}"
                );
            }
            if replacing {
                assert_eq!(fs::read(&archive)?, b"original");
            } else {
                assert!(!archive.exists());
            }
            assert!(
                stages(root.path(), COMPRESSION_STAGE)?.is_empty(),
                "{boundary}, replacing {replacing}"
            );
        }
    }
    Ok(())
}

fn cifs_noserverino_without_attribute_cache() -> Quirks {
    Quirks {
        mount_type: Some((
            "cifs",
            concat!(
                "rw,vers=3.1.1,cache=strict,uid=1000,forceuid,gid=1000,forcegid,soft,nounix,",
                "mapposix,reparse=nfs,nativesocket,symlink=native,actimeo=0,closetimeo=1"
            ),
        )),
        ..Quirks::cifs_noserverino()
    }
}

/// SMB shows a written file's server time only once it revalidates; NFSv4.2 sets it at close.
fn base_keeps_written_times(folder: &Path) -> Result<bool, Box<dyn Error>> {
    use rustix::fs::{AtFlags, CWD, StatxFlags, statx};
    use std::os::unix::fs::MetadataExt;
    let probe = folder.join("written-times-probe");
    let mut file = fs::File::create_new(&probe)?;
    file.write_all(b"probe")?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    let held = file.metadata()?;
    drop(file);
    let closed = statx(CWD, &probe, AtFlags::STATX_FORCE_SYNC, StatxFlags::MTIME)?;
    fs::remove_file(&probe)?;
    Ok((held.mtime(), held.mtime_nsec())
        == (closed.stx_mtime.tv_sec, i64::from(closed.stx_mtime.tv_nsec)))
}

#[test]
fn compression_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree"))?;
    fs::write(actual.join("tree/file.txt"), b"contents")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    let entries = [alias.join("tree")];
    let expected = BTreeMap::from([
        (PathBuf::from("tree"), CompressedEntry::Directory),
        (
            PathBuf::from("tree/file.txt"),
            CompressedEntry::File(b"contents".to_vec()),
        ),
    ]);

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::SevenZ,
    ] {
        let archive = root.path().join("archive");
        assert_eq!(
            inspect_archive_sources(&entries, &never_cancelled())?.files,
            1
        );
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, None)?,
            1
        );
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn zip_and_seven_z_refuse_non_utf8_names_instead_of_mangling_them() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root
        .path()
        .join(OsString::from_vec(b"name-\xff.txt".to_vec()));
    fs::write(&source, b"contents")?;
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let archive = root.path().join("archive.out");
        let error =
            write_compression_fixture(&archive, std::slice::from_ref(&source), format, None)
                .expect_err("a non-UTF-8 name cannot be stored losslessly");
        assert!(error.contains("non-UTF-8 name"), "{format:?}: {error}");
    }
    let archive = root.path().join("archive.tar");
    write_compression_fixture(
        &archive,
        std::slice::from_ref(&source),
        ArchiveFormat::Tar,
        None,
    )?;
    Ok(())
}

#[test]
fn encrypted_seven_z_archives_are_still_compressed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("zeros.bin");
    fs::write(&source, vec![0u8; 4 << 20])?;
    let plain = root.path().join("plain.7z");
    let encrypted = root.path().join("encrypted.7z");
    write_compression_fixture(
        &plain,
        std::slice::from_ref(&source),
        ArchiveFormat::SevenZ,
        None,
    )?;
    write_compression_fixture(
        &encrypted,
        std::slice::from_ref(&source),
        ArchiveFormat::SevenZ,
        Some("secret"),
    )?;
    let plain_len = fs::metadata(&plain)?.len();
    let encrypted_len = fs::metadata(&encrypted)?.len();
    assert!(
        encrypted_len < 1 << 20,
        "encrypted archive should compress ({encrypted_len} bytes, plain {plain_len} bytes)"
    );
    Ok(())
}

const SOURCE_TIME: i64 = 1_000_000_000;

#[derive(PartialEq, Eq)]
struct CompressedMetadata {
    mode: Option<u32>,
    modified: Option<i64>,
}

impl std::fmt::Debug for CompressedMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.mode {
            Some(mode) => write!(formatter, "mode {mode:o}, ")?,
            None => formatter.write_str("no mode, ")?,
        }
        match self.modified {
            Some(seconds) => write!(formatter, "modified {seconds}"),
            None => formatter.write_str("no time"),
        }
    }
}

fn zip_dos_seconds(time: zip::DateTime) -> Result<i64, Box<dyn Error>> {
    Ok(glib::DateTime::from_local(
        i32::from(time.year()),
        i32::from(time.month()),
        i32::from(time.day()),
        i32::from(time.hour()),
        i32::from(time.minute()),
        f64::from(time.second()),
    )?
    .to_unix())
}

fn zip_extended_time(entry: &zip::read::ZipFile<'_, fs::File>) -> Option<i64> {
    entry.extra_data_fields().find_map(|field| match field {
        zip::ExtraField::ExtendedTimestamp(stamp) => stamp.mod_time().map(i64::from),
        _ => None,
    })
}

fn read_compressed_metadata(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<BTreeMap<PathBuf, CompressedMetadata>, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    let mut result = BTreeMap::new();
    match format {
        ArchiveFormat::Zip => {
            let mut archive = zip::ZipArchive::new(file)?;
            for index in 0..archive.len() {
                let options =
                    zip::read::ZipReadOptions::new().password(password.map(str::as_bytes));
                let entry = archive.by_index_with_options(index, options)?;
                let modified = match zip_extended_time(&entry) {
                    Some(seconds) => Some(seconds),
                    None => entry.last_modified().map(zip_dos_seconds).transpose()?,
                };
                let metadata = CompressedMetadata {
                    mode: entry.unix_mode().map(|mode| mode & 0o7777),
                    modified,
                };
                let name = PathBuf::from(entry.name().expect("member name").trim_end_matches('/'));
                assert!(result.insert(name, metadata).is_none());
            }
        }
        ArchiveFormat::Tar | ArchiveFormat::TarGz => {
            let reader: Box<dyn Read> = if format == ArchiveFormat::TarGz {
                Box::new(flate2::read::GzDecoder::new(file))
            } else {
                Box::new(file)
            };
            for entry in tar::Archive::new(reader).entries()? {
                let entry = entry?;
                let metadata = CompressedMetadata {
                    mode: Some(entry.header().mode()? & 0o7777),
                    modified: Some(i64::try_from(entry.header().mtime()?)?),
                };
                assert!(
                    result
                        .insert(entry.path()?.into_owned(), metadata)
                        .is_none()
                );
            }
        }
        ArchiveFormat::SevenZ => {
            let archive = sevenz_rust2::ArchiveReader::new(
                file,
                password
                    .map(sevenz_rust2::Password::from)
                    .unwrap_or_default(),
            )?;
            for entry in &archive.archive().files {
                let mode = (entry.has_windows_attributes && entry.windows_attributes & 0x8000 != 0)
                    .then_some((entry.windows_attributes >> 16) & 0o7777);
                let modified = if entry.has_last_modified_date {
                    let time = std::time::SystemTime::from(entry.last_modified_date);
                    Some(i64::try_from(
                        time.duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    )?)
                } else {
                    None
                };
                let metadata = CompressedMetadata { mode, modified };
                assert!(
                    result
                        .insert(PathBuf::from(entry.name()), metadata)
                        .is_none()
                );
            }
        }
        ArchiveFormat::Rar => return Err("RAR compression is not supported".into()),
    }
    Ok(result)
}

#[test]
fn compression_records_source_times_and_modes_in_every_format() -> Result<(), Box<dyn Error>> {
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
        (ArchiveFormat::SevenZ, None),
    ] {
        let context = format!("{format:?} password={}", password.is_some());
        let root = tempfile::tempdir()?;
        let source = root.path().join("dir");
        fs::create_dir(&source)?;
        fs::write(source.join("run.sh"), b"#!/bin/sh\n")?;
        fs::write(source.join("secret.txt"), b"secret")?;
        fs::set_permissions(source.join("run.sh"), fs::Permissions::from_mode(0o755))?;
        fs::set_permissions(source.join("secret.txt"), fs::Permissions::from_mode(0o600))?;
        // 7z refuses links (`compression_reports_unsupported_7z_links_without_committing`).
        let with_link = format != ArchiveFormat::SevenZ;
        if with_link {
            std::os::unix::fs::symlink("run.sh", source.join("link"))?;
            set_times_without_following(&source.join("link"), SOURCE_TIME)?;
        }
        set_times_without_following(&source.join("run.sh"), SOURCE_TIME)?;
        set_times_without_following(&source.join("secret.txt"), SOURCE_TIME)?;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o750))?;
        set_times_without_following(&source, SOURCE_TIME)?;
        let mut expected = BTreeMap::from([
            (PathBuf::from("dir"), (0o750, SOURCE_TIME)),
            (PathBuf::from("dir/run.sh"), (0o755, SOURCE_TIME)),
            (PathBuf::from("dir/secret.txt"), (0o600, SOURCE_TIME)),
        ]);
        if with_link {
            expected.insert(PathBuf::from("dir/link"), (0o777, SOURCE_TIME));
        }
        let expected = expected
            .into_iter()
            .map(|(name, (mode, modified))| {
                (
                    name,
                    CompressedMetadata {
                        mode: Some(mode),
                        modified: Some(modified),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let archive = root.path().join("archive");

        write_compression_fixture(&archive, &[source], format, password)?;

        assert_eq!(
            read_compressed_metadata(&archive, format, password)?,
            expected,
            "{context}"
        );
        if format == ArchiveFormat::Zip {
            let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
            let local = glib::DateTime::from_unix_local(SOURCE_TIME)?;
            for index in 0..zip.len() {
                let entry = zip.by_index_raw(index)?;
                let dos = entry.last_modified().ok_or("ZIP entry has no DOS time")?;
                assert_eq!(
                    (
                        i32::from(dos.year()),
                        i32::from(dos.month()),
                        i32::from(dos.day()),
                        i32::from(dos.hour()),
                        i32::from(dos.minute()),
                        i32::from(dos.second()),
                    ),
                    (
                        local.year(),
                        local.month(),
                        local.day_of_month(),
                        local.hour(),
                        local.minute(),
                        local.second(),
                    ),
                    "{context}: DOS time of `{}`",
                    entry.name().expect("member name")
                );
                assert_eq!(
                    zip_extended_time(&entry),
                    Some(SOURCE_TIME),
                    "{context}: UT field of `{}`",
                    entry.name().expect("member name")
                );
            }
        }
    }
    Ok(())
}

#[test]
fn zip_times_outside_the_dos_range_are_clamped_without_failing() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir(&source)?;
    let epoch = source.join("epoch.txt");
    let beyond_i32 = source.join("beyond-i32.txt");
    let beyond_dos = source.join("beyond-dos.txt");
    let beyond_i32_time = i64::from(u32::MAX) + 10;
    let beyond_dos_time = 4_360_000_000;
    for (path, seconds) in [
        (&epoch, 0),
        (&beyond_i32, beyond_i32_time),
        (&beyond_dos, beyond_dos_time),
    ] {
        fs::write(path, b"time")?;
        set_times_without_following(path, seconds)?;
    }
    let archive = root.path().join("archive.zip");

    write_compression_fixture(
        &archive,
        &[epoch, beyond_i32, beyond_dos],
        ArchiveFormat::Zip,
        None,
    )?;

    let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    let dos_parts = |time: zip::DateTime| {
        (
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second(),
        )
    };
    let entry = zip.by_name("epoch.txt")?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some(dos_parts(zip::DateTime::DEFAULT))
    );
    assert_eq!(zip_extended_time(&entry), Some(0));
    drop(entry);
    let entry = zip.by_name("beyond-i32.txt")?;
    let local = glib::DateTime::from_unix_local(beyond_i32_time)?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some((
            u16::try_from(local.year())?,
            u8::try_from(local.month())?,
            u8::try_from(local.day_of_month())?,
            u8::try_from(local.hour())?,
            u8::try_from(local.minute())?,
            u8::try_from(local.second() & !1)?,
        ))
    );
    assert_eq!(zip_extended_time(&entry), None);
    drop(entry);
    let entry = zip.by_name("beyond-dos.txt")?;
    assert_eq!(
        entry.last_modified().map(dos_parts),
        Some((2107, 12, 31, 23, 59, 58))
    );
    assert_eq!(zip_extended_time(&entry), None);
    Ok(())
}
