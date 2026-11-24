// SPDX-License-Identifier: MIT

use std::{fs, path::Path};

use serde_json::{Value, json};

use crate::{Application, COMMAND_TIMEOUT};

/// What the text file says first.
pub(crate) const DREW_NOTHING: &str =
    "The Rune target draws offscreen and put nothing on the display, so there is no image.";

pub(crate) fn take(application: &mut Application, destination: &Path) -> Result<(), String> {
    let reply = application.send(&json!({ "query": "tree" }), COMMAND_TIMEOUT)?;
    write_offscreen(destination, &step_text(&reply))
}

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

pub(crate) fn take_last(destination: &Path, with_tree: bool) -> Result<(), String> {
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
