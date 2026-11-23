// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    io::Write as _,
    sync::{Arc, Mutex},
};

use strata_qa::Modifier;

const VARIABLE: &str = "STRATA_INPUT_LOG";

#[derive(Clone)]
pub(super) struct InputLog(Option<Arc<Mutex<File>>>);

/// A modifier as the log names it.
pub(super) fn modifier_name(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Ctrl => "ctrl",
        Modifier::Shift => "shift",
        Modifier::Alt => "alt",
        Modifier::AltGr => "altgr",
    }
}

fn direction(pressed: bool) -> &'static str {
    if pressed { "down" } else { "up" }
}

impl InputLog {
    pub(super) fn from_environment() -> Result<Self, String> {
        let Some(path) = std::env::var_os(VARIABLE) else {
            return Ok(Self(None));
        };
        let file = File::options()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("cannot open {VARIABLE} {}: {error}", path.display()))?;
        Ok(Self(Some(Arc::new(Mutex::new(file)))))
    }

    fn line(&self, line: &str) {
        if let Some(file) = &self.0
            && let Ok(mut file) = file.lock()
        {
            let _ = writeln!(file, "{line}");
        }
    }

    pub(super) fn key(&self, name: &str, pressed: bool) {
        self.line(&format!("key {name} {}", direction(pressed)));
    }

    pub(super) fn motion(&self, (x, y): (i16, i16)) {
        self.line(&format!("motion {x} {y}"));
    }

    pub(super) fn button(&self, (x, y): (i16, i16), number: u8, pressed: bool) {
        self.line(&format!(
            "pointer {x} {y} button {number} {}",
            direction(pressed)
        ));
    }
}
