// SPDX-License-Identifier: MIT

use std::process::{Command, Stdio};

use x11rb::{
    protocol::{xkb, xproto},
    wrapper::ConnectionExt as _,
};

use crate::{
    DEFAULT_LAYOUT, Display, Server,
    fixture::{Layout, Manifest},
    process::spawn,
};

const ALL_MODIFIERS: u16 = 0xff;

impl Display {
    fn setxkbmap(&self, arguments: &[&str]) -> Result<String, String> {
        // The explicit display and the PATH-only environment keep this off the desktop.
        let output = spawn(
            Command::new("setxkbmap")
                .args(["-display", &self.name])
                .args(arguments)
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .and_then(|child| child.wait_with_output().map_err(|error| error.to_string()))
        .map_err(|error| format!("cannot run setxkbmap: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "setxkbmap {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn lock(&self, group: u8, affect: u16, locks: u16) -> Result<(), String> {
        let connection = self.x11()?;
        xkb::latch_lock_state(
            connection,
            xkb::ID::USE_CORE_KBD.into(),
            xproto::ModMask::from(affect),
            xproto::ModMask::from(locks),
            true,
            xkb::Group::from(group),
            xproto::ModMask::from(0u16),
            false,
            0,
        )
        .map_err(|error| error.to_string())?;
        connection.sync().map_err(|error| error.to_string())
    }

    fn switch_layout(&self, layout: &Layout) -> Result<(), String> {
        let mut arguments = vec!["-layout", layout.layouts.as_str()];
        if !layout.variants.is_empty() {
            arguments.extend(["-variant", layout.variants.as_str()]);
        }
        self.setxkbmap(&arguments)?;
        let caps = if layout.caps_lock {
            u16::from(xproto::ModMask::LOCK)
        } else {
            0
        };
        self.lock(layout.group, caps, caps)
    }

    /// setxkbmap keeps locks, so they are cleared first.
    pub(crate) fn restore_layout(&self) -> Result<(), String> {
        self.lock(0, ALL_MODIFIERS, 0)?;
        self.setxkbmap(&["-layout", DEFAULT_LAYOUT])?;
        let query = self.setxkbmap(&["-query"])?;
        let field = |name: &str| {
            query.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim() == name).then(|| value.trim().to_owned())
            })
        };
        let state = xkb::get_state(self.x11()?, xkb::ID::USE_CORE_KBD.into())
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        let clean = u8::from(state.group) == 0
            && u8::from(state.locked_group) == 0
            && u16::from(state.locked_mods) == 0;
        if field("layout").as_deref() != Some(DEFAULT_LAYOUT)
            || field("variant").is_some()
            || !clean
        {
            return Err(format!(
                "the keyboard was not restored: layout {:?}, variant {:?}, group {}, locked modifiers {:#x}",
                field("layout"),
                field("variant"),
                u8::from(state.group),
                u16::from(state.locked_mods)
            ));
        }
        Ok(())
    }
}

pub(crate) fn switch(manifest: &Manifest, display: &Display, name: &str) -> Result<(), String> {
    // The Wayland injector's virtual keyboard keeps the US keymap it uploaded.
    if matches!(display.kind, Server::Wayland(_)) {
        return if name == DEFAULT_LAYOUT {
            Ok(())
        } else {
            display.x11().map(|_| ())
        };
    }
    if name == DEFAULT_LAYOUT {
        return display.restore_layout();
    }
    let layout = manifest
        .layouts
        .get(name)
        .ok_or_else(|| format!("no layout {name:?} in the manifest"))?;
    display.switch_layout(layout)
}
