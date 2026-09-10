// SPDX-License-Identifier: MIT

use super::{ensure_rename_field, icons_card_extent, new_card, parts, rename_field, set_slot};
use crate::test_support::gtk_test;
use gtk::{gdk, glib, prelude::*};

#[test]
fn card_places_the_first_filename_line_directly_below_the_icon() {
    gtk_test(
        "ui::icons_cell::tests::card_places_the_first_filename_line_directly_below_the_icon",
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
            let field = ensure_rename_field(&card).expect("rename field");
            let window = gtk::Window::builder().child(&card).build();
            window.present();
            for size in [64, 256] {
                set_slot(&card, size);
                for (texture_width, texture_height) in [(16, 16), (16, 8), (8, 16)] {
                    let pixels =
                        glib::Bytes::from_owned(vec![255_u8; texture_width * texture_height * 4]);
                    let texture = gdk::MemoryTexture::new(
                        texture_width as i32,
                        texture_height as i32,
                        gdk::MemoryFormat::R8g8b8a8,
                        &pixels,
                        texture_width * 4,
                    );
                    icon.set_texture(texture.upcast_ref());
                    label.set_text(Some("todo.txt"));
                    label.set_visible(true);
                    field.set_visible(false);
                    pump_until(|| icon.width() == size && label.width() > 0);
                    let snapshot = gtk::Snapshot::new();
                    card.snapshot_child(&icon, &snapshot);
                    let drawn = snapshot.to_node().expect("rendered thumbnail").bounds();
                    let thumbnail_bottom = drawn.y() + drawn.height();

                    let mut short_text: Option<(f32, f32)> = None;
                    for name in ["todo.txt", "a filename that wraps onto two lines.txt"] {
                        label.set_text(Some(name));
                        pump_until(|| label.width() > 0);
                        let snapshot = gtk::Snapshot::new();
                        card.snapshot_child(&card.last_child().expect("labels"), &snapshot);
                        let text = snapshot.to_node().expect("rendered filename").bounds();
                        assert!(
                            text.y() >= thumbnail_bottom,
                            "filename must remain below an opaque thumbnail: size={size}, texture={texture_width}x{texture_height}, thumbnail={drawn:?}, text={text:?}"
                        );
                        if let Some((short_y, short_bottom)) = short_text {
                            assert!(
                                (text.y() - short_y).abs() <= 1.0,
                                "short and wrapped names must start on the same line: short={short_y}, wrapped={}",
                                text.y()
                            );
                            assert!(
                                text.y() + text.height() > short_bottom,
                                "the unused second line must remain below a short name"
                            );
                        } else {
                            short_text = Some((text.y(), text.y() + text.height()));
                        }
                    }

                    label.set_visible(false);
                    field.set_visible(true);
                    pump_until(|| field.width() > 0);
                    let field_bounds = field.compute_bounds(&card).expect("rename bounds");
                    assert!(
                        field_bounds.y() >= thumbnail_bottom,
                        "rename field must remain below an opaque thumbnail: size={size}, texture={texture_width}x{texture_height}, thumbnail_bottom={thumbnail_bottom}, field={field_bounds:?}"
                    );
                }
                field.set_visible(false);
                label.set_visible(true);
                pump_until(|| label.height() >= 36);
                assert_eq!(label.yalign(), 0.0);
                assert_eq!(label.min_lines(), 2);
                assert_eq!(label.nat_lines(), 2);
                assert_eq!(card.width_request(), size.max(116));
                assert_eq!(card.height_request(), size + 43);
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
fn allocated_default_is_square_and_airy_delta_stays_modest() {
    gtk_test(
        "ui::icons_cell::tests::allocated_default_is_square_and_airy_delta_stays_modest",
        || {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(include_str!("../../style.css"));
            gtk::style_context_add_provider_for_display(
                &gdk::Display::default().expect("display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            root.add_css_class("mode-icons");
            root.add_css_class("density-compact");
            root.set_halign(gtk::Align::Start);
            let card = new_card(64);
            let (icon, label) = parts(&card).expect("card parts");
            label.set_text(Some("a readable filename that wraps.txt"));
            root.append(&card);
            let window = gtk::Window::builder()
                .default_width(300)
                .child(&root)
                .build();
            window.present();
            pump_until(|| card.width() > 0 && label.height() >= 36);
            let compact = (card.width(), card.height());
            assert!(
                (104..=116).contains(&compact.0),
                "default card width should be compact: {compact:?}"
            );
            assert!(
                (compact.0 - compact.1).abs() <= 12,
                "default card should be roughly square: {compact:?}"
            );

            let compact_icon = icon.compute_bounds(&card).expect("compact icon bounds");
            root.remove(&card);
            window.close();
            let airy_root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            airy_root.add_css_class("mode-icons");
            airy_root.add_css_class("density-airy");
            airy_root.set_halign(gtk::Align::Start);
            airy_root.append(&card);
            let airy_window = gtk::Window::builder()
                .default_width(300)
                .child(&airy_root)
                .build();
            airy_window.present();
            pump_until(|| card.width() > 0 && card.height() > 0);
            let airy = (card.width(), card.height());
            let airy_icon = icon.compute_bounds(&card).expect("Airy icon bounds");
            assert!(
                (1..=12).contains(&(compact.0 - airy.0))
                    && (airy.1 - compact.1).abs() <= 12
                    && (1.0..=6.0).contains(&(compact_icon.x() - airy_icon.x())),
                "Airy padding should change only the modest interior inset: compact={compact:?}, airy={airy:?}, compact icon={compact_icon:?}, Airy icon={airy_icon:?}"
            );

            let field = ensure_rename_field(&card).expect("rename field");
            field.set_text("a readable renamed filename.txt");
            label.set_visible(false);
            field.set_visible(true);
            pump_until(|| field.width() > 0);
            let field_bounds = field.compute_bounds(&card).expect("rename bounds");
            assert!(field_bounds.x() >= 0.0);
            assert!(field_bounds.x() + field_bounds.width() <= card.width() as f32);
            assert!(field_bounds.y() + field_bounds.height() <= card.height() as f32);
            airy_window.close();
        },
    );
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
