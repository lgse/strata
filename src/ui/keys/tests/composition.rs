// SPDX-License-Identifier: MIT

use super::*;

/// Real key events, which `forward` needs; an input method's `filter_key`
/// builds them from a keycode.
mod key_event {
    use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Imp(RefCell<Option<gdk::Event>>);

    #[glib::object_subclass]
    impl ObjectSubclass for Imp {
        const NAME: &'static str = "StrataTestKeyEventBuilder";
        type Type = Builder;
        type ParentType = gtk::IMContext;
    }

    impl ObjectImpl for Imp {}

    impl IMContextImpl for Imp {
        fn filter_keypress(&self, event: &gdk::Event) -> bool {
            self.0.replace(Some(event.clone()));
            true
        }
    }

    glib::wrapper! {
        pub struct Builder(ObjectSubclass<Imp>) @extends gtk::IMContext;
    }

    pub fn press(window: &gtk::Window, key: crate::ui::keys::Key) {
        let key = crate::ui::keys::gdk_key(key);
        let display = WidgetExt::display(window);
        let keycode = display
            .map_keyval(key)
            .and_then(|keys| {
                keys.into_iter()
                    .find(|key| key.group() == 0 && key.level() == 0)
            })
            .expect("the keymap types the key")
            .keycode();
        let keyboard = display
            .default_seat()
            .and_then(|seat| seat.keyboard())
            .expect("a keyboard");
        let builder = glib::Object::new::<Builder>();
        builder.filter_key(
            true,
            &window.surface().expect("a presented window"),
            &keyboard,
            0,
            keycode,
            gdk::ModifierType::empty(),
            0,
        );
        let event = builder.imp().0.take().expect("a key event");
        #[expect(deprecated, reason = "GTK has no other way to deliver a key event")]
        display.put_event(&event);
        while glib::MainContext::default().iteration(false) {}
    }
}

#[test]
fn key_handlers_yield_to_a_composition_in_the_focused_field() {
    crate::test_support::gtk_test(
        "ui::keys::tests::composition::key_handlers_yield_to_a_composition_in_the_focused_field",
        || {
            use gtk::prelude::*;
            use std::{cell::Cell, rc::Rc};

            let fields: [fn() -> gtk::Widget; 2] = [
                || gtk::Text::new().upcast(),
                || gtk::TextView::new().upcast(),
            ];
            let typed = |field: &gtk::Widget| match field.downcast_ref::<gtk::TextView>() {
                Some(view) => {
                    let buffer = view.buffer();
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
                }
                None => field.downcast_ref::<gtk::Text>().expect("a field").text(),
            };
            for field in fields {
                let window = gtk::Window::new();
                let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let composing = field();
                let other = field();
                column.append(&composing);
                column.append(&other);
                window.set_child(Some(&column));
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let handled = Rc::new(Cell::new(0));
                let count = handled.clone();
                super::super::on_press(&keys, move |_, _| {
                    count.set(count.get() + 1);
                    gtk::glib::Propagation::Stop
                });
                window.add_controller(keys.clone());
                window.present();
                let press = |key| key_event::press(&window, key);
                let preedit =
                    |text: &str| composing.emit_by_name::<()>("preedit-changed", &[&text]);

                assert!(composing.grab_focus());
                preedit("ˇ");
                press(Key::a);
                assert_eq!(handled.get(), 0, "the input method takes a");
                assert_eq!(typed(&composing), "a");
                press(Key::Escape);
                assert_eq!(
                    handled.get(),
                    1,
                    "a key the input method passes back reaches the handler"
                );
                assert!(other.grab_focus());
                press(Key::a);
                assert_eq!(
                    handled.get(),
                    2,
                    "only the focused field's composition holds keys"
                );
                assert!(composing.grab_focus());
                preedit("");
                press(Key::a);
                assert_eq!(handled.get(), 3, "a finished composition releases the keys");
                window.destroy();
            }
        },
    );
}
