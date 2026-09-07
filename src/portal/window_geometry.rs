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
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let socket = socket_path(Path::new(&runtime), &signature)?;
    query_size(&socket, &app_id, QUERY_TIMEOUT).await
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
            let mut stream = Async::<UnixStream>::connect(socket).await.ok()?;
            stream.write_all(b"j/clients").await.ok()?;
            let mut reply = Vec::new();
            stream
                .take(MAX_REPLY_BYTES + 1)
                .read_to_end(&mut reply)
                .await
                .ok()?;
            if reply.len() as u64 > MAX_REPLY_BYTES {
                return None;
            }
            application_window_size(&reply, app_id)
        },
        async {
            Timer::after(timeout).await;
            None
        },
    )
    .await
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
