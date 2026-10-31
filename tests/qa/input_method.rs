// SPDX-License-Identifier: MIT

//! The header's `input_method`: a private fcitx5 on the run's display and session bus,
//! its group holding the US keyboard and the named input method. Ctrl+Space switches to
//! the input method, as on a fresh install.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    Display, POLL_INTERVAL, XDG_DIRECTORIES,
    fixture::Manifest,
    process::{spawn, tail, terminate},
};

pub(crate) const LOG: &str = "fcitx5.log";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const BUS_NAME: &str = "org.fcitx.Fcitx5";
const PROFILE: &str = "\
[Groups/0]
Name=Default
Default Layout=us
DefaultIM={name}

[Groups/0/Items/0]
Name=keyboard-us
Layout=

[Groups/0/Items/1]
Name={name}
Layout=

[GroupOrder]
0=Default
";

pub(crate) struct InputMethod {
    daemon: Child,
    pub(crate) log: PathBuf,
}

impl InputMethod {
    pub(crate) fn start(
        name: &str,
        manifest: &Manifest,
        home: &Path,
        runtime: &Path,
        display: &Display,
        bus: &str,
    ) -> Result<Self, String> {
        let io = |error: std::io::Error| error.to_string();
        let data_directories = manifest
            .environment
            .get("XDG_DATA_DIRS")
            .map_or("/usr/share", String::as_str);
        if !data_directories.split(':').any(|directory| {
            Path::new(directory)
                .join(format!("fcitx5/inputmethod/{name}.conf"))
                .exists()
        }) {
            return Err(format!(
                "fcitx5 has no input method {name:?} in {data_directories}; install its addon"
            ));
        }
        let configuration = home.join("config/fcitx5");
        fs::create_dir_all(&configuration).map_err(io)?;
        fs::write(
            configuration.join("profile"),
            PROFILE.replace("{name}", name),
        )
        .map_err(io)?;
        let log = runtime.join(LOG);
        let output = File::create(&log).map_err(io)?;
        let errors = output.try_clone().map_err(io)?;
        let mut command = Command::new("fcitx5");
        // No tray icon to wait for.
        command
            .args(["--disable", "notificationitem"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("XDG_DATA_DIRS", data_directories)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("DBUS_SESSION_BUS_ADDRESS", bus)
            .env("LC_ALL", "C.UTF-8")
            .envs(display.environment())
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(errors);
        for (variable, directory) in XDG_DIRECTORIES {
            command.env(variable, home.join(directory));
        }
        let mut daemon =
            spawn(&mut command).map_err(|error| format!("cannot start fcitx5: {error}"))?;
        let deadline = Instant::now() + START_TIMEOUT;
        while !owns_name(bus) {
            if !matches!(daemon.try_wait(), Ok(None)) || Instant::now() >= deadline {
                terminate(&mut daemon);
                return Err(format!("fcitx5 did not start: {}", tail(&log)));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(Self { daemon, log })
    }
}

impl Drop for InputMethod {
    fn drop(&mut self) {
        terminate(&mut self.daemon);
    }
}

fn owns_name(bus: &str) -> bool {
    spawn(
        Command::new("dbus-send")
            .arg(format!("--bus={bus}"))
            .args([
                "--print-reply",
                "--dest=org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.NameHasOwner",
            ])
            .arg(format!("string:{BUS_NAME}"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .and_then(|child| child.wait_with_output().map_err(|error| error.to_string()))
    .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains("boolean true"))
}
