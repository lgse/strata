// SPDX-License-Identifier: MIT

//! What a `screenshot` step and a run's closing `screenshot.png` take of each target. The
//! GTK application draws on the run's display, which the runner captures. The Rune target
//! draws offscreen and puts nothing there, so a capture would be an empty screen: it gets
//! a text file beside where the image would be instead, saying so, with the accessibility
//! tree as the step found it. The pinned image has no GPU adapter for Rune to draw its own
//! frame with (`docs/rune/parity/owner.md`, H3).

use std::{fs, path::Path};

use serde_json::json;

use crate::{Application, COMMAND_TIMEOUT, Display, target::Target};

/// What the text file says first.
pub(crate) const DREW_NOTHING: &str =
    "The Rune target draws offscreen and put nothing on the display, so there is no image.";

/// Takes the screen as `destination`, a `.png`: an image on GTK, and on Rune the tree as text
/// in `destination` with the extension `txt`.
pub(crate) fn take(
    application: &mut Application,
    display: &Display,
    destination: &Path,
) -> Result<(), String> {
    if !draws_offscreen() {
        return display.screenshot(destination);
    }
    let reply = application.send(&json!({ "query": "tree" }), COMMAND_TIMEOUT)?;
    let tree = reply["result"].as_str().unwrap_or_default();
    write_offscreen(
        destination,
        &format!("{DREW_NOTHING} Its accessibility tree as of this step follows.\n\n{tree}"),
    )
}

/// The closing screenshot, after the application has stopped answering steps: on Rune it
/// points at the run's `tree.txt`, which holds the last tree.
pub(crate) fn take_last(display: &Display, destination: &Path) -> Result<(), String> {
    if !draws_offscreen() {
        return display.screenshot(destination);
    }
    write_offscreen(
        destination,
        &format!("{DREW_NOTHING} Its last accessibility tree is tree.txt, beside this file.\n"),
    )
}

fn draws_offscreen() -> bool {
    Target::from_environment() == Ok(Target::Rune)
}

/// Writes `text` where the image `destination` would be, with the extension `txt`.
pub(crate) fn write_offscreen(destination: &Path, text: &str) -> Result<(), String> {
    let path = destination.with_extension("txt");
    fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}
