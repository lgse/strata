// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn custom_controls_and_artwork_do_not_start_marquees_from_chrome() {
    crate::test_support::gtk_test(
        "ui::marquee::tests::custom_controls_and_artwork_do_not_start_marquees_from_chrome",
        || {
            let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let blank = gtk::Box::new(gtk::Orientation::Vertical, 0);
            surface.append(&blank);
            assert!(is_inert_chrome(surface.upcast_ref(), blank.upcast_ref()));
            for role in [gtk::AccessibleRole::Slider, gtk::AccessibleRole::Img] {
                let control = glib::Object::builder::<gtk::DrawingArea>()
                    .property("accessible-role", role)
                    .build();
                blank.append(&control);
                assert!(
                    !is_inert_chrome(surface.upcast_ref(), control.upcast_ref()),
                    "{role:?} widgets handle their own presses"
                );
            }
        },
    );
}
