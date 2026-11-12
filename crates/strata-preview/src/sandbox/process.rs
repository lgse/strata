// SPDX-License-Identifier: MIT
//! Spawning, monitoring and terminating the bubblewrap renderer process.

use std::{
    io,
    ops::ControlFlow,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    process::{Pid, Signal, kill_process_group},
};

use super::Cancellation;

const NAMESPACE_WAIT: Duration = Duration::from_secs(1);

pub struct Renderer {
    pub(super) child: Child,
    pub(super) init: Option<rustix::fd::OwnedFd>,
    /// Held open so bubblewrap's closing write to the info pipe cannot raise SIGPIPE.
    _info: Option<rustix::fd::OwnedFd>,
}

impl Renderer {
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.await_namespace();
        }
        Ok(status)
    }

    pub(super) fn await_namespace(&mut self) {
        let Some(init) = self.init.take() else {
            return;
        };
        let deadline = Instant::now() + NAMESPACE_WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = timespec(remaining);
            let mut fds = [PollFd::new(&init, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Err(rustix::io::Errno::INTR) if !remaining.is_zero() => {}
                _ => return,
            }
        }
    }
}

pub(super) fn spawn_renderer(command: &mut Command) -> io::Result<Renderer> {
    use std::os::unix::process::CommandExt;

    let (info, info_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    command.stderr(Stdio::from(info_end));
    let spawned = command.process_group(0).spawn();
    // Drops the parent's write end, or the pipe never reports bubblewrap's exit.
    command.stderr(Stdio::null());
    let child = spawned?;
    let init = namespace_init(&info, &child).and_then(|pid| {
        rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
    });
    Ok(Renderer {
        child,
        init,
        _info: Some(info),
    })
}

fn timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: duration.as_secs() as i64,
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}

fn read_info(info: &rustix::fd::OwnedFd, report: &mut Vec<u8>) -> ControlFlow<Option<Pid>> {
    let mut chunk = [0; 512];
    match rustix::io::read(info, &mut chunk) {
        Ok(0) => ControlFlow::Break(reported_init(report)),
        Ok(read) => {
            report.extend_from_slice(&chunk[..read]);
            ControlFlow::Continue(())
        }
        Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => ControlFlow::Continue(()),
        Err(_) => ControlFlow::Break(None),
    }
}

/// Waits up to `remaining` for more of bubblewrap's info report, or its exit.
fn poll_info(
    info: &rustix::fd::OwnedFd,
    bwrap: Option<&rustix::fd::OwnedFd>,
    remaining: Duration,
    report: &mut Vec<u8>,
) -> ControlFlow<Option<Pid>> {
    let timeout = timespec(remaining);
    let mut fds = vec![PollFd::new(info, PollFlags::IN)];
    if let Some(bwrap) = bwrap {
        fds.push(PollFd::new(bwrap, PollFlags::IN));
    }
    match poll(&mut fds, Some(&timeout)) {
        Ok(_) if fds[0].revents().contains(PollFlags::IN) => read_info(info, report),
        Ok(_) if fds[0].revents().contains(PollFlags::HUP) => {
            ControlFlow::Break(reported_init(report))
        }
        Ok(_) if fds.get(1).is_some_and(|fd| !fd.revents().is_empty()) => {
            ControlFlow::Break(reported_init(report))
        }
        Ok(_) | Err(rustix::io::Errno::INTR) => ControlFlow::Continue(()),
        Err(_) => ControlFlow::Break(None),
    }
}

fn namespace_init(info: &rustix::fd::OwnedFd, child: &Child) -> Option<Pid> {
    let deadline = Instant::now() + NAMESPACE_WAIT;
    let bwrap = child_pidfd(child);
    let mut report = Vec::new();
    loop {
        if let Some(pid) = reported_init(&report) {
            return Some(pid);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || report.len() > 4096 {
            return None;
        }
        if let ControlFlow::Break(pid) = poll_info(info, bwrap.as_ref(), remaining, &mut report) {
            return pid;
        }
    }
}

fn reported_init(report: &[u8]) -> Option<Pid> {
    let report = std::str::from_utf8(report).ok()?;
    let (_, after) = report.split_once("\"child-pid\":")?;
    let digits = after.trim_start();
    let end = digits.find(|character: char| !character.is_ascii_digit())?;
    Pid::from_raw(digits[..end].parse().ok()?)
}

/// `None` on kernels without pidfd support; wait loops fall back to interval polling.
pub(super) fn child_pidfd(child: &Child) -> Option<rustix::fd::OwnedFd> {
    let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).ok()?)?;
    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
}

/// A pidfd wakes on child exit; poll errors degrade to a short sleep, and the
/// deadline bounds every path.
fn wait_step(pidfd: Option<&rustix::fd::OwnedFd>, deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let quantum = remaining.min(Duration::from_millis(20));
    let Some(pidfd) = pidfd else {
        thread::sleep(quantum);
        return;
    };
    let timespec = timespec(quantum);
    let mut fds = [PollFd::new(pidfd, PollFlags::IN)];
    if poll(&mut fds, Some(&timespec)).is_err() {
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn wait_for_renderer(
    child: &mut Renderer,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
) -> Result<ExitStatus, String> {
    wait_for_renderer_reporting(child, cancellation, wall_time_limit, &mut || {})
}

pub(super) fn wait_for_renderer_reporting(
    child: &mut Renderer,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
    progress: &mut dyn FnMut(),
) -> Result<ExitStatus, String> {
    let result = monitor_renderer(child, cancellation, wall_time_limit, progress);
    if result.is_err() {
        terminate(child);
    }
    result
}

pub(super) fn monitor_renderer(
    child: &mut Renderer,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
    progress: &mut dyn FnMut(),
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    let deadline = started + wall_time_limit;
    let pidfd = child_pidfd(&child.child);
    loop {
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".to_owned());
        }
        if Instant::now() >= deadline {
            return Err("The preview renderer timed out".to_owned());
        }
        progress();
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => wait_step(pidfd.as_ref(), deadline),
            Err(error) => {
                return Err(format!("Unable to monitor the preview renderer: {error}"));
            }
        }
    }
}

pub(super) fn kill_renderer(renderer: &mut Renderer) {
    if let Ok(raw_pid) = i32::try_from(renderer.id())
        && let Some(process_group) = Pid::from_raw(raw_pid)
    {
        let _killed = kill_process_group(process_group, Signal::KILL);
    }
    // bwrap is also the PID-namespace init process, so descendants that create a new
    // process group still die with it.
    let _killed = renderer.child.kill();
}

pub(super) fn terminate(renderer: &mut Renderer) {
    kill_renderer(renderer);
    let _waited = renderer.child.wait();
    renderer.await_namespace();
}
