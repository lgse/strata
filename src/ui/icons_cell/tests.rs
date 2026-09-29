// SPDX-License-Identifier: MIT

use super::{details_label, ensure_rename_field, new_card, parts, rename_field};
use crate::test_support::gtk_test;
use gtk::{glib, prelude::*};

fn pump_frames(widget: &impl IsA<gtk::Widget>) {
    let frames = std::rc::Rc::new(std::cell::Cell::new(0));
    let drawn = frames.clone();
    widget.add_tick_callback(move |_, _| {
        drawn.set(drawn.get() + 1);
        if drawn.get() >= 2 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    pump_until(|| frames.get() >= 2);
}

fn pump_until(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let context = glib::MainContext::default();
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "card must be allocated"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn new_card_has_no_rename_entry_until_needed() {
    gtk_test(
        "ui::icons_cell::tests::new_card_has_no_rename_entry_until_needed",
        || {
            let card = new_card(64);
            assert!(rename_field(&card).is_none());
            let field = ensure_rename_field(&card).expect("rename field");
            assert!(!gtk::prelude::WidgetExt::is_visible(&field));
            assert!(rename_field(&card).is_some());
        },
    );
}

#[test]
fn card_details_persist_with_rename_field() {
    gtk_test(
        "ui::icons_cell::tests::card_details_persist_with_rename_field",
        || {
            let card = new_card(64);
            let (_icon, label) = parts(&card).expect("card parts");
            label.set_text(Some("photo.png"));
            label.set_visible(true);

            let details = details_label(&card).expect("details label");
            assert!(details.text().is_empty());
            details.set_text("1920×1080");

            let window = gtk::Window::builder().child(&card).build();
            window.present();
            pump_frames(&card);

            assert!(details.is_visible());
            assert_eq!(details.text().as_str(), "1920×1080");

            let field = ensure_rename_field(&card).expect("rename field");
            label.set_visible(false);
            field.set_visible(true);
            pump_frames(&card);
            let found_details = details_label(&card).expect("details label after rename field");
            assert_eq!(found_details, details);
            assert!(found_details.is_visible());
            assert_eq!(found_details.text().as_str(), "1920×1080");

            window.close();
        },
    );
}
