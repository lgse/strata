// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use serde_json::Value;

use super::sway::Sway;

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

/// The sway the session host may run in, and the size of its output.
pub(super) struct Admitted {
    pub(super) sway: Sway,
    pub(super) screen: (u32, u32),
}

pub(super) fn admit(variable: impl Fn(&str) -> Option<String>) -> Result<Admitted, String> {
    let reached = REACHED_THROUGH.map(|name| (name, variable(name)));
    private(&reached)?;
    let socket = variable("SWAYSOCK").ok_or("the session host needs sway's SWAYSOCK")?;
    let sway = Sway::at(PathBuf::from(socket));
    let screen = headless(&sway.outputs()?)?;
    Ok(Admitted { sway, screen })
}

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
