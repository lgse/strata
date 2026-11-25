// SPDX-License-Identifier: MIT

//! D-Bus single instance, desktop activation, and opt-in `org.freedesktop.FileManager1` shim.
//!
//! Rune ships `rune::single_instance` with `org.freedesktop.Application` (`Activate`/`Open`),
//! while this shim remains needed for `org.freedesktop.FileManager1`, arbitrary `OsString`
//! (byte-string) arguments with forwarded `cwd`, forwarded activation tokens, and
//! `org.gtk.Application` `CommandLine` wire interop with remote exit status.

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
