// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn exhausted_autoscroll_keeps_destination_highlight_until_drag_leaves() {
    crate::test_support::gtk_test(
        "exhausted_autoscroll_keeps_destination_highlight_until_drag_leaves",
        || {
            let controller = gtk::DropControllerMotion::new();
            let tracker = DragAutoscroll {
                state: Weak::new(),
                controller: controller.downgrade(),
                pointer: Cell::new((0.0, 0.0)),
                burst_x: Cell::new(None),
                burst_y: Cell::new(None),
                frame_time: Cell::new(0),
                tick: RefCell::new(None),
                hovered: RefCell::new(None),
            };
            let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            tracker.set_drop_hover(Some(shell.clone()));
            tracker.stop_ticks();
            assert!(shell.has_css_class("drop-destination"));
            tracker.stop();
            assert!(!shell.has_css_class("drop-destination"));
        },
    );
}
