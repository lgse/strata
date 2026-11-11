//! Single instance and activation over D-Bus: one primary process per app id, and every later
//! launch handed to it.
//!
//! Rune lacks: on Linux, Rune hands a second launch to a new process, with no way to reach the
//! one already running. It has the window half: `AppHandle::activate(token)` shows and focuses
//! a window with a forwarded activation token, and a window opened with `rune::open_window`
//! from an app whose handle was given one is created with it.
//!
//! Rune issues: upstream [#22](https://github.com/anomalyco/rune/issues/22) (open-URL and
//! reopen through `org.freedesktop.Application`, single instance). U-P8-1 (draft, not filed):
//! on X11, `activate(token)` on a window already open drops the `DESKTOP_STARTUP_ID`. It sends
//! `_NET_ACTIVE_WINDOW` with no timestamp and never ends the startup notification, so window
//! managers that prevent focus stealing (mutter, KWin) may refuse it. Its text is in
//! `docs/decisions/S5-upstream.md`, "Drafts added by later work packages", `#u-p8-1`.
//!
//! Work package: P8, `docs/work/phase-1-leaves/P8-single-instance.md`. One change from it:
//! the `org.freedesktop.FileManager1` interface is served here, opted into with
//! [`Options::file_manager1`]. Its calls arrive as [`Reveal`] data; what to open and select
//! stays the app's policy.
//!
//! # The shape
//!
//! [`claim`] runs first in `main`. It connects to the session bus, exports GApplication's
//! interfaces at the app id's object path, then asks for the app id as a well-known name with
//! `DO_NOT_QUEUE`. The bus decides, so of launches at the same moment exactly one becomes the
//! [`Primary`], and the others forward their launch to it and exit with its answer:
//!
//! ```ignore
//! let id = AppId::new("org.example.Files")?;
//! let primary = match claim(&id, Launch::from_env(), &Options::default()) {
//!     Claim::Primary(p) | Claim::Standalone(p, _) => p,
//!     Claim::Forwarded(status) => std::process::exit(status),
//! };
//! rune::set_app_id("org.example.Files");
//! let app = rune::App::new(spec).windowless().args(Vec::new());
//! primary.attach_rune(app.handle(), on_event);
//! app.run();
//! primary.release();
//!
//! // On Rune's loop, in arrival order: this process's own launch first.
//! fn on_event(s: &mut State, event: Event, tx: &mut rune::Tx) {
//!     if let Event::CommandLine(cmd) = event {
//!         // Open a window for the launch. A window's app whose handle is given the token
//!         // (`app.handle().activate(token)`) before `rune::open_window` is created with it,
//!         // so the compositor lets it take focus.
//!         let token = cmd.activation().token();
//!         // …; dropping `cmd` answers the launch with exit status 0.
//!     }
//! }
//! ```
//!
//! # The wire protocol
//!
//! GApplication's, from `gio/gapplicationimpl-dbus.c`, so this primary and a GApplication of
//! the same id stand in for each other: a launch of either reaches whichever runs. That covers
//! an app moving from GTK to Rune while the old version is still running.
//!
//! - `org.gtk.Application`: `Activate`, `Open` and `CommandLine(o, aay, a{sv}) → i`. The
//!   arguments are byte strings, so non-UTF-8 paths survive. The platform data carries `cwd`,
//!   `activation-token` (`XDG_ACTIVATION_TOKEN`) and `desktop-startup-id`
//!   (`DESKTOP_STARTUP_ID`). The primary prints to the launch's stdout and stderr by calling
//!   the launch's `org.gtk.private.CommandLine` object, then answers with its exit status.
//! - `org.freedesktop.Application`: `Activate`, `Open` and `ActivateAction`, for D-Bus
//!   activation and `gapplication`.
//! - `org.gtk.Actions`, with no actions: a GApplication launch lists them before it forwards.
//!
//! With no session bus, or one that doesn't answer within [`Options::bus_timeout`], a launch
//! runs alone ([`Claim::Standalone`]), so two such launches each run their own instance.
//!
//! # Threads
//!
//! The primary's connection lives on a thread of its own ([`THREAD_NAME`]), running a tokio
//! current-thread runtime, as Rune runs its own D-Bus services. Events reach the app from
//! there through [`Primary::attach_rune`] (`AppHandle::update`) or [`Primary::attach`].
//!
//! # Versions
//!
//! zbus types appear in this module's API ([`Primary::connection`], the [`zbus`] re-export,
//! `OwnedVariant` in [`Event::Action`]). A move to another zbus major is a minor release of
//! this crate, as a Rune rev move is.

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
