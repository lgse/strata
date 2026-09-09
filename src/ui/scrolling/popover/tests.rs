// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn only_marked_listing_ancestors_receive_forwarded_wheel_events() {
    crate::test_support::gtk_test(
        "ui::scrolling::popover::tests::only_marked_listing_ancestors_receive_forwarded_wheel_events",
        || {
            let row = gtk::Label::new(Some("file"));
            let listing = gtk::ScrolledWindow::builder().child(&row).build();
            assert!(listing_ancestor(row.upcast_ref()).is_none());
            listing.add_css_class("browser-listing-scroll");
            assert_eq!(listing_ancestor(row.upcast_ref()), Some(listing.clone()));
            assert_eq!(listing_ancestor(listing.upcast_ref()), Some(listing));
            let sidebar = gtk::ScrolledWindow::new();
            assert!(listing_ancestor(sidebar.upcast_ref()).is_none());
        },
    );
}

#[test]
fn forwarded_scroll_clamps_at_both_edges_and_ignores_zero_axes() {
    crate::test_support::gtk_test(
        "ui::scrolling::popover::tests::forwarded_scroll_clamps_at_both_edges_and_ignores_zero_axes",
        || {
            let adjustment = gtk::Adjustment::new(20.0, 10.0, 210.0, 1.0, 10.0, 100.0);
            apply_adjustment_scroll(&adjustment, 0.0, gtk::gdk::ScrollUnit::Wheel);
            assert_eq!(adjustment.value(), 20.0);
            apply_adjustment_scroll(&adjustment, 100.0, gtk::gdk::ScrollUnit::Wheel);
            assert_eq!(adjustment.value(), 110.0);
            apply_adjustment_scroll(&adjustment, -100.0, gtk::gdk::ScrollUnit::Surface);
            assert_eq!(adjustment.value(), 10.0);
        },
    );
}

#[test]
fn parent_surface_controller_exists_only_while_the_popover_is_mapped() {
    crate::test_support::gtk_test(
        "ui::scrolling::popover::tests::parent_surface_controller_exists_only_while_the_popover_is_mapped",
        || {
            let popover = gtk::Popover::builder()
                .child(&gtk::Label::new(Some("Options")))
                .build();
            dismiss_on_outside_scroll(&popover);
            let button = gtk::MenuButton::builder().popover(&popover).build();
            let window = gtk::Window::builder().child(&button).build();
            window.present();
            let controllers = window.observe_controllers();
            let baseline = controllers.n_items();
            for _ in 0..3 {
                popover.popup();
                assert!(popover.is_mapped());
                assert_eq!(controllers.n_items(), baseline + 1);
                popover.popdown();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while popover.is_mapped() {
                    assert!(std::time::Instant::now() < deadline);
                    glib::MainContext::default().iteration(false);
                }
                assert_eq!(controllers.n_items(), baseline);
            }
            window.close();
        },
    );
}

#[test]
fn wheel_steps_follow_the_viewport_and_surface_steps_do_not() {
    let wheel = scroll_delta_for_unit(1.0, 1000.0, gtk::gdk::ScrollUnit::Wheel);
    assert!((wheel - 100.0).abs() < 1e-9);
    assert!(scroll_delta_for_unit(1.0, 8000.0, gtk::gdk::ScrollUnit::Wheel) > wheel);
    assert_eq!(
        scroll_delta_for_unit(4.0, 100.0, gtk::gdk::ScrollUnit::Surface),
        10.0
    );
    assert_eq!(
        scroll_delta_for_unit(1.0, 50.0, gtk::gdk::ScrollUnit::Surface),
        scroll_delta_for_unit(1.0, 999.0, gtk::gdk::ScrollUnit::Surface)
    );
}
