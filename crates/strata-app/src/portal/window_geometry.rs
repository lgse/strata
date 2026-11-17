// SPDX-License-Identifier: MIT

//! Hyprland centering rule and single-window parent sizing hints (`src/portal/window_geometry.rs`).

use std::{
    io::{Read as _, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::Value;

const QUERY_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_REPLY_BYTES: u64 = 1024 * 1024;

pub(crate) async fn parent_size_hint(app_id: &str, parent: Option<&str>) -> Option<(i32, i32)> {
    let parent = parent?;
    if !parent.starts_with("wayland:") || app_id.is_empty() {
        return None;
    }
    let socket = hyprland_socket()?;
    let app_id = app_id.to_owned();
    gio::spawn_blocking(move || query_size(&socket, &app_id))
        .await
        .ok()
        .flatten()
}

pub(crate) async fn prepare_chooser_placement() {
    let Some(socket) = hyprland_socket() else {
        return;
    };
    let installed = gio::spawn_blocking(move || install_centering_rule(&socket))
        .await
        .unwrap_or(false);
    if !installed {
        tracing::debug!("Hyprland chooser centering unavailable; retaining compositor placement");
    }
}

fn hyprland_socket() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    socket_path(Path::new(&runtime), &signature)
}

pub(crate) fn socket_path(runtime: &Path, signature: &str) -> Option<PathBuf> {
    let valid = is_instance_signature(signature) && runtime.is_absolute();
    valid.then(|| runtime.join("hypr").join(signature).join(".socket.sock"))
}

/// A Hyprland instance signature: a path component of letters, digits, `_` and `-` only.
fn is_instance_signature(signature: &str) -> bool {
    !signature.is_empty()
        && signature
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn centering_commands() -> [String; 3] {
    let class = format!("^{}$", super::CHOOSER_APPLICATION_ID.replace('.', "[.]"));
    let legacy_match =
        format!("keyword windowrule[strata-file-chooser-center]:match:class {class}");
    [
        format!(
            "/eval hl.window_rule({{name='strata-file-chooser-center',match={{class='{class}'}},center=true}})"
        ),
        format!("/{legacy_match}"),
        format!(
            "[[BATCH]] {legacy_match};keyword windowrule[strata-file-chooser-center]:center on"
        ),
    ]
}

fn install_centering_rule(socket: &Path) -> bool {
    let commands = centering_commands();
    let Some(reply) = exchange(socket, commands[0].as_bytes(), 8192) else {
        return false;
    };
    let reply = String::from_utf8_lossy(&reply);
    match reply.trim() {
        "ok" => return true,
        "unknown request" | "eval is only supported with the lua config manager" => {}
        _ => return false,
    }
    if exchange(socket, commands[1].as_bytes(), 8192).as_deref() != Some(b"ok") {
        return false;
    }
    let Some(reply) = exchange(socket, commands[2].as_bytes(), 8192) else {
        return false;
    };
    String::from_utf8_lossy(&reply)
        .split("\n\n\n")
        .map(str::trim)
        .eq(["ok", "ok"])
}

fn query_size(socket: &Path, app_id: &str) -> Option<(i32, i32)> {
    let reply = exchange(socket, b"j/clients", MAX_REPLY_BYTES)?;
    application_window_size(&reply, app_id)
}

fn exchange(socket: &Path, command: &[u8], max_reply_bytes: u64) -> Option<Vec<u8>> {
    let mut stream = UnixStream::connect(socket).ok()?;
    let _ = stream.set_read_timeout(Some(QUERY_TIMEOUT));
    let _ = stream.set_write_timeout(Some(QUERY_TIMEOUT));
    stream.write_all(command).ok()?;
    let mut reply = Vec::new();
    stream
        .take(max_reply_bytes + 1)
        .read_to_end(&mut reply)
        .ok()?;
    (reply.len() as u64 <= max_reply_bytes).then_some(reply)
}

/// One of the windows `hyprctl -j clients` lists, with the fields the size hint reads.
struct ApplicationWindow {
    class: String,
    initial_class: String,
    mapped: bool,
    hidden: bool,
    size: [i32; 2],
}

impl ApplicationWindow {
    /// Mapped, not hidden, and with a size.
    fn is_shown(&self) -> bool {
        self.mapped && !self.hidden && self.size.iter().all(|size| *size > 0)
    }

    /// `None` when a field is missing or of another type, as a typed parse would be.
    fn read(window: &Value) -> Option<Self> {
        let text = |key: &str| window.get(key)?.as_str().map(str::to_owned);
        let flag = |key: &str| window.get(key)?.as_bool();
        let extent = |value: &Value| i32::try_from(value.as_i64()?).ok();
        let [width, height] = window.get("size")?.as_array()?.as_slice() else {
            return None;
        };
        Some(Self {
            class: text("class")?,
            initial_class: text("initialClass")?,
            mapped: flag("mapped")?,
            hidden: flag("hidden")?,
            size: [extent(width)?, extent(height)?],
        })
    }
}

pub(crate) fn application_window_size(reply: &[u8], app_id: &str) -> Option<(i32, i32)> {
    if app_id.is_empty() {
        return None;
    }
    let listed: Value = serde_json::from_slice(reply).ok()?;
    let windows = listed
        .as_array()?
        .iter()
        .map(ApplicationWindow::read)
        .collect::<Option<Vec<_>>>()?;
    let mut matches = windows.iter().filter(|window| {
        [&window.class, &window.initial_class]
            .iter()
            .any(|class| class.eq_ignore_ascii_case(app_id))
    });
    let window = matches.next()?;
    let unique = matches.next().is_none();
    (unique && window.is_shown()).then_some((window.size[0], window.size[1]))
}
