// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use ashpd::{MaybeAppID, WindowIdentifierType};
use async_io::{Async, Timer};
use futures_lite::{AsyncReadExt, AsyncWriteExt, future};
use serde::Deserialize;

const QUERY_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_REPLY_BYTES: u64 = 1024 * 1024;

// xdg-foreign exports transiency, not geometry. Only use a compositor sizing
// hint when the requesting application has a single identifiable window.
pub(super) async fn parent_size_hint(
    app_id: Option<&MaybeAppID>,
    parent: Option<&WindowIdentifierType>,
) -> Option<(i32, i32)> {
    if !matches!(parent, Some(WindowIdentifierType::Wayland(_))) {
        return None;
    }
    let app_id = app_id?.to_string();
    if app_id.is_empty() {
        return None;
    }
    query_size(&hyprland_socket()?, &app_id, QUERY_TIMEOUT).await
}

pub(super) async fn prepare_chooser_placement() {
    let Some(socket) = hyprland_socket() else {
        return;
    };
    if !install_centering_rule(&socket, QUERY_TIMEOUT).await {
        tracing::debug!("Hyprland chooser centering unavailable; retaining compositor placement");
    }
}

fn hyprland_socket() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    socket_path(Path::new(&runtime), &signature)
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

async fn install_centering_rule(socket: &Path, timeout: Duration) -> bool {
    future::or(
        async {
            let commands = centering_commands();
            let Some(reply) = exchange(socket, commands[0].as_bytes(), 8192).await else {
                return false;
            };
            let reply = String::from_utf8_lossy(&reply);
            match reply.trim() {
                "ok" => return true,
                "unknown request" | "eval is only supported with the lua config manager" => {}
                _ => return false,
            }
            // Probe named-rule support without an effect. Then set both fields in
            // one IPC batch so a config reload cannot remove the match in between.
            if exchange(socket, commands[1].as_bytes(), 8192)
                .await
                .as_deref()
                != Some(b"ok")
            {
                return false;
            }
            let Some(reply) = exchange(socket, commands[2].as_bytes(), 8192).await else {
                return false;
            };
            String::from_utf8_lossy(&reply)
                .split("\n\n\n")
                .map(str::trim)
                .eq(["ok", "ok"])
        },
        async {
            Timer::after(timeout).await;
            false
        },
    )
    .await
}

fn socket_path(runtime: &Path, signature: &str) -> Option<PathBuf> {
    if !runtime.is_absolute()
        || signature.is_empty()
        || !signature
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    Some(runtime.join("hypr").join(signature).join(".socket.sock"))
}

async fn query_size(socket: &Path, app_id: &str, timeout: Duration) -> Option<(i32, i32)> {
    future::or(
        async {
            let reply = exchange(socket, b"j/clients", MAX_REPLY_BYTES).await?;
            application_window_size(&reply, app_id)
        },
        async {
            Timer::after(timeout).await;
            None
        },
    )
    .await
}

async fn exchange(socket: &Path, command: &[u8], max_reply_bytes: u64) -> Option<Vec<u8>> {
    let mut stream = Async::<UnixStream>::connect(socket).await.ok()?;
    stream.write_all(command).await.ok()?;
    let mut reply = Vec::new();
    stream
        .take(max_reply_bytes + 1)
        .read_to_end(&mut reply)
        .await
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

fn application_window_size(reply: &[u8], app_id: &str) -> Option<(i32, i32)> {
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
