// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ensure_rename_field, icons_card_extent, new_card, parts, rename_field, set_slot};
use crate::test_support::gtk_test;
use gtk::{gdk, glib, prelude::*};

#[test]
fn card_centers_rendering_without_changing_its_extent() {
    gtk_test(
        "ui::icons_cell::tests::card_centers_rendering_without_changing_its_extent",
        || {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(include_str!("../../style.css"));
            gtk::style_context_add_provider_for_display(
                &gdk::Display::default().expect("display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let card = new_card(64);
            let (icon, label) = parts(&card).expect("card parts");
            let pixels = glib::Bytes::from_owned(vec![255_u8; 16 * 16 * 4]);
            let texture =
                gdk::MemoryTexture::new(16, 16, gdk::MemoryFormat::R8g8b8a8, &pixels, 16 * 4);
            icon.set_texture(texture.upcast_ref());
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            for size in [64, 96, 128, 192, 256, 64] {
                set_slot(&card, size);
                for name in ["todo.txt", "a filename that wraps onto two lines.txt"] {
                    label.set_text(Some(name));
                    pump_until(|| icon.width() == size && label.width() > 0);
                    let snapshot = gtk::Snapshot::new();
                    card.snapshot_child(&icon, &snapshot);
                    let drawn = snapshot.to_node().expect("rendered icon").bounds();
                    let allocated = icon.compute_bounds(&card).expect("icon bounds");
                    assert_eq!(drawn.width(), (size - 18) as f32);
                    assert_eq!(drawn.height(), (size - 18) as f32);
                    assert_eq!(drawn.y() - allocated.y(), 9.0);
                    assert_eq!(drawn.x() - allocated.x(), 9.0);
                    if name == "todo.txt" {
                        let snapshot = gtk::Snapshot::new();
                        card.snapshot_child(&card.last_child().expect("labels"), &snapshot);
                        let text = snapshot.to_node().expect("rendered filename").bounds();
                        let top = drawn.y();
                        let bottom = card.height() as f32 - text.y() - text.height();
                        assert!(
                            (top - bottom).abs() <= label.height() as f32 / 4.0,
                            "balanced visible content: top={top}, bottom={bottom}"
                        );
                    }
                    assert_eq!(label.yalign(), 0.5);
                    assert_eq!(label.min_lines(), 2);
                    assert_eq!(label.nat_lines(), 2);
                    assert!(label.height() >= 36);
                    assert_eq!(card.width_request(), size.max(156));
                    assert_eq!(card.height_request(), size + 43);
                }
            }
            window.close();
        },
    );
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
fn card_keeps_a_fixed_size_request() {
    gtk_test(
        "ui::icons_cell::tests::card_keeps_a_fixed_size_request",
        || {
            let card = new_card(64);
            let (width, height) = icons_card_extent(64);
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
            let (icon, _) = parts(&card).expect("icon and label");
            icon.set_slot(512);
            set_slot(&card, 64);
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
        },
    );
}

#[test]
fn new_card_has_no_rename_entry_until_needed() {
    gtk_test(
        "ui::icons_cell::tests::new_card_has_no_rename_entry_until_needed",
        || {
            let card = new_card(64);
            let (width, height) = icons_card_extent(64);
            assert!(rename_field(&card).is_none());
            let field = ensure_rename_field(&card).expect("rename field");
            assert!(field.has_css_class("inline-rename"));
            assert!(!gtk::prelude::WidgetExt::is_visible(&field));
            assert_eq!(card.width_request(), width);
            assert_eq!(card.height_request(), height);
            assert!(rename_field(&card).is_some());
        },
    );
}
