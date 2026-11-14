// SPDX-License-Identifier: MIT

//! The filter's results as the QA backend reads them in Columns: each result's path, which the
//! `result` target aims by, and the Enter mark, read as the `marked` state.

use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};

/// A nested result shows its path under its name, which the `result` target finds it by, and
/// is described by its kind, as on GTK (`describe_entry`); the top result carries the Enter
/// mark while the field has the keyboard, and loses it once Down picks it (GTK's
/// `ENTER_TARGET_CLASS`).
#[test]
fn a_result_is_found_by_its_path_and_marked_while_enter_opens_it() {
    staged(
        "filter::a_result_is_found_by_its_path_and_marked_while_enter_opens_it",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            let result = json!({ "result": "pictures/photo.txt" });
            for step in [
                json!({ "key": "ctrl+f" }),
                json!({ "wait": { "field": {} } }),
                json!({ "type": "photo" }),
                json!({ "wait": { "present": result } }),
                json!({ "wait": { "state": { "target": result, "marked": true } } }),
                // Described by its kind, as on GTK, not by its path.
                json!({ "expect": { "description": { "target": result, "text": "File" } } }),
                json!({ "key": "Down" }),
                json!({ "wait": { "state": { "target": result, "marked": false, "selected": true } } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
