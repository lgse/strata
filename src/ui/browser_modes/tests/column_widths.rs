// SPDX-License-Identifier: GPL-3.0-or-later

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
fn mode_fits_default_width_and_remains_resizable() {
    crate::test_support::gtk_test(
        "ui::browser_modes::tests::column_widths::mode_fits_default_width_and_remains_resizable",
        || {
            let themes = ThemeManager::shared();
            crate::ui::prepare_portal_ui();
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            for size in [TextSize::Small, TextSize::Medium, TextSize::Large] {
                themes.set_text_size(size);
                for density in ["density-compact", "density-airy"] {
                    for width in [480, 1000] {
                        let columns = ListColumnLayout::new();
                        let headings = list_headings(&browser, 0, columns.clone());
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
                        for resized in [80, 220] {
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
