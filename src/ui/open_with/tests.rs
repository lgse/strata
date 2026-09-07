// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::time::Instant;

#[test]
fn empty_chooser_disables_open_and_restores_focus_after_backdrop_dismissal() {
    crate::test_support::gtk_test(
        "ui::open_with::tests::empty_chooser_disables_open_and_restores_focus_after_backdrop_dismissal",
        || {
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&parent));
            let window = gtk::Window::new();
            window.set_child(Some(&overlay));
            window.present();
            let closed = Rc::new(Cell::new(0));
            let result = closed.clone();
            show(
                &parent,
                vec![],
                vec![],
                Rc::new(move || result.set(result.get() + 1)),
            );
            let layer = overlay
                .last_child()
                .expect("modal layer")
                .downcast::<gtk::Box>()
                .expect("layer box");
            fn find_open(widget: &gtk::Widget) -> Option<gtk::Button> {
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && button.label().as_deref() == Some("Open")
                {
                    return Some(button.clone());
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    if let Some(button) = find_open(&widget) {
                        return Some(button);
                    }
                    child = widget.next_sibling();
                }
                None
            }
            assert!(
                !find_open(layer.upcast_ref())
                    .expect("Open button")
                    .is_sensitive()
            );
            dismiss_modal_layer(&layer, &overlay, None);
            let deadline = Instant::now() + Duration::from_secs(3);
            while closed.get() == 0 {
                assert!(Instant::now() < deadline);
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(closed.get(), 1);
            window.close();
        },
    );
}
