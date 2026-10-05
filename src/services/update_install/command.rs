// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::{self, Read},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use super::{InstallCancel, InstallStop};

const MAX_OUTPUT: usize = 64 * 1024;

pub(super) fn run(
    command: &mut Command,
    cancel: &InstallCancel,
    timeout: Duration,
) -> Result<Output, InstallStop> {
    cancel.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let spawn_deadline = Instant::now() + Duration::from_secs(2);
    let mut child = loop {
        cancel.check()?;
        match command.spawn() {
            Ok(child) => break child,
            // Concurrent fork/exec can briefly inherit another thread's writable descriptor.
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < spawn_deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return Err(InstallStop::Failed(format!(
                    "Could not run {:?}: {error}",
                    command.get_program()
                )));
            }
        }
    };
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Missing verification stdout".to_owned())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Missing verification stderr".to_owned())?;
        rustix::fs::fcntl_setfl(&stdout, rustix::fs::OFlags::NONBLOCK)
            .map_err(|error| error.to_string())?;
        rustix::fs::fcntl_setfl(&stderr, rustix::fs::OFlags::NONBLOCK)
            .map_err(|error| error.to_string())?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let deadline = Instant::now() + timeout;
        loop {
            cancel.check()?;
            if Instant::now() >= deadline {
                return Err(InstallStop::Failed(
                    "Update verification timed out".to_owned(),
                ));
            }
            drain(&mut stdout, &mut out)?;
            drain(&mut stderr, &mut err)?;
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                drain(&mut stdout, &mut out)?;
                drain(&mut stderr, &mut err)?;
                return Ok(Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        let _killed = child.kill();
        let _waited = child.wait();
    }
    result
}

fn drain(reader: &mut impl Read, output: &mut Vec<u8>) -> Result<(), InstallStop> {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if output.len() + count > MAX_OUTPUT {
                    return Err(InstallStop::Failed(
                        "Update verification produced too much output".to_owned(),
                    ));
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(InstallStop::Failed(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;
