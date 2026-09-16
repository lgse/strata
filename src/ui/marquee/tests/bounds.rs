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

#[test]
fn list_name_policy_uses_mapped_row_coordinates_and_preserves_other_targets() {
    gtk_test(
        "ui::marquee::tests::bounds::list_name_policy_uses_mapped_row_coordinates_and_preserves_other_targets",
        || {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("list-row");
            row.set_size_request(480, 64);
            let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            name_cell.add_css_class("list-name-cell");
            name_cell.set_hexpand(true);
            name_cell.set_margin_top(8);
            name_cell.set_margin_bottom(8);
            let name = gtk::Label::new(Some("file.txt"));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name_cell.append(&name);
            let metadata = gtk::Label::new(Some("File"));
            metadata.set_size_request(80, -1);
            row.append(&name_cell);
            row.append(&metadata);
            let surface = gtk::Fixed::new();
            surface.put(&row, 24.0, 18.0);
            let window = gtk::Window::builder()
                .child(&surface)
                .default_width(520)
                .default_height(110)
                .build();
            window.present();
            pump_until(|| name.width() > 200);
            let targets = Rc::new(RefCell::new(vec![MarqueeTarget {
                selection: gtk::MultiSelection::new(Some(gtk::StringList::new(&["file.txt"]))),
                visit_items: Rc::new({
                    let row = row.clone();
                    move |visit| visit(0, row.upcast_ref())
                }),
            }]));
            let predicate = item_content_predicate(
                targets,
                Rc::new(crate::ui::pointer::hits_list_item_content),
            );
            let hit = |widget: &gtk::Widget, x: f32, y: f32| {
                let point = widget
                    .compute_point(&surface, &graphene::Point::new(x, y))
                    .expect("point on the collection surface");
                predicate(
                    surface.upcast_ref(),
                    f64::from(point.x()),
                    f64::from(point.y()),
                )
            };
            assert!(hit(name.upcast_ref(), 4.0, name.height() as f32 / 2.0));
            assert!(!hit(
                name.upcast_ref(),
                name.width() as f32 - 4.0,
                name.height() as f32 / 2.0
            ));
            assert!(!hit(row.upcast_ref(), name_cell.width() as f32 / 2.0, 1.0));
            assert!(!hit(
                row.upcast_ref(),
                name_cell.width() as f32 / 2.0,
                row.height() as f32 - 1.0
            ));
            assert!(hit(
                metadata.upcast_ref(),
                4.0,
                metadata.height() as f32 / 2.0
            ));
            assert!(!predicate(surface.upcast_ref(), 2.0, 2.0));
            let point = metadata
                .compute_point(
                    &surface,
                    &graphene::Point::new(4.0, metadata.height() as f32 / 2.0),
                )
                .expect("mapped metadata position");
            row.set_visible(false);
            assert!(!predicate(
                surface.upcast_ref(),
                f64::from(point.x()),
                f64::from(point.y())
            ));
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
