// SPDX-License-Identifier: MIT

use super::*;
use std::cell::Cell;

#[test]
fn release_requires_a_click_on_the_original_bound_item() {
    gtk_test(
        "ui::pointer::tests::click::release_requires_a_click_on_the_original_bound_item",
        || {
            let calls = Rc::new(Cell::new(0));
            let bound = Rc::new(RefCell::new(None::<gtk::GestureClick>));
            let factory = gtk::SignalListItemFactory::new();
            let calls_for_setup = calls.clone();
            let bound_for_setup = bound.clone();
            factory.connect_setup(move |_, object| {
                let item = object.downcast_ref::<gtk::ListItem>().expect("list item");
                let row = gtk::Label::new(Some("entry"));
                let click = gtk::GestureClick::new();
                let calls = calls_for_setup.clone();
                connect_click_release(&click, item, move |_, _| calls.set(calls.get() + 1));
                row.add_controller(click.clone());
                item.set_child(Some(&row));
                bound_for_setup.replace(Some(click));
            });
            let model = gtk::StringList::new(&["original"]);
            let selection = gtk::NoSelection::new(Some(model.clone()));
            let list = gtk::ListView::new(Some(selection), Some(factory));
            let window = gtk::Window::builder().child(&list).build();
            window.present();
            pump_until(|| bound.borrow().is_some());
            let click = bound.borrow().clone().expect("bound click");
            let press = || click.emit_by_name::<()>("pressed", &[&1i32, &10.0f64, &10.0f64]);
            let release = |x: f64| click.emit_by_name::<()>("released", &[&1i32, &x, &10.0f64]);
            press();
            assert_eq!(calls.get(), 0);
            release(10.0);
            assert_eq!(calls.get(), 1);
            release(10.0);
            assert_eq!(calls.get(), 1, "release without a press");

            press();
            release(100.0);
            assert_eq!(calls.get(), 1, "drag threshold crossed");

            press();
            click.emit_by_name::<()>("cancel", &[&None::<gtk::gdk::EventSequence>]);
            release(10.0);
            assert_eq!(calls.get(), 1, "claimed by another gesture");

            press();
            model.splice(0, 1, &["replacement"]);
            while glib::MainContext::default().iteration(false) {}
            release(10.0);
            assert_eq!(calls.get(), 1, "recycled item");
            window.close();
        },
    );
}
