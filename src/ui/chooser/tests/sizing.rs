// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

pub(super) fn capture(window: &gtk::Window, output: &Path, name: &str) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(window)).snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let texture = window
        .renderer()
        .expect("renderer")
        .render_texture(snapshot.to_node().expect("chooser render node"), None);
    std::fs::create_dir_all(output).expect("capture directory");
    texture
        .save_to_png(output.join(format!("{name}.png")))
        .expect("save chooser capture");
}
