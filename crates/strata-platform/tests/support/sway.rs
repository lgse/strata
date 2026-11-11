//! A nested headless sway, with Xwayland, and its IPC.
//!
//! Traps found by S1, S2 and S3 under headless sway (docs/testing.md):
//!
//! - `/usr/bin/sway` may carry `cap_sys_nice` and go realtime. Where `RLIMIT_RTTIME` is 0 the
//!   kernel kills it at once, so it starts as `setpriv --no-new-privs sway`, which drops the
//!   file capability.
//! - sway 1.9 refuses to start when it sees NVIDIA's kernel module (a container on an NVIDIA
//!   host) unless given `--unsupported-gpu`; sway 1.12 accepts the flag too.
//! - `swaybg_command -`: no background client.
//! - Xwayland takes the next free display in the shared `/tmp/.X11-unix`; sway puts it in
//!   `DISPLAY` for its children, so the harness asks sway for it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Options, timeout};

pub struct Sway {
    child: Child,
    socket: PathBuf,
    wayland: String,
    display: Option<String>,
    log: PathBuf,
}

impl Sway {
    /// Starts sway and waits for its sockets (and Xwayland's display). `spawned` hears sway's
    /// pid at once, so the watchdog knows it from the start.
    pub(super) fn start(
        dir: &Path,
        logs: &Path,
        opts: &Options,
        renderer: &str,
        env: &[(String, String)],
        spawned: impl FnOnce(u32),
    ) -> Sway {
        let config = dir.join("sway.conf");
        let (w, h) = opts.output;
        std::fs::write(
            &config,
            format!(
                "swaybg_command -\n\
                 output HEADLESS-1 resolution {w}x{h} scale {scale}\n\
                 xwayland {xwayland}\n\
                 default_border none\n\
                 default_floating_border none\n\
                 focus_follows_mouse no\n\
                 focus_on_window_activation focus\n\
                 {extra}\n",
                scale = opts.scale,
                xwayland = if opts.xwayland { "enable" } else { "disable" },
                extra = opts.sway_config,
            ),
        )
        .expect("sway config");
        let log = logs.join("sway.log");
        let out = File::create(&log).expect("sway log");
        let err = out.try_clone().expect("sway log");
        let mut cmd = Command::new("setpriv");
        cmd.args(["--no-new-privs", "sway", "--unsupported-gpu"]);
        // `STRATA_SWAY_DEBUG=1` logs sway's debug output to sway.log.
        if std::env::var("STRATA_SWAY_DEBUG").is_ok_and(|v| v == "1") {
            cmd.arg("-d");
        }
        let mut child = cmd
            .arg("-c")
            .arg(&config)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", renderer)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .unwrap_or_else(|e| panic!("starting sway through setpriv: {e}"));
        spawned(child.id());

        let deadline = Instant::now() + timeout();
        let (wayland, socket) = loop {
            if let Ok(Some(status)) = child.try_wait() {
                panic!("sway exited at start ({status})\n{}", super::tail(&log, 40));
            }
            if let Some(found) = sockets(dir)
                && ipc(&found.1, &["-t", "get_version"]).is_ok()
            {
                break found;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("sway did not start\n{}", super::tail(&log, 40));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut sway = Sway {
            child,
            socket,
            wayland,
            display: None,
            log,
        };
        if opts.xwayland {
            sway.display = Some(sway.x_display(dir));
        }
        sway
    }

    /// sway sets `DISPLAY` for what it runs once Xwayland's socket exists (at start, even when
    /// Xwayland itself starts on first use), so ask a child of sway for it.
    fn x_display(&self, dir: &Path) -> String {
        let file = dir.join("display");
        self.command(&format!("exec printf %s \"$DISPLAY\" > {}", file.display()));
        let deadline = Instant::now() + timeout();
        loop {
            let display = std::fs::read_to_string(&file).unwrap_or_default();
            if !display.is_empty() {
                return display;
            }
            assert!(
                Instant::now() < deadline,
                "sway gave Xwayland no DISPLAY\n{}",
                super::tail(&self.log, 40)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(super) fn wayland_display(&self) -> &str {
        &self.wayland
    }

    pub(super) fn x_display_name(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Runs sway commands and panics unless every one succeeds.
    pub(super) fn command(&self, cmd: &str) -> Value {
        let reply =
            ipc(&self.socket, &["--", cmd]).unwrap_or_else(|e| panic!("swaymsg {cmd}: {e}"));
        let ok = reply
            .as_array()
            .is_some_and(|r| r.iter().all(|c| c["success"] == true));
        assert!(ok, "swaymsg {cmd}: {reply}");
        reply
    }

    pub(super) fn tree(&self) -> Value {
        ipc(&self.socket, &["-t", "get_tree"]).unwrap_or_else(|e| panic!("swaymsg get_tree: {e}"))
    }

    pub(super) fn stop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = ipc(&self.socket, &["exit"]);
            let deadline = Instant::now() + Duration::from_secs(3);
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The Wayland socket name and IPC socket sway made in `dir`, once both exist.
fn sockets(dir: &Path) -> Option<(String, PathBuf)> {
    let mut wayland = None;
    let mut ipc = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("wayland-") && !name.ends_with(".lock") {
            wayland = Some(name);
        } else if name.starts_with("sway-ipc.") && name.ends_with(".sock") {
            ipc = Some(entry.path());
        }
    }
    Some((wayland?, ipc?))
}

fn ipc(socket: &Path, args: &[&str]) -> Result<Value, String> {
    let out = Command::new("swaymsg")
        .arg("-r")
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SWAYSOCK", socket)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(format!(
            "{}: {text}{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    // `swaymsg exit` replies with nothing.
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    XdgShell,
    Xwayland,
}

/// A rectangle in layout points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A toplevel as sway's tree reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// sway's container id: `[con_id=…]` in commands.
    pub id: i64,
    pub pid: u32,
    pub shell: Shell,
    /// The Wayland app id, or the X11 class.
    pub app_id: Option<String>,
    pub title: String,
    /// The client's own area (no border), in layout points.
    pub content: Rect,
    pub focused: bool,
    /// Floating rather than tiled.
    pub floating: bool,
}

impl Rect {
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.width
            && o.x < self.x + self.width
            && self.y < o.y + o.height
            && o.y < self.y + self.height
    }
}

impl Window {
    /// A point inside the window: `fx` and `fy` are fractions of its width and height.
    pub fn at(&self, fx: f64, fy: f64) -> (f64, f64) {
        let c = self.content;
        (c.x + c.width * fx, c.y + c.height * fy)
    }
}

/// Every toplevel in the tree, tiled or floating.
pub(super) fn windows(tree: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

fn walk(node: &Value, out: &mut Vec<Window>) {
    if let Some(w) = window(node) {
        out.push(w);
    }
    for key in ["nodes", "floating_nodes"] {
        for child in node[key].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
}

fn window(n: &Value) -> Option<Window> {
    let shell = match n["shell"].as_str()? {
        "xdg_shell" => Shell::XdgShell,
        "xwayland" => Shell::Xwayland,
        _ => return None,
    };
    let num = |v: &Value| v.as_f64().unwrap_or(0.0);
    let (r, wr) = (&n["rect"], &n["window_rect"]);
    let app_id = n["app_id"]
        .as_str()
        .or_else(|| n["window_properties"]["class"].as_str())
        .map(str::to_owned);
    Some(Window {
        id: n["id"].as_i64()?,
        pid: u32::try_from(n["pid"].as_i64()?).ok()?,
        shell,
        app_id,
        title: n["name"].as_str().unwrap_or_default().to_owned(),
        content: Rect {
            x: num(&r["x"]) + num(&wr["x"]),
            y: num(&r["y"]) + num(&wr["y"]),
            width: num(&wr["width"]),
            height: num(&wr["height"]),
        },
        focused: n["focused"] == true,
        floating: n["type"] == "floating_con",
    })
}
