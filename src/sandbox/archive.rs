// SPDX-License-Identifier: MIT

//! Sandboxed RAR extraction. UnRAR's C parser runs only in a bubblewrapped
//! child with a read-only bind of the archive and no filesystem write access
//! at all; it streams member bytes back over its stdout pipe using the
//! [`crate::rar_extraction`] wire format. The parent never calls into UnRAR
//! directly and performs every real destination write itself, unchanged.

use std::{
    fs,
    io::Read,
    path::Path,
    process::{Child, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crate::rar_extraction as wire;

use super::*;

/// Bulk decompression, not a quick preview render: a legitimate large archive
/// can need much more CPU than the shared preview budget.
const RAR_CPU_TIME_LIMIT_SECS: u64 = 120;
/// Applied per read attempt, not to the stream as a whole: as long as the
/// child keeps producing *some* output, a large member can take as long as
/// it needs. Only a genuine stall (hung or killed child) times out.
const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) enum Member<'a> {
    Directory,
    File { size: u64, body: &'a mut dyn Read },
}

/// Streams `archive_path` through the sandboxed helper, calling `on_member`
/// for each entry in archive order. `on_member` is responsible for the real
/// destination write; this function only ever hands it a bounded reader over
/// that member's declared byte count.
///
/// An `on_member` failure (including one caused by `cancelled` being set)
/// stops the stream and terminates the child; the caller decides whether the
/// resulting error means "cancelled" by checking `cancelled` itself, matching
/// how [`crate::adapters`]'s extraction session already treats that flag as
/// the single source of truth.
pub(crate) fn stream_rar(
    archive_path: &Path,
    password: Option<&str>,
    cancelled: &AtomicBool,
    on_member: impl FnMut(&str, Member<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut child = spawn(archive_path, password)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Missing RAR extraction pipe".to_owned())?;
    let reader = TimedPipeReader {
        fd: stdout,
        timeout: READ_INACTIVITY_TIMEOUT,
        cancelled,
    };
    let result = drive(reader, on_member);
    if result.is_err() {
        terminate(&mut child);
    } else {
        reap(&mut child, cancelled)?;
    }
    result
}

/// The record-parsing and dispatch logic, independent of where `reader`
/// bytes actually come from — a real sandboxed child's pipe in production, a
/// fixed in-memory fixture in tests.
fn drive(
    mut reader: impl Read,
    mut on_member: impl FnMut(&str, Member<'_>) -> Result<(), String>,
) -> Result<(), String> {
    wire::read_magic(&mut reader).map_err(|error| error.to_string())?;
    loop {
        let record = wire::read_record(&mut reader).map_err(|error| error.to_string())?;
        match record {
            wire::Record::End => return Ok(()),
            wire::Record::Error(message) => return Err(message),
            wire::Record::Directory(name) => on_member(&name, Member::Directory)?,
            wire::Record::File(name, size) => {
                let mut body = (&mut reader).take(size);
                on_member(
                    &name,
                    Member::File {
                        size,
                        body: &mut body,
                    },
                )?;
                // The callback may stop reading before the declared size (an
                // error partway through); drain the rest so the trailer
                // that follows lines up on the wire regardless.
                std::io::copy(&mut body, &mut std::io::sink())
                    .map_err(|error| error.to_string())?;
                wire::read_file_trailer(&mut reader).map_err(|error| error.to_string())??;
            }
        }
    }
}

fn reap(child: &mut Child, cancelled: &AtomicBool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            terminate(child);
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("The sandboxed RAR extraction helper failed".to_owned())
                };
            }
            Ok(None) if Instant::now() >= deadline => {
                terminate(child);
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate(child);
                return Err(format!(
                    "Unable to monitor the RAR extraction helper: {error}"
                ));
            }
        }
    }
}

fn spawn(archive_path: &Path, password: Option<&str>) -> Result<Child, String> {
    let archive_path = archive_path
        .canonicalize()
        .map_err(|error| format!("Unable to open RAR archive: {error}"))?;
    let output = PrivateOutput::create().map_err(|error| error.to_string())?;
    let current_executable = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the Strata executable: {error}"))?;
    let running_executable = PathBuf::from(format!("/proc/{}/exe", std::process::id()));
    let executable =
        resolve_renderer_executable(&current_executable, &running_executable, output.path())?;
    let bwrap = crate::trusted_command::resolve("bwrap")
        .map_err(|error| format!("Unable to start the RAR extraction sandbox: {error}"))?;
    let mut command = rar_extraction_command(&bwrap, &executable, &archive_path, password)?;
    spawn_renderer(&mut command)
        .map_err(|error| format!("Unable to start the RAR extraction sandbox: {error}"))
}

fn rar_extraction_command(
    bwrap: &Path,
    executable: &Path,
    archive_path: &Path,
    password: Option<&str>,
) -> Result<Command, String> {
    let sandbox_input = sandbox_input_path(archive_path);
    let mut command = runtime_command(bwrap, false);
    command.arg("--ro-bind").arg(executable).arg("/app/strata");
    command
        .arg("--ro-bind")
        .arg(archive_path)
        .arg(&sandbox_input);
    command.args(["--setenv", "MALLOC_ARENA_MAX", "1"]);
    command.arg("--");
    command
        .arg("/usr/bin/prlimit")
        .arg(format!("--as={ADDRESS_SPACE_LIMIT_BYTES}"))
        .arg(format!("--cpu={RAR_CPU_TIME_LIMIT_SECS}"))
        .arg("--fsize=0")
        .arg("--");
    command
        .args(["/app/strata", "--preview-helper", "extract-rar"])
        .arg(&sandbox_input);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    if let Some(password) = password {
        let secret = stage_secret_anon(password.as_bytes())?;
        // Duplicates only this child's stdin; concurrent spawns cannot inherit the secret.
        command.stdin(Stdio::from(fs::File::from(secret)));
        command.arg("0");
    }
    Ok(command)
}

struct TimedPipeReader<'a, F> {
    fd: F,
    timeout: Duration,
    cancelled: &'a AtomicBool,
}

impl<F: std::os::fd::AsFd> Read for TimedPipeReader<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let deadline = Instant::now() + self.timeout;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "Operation cancelled",
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "RAR extraction progress timed out",
                ));
            }
            let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: remaining.min(Duration::from_millis(20)).as_nanos() as i64,
            };
            if poll(&mut fds, Some(&timeout))? != 0 {
                match rustix::io::read(&self.fd, &mut *buffer) {
                    Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
                    result => return result.map_err(Into::into),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
