// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn icon_fallbacks_never_exceed_folder_height_and_thumbnails_keep_their_size() {
    gtk_test(
        "ui::thumbnail::slot::tests::icon_fallbacks_never_exceed_folder_height_and_thumbnails_keep_their_size",
        || {
            crate::ui::prepare_portal_ui();
            for size in [32, 64, 128, 256] {
                let slot = ThumbnailSlot::new(size);
                slot.limit_fallback_height_to_folder();
                slot.allocate(size, size, -1, None);
                for icon in [crate::assets::icons::FOLDER, "strata-file-text"] {
                    let texture =
                        crate::assets::primary_icon_paintable(icon).expect("icon texture");
                    slot.set_fallback(icon, Some(&texture));
                    let scale = slot.imp().fallback_scale.get();
                    if icon == crate::assets::icons::FOLDER {
                        assert_eq!(scale, 1.0);
                    } else {
                        assert!(scale < 1.0);
                        assert_eq!(scale, folder_height_scale(&texture));
                    }
                    let snapshot = gtk::Snapshot::new();
                    slot.imp().snapshot(&snapshot);
                    let bounds = snapshot.to_node().expect("fallback node").bounds();
                    assert!((f64::from(bounds.height()) - f64::from(size) * scale).abs() < 0.001);
                    assert_eq!(bounds.width(), bounds.height());
                    slot.set_texture(&texture);
                    let snapshot = gtk::Snapshot::new();
                    slot.imp().snapshot(&snapshot);
                    assert_eq!(
                        snapshot
                            .to_node()
                            .expect("thumbnail node")
                            .bounds()
                            .height(),
                        size as f32
                    );
                }
                let pixels = glib::Bytes::from_owned(vec![255_u8; 24 * 24 * 4]);
                let full =
                    gdk::MemoryTexture::new(24, 24, gdk::MemoryFormat::R8g8b8a8, &pixels, 24 * 4);
                assert_eq!(folder_height_scale(full.upcast_ref()), 19.0 / 24.0);
            }
        },
    );
}

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
