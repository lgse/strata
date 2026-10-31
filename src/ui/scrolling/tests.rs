// SPDX-License-Identifier: MIT

use super::*;

fn wait_for_allocation(widget: &gtk::Widget) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while widget.width() <= 0 || widget.height() <= 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "widget was not allocated"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn middle_gesture(scroll: &gtk::ScrolledWindow) -> gtk::GestureClick {
    let controllers = scroll.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == gtk::gdk::BUTTON_MIDDLE)
        .expect("middle-click autoscroll gesture")
}

fn press(gesture: &gtk::GestureClick, x: f64, y: f64) {
    gesture.emit_by_name::<()>("pressed", &[&1i32, &x, &y]);
}

#[test]
fn middle_click_autoscroll_skips_rows_and_anchors_on_background() {
    crate::test_support::gtk_test(
        "ui::scrolling::tests::middle_click_autoscroll_skips_rows_and_anchors_on_background",
        || {
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hexpand(true);
            scroll.set_vexpand(true);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("file-row");
            row.set_height_request(40);
            row.append(&gtk::Label::new(Some("a.txt")));
            content.append(&row);
            scroll.set_child(Some(&content));
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let window = gtk::Window::new();
            window.set_child(Some(&overlay));
            window.set_default_size(240, 200);
            window.present();
            wait_for_allocation(scroll.upcast_ref());

            // Make the view scrollable so `AutoScroll::start` accepts the anchor.
            let adjustment = scroll.vadjustment();
            adjustment.set_upper(adjustment.lower() + adjustment.page_size() + 400.0);
            install_autoscroll(&scroll, &overlay);
            let gesture = middle_gesture(&scroll);

            let bounds = row
                .compute_bounds(&scroll)
                .expect("row is allocated within the scroll");
            press(
                &gesture,
                f64::from(bounds.x()) + 4.0,
                f64::from(bounds.y()) + f64::from(bounds.height()) / 2.0,
            );
            assert!(
                !autoscroll_is_running(),
                "a middle press over a row must not anchor autoscroll"
            );

            press(&gesture, 4.0, scroll.height() as f64 - 4.0);
            assert!(
                autoscroll_is_running(),
                "a middle press over the view background must anchor autoscroll"
            );

            assert!(stop_autoscroll());
            window.destroy();
        },
    );
}
