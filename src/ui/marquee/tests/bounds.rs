// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;
use std::cell::RefCell;

#[test]
fn a_point_beyond_label_text_but_inside_row_bounds_is_item_space() {
    gtk_test(
        "ui::marquee::tests::bounds::a_point_beyond_label_text_but_inside_row_bounds_is_item_space",
        || {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("file-row");
            let label = gtk::Label::new(Some("file.txt"));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&label);
            let window = gtk::Window::builder()
                .child(&row)
                .default_width(400)
                .build();
            window.present();
            pump_until(|| label.width() > 200);

            let targets: MarqueeTargets = Rc::new(RefCell::new(vec![MarqueeTarget {
                selection: gtk::MultiSelection::new(Some(gtk::StringList::new(&["file.txt"]))),
                visit_items: Rc::new({
                    let row = row.clone();
                    move |visit| visit(0, row.upcast_ref())
                }),
            }]));
            let predicate = item_bounds_predicate(targets);

            let beyond_text = f64::from(label.width()) - 4.0;
            assert!(
                predicate(row.upcast_ref(), beyond_text, f64::from(row.height()) / 2.0),
                "a point inside the row but beyond rendered text is item space by geometry"
            );
            assert!(
                !predicate(row.upcast_ref(), -2.0, f64::from(row.height()) / 2.0),
                "a point outside the row is not item space"
            );
            window.close();
        },
    );
}

fn pump_until(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "widget should be allocated"
        );
        glib::MainContext::default().iteration(true);
    }
}
