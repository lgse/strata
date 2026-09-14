// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;
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

#[test]
fn application_size_hint_survives_presentation() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::sizing::application_size_hint_survives_presentation",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture directory");
            std::fs::write(root.path().join("notes.txt"), "Notes").expect("fixture file");
            ThemeManager::shared().set_browser_mode(BrowserMode::List);
            for (label, parent_size_hint) in [
                ("monitor-fallback", None),
                ("split-window", Some((960, 540))),
            ] {
                let request = ChooserRequest {
                    token: format!("size-{label}"),
                    title: "Choose a file".into(),
                    accept_label: "Open".into(),
                    modal: false,
                    parent: None,
                    parent_size_hint,
                    initial_directory: root.path().into(),
                    kind: ChooserKind::Open {
                        directory: false,
                        multiple: false,
                    },
                    filters: Vec::new(),
                    current_filter: None,
                    choices: Vec::new(),
                };
                let state = build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {})
                    .expect("chooser");
                let context = glib::MainContext::default();
                let deadline = Instant::now() + Duration::from_secs(5);
                while state.window.width() == 0 || !state.window.is_mapped() {
                    context.iteration(false);
                    assert!(
                        Instant::now() < deadline,
                        "chooser receives its initial allocation"
                    );
                }
                if parent_size_hint.is_some() {
                    assert!(
                        state.window.width() <= 960,
                        "width: {}",
                        state.window.width()
                    );
                    assert!(
                        state.window.height() <= 540,
                        "height: {}",
                        state.window.height()
                    );
                    assert!(
                        state.window.default_width() <= 768,
                        "monitor entry must not restore the fullscreen default"
                    );
                }
                if let Some(output) = std::env::var_os("STRATA_CHOOSER_SIZE_VISUALS") {
                    capture(&state.window, Path::new(&output), label);
                }
                state.window.set_default_size(720, 480);
                let surface = state.window.surface().expect("chooser surface");
                let display = gtk::prelude::WidgetExt::display(&state.window);
                let monitor = display
                    .monitor_at_surface(&surface)
                    .expect("chooser monitor");
                surface.emit_by_name::<()>("enter-monitor", &[&monitor]);
                assert_eq!(state.window.default_size(), (720, 480));
                state.cancel();
                while context.pending() {
                    context.iteration(false);
                }
            }
        },
    );
}
