// SPDX-License-Identifier: MIT

mod click;

use super::*;
use crate::test_support::gtk_test;

#[test]
fn drag_threshold_matches_gtk_on_both_axes() {
    let press = (10.0, 20.0);
    for point in [(18.0, 28.0), (2.0, 12.0), press] {
        assert!(!exceeds_drag_threshold(press, point, 8));
    }
    for point in [(19.0, 20.0), (1.0, 20.0), (10.0, 29.0), (10.0, 11.0)] {
        assert!(exceeds_drag_threshold(press, point, 8));
    }
}

#[test]
fn expanded_labels_leave_inert_space_but_icons_and_editors_do_not() {
    gtk_test(
        "ui::pointer::tests::expanded_labels_leave_inert_space_but_icons_and_editors_do_not",
        || {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("file-row");
            let icon = super::super::thumbnail::ThumbnailSlot::new(18);
            let label = gtk::Label::new(Some("file.txt"));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&icon);
            row.append(&label);
            let window = gtk::Window::builder()
                .child(&row)
                .default_width(400)
                .build();
            window.present();
            pump_until(|| label.width() > 200);
            let hit = |widget: &gtk::Widget, x: f32| {
                let point = widget
                    .compute_point(
                        &row,
                        &gtk::graphene::Point::new(x, widget.height() as f32 / 2.0),
                    )
                    .expect("point in row");
                hits_item_content(row.upcast_ref(), f64::from(point.x()), f64::from(point.y()))
            };
            assert_eq!(icon.accessible_role(), gtk::AccessibleRole::Img);
            assert!(hit(icon.upcast_ref(), 9.0));
            assert!(hit(label.upcast_ref(), 4.0));
            assert!(!hit(label.upcast_ref(), label.width() as f32 - 4.0));

            label.set_xalign(1.0);
            pump_until(|| label.layout_offsets().0 > 100);
            assert!(!hit(label.upcast_ref(), 4.0));
            assert!(hit(label.upcast_ref(), label.width() as f32 - 4.0));

            label.set_text(&"very long filename ".repeat(40));
            pump_until(|| label.layout().is_ellipsized());
            assert!(hit(label.upcast_ref(), label.width() as f32 / 2.0));
            window.close();
        },
    );
}

#[test]
fn icons_have_inert_gutters_beside_the_thumbnail() {
    gtk_test(
        "ui::pointer::tests::icons_have_inert_gutters_beside_the_thumbnail",
        || {
            let card = super::super::icons_cell::new_card(64);
            let (icon, label) = super::super::icons_cell::parts(&card).expect("card parts");
            label.set_text(Some("file.txt"));
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            pump_until(|| icon.width() > 0);
            let bounds = icon.compute_bounds(&card).expect("icon bounds");
            let y = f64::from(bounds.y() + bounds.height() / 2.0);
            assert!(hits_item_content(
                card.upcast_ref(),
                f64::from(bounds.x() + 10.0),
                y
            ));
            assert!(!hits_item_content(card.upcast_ref(), 2.0, y));
            let field = super::super::icons_cell::ensure_rename_field(&card).expect("rename field");
            field.set_visible(true);
            pump_until(|| field.width() > 0);
            let bounds = field.compute_bounds(&card).expect("field bounds");
            assert!(hits_item_content(
                card.upcast_ref(),
                f64::from(bounds.x() + 4.0),
                f64::from(bounds.y() + 4.0)
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
