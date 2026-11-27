// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use crate::{
    REPOSITORY,
    process::{spawn, tail, terminate},
    variables::PYTHON,
};

pub(crate) const LOG: &str = "volume-monitor.log";
/// Under `$environment`: one `<method> <device id>` line per call the application made.
pub(crate) const CALLS: &str = "volume-monitor.calls";
const START_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn prepare(command: &mut Command) {
    command
        .env_remove("GIO_USE_VFS")
        .env("GVFS_MONITOR_DIR", monitor_directory());
}

/// The folder of the test monitor's file, which GIO reads on the main thread at start-up.
pub(crate) fn monitor_directory() -> PathBuf {
    Path::new(REPOSITORY).join("tests/qa/volume_monitors")
}

pub(crate) struct VolumeMonitor {
    child: Child,
    pub(crate) log: PathBuf,
}

impl VolumeMonitor {
    pub(crate) fn start(bus: &str, environment: &Path, runtime: &Path) -> Result<Self, String> {
        let io = |error: std::io::Error| error.to_string();
        let log = runtime.join(LOG);
        let errors = File::create(&log).map_err(io)?;
        let mut child = spawn(
            Command::new(PYTHON)
                .arg(Path::new(REPOSITORY).join("tests/qa/volume_monitor.py"))
                .arg(environment.join(CALLS))
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("DBUS_SESSION_BUS_ADDRESS", bus)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(errors),
        )
        .map_err(|error| format!("cannot start the volume monitor: {error}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, ready) = mpsc::channel();
        std::thread::spawn(move || {
            let first = BufReader::new(stdout).lines().next();
            let _ = sender.send(matches!(first, Some(Ok(line)) if line == "ready"));
        });
        if ready.recv_timeout(START_TIMEOUT) != Ok(true) {
            terminate(&mut child);
            return Err(format!("the volume monitor did not start: {}", tail(&log)));
        }
        Ok(Self { child, log })
    }
}

impl VolumeMonitor {
    /// Its log, as a failed or kept run's artifact.
    pub(crate) fn artifact(&self) -> (&'static str, String) {
        (LOG, std::fs::read_to_string(&self.log).unwrap_or_default())
    }
}

impl Drop for VolumeMonitor {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}
