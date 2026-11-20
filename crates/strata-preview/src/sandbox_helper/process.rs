// SPDX-License-Identifier: MIT
//! Bounded, time-limited capture of renderer subprocess output.

use std::{
    io::{self, Read},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A child whose stdout is drained on a reader thread, up to one byte past the limit.
struct Capture {
    child: Child,
    reader: thread::JoinHandle<()>,
    receiver: mpsc::Receiver<io::Result<Vec<u8>>>,
    max_bytes: u64,
}

impl Capture {
    fn spawn(command: &mut Command, max_bytes: u64) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Unable to capture provider output"))?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut data = Vec::new();
            let result = stdout
                .take(max_bytes.saturating_add(1))
                .read_to_end(&mut data)
                .map(|_| data);
            let _sent = sender.send(result);
        });
        Ok(Self {
            child,
            reader,
            receiver,
            max_bytes,
        })
    }

    fn try_output(&self) -> io::Result<Option<Vec<u8>>> {
        match self.receiver.try_recv() {
            Ok(Ok(data)) if data.len() as u64 > self.max_bytes => Err(io::Error::other(
                "Preview provider output exceeded its limit",
            )),
            Ok(result) => result.map(Some),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(io::Error::other("Unable to read provider output"))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
        }
    }

    /// Records whichever of the exit status and the output arrived since the last poll.
    fn poll(
        &mut self,
        status: &mut Option<ExitStatus>,
        output: &mut Option<Vec<u8>>,
    ) -> io::Result<()> {
        if status.is_none() {
            *status = self.child.try_wait()?;
        }
        if output.is_none() {
            *output = self.try_output()?;
        }
        Ok(())
    }

    fn stop(mut self) {
        stop_child(&mut self.child);
        let _joined = self.reader.join();
    }

    fn finish(self) {
        let _joined = self.reader.join();
    }
}

pub(super) fn bounded_output_with_timeout(
    command: &mut Command,
    max_bytes: u64,
    timeout: Duration,
) -> io::Result<Option<Output>> {
    if timeout.is_zero() {
        return Ok(None);
    }
    let mut capture = Capture::spawn(command, max_bytes)?;
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut output = None;
    loop {
        if let Err(error) = capture.poll(&mut status, &mut output) {
            capture.stop();
            return Err(error);
        }
        if let Some(status) = status
            && let Some(stdout) = output.take()
        {
            capture.finish();
            return Ok(Some(Output {
                status,
                stdout,
                stderr: Vec::new(),
            }));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            capture.stop();
            return Ok(None);
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(remaining));
    }
}

pub(super) fn stop_child(child: &mut Child) {
    let _killed = child.kill();
    let _waited = child.wait();
}

pub fn run_command_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<bool> {
    if timeout.is_zero() {
        return Ok(false);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            stop_child(&mut child);
            return Ok(false);
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(remaining));
    }
}

pub(super) fn read_limited(reader: impl Read, max_bytes: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut data)?;
    if data.len() as u64 > max_bytes {
        return Err(io::Error::other(
            "Preview provider output exceeded its limit",
        ));
    }
    Ok(data)
}

pub(super) fn bounded_output(command: &mut Command, max_bytes: u64) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let read = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Unable to capture provider output"))
        .and_then(|stdout| read_limited(stdout, max_bytes));
    let stdout = match read {
        Ok(stdout) => stdout,
        Err(error) => {
            let _killed = child.kill();
            let _waited = child.wait();
            return Err(error);
        }
    };
    let status = child.wait()?;
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

/// The stdout of a run that succeeded and printed something.
pub(super) fn successful_stdout(output: io::Result<Output>) -> Option<Vec<u8>> {
    output
        .ok()
        .filter(|output| output.status.success() && !output.stdout.is_empty())
        .map(|output| output.stdout)
}
