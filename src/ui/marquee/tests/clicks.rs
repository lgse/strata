// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn only_a_completed_plain_background_click_clears_selection() {
    gtk_test(
        "ui::marquee::tests::clicks::only_a_completed_plain_background_click_clears_selection",
        || {
            let view = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let scroll = gtk::ScrolledWindow::builder().child(&view).build();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));
            let clears = Rc::new(Cell::new(0));
            let on_clear = clears.clone();
            let marquee = install(MarqueeSetup {
                view: view.clone().upcast(),
                surface: scroll.clone().upcast(),
                scroll: scroll.clone(),
                overlay: overlay.clone(),
                targets: Rc::new(RefCell::new(Vec::new())),
                is_item: Rc::new(|_, _, _| false),
                clear_selection: Rc::new(move || on_clear.set(on_clear.get() + 1)),
            });
            let state = &marquee.state;
            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            assert_eq!(
                clears.get(),
                0,
                "press preserves the selection for dragging"
            );
            state.finish();
            assert_eq!(clears.get(), 1);
            state.finish();
            assert_eq!(clears.get(), 1, "unpaired release");

            for modifier in [
                gtk::gdk::ModifierType::CONTROL_MASK,
                gtk::gdk::ModifierType::SHIFT_MASK,
            ] {
                state.begin((0.0, 0.0), modifier);
                state.finish();
                assert_eq!(clears.get(), 1, "modified background click");
            }

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.clear_on_click.set(false);
            state.finish();
            assert_eq!(
                clears.get(),
                1,
                "inert space inside an item still clicks the item"
            );

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.dragging.set(true);
            state.finish();
            assert_eq!(clears.get(), 1, "marquee release keeps its selection");

            state.begin((0.0, 0.0), gtk::gdk::ModifierType::empty());
            state.end();
            state.finish();
            assert_eq!(clears.get(), 1, "cancelled gesture");
        },
    );
}
