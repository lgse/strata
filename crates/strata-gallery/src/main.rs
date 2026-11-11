//! `cargo run -p strata-gallery` opens the gallery on its index; Escape goes back to it.
//! `-- --snap out.png --scene gallery-index-dark` renders a scene headless, and
//! `-- --scene <scene> --script tests/scripts/<id>/<name>.json` drives one.

use strata_gallery::{scenes, spec};

fn main() {
    // AppSpec takes a 'static slice; the list is built once for the process.
    rune::run(spec(Vec::leak(scenes::all_scenes())));
}
