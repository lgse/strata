// SPDX-License-Identifier: MIT

use super::{
    details_label, ensure_rename_field, modified_label, name_tooltip, new_card, parts, rename_field,
};
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

fn label_center(card: &gtk::Box, label: &gtk::Inscription) -> (f64, f64) {
    let point = label
        .compute_point(
            card,
            &gtk::graphene::Point::new(label.width() as f32 / 2.0, label.height() as f32 / 2.0),
        )
        .expect("label point in card");
    (f64::from(point.x()), f64::from(point.y()))
}

#[test]
fn name_tooltip_shows_truncated_names_and_the_modified_date() {
    gtk_test(
        "ui::icons_cell::tests::name_tooltip_shows_truncated_names_and_the_modified_date",
        || {
            let long_name = "2026-10-07-release-candidate-build-final-v3-with-extra-suffix.tar.gz";
            let card = new_card(64);
            let (_icon, label) = parts(&card).expect("card parts");
            label.set_text(Some(long_name));
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            pump_frames(&card);
            let (x, y) = label_center(&card, &label);

            assert_eq!(name_tooltip::text(&card, x, y).as_deref(), Some(long_name));

            let renamed = "2026-10-07-release-candidate-build-final-v4-renamed-pending.tar.gz";
            label.set_text(Some(renamed));
            pump_frames(&card);
            assert_eq!(name_tooltip::text(&card, x, y).as_deref(), Some(renamed));

            modified_label(&card)
                .expect("modified label")
                .set_text("12 Nov 2026");
            assert_eq!(
                name_tooltip::text(&card, x, y).as_deref(),
                Some(format!("{renamed}\n12 Nov 2026").as_str())
            );

            label.set_text(Some("a.txt"));
            pump_frames(&card);
            let (x, y) = label_center(&card, &label);
            assert_eq!(
                name_tooltip::text(&card, x, y).as_deref(),
                Some("a.txt\n12 Nov 2026")
            );

            window.close();
        },
    );
}

#[test]
fn name_tooltip_skips_names_that_fit_and_hidden_captions() {
    gtk_test(
        "ui::icons_cell::tests::name_tooltip_skips_names_that_fit_and_hidden_captions",
        || {
            let card = new_card(64);
            let (_icon, label) = parts(&card).expect("card parts");
            label.set_text(Some("a.txt"));
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            pump_frames(&card);
            let (x, y) = label_center(&card, &label);
            assert_eq!(name_tooltip::text(&card, x, y), None);

            label.set_text(Some(
                "a-very-long-file-name-that-cannot-fit-in-two-caption-lines.txt",
            ));
            pump_frames(&card);
            let (x, y) = label_center(&card, &label);
            assert!(name_tooltip::text(&card, x, y).is_some());

            let field = ensure_rename_field(&card).expect("rename field");
            label.set_visible(false);
            field.set_visible(true);
            pump_frames(&card);
            assert_eq!(name_tooltip::text(&card, x, y), None);

            window.close();
        },
    );
}
