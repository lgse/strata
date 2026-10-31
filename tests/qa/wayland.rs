// SPDX-License-Identifier: MIT

//! `STRATA_QA_DISPLAY=wayland`: a private headless sway in the run's runtime directory,
//! with no input devices of its own and no XWayland. The application sends its input
//! through Wayland virtual devices instead of XTEST.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    Display, POLL_INTERVAL, Server,
    process::{spawn, tail, terminate},
};

pub(crate) const SWAY_SOCKET_VARIABLE: &str = "SWAYSOCK";
const SWAY_START_TIMEOUT: Duration = Duration::from_secs(15);
/// Xvfb's screen.
const SCREEN: (u32, u32) = (1440, 900);
/// Xvfb's 660 ms autorepeat delay, and every window floating in the output's corner, where
/// the window-manager-less X display leaves it.
const SWAY_CONFIG: &str = "\
output HEADLESS-1 resolution {width}x{height} position 0 0 scale 1
default_border none
default_floating_border none
focus_follows_mouse no
mouse_warping none
xwayland disable
input type:keyboard repeat_delay 660
for_window [app_id=\".*\"] floating enable, move position 0 0
";

pub(crate) struct Sway {
    pub(crate) ipc: PathBuf,
    runtime: PathBuf,
}

impl Sway {
    pub(crate) fn screenshot(&self, socket: &str, destination: &Path) -> Result<Child, String> {
        spawn(
            Command::new("grim")
                .arg(destination)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("XDG_RUNTIME_DIR", &self.runtime)
                .env("WAYLAND_DISPLAY", socket)
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .map_err(|error| format!("cannot run grim: {error}"))
    }
}

pub(crate) fn start(scratch: &Path, runtime: &Path) -> Result<Display, String> {
    let io = |error: std::io::Error| error.to_string();
    let config = scratch.join("sway.conf");
    fs::write(
        &config,
        SWAY_CONFIG
            .replace("{width}", &SCREEN.0.to_string())
            .replace("{height}", &SCREEN.1.to_string()),
    )
    .map_err(io)?;
    let log = scratch.join("sway.log");
    let output = File::create(&log).map_err(io)?;
    let errors = output.try_clone().map_err(io)?;
    let home = scratch.join("sway-home");
    fs::create_dir_all(&home).map_err(io)?;
    let mut server = spawn(
        Command::new("sway")
            .arg("--config")
            .arg(&config)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_HEADLESS_OUTPUTS", "1")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(errors),
    )
    .map_err(|error| format!("cannot start sway: {error}"))?;
    let deadline = Instant::now() + SWAY_START_TIMEOUT;
    loop {
        if let (Some(socket), Some(ipc)) = (find(runtime, is_wayland_socket), find(runtime, is_ipc))
            && output_active(&runtime.join(&ipc))
        {
            return Ok(Display {
                name: socket,
                server,
                log,
                kind: Server::Wayland(Sway {
                    ipc: runtime.join(ipc),
                    runtime: runtime.to_owned(),
                }),
            });
        }
        if !matches!(server.try_wait(), Ok(None)) || Instant::now() >= deadline {
            terminate(&mut server);
            return Err(format!("sway did not start: {}", tail(&log)));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn find(runtime: &Path, matches: fn(&str) -> bool) -> Option<String> {
    fs::read_dir(runtime)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|name| matches(name))
}

fn is_wayland_socket(name: &str) -> bool {
    name.starts_with("wayland-") && !name.ends_with(".lock")
}

fn is_ipc(name: &str) -> bool {
    name.starts_with("sway-ipc.") && name.ends_with(".sock")
}

/// The configuration has run once the output it sizes is up.
fn output_active(ipc: &Path) -> bool {
    let output = spawn(
        Command::new("swaymsg")
            .arg("--socket")
            .arg(ipc)
            .args(["--type", "get_outputs", "--raw"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .and_then(|child| child.wait_with_output().map_err(|error| error.to_string()));
    let Ok(output) = output else {
        return false;
    };
    serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).is_ok_and(|outputs| {
        outputs.iter().any(|output| {
            output["active"] == serde_json::Value::Bool(true)
                && output["rect"]["width"] == SCREEN.0
                && output["rect"]["height"] == SCREEN.1
        })
    })
}
