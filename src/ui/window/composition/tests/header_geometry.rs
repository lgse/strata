// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::theme::TextSize;

fn descendant(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = descendant(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn settle() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn icon_and_list_subheaders_preserve_compact_hierarchy() {
    gtk_test(
        "ui::window::composition::tests::header_geometry::icon_and_list_subheaders_preserve_compact_hierarchy",
        || {
            crate::ui::prepare_portal_ui();
            let fixture = Fixture::new();
            let directory = tempfile::tempdir().expect("fixture directory");
            fixture
                .content
                .browser
                .browser()
                .navigate(crate::model::Location::local(
                    directory.path().to_path_buf(),
                ));
            for size in [TextSize::Small, TextSize::Medium, TextSize::Large] {
                fixture.preferences.set_text_size(size);
                for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Icons] {
                    fixture.preferences.set_browser_mode(mode);
                    settle();
                    let pane_header =
                        descendant(&fixture.content.browser.widget(), "mode-pane-header")
                            .expect("directory toolbar");
                    let main_header = fixture
                        .content
                        .header
                        .content
                        .ancestor(gtk::HeaderBar::static_type())
                        .expect("main header");
                    let pane_bounds = pane_header
                        .compute_bounds(&fixture.window)
                        .expect("pane header bounds");
                    let main_bounds = main_header
                        .compute_bounds(&fixture.window)
                        .expect("main header bounds");
                    assert_eq!(
                        pane_bounds.height(),
                        main_bounds.height() - 2.0,
                        "{size:?}, {mode:?}"
                    );
                    let main_button = fixture
                        .content
                        .header
                        .search
                        .compute_bounds(&fixture.window)
                        .expect("main button bounds");
                    let main_icon = descendant(&main_header, "chrome-icon")
                        .expect("main header icon")
                        .compute_bounds(&fixture.window)
                        .expect("main icon bounds");
                    assert_eq!(main_icon.width(), 16.0);
                    assert_eq!(main_icon.height(), 16.0);
                    for class in ["list-navigation-button", "column-header-action"] {
                        let button = descendant(&pane_header, class).expect("toolbar action");
                        let bounds = button
                            .compute_bounds(&fixture.window)
                            .expect("pane button bounds");
                        assert_eq!(
                            bounds.height(),
                            main_button.height(),
                            "{size:?}, {mode:?}, {class}"
                        );
                        let icon = descendant(&button, "chrome-icon")
                            .expect("pane toolbar icon")
                            .compute_bounds(&fixture.window)
                            .expect("pane icon bounds");
                        assert_eq!(icon.width(), main_icon.width());
                        assert_eq!(icon.height(), main_icon.height());
                        assert!(bounds.y() > pane_bounds.y());
                        assert!(
                            bounds.y() + bounds.height() < pane_bounds.y() + pane_bounds.height()
                        );
                    }
                }
            }
            fixture.close();
        },
    );
}
