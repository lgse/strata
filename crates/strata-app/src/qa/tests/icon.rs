// SPDX-License-Identifier: MIT

//! An entry's `part: icon`, which scenarios click and drag from: the row's drawn icon.

use serde_json::json;
use strata_qa::Remembered;

use super::{launched, passes, staged};

#[test]
fn a_click_on_a_rows_icon_lands_on_the_icon_the_view_draws() {
    staged(
        "icon::a_click_on_a_rows_icon_lands_on_the_icon_the_view_draws",
        None,
        |backend| {
            let _folder = launched(&backend);
            let kept = Remembered::default();
            passes(
                &backend,
                &kept,
                &json!({ "click": { "entry": "todo.txt", "part": "icon" } }),
            );
            passes(
                &backend,
                &kept,
                &json!({ "wait": { "selected": ["todo.txt"] } }),
            );
        },
    );
}
