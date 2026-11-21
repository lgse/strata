// SPDX-License-Identifier: MIT

//! The header's `volume_monitor`: `tests/qa/volume_monitor.py` on the run's private session
//! bus, which GIO's remote volume monitor module lists drives, volumes and mounts from,
//! as it would from GVfs's udisks2 monitor. It starts before the application, whose volume
//! monitor asks each remote monitor once whether it is there, and stops after it, since
//! the module warns when its monitor leaves the bus.

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

/// Points GIO's remote volume monitor module at the folder holding only the test monitor's
/// file. The module stays unloaded under `GIO_USE_VFS=local`, which every other run keeps, so
/// this run's application starts on GIO's default VFS, as a desktop with GVfs does.
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
