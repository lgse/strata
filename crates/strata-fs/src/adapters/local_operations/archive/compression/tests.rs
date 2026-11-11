// SPDX-License-Identifier: MIT

mod cancellation;
mod contents;
mod keep_both;
mod metadata;
mod policy;
mod replacing;
mod staging;

use super::super::fixtures::{
    COMPRESSION_STAGE, HomeTrashGuard, compression_stage_mode, names, never_cancelled,
    set_times_without_following, stages, tempdir_on_home_device, write_compression_fixture,
};
use super::{ArchiveError, inspect_archive_sources, process_umask, write_staged_archive};
use crate::{
    adapters::local_operations::tests::fs_quirks::{
        FsQuirks, Op, Preset, Quirks, Trash, evict, recreated,
    },
    services::{ArchiveFormat, PlacedSnapshot, ReplacedOriginal, TransferConflict},
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};
use contents::{CompressedEntry, read_compressed_entries};
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
        if !self.spawned.get() {
            return;
        }
        let context = glib::MainContext::default();
        let deadline = Instant::now() + HOLD_LIMIT;
        while !self.ended.load(Ordering::Acquire) && Instant::now() < deadline {
            context.iteration(false);
            std::thread::yield_now();
        }
    }
}

fn serial() -> Result<crate::test_support::MainContextGuard<'static>, String> {
    ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())
}

type Staged = Result<(String, Option<ReplacedOriginal>, Option<PlacedSnapshot>), ArchiveError>;

/// Stages and publishes `archive` with an encoder that writes `contents`.
fn stage_archive(
    destination: &Path,
    archive: &Path,
    conflict: TransferConflict,
    contents: &'static [u8],
) -> Staged {
    glib::MainContext::default().block_on(write_staged_archive(
        destination,
        archive,
        conflict,
        never_cancelled(),
        None,
        move |mut file| {
            file.write_all(contents)
                .map_err(|error| error.to_string())?;
            Ok(())
        },
    ))
}

/// Spawns a replacing compression whose encoder writes `contents`, then waits on `writer`.
fn spawn_held_compression(
    writer: &HeldWriter,
    destination: &Path,
    archive: &Path,
    contents: &'static [u8],
) -> glib::JoinHandle<Staged> {
    let hold = writer.hold();
    let (destination, archive) = (destination.to_path_buf(), archive.to_path_buf());
    writer.spawn(async move {
        write_staged_archive(
            &destination,
            &archive,
            TransferConflict::ReplaceExisting,
            never_cancelled(),
            None,
            move |mut file| {
                file.write_all(contents)
                    .map_err(|error| error.to_string())?;
                Ok(hold()?)
            },
        )
        .await
    })
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

fn expect_contents(expected: &[(PathBuf, &[u8])], context: &str) -> Result<(), Box<dyn Error>> {
    for (path, contents) in expected {
        assert_eq!(fs::read(path)?, *contents, "{context}: {}", path.display());
    }
    Ok(())
}

/// Checks `existing.zip` is all `folder` holds, and that it holds `contents`.
fn expect_only_existing(
    folder: &Path,
    contents: &[u8],
    context: &str,
) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        fs::read(folder.join("existing.zip"))?,
        contents,
        "{context}"
    );
    assert_eq!(names(folder)?, ["existing.zip"], "{context}");
    Ok(())
}
