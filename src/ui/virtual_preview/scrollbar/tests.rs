// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::StableScrollbar;
use crate::test_support::gtk_test;

#[test]
fn logical_thumb_commands_reach_both_document_edges() {
    gtk_test(
        "ui::virtual_preview::scrollbar::tests::logical_thumb_commands_reach_both_document_edges",
        || {
            let document = gtk::Adjustment::new(400.0, 10.0, 1_010.0, 10.0, 90.0, 100.0);
            let presentation = gtk::Adjustment::new(2.0, 0.0, 10.0, 0.1, 1.0, 1.0);
            let (state, _bar) = StableScrollbar::new(&document, &presentation);
            state.thumb.set_value(state.thumb.lower());
            assert_eq!(
                document.value(),
                10.0,
                "logical Home reaches the document start"
            );
            state
                .thumb
                .set_value(state.thumb.upper() - state.thumb.page_size());
            assert_eq!(
                document.value(),
                910.0,
                "logical End reaches the document end"
            );
        },
    );
}

#[test]
fn held_endpoints_remain_reachable_after_gtk_reanchors_the_document() {
    gtk_test(
        "ui::virtual_preview::scrollbar::tests::held_endpoints_remain_reachable_after_gtk_reanchors_the_document",
        || {
            let scroll = gtk::ScrolledWindow::new();
            let document = scroll.vadjustment();
            document.configure(10.0, 10.0, 1_010.0, 10.0, 90.0, 100.0);
            let (state, _bar) = StableScrollbar::new(&document, &document);
            state.begin_drag();
            state.thumb.set_value(910.0);
            document.configure(650.0, 10.0, 2_010.0, 10.0, 90.0, 100.0);
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                document.value(),
                1_910.0,
                "a bottom drag reaches the expanded document end"
            );

            state.thumb.set_value(10.0);
            document.configure(250.0, 10.0, 3_010.0, 10.0, 90.0, 100.0);
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                document.value(),
                10.0,
                "a top drag returns to the document start"
            );

            document.configure(500.0, 10.0, 4_010.0, 10.0, 90.0, 100.0);
            state.thumb.set_value(460.0);
            let middle = document.value();
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                document.value(),
                middle,
                "a queued edge correction cannot override a later interior drag"
            );
            state.finish_drag();
        },
    );
}

#[test]
fn queued_release_does_not_thaw_a_new_drag() {
    gtk_test(
        "ui::virtual_preview::scrollbar::tests::queued_release_does_not_thaw_a_new_drag",
        || {
            let scroll = gtk::ScrolledWindow::new();
            let document = scroll.vadjustment();
            document.configure(0.0, 0.0, 1_000.0, 10.0, 90.0, 100.0);
            let (state, _bar) = StableScrollbar::new(&document, &document);
            state.begin_drag();
            state.finish_after_event();
            state.begin_drag();
            document.configure(500.0, 0.0, 2_000.0, 10.0, 90.0, 100.0);
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            state.thumb.set_value(450.0);
            assert_eq!(
                document.value(),
                950.0,
                "the new drag still uses its frozen range"
            );
            state.finish_drag();
        },
    );
}

#[test]
fn unmapping_a_held_scrollbar_restores_live_updates_before_reopening() {
    gtk_test(
        "ui::virtual_preview::scrollbar::tests::unmapping_a_held_scrollbar_restores_live_updates_before_reopening",
        || {
            let scroll = gtk::ScrolledWindow::new();
            let document = scroll.vadjustment();
            document.configure(0.0, 0.0, 1_000.0, 10.0, 90.0, 100.0);
            let (state, bar) = StableScrollbar::new(&document, &document);
            let window = gtk::Window::builder().child(&bar).build();
            window.present();
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            state.begin_drag();
            document.configure(500.0, 0.0, 2_000.0, 10.0, 90.0, 100.0);
            window.set_visible(false);
            document.set_value(600.0);
            assert_eq!(
                state.thumb.value(),
                600.0,
                "an unmapped drag must not leave the range frozen"
            );
            window.present();
            state.thumb.set_value(700.0);
            assert_eq!(
                document.value(),
                700.0,
                "the reopened scrollbar remains usable"
            );
            window.close();
        },
    );
}

#[test]
fn held_thumb_ignores_reestimates_and_resynchronizes_without_scrolling_on_release() {
    gtk_test(
        "ui::virtual_preview::scrollbar::tests::held_thumb_ignores_reestimates_and_resynchronizes_without_scrolling_on_release",
        || {
            let scroll = gtk::ScrolledWindow::new();
            let document = scroll.vadjustment();
            document.configure(10.0, 10.0, 1_010.0, 10.0, 90.0, 100.0);
            let (state, bar) = StableScrollbar::new(&document, &document);
            state.begin_drag();
            state.thumb.set_value(460.0);
            assert_eq!(document.value(), 460.0);

            document.configure(700.0, 10.0, 2_010.0, 10.0, 90.0, 100.0);
            for _ in 0..10 {
                state.thumb.set_value(460.0);
            }
            assert_eq!(
                document.value(),
                700.0,
                "a held thumb must not undo GTK's anchor correction"
            );

            document.configure(1_400.0, 10.0, 2_010.0, 10.0, 90.0, 100.0);
            state.thumb.set_value(470.0);
            assert!(
                document.value() >= 1_400.0,
                "downward thumb motion must not undo GTK's forward anchor correction"
            );
            document.configure(200.0, 10.0, 2_010.0, 10.0, 90.0, 100.0);
            state.thumb.set_value(460.0);
            assert!(
                document.value() <= 200.0,
                "upward thumb motion must not undo GTK's backward anchor correction"
            );

            state.thumb.set_value(910.0);
            assert_eq!(
                document.value(),
                1_910.0,
                "the bottom remains reachable after the document grows"
            );
            state.thumb.set_value(10.0);
            assert_eq!(document.value(), 10.0, "the top remains reachable");
            state.thumb.set_value(235.0);
            document.configure(650.0, 10.0, 3_010.0, 10.0, 90.0, 100.0);
            state.finish_drag();
            assert_eq!(
                document.value(),
                650.0,
                "release must not move the document"
            );

            state.thumb.set_value(800.0);
            assert_eq!(
                document.value(),
                800.0,
                "keyboard scrolling uses the current document range"
            );
            document.set_value(1_000.0);
            assert_eq!(
                state.thumb.value(),
                1_000.0,
                "wheel and find scrolling update the thumb"
            );

            let weak = std::rc::Rc::downgrade(&state);
            drop(state);
            drop(bar);
            assert!(
                weak.upgrade().is_none(),
                "closing the preview releases the scrollbar model"
            );
        },
    );
}
