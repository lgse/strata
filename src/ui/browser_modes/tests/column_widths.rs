// SPDX-License-Identifier: MIT

use super::super::*;
use crate::ui::theme::{TextSize, ThemeManager};

fn settle() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn assert_aligned(heading: &gtk::Widget, mode: &gtk::Label, table: &gtk::Box) {
    let heading = heading.compute_bounds(table).expect("heading bounds");
    let mode = mode.compute_bounds(table).expect("Mode bounds");
    assert_eq!(heading.x(), mode.x());
    assert_eq!(heading.width(), mode.width());
}

#[test]
fn resizing_starts_at_the_visible_header_not_the_loading_placeholder() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::column_widths::resizing_starts_at_the_visible_header_not_the_loading_placeholder",
        || {
            crate::ui::prepare_portal_ui();
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            for viewport in [480, 1200] {
                let columns = ListColumnLayout::new();
                let loading = list_loading_skeleton(&columns);
                let (headings, _) = list_headings(&browser, 0, columns.clone());
                let stack = gtk::Stack::new();
                stack.add_named(&loading, Some("loading"));
                stack.add_named(&headings, Some("content"));
                stack.set_visible_child_name("content");
                let scroll = gtk::ScrolledWindow::builder().child(&stack).build();
                let window = gtk::Window::builder()
                    .default_width(viewport)
                    .child(&scroll)
                    .build();
                window.present();
                settle();
                let mut heading = headings.first_child();
                for index in 0..LIST_COLUMN_WIDTHS.len() {
                    let cell = heading.expect("heading cell");
                    heading = cell.next_sibling();
                    let overlay = cell.first_child().expect("heading overlay");
                    let handle = overlay.last_child().expect("resize handle");
                    let controllers = handle.observe_controllers();
                    let drag = (0..controllers.n_items())
                        .find_map(|position| {
                            controllers
                                .item(position)?
                                .downcast::<gtk::GestureDrag>()
                                .ok()
                        })
                        .expect("resize gesture");
                    let visible_width = || {
                        cell.compute_bounds(&headings)
                            .expect("visible heading bounds")
                            .width()
                            .round() as i32
                    };
                    let original = visible_width();
                    drag.emit_by_name::<()>("drag-begin", &[&0.0f64, &0.0f64]);
                    for offset in [0.0f64, 24.0, 12.0] {
                        drag.emit_by_name::<()>("drag-update", &[&offset, &0.0f64]);
                        settle();
                        assert_eq!(
                            visible_width(),
                            original + offset as i32,
                            "column {index} in viewport {viewport} jumped"
                        );
                    }
                    drag.emit_by_name::<()>("drag-end", &[&12.0f64, &0.0f64]);
                }
                window.close();
            }
        },
    );
}

#[test]
fn mode_fits_default_width_and_remains_resizable() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::column_widths::mode_fits_default_width_and_remains_resizable",
        || {
            let themes = ThemeManager::shared();
            crate::ui::prepare_portal_ui();
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            for size in [11, 13, 15, 24, 32, 48].map(TextSize::new) {
                themes.set_text_size(size);
                for density in ["density-compact", "density-airy"] {
                    for width in [480, 1000] {
                        let columns = ListColumnLayout::new();
                        let (headings, _) = list_headings(&browser, 0, columns.clone());
                        let row = assemble_list_row();
                        let mut child = row.first_child();
                        for index in 0..5 {
                            let cell = child.expect("row cell");
                            register_list_column_cell(&columns, index, &cell);
                            child = cell.next_sibling();
                        }
                        let (_, name, _, mode, _, _, _) = list_row_parts(&row).expect("row parts");
                        name.set_label("Permissions");
                        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        table.add_css_class("mode-list");
                        table.add_css_class(density);
                        table.append(&headings);
                        table.append(&row);
                        let scroll = gtk::ScrolledWindow::builder().child(&table).build();
                        let window = gtk::Window::builder()
                            .default_width(width)
                            .default_height(120)
                            .child(&scroll)
                            .build();
                        window.present();
                        settle();
                        let widest = (0..=0o777)
                            .map(|bits| {
                                super::super::super::browser::format_permissions(0o040000 | bits)
                            })
                            .max_by_key(|text| mode.create_pango_layout(Some(text)).pixel_size().0)
                            .expect("permission strings");
                        mode.set_label(&widest);
                        settle();
                        assert!(
                            !mode.layout().is_ellipsized(),
                            "{size:?}, {density}, viewport {width}: {widest}, cell {}",
                            mode.width()
                        );
                        let heading = headings
                            .first_child()
                            .expect("Name heading")
                            .next_sibling()
                            .expect("Mode heading");
                        assert_aligned(&heading, &mode, &table);
                        if width == 480 {
                            assert!(
                                scroll.hadjustment().upper() > scroll.hadjustment().page_size()
                            );
                        }
                        for resized in [80, (220.0 * themes.interface_scale()).ceil() as i32] {
                            set_list_column_width(&columns, 1, resized);
                            settle();
                            assert_eq!(mode.width_request(), resized);
                            assert_aligned(&heading, &mode, &table);
                            assert_eq!(mode.layout().is_ellipsized(), resized == 80);
                        }
                        assert!(!columns.name_manually_resized.get());
                        window.close();
                    }
                }
            }
        },
    );
}
