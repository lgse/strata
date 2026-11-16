// SPDX-License-Identifier: MIT

//! Hyprland centering rule and single-window parent sizing hints (`src/portal/window_geometry.rs`).

use std::{
    io::{Read as _, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

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
    let valid_chars = signature
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !runtime.is_absolute() || signature.is_empty() || !valid_chars {
        return None;
    }
    Some(runtime.join("hypr").join(signature).join(".socket.sock"))
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

#[derive(Deserialize)]
struct ApplicationWindow {
    class: String,
    #[serde(rename = "initialClass")]
    initial_class: String,
    mapped: bool,
    hidden: bool,
    size: [i32; 2],
}

pub(crate) fn application_window_size(reply: &[u8], app_id: &str) -> Option<(i32, i32)> {
    if app_id.is_empty() {
        return None;
    }
    let windows: Vec<ApplicationWindow> = serde_json::from_slice(reply).ok()?;
    let mut matches = windows.iter().filter(|window| {
        [&window.class, &window.initial_class]
            .iter()
            .any(|class| class.eq_ignore_ascii_case(app_id))
    });
    let window = matches.next()?;
    if matches.next().is_some()
        || !window.mapped
        || window.hidden
        || window.size.iter().any(|size| *size <= 0)
    {
        return None;
    }
    Some((window.size[0], window.size[1]))
}
