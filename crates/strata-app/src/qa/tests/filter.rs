// SPDX-License-Identifier: MIT

use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};

#[test]
fn a_result_is_found_by_its_path_and_marked_while_enter_opens_it() {
    staged(
        "filter::a_result_is_found_by_its_path_and_marked_while_enter_opens_it",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();
            let result = json!({ "result": "pictures/photo.txt" });
            let expected_path = folder
                .path()
                .join("pictures/photo.txt")
                .display()
                .to_string();
            for step in [
                json!({ "key": "ctrl+f" }),
                json!({ "wait": { "field": {} } }),
                json!({ "type": "photo" }),
                json!({ "wait": { "present": result } }),
                json!({ "wait": { "state": { "target": result, "marked": true } } }),
                json!({ "expect": { "description": { "target": result, "text": { "contains": expected_path } } } }),
                json!({ "key": "Down" }),
                json!({ "wait": { "state": { "target": result, "marked": false, "selected": true } } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
