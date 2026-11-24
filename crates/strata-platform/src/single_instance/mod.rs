// SPDX-License-Identifier: MIT

//! D-Bus single instance, desktop activation, and opt-in `org.freedesktop.FileManager1` shim.
//!
//! Rune lacks session-bus single-instance registration (`org.freedesktop.Application` /
//! GApplication wire interop), arbitrary byte-string arguments, forwarded `cwd` and activation
//! tokens, remote exit status, and `org.freedesktop.FileManager1`
//! ([anomalyco/rune#22](https://github.com/anomalyco/rune/issues/22)).

mod app_id;
mod event;
mod forward;
mod interfaces;
mod launch;
mod primary;

#[cfg(test)]
mod tests;

pub use app_id::{AppId, InvalidAppId};
pub use event::{CommandLine, Event, Reveal, RevealMethod};
pub use launch::{Activation, Launch};
pub use primary::{Claim, NameRequest, Options, Primary, THREAD_NAME, Unclaimed, claim};
pub use zbus;
