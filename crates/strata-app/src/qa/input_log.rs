// SPDX-License-Identifier: MIT

//! With `STRATA_INPUT_LOG` set, each synthetic event is appended in the format
//! `.agents/skills/qa-scenarios/reference.md` defines. Coordinates are window points. A key
//! is written by its name: Rune has no keycodes.

use std::{cell::RefCell, fs::File, io::Write as _};

use strata_qa::Modifier;

const VARIABLE: &str = "STRATA_INPUT_LOG";

pub(super) struct InputLog(Option<RefCell<File>>);

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
        Ok(Self(Some(RefCell::new(file))))
    }

    fn line(&self, line: &str) {
        if let Some(file) = &self.0 {
            let _ = writeln!(file.borrow_mut(), "{line}");
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
