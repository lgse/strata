// SPDX-License-Identifier: MIT

//! What a `screenshot` step and a run's closing `screenshot.png` write. The Rune QA target
//! draws offscreen (and the pinned image has no GPU adapter), so a capture would be empty:
//! it gets a text file beside where the image would be instead, saying so, with the
//! accessibility tree as the step found it.

use std::{fs, path::Path};

use serde_json::{Value, json};

use crate::{Application, COMMAND_TIMEOUT, Display};

/// What the text file says first.
pub(crate) const DREW_NOTHING: &str =
    "The Rune target draws offscreen and put nothing on the display, so there is no image.";

/// Takes the screen as `destination`, a `.png`: writes the tree as text in `destination` with
/// the extension `txt`.
pub(crate) fn take(
    application: &mut Application,
    _display: &Display,
    destination: &Path,
) -> Result<(), String> {
    let reply = application.send(&json!({ "query": "tree" }), COMMAND_TIMEOUT)?;
    write_offscreen(destination, &step_text(&reply))
}

/// What a Rune `screenshot` step writes for the `query tree` reply: the tree, or what the
/// application answered instead.
pub(crate) fn step_text(reply: &Value) -> String {
    match reply["result"].as_str() {
        Some(tree) => {
            format!("{DREW_NOTHING} Its accessibility tree as of this step follows.\n\n{tree}")
        }
        None => {
            format!("{DREW_NOTHING} Asked for its accessibility tree, it answered:\n\n{reply}\n")
        }
    }
}

/// The closing screenshot, after the application has stopped answering steps. It says where
/// the last tree is: the run's `tree.txt`, when the application still answered at the end
/// (`with_tree`), and otherwise that there is none.
pub(crate) fn take_last(
    _display: &Display,
    destination: &Path,
    with_tree: bool,
) -> Result<(), String> {
    write_offscreen(destination, &closing_text(with_tree))
}

pub(crate) fn closing_text(with_tree: bool) -> String {
    if with_tree {
        format!("{DREW_NOTHING} Its last accessibility tree is tree.txt, beside this file.\n")
    } else {
        format!(
            "{DREW_NOTHING} The application no longer answered at the end, so no last tree \
             was read and there is no tree.txt.\n"
        )
    }
}

/// Writes `text` where the image `destination` would be, with the extension `txt`.
pub(crate) fn write_offscreen(destination: &Path, text: &str) -> Result<(), String> {
    let path = destination.with_extension("txt");
    fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}
