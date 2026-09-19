// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{browser_modes::BrowserMode, preferences::PreferenceManager};
use std::time::{Duration, Instant};

fn widget_with_tooltip(widget: &gtk::Widget, tooltip: &str) -> Option<gtk::Widget> {
    if widget.tooltip_text().as_deref() == Some(tooltip) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = widget_with_tooltip(&widget, tooltip) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn chooser_keeps_full_chrome_with_minimal_mode_saved() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::minimal_chrome::chooser_keeps_full_chrome_with_minimal_mode_saved",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            assert!(
                PreferenceManager::shared().minimal_mode(),
                "saved fixture enables minimal mode"
            );
            let directory = tempfile::tempdir().expect("fixture");
            std::fs::write(directory.path().join("a.txt"), b"chooser").expect("fixture file");
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new_chooser(ChooserFileSource::new(), false);
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(600)
                    .build();
                window.present();
                view.navigate_location(Location::local(directory.path()));
                let deadline = Instant::now() + Duration::from_secs(5);
                while !view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
                {
                    assert!(Instant::now() < deadline, "{mode:?} chooser loads");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(2));
                }
                let refresh = widget_with_tooltip(&view.widget(), "Refresh (F5)")
                    .expect("chooser refresh stays");
                assert!(refresh.is_visible(), "{mode:?} chooser refresh stays");
                let filter_tooltip = match mode {
                    BrowserMode::Columns => "Filter this pane (Ctrl+F)",
                    BrowserMode::Icons => "Filter icons (Ctrl+F)",
                    BrowserMode::List => "Filter list (Ctrl+F)",
                };
                let filter = widget_with_tooltip(&view.widget(), filter_tooltip)
                    .expect("chooser filter stays");
                assert!(filter.is_visible(), "{mode:?} chooser filter stays");
                window.destroy();
            }
        },
    );
}
