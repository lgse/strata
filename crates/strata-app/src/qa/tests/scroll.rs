// SPDX-License-Identifier: MIT

//! Scroll panes through the engine: a target the Columns strip has scrolled away is wheeled
//! back into view before it is clicked, as
//! `locations/scrolled-away-crumbs-and-columns-stay-reachable` asks at its line 22.

use serde_json::json;
use strata_qa::{Backend as _, Remembered};

use super::{now, passes, staged};

/// Folders nested this deep open more columns than the strip shows.
const DEPTH: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];

#[test]
fn a_click_in_a_column_scrolled_out_of_the_strip_wheels_it_back_first() {
    staged(
        "scroll::a_click_in_a_column_scrolled_out_of_the_strip_wheels_it_back_first",
        None,
        |backend| {
            let folder = tempfile::tempdir().expect("folder");
            let deepest = DEPTH
                .iter()
                .fold(folder.path().to_path_buf(), |path, name| path.join(name));
            std::fs::create_dir_all(&deepest).expect("the folders");
            std::fs::create_dir(folder.path().join("a/z")).expect("a folder after b");
            backend.open(folder.path());
            now(backend.ready()).expect("the first listing");
            let kept = Remembered::default();
            for name in DEPTH {
                passes(&backend, &kept, &json!({ "select": { "entry": name } }));
                passes(&backend, &kept, &json!({ "wait": { "directory": name } }));
            }
            let z = json!({ "entry": "z", "dir": "a" });
            let strip = json!({ "ancestor": "scroll pane", "of": { "pane": "a" } });
            let out_of_view =
                json!({ "not": { "inside": { "target": z, "of": strip, "axis": "x" } } });
            for step in [
                json!({ "expect": out_of_view }),
                json!({ "select": z }),
                json!({ "wait": { "selected": ["z"], "dir": "a" } }),
            ] {
                passes(&backend, &kept, &step);
            }
        },
    );
}
