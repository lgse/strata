// SPDX-License-Identifier: MIT

use strata_gallery::{scenes, spec};

fn main() {
    // AppSpec takes a 'static slice; the list is built once for the process.
    rune::run(spec(Vec::leak(scenes::all_scenes())));
}
