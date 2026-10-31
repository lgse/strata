// SPDX-License-Identifier: MIT

use std::{
    fs::{self, File},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{EXIT_TIMEOUT, POLL_INTERVAL, SESSION_BUS_CONFIG};

const BUS_START_TIMEOUT: Duration = Duration::from_secs(30);

/// Held around every spawn, so only Xvfb ever inherits its `-displayfd` pipe.
pub(crate) static SPAWN: Mutex<()> = Mutex::new(());

pub(crate) fn spawn(command: &mut Command) -> Result<Child, String> {
    let _spawning = SPAWN.lock().map_err(|error| error.to_string())?;
    command.spawn().map_err(|error| error.to_string())
}

pub(crate) fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let deadline = Instant::now() + timeout;
    // Only an unreaped child's pid is still its own, so the pidfd opens after
    // a first check.
    if let Some(status) = child.try_wait().ok()? {
        return Some(status);
    }
    let pidfd = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .and_then(|pid| {
            rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
        });
    loop {
        if let Some(status) = child.try_wait().ok()? {
            return Some(status);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let woken = pidfd.as_ref().is_some_and(|pidfd| {
            let limit = Timespec {
                tv_sec: i64::try_from(remaining.as_secs()).unwrap_or(i64::MAX),
                tv_nsec: i64::from(remaining.subsec_nanos()),
            };
            poll(&mut [PollFd::new(pidfd, PollFlags::IN)], Some(&limit)).is_ok()
        });
        if !woken {
            std::thread::sleep(remaining.min(POLL_INTERVAL));
        }
    }
}

/// SIGTERM first, so Xvfb removes its lock file and socket.
pub(crate) fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = rustix::process::kill_process(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::TERM,
    );
    if wait_for_exit(child, EXIT_TIMEOUT).is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) struct Bus {
    daemon: Child,
    pub(crate) address: String,
}

impl Bus {
    pub(crate) fn start(runtime: &Path) -> Result<Self, String> {
        let socket = runtime.join("bus");
        let config = runtime.join("session-bus.conf");
        fs::write(
            &config,
            SESSION_BUS_CONFIG.replace("{socket}", &socket.display().to_string()),
        )
        .map_err(|error| error.to_string())?;
        let log =
            File::create(runtime.join("dbus-daemon.log")).map_err(|error| error.to_string())?;
        let errors = log.try_clone().map_err(|error| error.to_string())?;
        let mut daemon = spawn(
            Command::new("dbus-daemon")
                .args(["--nofork", "--nopidfile"])
                .arg(format!("--config-file={}", config.display()))
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .stdin(Stdio::null())
                .stdout(log)
                .stderr(errors),
        )
        .map_err(|error| format!("cannot start dbus-daemon: {error}"))?;
        let deadline = Instant::now() + BUS_START_TIMEOUT;
        while !socket.exists() {
            if matches!(daemon.try_wait(), Ok(Some(_))) || Instant::now() >= deadline {
                terminate(&mut daemon);
                return Err(format!(
                    "the private session bus did not start: {}",
                    tail(&runtime.join("dbus-daemon.log"))
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(Self {
            daemon,
            address: format!("unix:path={}", socket.display()),
        })
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        terminate(&mut self.daemon);
    }
}

pub(crate) fn tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}
