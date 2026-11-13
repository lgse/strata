// SPDX-License-Identifier: MIT

//! `replaced`, as GTK's rows answer it: a reload replaces the rows of the column it reloads,
//! and a change the browser splices in keeps the others (`qa::replaced`).
//! `locations/unreadable-location-reports-error` waits for it after F5, at its line 7.

use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};

#[test]
fn f5_replaces_a_remembered_row_and_a_new_file_beside_it_does_not() {
    staged(
        "replaced::f5_replaces_a_remembered_row_and_a_new_file_beside_it_does_not",
        None,
        |backend| {
            let folder = launched(&backend);
            let kept = Remembered::default();
            let replaced =
                json!({ "replaced": { "target": { "entry": "todo.txt" }, "since": "listed" } });
            let remember = json!({ "remember": { "entry": "todo.txt" }, "as": "listed" });
            passes(&backend, &kept, &remember);

            std::fs::write(folder.path().join("added.txt"), "new\n").expect("a new file");
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "entry": "added.txt" } }),
            );
            passes(&backend, &kept, &json!({ "sleep": 300 }));
            let held = passes(&backend, &kept, &json!({ "holds": replaced }));
            assert_eq!(held, json!(false), "a file listed beside it keeps the row");

            passes(&backend, &kept, &json!({ "key": "F5" }));
            passes(&backend, &kept, &json!({ "wait": replaced }));
        },
    );
}
