// SPDX-License-Identifier: MIT

use super::{ensure_rename_field, new_card, parts, rename_field, set_slot};
use crate::test_support::gtk_test;
use gtk::{gdk, glib, prelude::*};

#[test]
fn thumbnails_do_not_obscure_wrapped_names_or_rename_fields() {
    gtk_test(
        "ui::icons_cell::tests::thumbnails_do_not_obscure_wrapped_names_or_rename_fields",
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
            for size in [32, 64, 256] {
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
                    let mut short_height = None;
                    for name in ["todo.txt", "a filename that wraps onto two lines.txt"] {
                        label.set_text(Some(name));
                        pump_frames(&card);
                        let snapshot = gtk::Snapshot::new();
                        card.snapshot_child(&icon, &snapshot);
                        let drawn = snapshot.to_node().expect("rendered thumbnail").bounds();
                        let thumbnail_bottom = drawn.y() + drawn.height();
                        let snapshot = gtk::Snapshot::new();
                        card.snapshot_child(&card.last_child().expect("labels"), &snapshot);
                        let text = snapshot.to_node().expect("rendered filename").bounds();
                        assert!(
                            text.y() >= thumbnail_bottom,
                            "filename must remain below an opaque thumbnail: size={size}, texture={texture_width}x{texture_height}, thumbnail={drawn:?}, text={text:?}"
                        );
                        if let Some(height) = short_height {
                            assert!(
                                text.height() > height,
                                "wrapped name must retain its second line"
                            );
                        } else {
                            short_height = Some(text.height());
                        }
                    }

                    label.set_visible(false);
                    field.set_visible(true);
                    pump_frames(&card);
                    let icon_bounds = icon.compute_bounds(&card).expect("icon bounds");
                    let thumbnail_bottom = icon_bounds.y() + icon_bounds.height();
                    let field_bounds = field.compute_bounds(&card).expect("rename bounds");
                    assert!(
                        field_bounds.y() >= thumbnail_bottom,
                        "rename field must remain below an opaque thumbnail: size={size}, texture={texture_width}x{texture_height}, thumbnail_bottom={thumbnail_bottom}, field={field_bounds:?}"
                    );
                }
                field.set_visible(false);
                label.set_visible(true);
                pump_frames(&card);
            }
            window.close();
        },
    );
}

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
