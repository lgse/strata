// SPDX-License-Identifier: MIT

//! Runs UnRAR in a read-only bubblewrap child; the parent writes extracted members.

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

// Bulk extraction needs more CPU than a preview render.
const RAR_CPU_TIME_LIMIT_SECS: u64 = 120;
// Limit stalls, not the total time spent extracting a large archive.
const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) enum Member<'a> {
    Directory,
    File { size: u64, body: &'a mut dyn Read },
}

/// Callback errors lose their kind; only decoder failures retain it across the wire.
pub(crate) fn stream_rar(
    archive_path: &Path,
    password: Option<&str>,
    cancelled: &AtomicBool,
    on_member: impl FnMut(&str, Member<'_>, wire::WireMetadata) -> Result<(), String>,
) -> Result<(), wire::Failure> {
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

fn drive(
    mut reader: impl Read,
    mut on_member: impl FnMut(&str, Member<'_>, wire::WireMetadata) -> Result<(), String>,
) -> Result<(), wire::Failure> {
    wire::read_magic(&mut reader).map_err(|error| error.to_string())?;
    loop {
        let record = wire::read_record(&mut reader).map_err(|error| error.to_string())?;
        match record {
            wire::Record::End => return Ok(()),
            wire::Record::Error(failure) => return Err(failure),
            wire::Record::Directory(name, metadata) => {
                on_member(&name, Member::Directory, metadata)?;
            }
            wire::Record::File(name, size, metadata) => {
                let mut body = wire::FileBody::new(&mut reader, size);
                on_member(
                    &name,
                    Member::File {
                        size,
                        body: &mut body,
                    },
                    metadata,
                )?;
                std::io::copy(&mut body, &mut std::io::sink()).map_err(wire::Failure::from_io)?;
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
        .arg(option_env!("STRATA_SANDBOX_PRLIMIT").unwrap_or("/usr/bin/prlimit"))
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
