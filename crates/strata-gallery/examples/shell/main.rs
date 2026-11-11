//! The shell demo (X1, M1): a themed window with Strata's header, on every strata-platform
//! foundation at once.
//!
//! ```text
//! cargo run -p strata-gallery --example shell -- [--theme ID] [--overrides PATH] [--native]
//!     [--all-buttons] [PATH…]
//! ```
//!
//! - **Single instance (P8).** The first launch owns `io.github.wmfeht.StrataComponents.Shell`
//!   on the session bus (never Strata's own id, and FileManager1 isn't opted into). A later
//!   launch hands its arguments, working directory and activation token over and exits; the
//!   running one opens a window per path, as Strata does, and focuses the last with the token.
//!   A launch without paths brings the newest window forward. The app quits when its last
//!   window closes.
//! - **Decorations (P3).** Each window is borderless (`Policy::Auto`; `--native` keeps the
//!   frame), draws Strata's header (`strata_components::header`) with its window buttons
//!   routed to `csd`, moves and maximises from the header, opens the window menu where the
//!   window manager has one, and resizes from its edges where the platform can (X11).
//!   `--all-buttons` turns on Minimize and Maximize, which a fresh Strata profile hides.
//! - **Desktop settings (P5).** The text scale and the animations switch rebuild the look; the
//!   double-click interval is shown, though the header pairs presses within Rune's fixed 0.5 s
//!   (draft U13, not filed).
//! - **Look (K1, K2, K4).** A bundled theme (`--theme`, default `tokyo-night`), Strata's
//!   default shape, density and type at the desktop's text scale, then the user's overrides
//!   file: `--overrides PATH`, else `$XDG_CONFIG_HOME/strata/overrides.toml`. The demo only
//!   reads it, never writes or creates it. It watches it through a `gio::FileMonitor` on the
//!   GLib bridge and reads it again 75 ms after the last change; every window moves to the new
//!   look as a theme switch does.
//! - **GLib bridge (P9) into StrataDialog (C15).** The header's Mounted volumes button lists
//!   `gio::VolumeMonitor`'s mounts, kept current as they come and go.
//!
//! The window's view is the gallery's (`strata_gallery::scenes::x1::shell`), which the `x1`
//! page draws on fake data. It logs JSON lines on stdout: `claim`, `launch`, `window`,
//! `ready`, `csd`, `desktop`, `overrides` and `volumes`.

mod args;
mod controller;
mod window;

use std::io::Write;

use serde_json::{Value, json};

fn main() {
    let options = match args::Options::parse(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("shell: {why}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    if options.help {
        println!("{}", args::USAGE);
        return;
    }
    std::process::exit(controller::run(options));
}

/// One JSON line on stdout.
pub fn log(event: &str, fields: Value) {
    let mut line = json!({ "ev": event });
    if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
        obj.extend(fields);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
