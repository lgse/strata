// SPDX-License-Identifier: MIT

//! What keeps the session host off a desktop. It puts a virtual keyboard and pointer on the
//! seat of the compositor it reaches, so it runs only in a private, headless sway: every
//! output sway lists must be a headless one (`HEADLESS-*`, as `WLR_BACKENDS=headless` names
//! them), and no variable it reaches the session through may point into the desktop's
//! runtime directory, `/run/user/<uid>`, the bus's included. The session bus it expects is
//! the runner's private one, or none. A refusal ends the host; it never falls back.

use serde_json::Value;

/// The prefix wlroots' headless backend names its outputs with.
const HEADLESS: &str = "HEADLESS-";
/// Where systemd puts a desktop session's runtime directory, and so its sockets.
const DESKTOP_RUNTIME: &str = "/run/user/";
/// The variables the host reaches its compositor and bus through.
pub(super) const REACHED_THROUGH: [&str; 4] = [
    "XDG_RUNTIME_DIR",
    "WAYLAND_DISPLAY",
    "SWAYSOCK",
    "DBUS_SESSION_BUS_ADDRESS",
];

/// The size of the active output of sway's `GET_OUTPUTS` reply, when every output is
/// headless.
pub(super) fn headless(outputs: &Value) -> Result<(u32, u32), String> {
    let outputs = outputs.as_array().map_or(&[][..], Vec::as_slice);
    if outputs.is_empty() {
        return Err("sway lists no output".to_owned());
    }
    for output in outputs {
        let name = output["name"].as_str().unwrap_or_default();
        if !name.starts_with(HEADLESS) {
            return Err(format!(
                "sway has the output {name:?}, which is not a headless one: the session host \
                 runs only in a private headless sway (WLR_BACKENDS=headless), never on a desktop"
            ));
        }
    }
    let active = outputs
        .iter()
        .find(|output| output["active"] == true)
        .ok_or("sway has no active output")?;
    let pixels = |field: &str| {
        let value = active["rect"][field]
            .as_u64()
            .ok_or("sway gave the output no size")?;
        u32::try_from(value).map_err(|error| error.to_string())
    };
    Ok((pixels("width")?, pixels("height")?))
}

/// Refuses a session reached through the desktop's runtime directory. `variables` holds each
/// of [`REACHED_THROUGH`] with its value, if it has one.
pub(super) fn private(variables: &[(&str, Option<String>)]) -> Result<(), String> {
    for (name, value) in variables {
        if let Some(value) = value
            && value.contains(DESKTOP_RUNTIME)
        {
            return Err(format!(
                "{name}={value} is in the desktop's runtime directory: the session host runs \
                 only in a private session, never on a desktop"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
