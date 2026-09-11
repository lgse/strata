// SPDX-License-Identifier: MIT

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
            for size in [11, 13, 15, 24, 32].map(TextSize::new) {
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
                        main_bounds.height(),
                        "{size:?}, {mode:?}"
                    );
                    if size == TextSize::default() {
                        assert_eq!(main_bounds.height(), 41.0, "compact header: {size:?}");
                    }
                    let toggle_icon = fixture
                        .content
                        .header
                        .sidebar_toggle
                        .child()
                        .expect("toggle icon");
                    let toggle_image = toggle_icon.downcast::<gtk::Image>().expect("toggle image");
                    assert_eq!(
                        toggle_image.pixel_size(),
                        (17.0 * size.root_font_px() as f64 / 13.0).round() as i32
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
                    let icon_size = (16.0 * size.root_font_px() as f64 / 13.0).round() as f32;
                    assert_eq!(main_icon.width(), icon_size);
                    assert_eq!(main_icon.height(), icon_size);
                    let close = descendant(&main_header, "header-actions")
                        .expect("main actions")
                        .last_child()
                        .expect("close button");
                    let filter = descendant(&pane_header, "icons-header-actions")
                        .expect("pane actions")
                        .last_child()
                        .expect("filter button");
                    let close_icon = descendant(&close, "chrome-icon")
                        .expect("close icon")
                        .compute_bounds(&fixture.window)
                        .expect("close icon bounds");
                    let filter_icon = descendant(&filter, "chrome-icon")
                        .expect("filter icon")
                        .compute_bounds(&fixture.window)
                        .expect("filter icon bounds");
                    assert_eq!(
                        filter_icon.center().x(),
                        close_icon.center().x(),
                        "filter and close alignment: {size:?}, {mode:?}"
                    );
                    for class in ["list-navigation-button", "column-header-action"] {
                        let button = descendant(&pane_header, class).expect("toolbar action");
                        let bounds = button
                            .compute_bounds(&fixture.window)
                            .expect("pane button bounds");
                        if size == TextSize::default() {
                            assert_eq!(
                                bounds.height(),
                                if class == "column-header-action" {
                                    28.0
                                } else {
                                    main_button.height()
                                },
                                "{size:?}, {mode:?}, {class}"
                            );
                        }
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

#[test]
fn preview_header_matches_column_height_and_navigation_controls() {
    gtk_test(
        "ui::window::composition::tests::header_geometry::preview_header_matches_column_height_and_navigation_controls",
        || {
            crate::ui::prepare_portal_ui();
            let fixture = Fixture::new();
            let directory = tempfile::tempdir().expect("fixture directory");
            let path = directory.path().join("preview.txt");
            std::fs::write(&path, "Preview geometry").expect("write preview fixture");
            fixture
                .content
                .browser
                .browser()
                .navigate(crate::model::Location::local(
                    directory.path().to_path_buf(),
                ));
            fixture.preferences.set_browser_mode(BrowserMode::Columns);
            fixture.content.preview.show(crate::model::FileEntry {
                location: crate::model::Location::local(path),
                native_name: "preview.txt".into(),
                thumbnail_path: None,
                display_name: "preview.txt".into(),
                kind: crate::model::EntryKind::File,
                size: crate::model::MetadataValue::Known(16),
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                mode: crate::model::MetadataValue::Unknown,
                is_hidden: false,
            });
            for size in [11, 13, 15, 24, 32].map(TextSize::new) {
                fixture.preferences.set_text_size(size);
                settle();
                let root = fixture.window.clone().upcast::<gtk::Widget>();
                let bounds = |widget: &gtk::Widget| {
                    widget
                        .compute_bounds(&fixture.window)
                        .expect("widget bounds")
                };
                let dimensions =
                    |widget: &gtk::Widget| (bounds(widget).width(), bounds(widget).height());
                let preview = descendant(&root, "preview-header").expect("preview-header");
                let column = descendant(&root, "column-header").expect("column-header");
                assert_eq!(
                    bounds(&preview).height(),
                    bounds(&column).height(),
                    "{size:?}"
                );
                let close = descendant(&root, "header-actions")
                    .expect("header actions")
                    .last_child()
                    .expect("window close button");
                let preview_close = descendant(&preview, "preview-close").expect("preview-close");
                assert_eq!(dimensions(&preview_close), dimensions(&close), "{size:?}");
                let icon = descendant(&close, "chrome-icon").expect("chrome-icon");
                let preview_icon = descendant(&preview_close, "chrome-icon").expect("chrome-icon");
                assert_eq!(dimensions(&preview_icon), dimensions(&icon));
                if size == TextSize::default()
                    && let Some(output) = std::env::var_os("STRATA_PREVIEW_HEADER_CAPTURE")
                {
                    let snapshot = gtk::Snapshot::new();
                    gtk::WidgetPaintable::new(Some(&fixture.window)).snapshot(
                        &snapshot,
                        f64::from(fixture.window.width()),
                        f64::from(fixture.window.height()),
                    );
                    fixture
                        .window
                        .renderer()
                        .expect("window renderer")
                        .render_texture(snapshot.to_node().expect("window render node"), None)
                        .save_to_png(std::path::PathBuf::from(output))
                        .expect("save preview header capture");
                }
                assert_eq!(
                    bounds(&preview_icon).center().x(),
                    bounds(&icon).center().x(),
                    "{size:?}"
                );
                let mut child = preview.first_child();
                while let Some(widget) = child {
                    if widget.has_css_class("preview-header-action") && widget.is_visible() {
                        assert_eq!(dimensions(&widget), dimensions(&close));
                    }
                    child = widget.next_sibling();
                }
            }
            fixture.close();
        },
    );
}
