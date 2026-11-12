// SPDX-License-Identifier: MIT

//! A staged archive stays private until it is published, and never is once cancelled.

use super::*;
use crate::adapters::local_operations::tests::fs_quirks::Racer;

/// Checks the archive holds `contents` with `mode`, and no compression stage is left.
fn expect_published(
    destination: &Path,
    archive: &Path,
    contents: &[u8],
    mode: u32,
) -> Result<(), Box<dyn Error>> {
    assert_eq!(fs::read(archive)?, contents);
    assert_eq!(fs::metadata(archive)?.permissions().mode() & 0o777, mode);
    assert!(stages(destination, COMPRESSION_STAGE)?.is_empty());
    Ok(())
}

#[test]
fn compression_staging_stays_private_while_encoding() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
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
    let mut task = spawn_held_compression(&writer, &destination, &archive, b"replacement");
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
    expect_published(&destination, &archive, b"replacement", 0o640)
}

#[test]
fn compression_new_archive_staging_stays_private_until_publish() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let archive = destination.join("created.zip");
    let writer = HeldWriter::default();
    let mut task = spawn_held_compression(&writer, &destination, &archive, b"created");
    writer.wait_until_held(&mut task)?;
    assert_eq!(compression_stage_mode(&destination)?, 0o600);

    writer.release();
    assert_eq!(
        glib::MainContext::default().block_on(task)?,
        Ok(("created.zip".to_owned(), None, None))
    );
    expect_published(&destination, &archive, b"created", 0o666 & !process_umask())
}

#[test]
fn cancelling_staged_compression_waits_for_worker_exit_before_cleanup() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
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
                super::super::check_archive_cancelled(&worker_cancelled)
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
    expect_contents(&[(archive, b"original")], "cancelled")
}

/// Where a staged compression is stopped, and the mount it is staged on.
const BOUNDARIES: [(&str, Preset); 6] = [
    ("encoding", Quirks::default),
    ("close", Quirks::default),
    ("close, then evicted", Quirks::vfat),
    ("close on SMB without serverino", Quirks::cifs_noserverino),
    (
        "close on SMB without serverino, actimeo=0",
        cifs_noserverino_without_attribute_cache,
    ),
    ("close error", Quirks::default),
];

fn cancels_at_close(boundary: &str) -> bool {
    matches!(boundary, "close" | "close, then evicted") || boundary.starts_with("close on SMB")
}

/// Cancels once the stage is closed, evicting it first when asked.
fn cancel_on_close(folder: &Path, evicted: bool, cancelled: &Arc<AtomicBool>) -> Racer {
    let (folder, on_close) = (folder.to_path_buf(), cancelled.clone());
    Box::new(move || {
        if evicted {
            let stage = stages(&folder, COMPRESSION_STAGE)
                .ok()
                .and_then(|mut found| found.pop())
                .expect("the stage");
            assert!(evict(&folder.join(stage)), "evict the stage");
        }
        on_close.store(true, Ordering::Release);
    })
}

fn boundary_quirks(
    boundary: &str,
    preset: Preset,
    folder: &Path,
    cancelled: &Arc<AtomicBool>,
) -> Quirks {
    let mut quirks = Quirks {
        trash: Trash::Unsupported,
        ..preset()
    };
    if cancels_at_close(boundary) {
        let evicted = boundary == "close, then evicted";
        quirks.after.push((
            Op::Close,
            ".strata-copy-*",
            0,
            cancel_on_close(folder, evicted, cancelled),
        ));
    } else if boundary == "close error" {
        quirks
            .fail
            .push((Op::Close, ".strata-copy-*", 0, Errno::IO));
    }
    quirks
}

/// Writes `archive.zip`, stamping a server time or cancelling as `boundary` asks.
fn stage_until(
    boundary: &'static str,
    folder: &Path,
    replacing: bool,
    cancelled: Arc<AtomicBool>,
) -> Staged {
    let server_stamped = boundary.starts_with("close on SMB");
    let on_write = cancelled.clone();
    glib::MainContext::default().block_on(write_staged_archive(
        folder,
        &folder.join("archive.zip"),
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
    ))
}

fn ended_at(boundary: &str, result: &Staged) -> bool {
    if boundary == "close error" {
        matches!(result, Err(ArchiveError::Failed(_)))
    } else {
        matches!(result, Err(ArchiveError::Cancelled))
    }
}

fn expect_unpublished(
    boundary: &'static str,
    preset: Preset,
    replacing: bool,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    if boundary == "close, then evicted" && !base_keeps_written_times(root.path())? {
        eprintln!(
            "skipped on a base that moves a written file's time after its descriptor \
             showed it: {boundary}"
        );
        return Ok(());
    }
    let archive = root.path().join("archive.zip");
    if replacing {
        fs::write(&archive, b"original")?;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let quirks = boundary_quirks(boundary, preset, root.path(), &cancelled);
    let _scope = FsQuirks::install(root.path(), quirks);

    let result = stage_until(boundary, root.path(), replacing, cancelled);

    assert!(
        ended_at(boundary, &result),
        "{boundary}, replacing {replacing}: {result:?}"
    );
    if replacing {
        assert_eq!(fs::read(&archive)?, b"original");
    } else {
        assert!(!archive.exists());
    }
    assert!(
        stages(root.path(), COMPRESSION_STAGE)?.is_empty(),
        "{boundary}, replacing {replacing}"
    );
    Ok(())
}

#[test]
fn staged_archives_are_not_published_after_cancellation_or_close_failure()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for replacing in [false, true] {
        for (boundary, preset) in BOUNDARIES {
            expect_unpublished(boundary, preset, replacing)?;
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
