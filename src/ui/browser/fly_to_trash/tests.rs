// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn prepared_flights_stay_hidden_until_play_and_survive_row_removal() {
    crate::test_support::gtk_test(
        "ui::browser::fly_to_trash::tests::prepared_flights_stay_hidden_until_play_and_survive_row_removal",
        || {
            use gtk::prelude::*;
            use std::{cell::Cell, rc::Rc, time::Duration};

            crate::ui::prepare_portal_ui();
            crate::ui::motion::set_reduce_motion(false);
            if let Some(settings) = gtk::Settings::default() {
                settings.set_gtk_enable_animations(true);
            }
            let fixture = tempfile::tempdir().expect("fixture");
            let path = fixture.path().join("sample.txt");
            std::fs::write(&path, b"sample").expect("fixture file");
            let entry = glib::MainContext::default()
                .block_on(crate::adapters::query_file_entry(
                    crate::model::Location::local(&path),
                ))
                .expect("file entry");

            for restore in [false, true] {
                let overlay = gtk::Overlay::new();
                let source = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let row = gtk::Label::new(Some(&entry.display_name));
                row.add_css_class("list-row");
                row.set_size_request(240, 48);
                source.append(&row);
                overlay.set_child(Some(&source));
                let trash = gtk::Button::with_label("Trash");
                trash.set_halign(gtk::Align::End);
                trash.set_valign(gtk::Align::End);
                overlay.add_overlay(&trash);
                let window = gtk::Window::builder()
                    .default_width(640)
                    .default_height(480)
                    .child(&overlay)
                    .build();
                window.present();
                let context = glib::MainContext::default();
                while context.iteration(false) {}

                let mut animated_entry = entry.clone();
                if restore {
                    animated_entry.location = crate::model::Location::uri("trash:///sample.txt");
                }
                let entries = vec![animated_entry; 65];
                let prepared = if restore {
                    prepare_fly_from_trash(&source.clone().upcast(), entries.iter(), &trash)
                } else {
                    prepare_fly_to_trash(&source.clone().upcast(), entries.iter(), &trash)
                }
                .expect("a visible row from a large selection should prepare a flight");
                let flyer = prepared
                    .flyers
                    .as_ref()
                    .and_then(|flyers| flyers.first())
                    .expect("prepared flyer")
                    .widget
                    .clone();
                assert!(!flyer.is_visible());

                source.remove(&row);
                let finished = Rc::new(Cell::new(false));
                let marked = finished.clone();
                prepared.play(move || marked.set(true));
                assert!(flyer.is_visible());

                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while !finished.get() && std::time::Instant::now() < deadline {
                    while context.iteration(false) {}
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(finished.get());
                assert!(flyer.parent().is_none());
                window.close();
            }
        },
    );
}

#[test]
fn outbound_restore_keeps_the_pre_restore_listing_frozen_until_landing() {
    crate::test_support::gtk_test(
        "ui::browser::fly_to_trash::tests::outbound_restore_keeps_the_pre_restore_listing_frozen_until_landing",
        || {
            use gtk::prelude::*;
            use std::{cell::Cell, rc::Rc, time::Duration};

            crate::ui::prepare_portal_ui();
            crate::ui::motion::set_reduce_motion(false);
            if let Some(settings) = gtk::Settings::default() {
                settings.set_gtk_enable_animations(true);
            }
            let fixture = tempfile::tempdir().expect("fixture");
            let path = fixture.path().join("restored.txt");
            std::fs::write(&path, b"restored").expect("fixture file");
            let entry = glib::MainContext::default()
                .block_on(crate::adapters::query_file_entry(
                    crate::model::Location::local(&path),
                ))
                .expect("file entry");

            let overlay = gtk::Overlay::new();
            let source = gtk::Box::new(gtk::Orientation::Vertical, 0);
            source.append(&gtk::Label::new(Some("Existing file")));
            let root = crate::ui::blur::BlurBin::new(&source);
            overlay.set_child(Some(&root));
            let trash = gtk::Button::with_label("Trash");
            trash.set_halign(gtk::Align::End);
            trash.set_valign(gtk::Align::End);
            overlay.add_overlay(&trash);
            let window = gtk::Window::builder()
                .default_width(640)
                .default_height(480)
                .child(&overlay)
                .build();
            window.present();
            let context = glib::MainContext::default();
            while context.iteration(false) {}

            let prepared = prepare_fly_from_trash(&source.clone().upcast(), [&entry], &trash)
                .expect("outbound restore flight");
            assert!(root.is_frozen());

            let finished = Rc::new(Cell::new(false));
            let thawed_on_landing = Rc::new(Cell::new(false));
            let marked = finished.clone();
            let observed = thawed_on_landing.clone();
            let root_on_done = root.clone();
            prepared.play(move || {
                observed.set(!root_on_done.is_frozen());
                marked.set(true);
            });
            assert!(root.is_frozen());

            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !finished.get() && std::time::Instant::now() < deadline {
                while context.iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(finished.get());
            assert!(thawed_on_landing.get());
            assert!(!root.is_frozen());
            window.close();
        },
    );
}
