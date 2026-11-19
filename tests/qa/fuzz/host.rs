// SPDX-License-Identifier: MIT

//! Where a Rune replay runs: on the offscreen host, as every scenario run does by default,
//! or on the session host, a real window in a private headless sway with input through
//! Wayland's virtual keyboard and pointer (`docs/testing.md`, "The session host").
//!
//! The host is the Rune target's. A GTK replay runs on Xvfb whatever the host, as the GTK
//! side of every parity run does, so it stays the reference a session-host replay is held to.
//! An alphabet names the host its sequences need (`host = "session"`), and `--host` replays
//! a command's Rune side on another, as a survey.

use crate::{DisplayKind, target::Target};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Host {
    #[default]
    Offscreen,
    Session,
}

impl Host {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text {
            "offscreen" => Ok(Self::Offscreen),
            "session" => Ok(Self::Session),
            other => Err(format!("--host is offscreen or session, not {other:?}")),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Offscreen => "offscreen",
            Self::Session => "session",
        }
    }

    /// The display a replay of `target` starts on this host: the runner's private sway for a
    /// Rune replay on the session host, where `strata-rune` serves from a real window, and
    /// otherwise what `STRATA_QA_DISPLAY` names, Xvfb by default.
    pub(crate) fn display(self, target: Target) -> Result<DisplayKind, String> {
        match (self, target) {
            (Self::Session, Target::Rune) => Ok(DisplayKind::Wayland),
            _ => DisplayKind::from_environment(),
        }
    }

    /// What the artifacts' folder adds to the targets' label: nothing on the offscreen host,
    /// so that its folders keep their names, and `-session` when a Rune replay is on the
    /// session host, so that a survey there never removes what an offscreen run kept.
    pub(crate) fn suffix(self, targets: &[Target]) -> &'static str {
        if self == Self::Session && targets.contains(&Target::Rune) {
            "-session"
        } else {
            ""
        }
    }
}
