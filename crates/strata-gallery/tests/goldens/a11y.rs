//! Each golden scene's accessibility tree, without bounds, scroll offsets, text selection or
//! shortcuts, against `tests/golden/a11y/<id>/<scene>.json`.

use std::collections::BTreeSet;

use crate::support::{self, Mode};

pub fn check(id: &str) {
    let page = support::page(id);
    let mut failures = Vec::new();
    for &(name, scene) in page.scenes {
        if support::mode(id, name) == Mode::Skip {
            continue;
        }
        let mut tree = support::mount(scene).accessibility_json();
        support::strip(&mut tree);
        support::text_golden(
            "a11y",
            id,
            name,
            name,
            &support::pretty(&tree),
            &mut failures,
        );
    }
    let names: BTreeSet<String> = page.scenes.iter().map(|(n, _)| (*n).to_owned()).collect();
    support::orphans("a11y", id, "json", &names, &mut failures);
    support::report(id, "a11y", failures);
}
