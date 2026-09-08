// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::test_support::gtk_test;

#[test]
fn rendering_inset_preserves_measurement_and_texture_aspect_ratio() {
    gtk_test(
        "ui::thumbnail::slot::tests::rendering_inset_preserves_measurement_and_texture_aspect_ratio",
        || {
            let slot = ThumbnailSlot::new(64);
            slot.allocate(64, 64, -1, None);
            let resizes = slot.resize_calls();
            for (width, height) in [(16, 16), (32, 16), (16, 32)] {
                let pixels = glib::Bytes::from_owned(vec![255_u8; width * height * 4]);
                let texture = gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gdk::MemoryFormat::R8g8b8a8,
                    &pixels,
                    width * 4,
                );
                for fallback in [false, true] {
                    if fallback {
                        slot.set_fallback("test-icon", Some(texture.upcast_ref()));
                    } else {
                        slot.set_texture(texture.upcast_ref());
                    }
                    for (inset, expected_side) in [(0, 64.0), (9, 46.0), (-1, 64.0), (100, 1.0)] {
                        slot.set_content_inset(inset);
                        let snapshot = gtk::Snapshot::new();
                        slot.imp().snapshot(&snapshot);
                        let bounds = snapshot.to_node().expect("texture node").bounds();
                        assert_eq!(bounds.width().max(bounds.height()), expected_side);
                        assert_eq!(
                            bounds.width() / bounds.height(),
                            width as f32 / height as f32
                        );
                        assert_eq!(bounds.x() + bounds.width() / 2.0, 32.0);
                        assert_eq!(bounds.y() + bounds.height() / 2.0, 32.0);
                        assert_eq!(slot.slot_size(), 64);
                        assert_eq!(slot.resize_calls(), resizes);
                        assert_eq!(slot.measure(gtk::Orientation::Vertical, -1).0, 64);
                    }
                }
            }
        },
    );
}
