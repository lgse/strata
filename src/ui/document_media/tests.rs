// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn cached_images_rebind_with_alt_text_and_release_cancelled_previews() {
    crate::test_support::gtk_test(
        "ui::document_media::tests::cached_images_rebind_with_alt_text_and_release_cancelled_previews",
        || {
            let cache = MediaCache::new(None);
            let cancellation = cache.cancellation.clone();
            let texture = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_static(&[200, 50, 20, 255]),
                4,
            )
            .upcast::<gdk::Texture>();
            let source = DocumentMedia::Image("image.png".into());
            cache.entries.borrow_mut().insert(
                0,
                Entry {
                    source: source.clone(),
                    alt: "A test image".into(),
                    result: Some(Ok(texture.clone())),
                    rows: Vec::new(),
                },
            );
            for _ in 0..2 {
                let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
                cache.bind(0, &source, "A test image", &row);
                let picture = row
                    .first_child()
                    .expect("content")
                    .first_child()
                    .expect("media frame")
                    .first_child()
                    .expect("image")
                    .downcast::<gtk::Picture>()
                    .expect("picture");
                assert_eq!(picture.alternative_text().as_deref(), Some("A test image"));
                assert_eq!(picture.paintable(), Some(texture.clone().upcast()));
                assert!(
                    !cache.running.get(),
                    "a cached image must not restart the decoder"
                );
            }
            drop(cache);
            assert!(cancellation.is_cancelled());
        },
    );
}

#[test]
fn failed_diagrams_preserve_source_and_do_not_interpret_markup() {
    crate::test_support::gtk_test(
        "ui::document_media::tests::failed_diagrams_preserve_source_and_do_not_interpret_markup",
        || {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let source = "flowchart LR\nA[<script>bad()</script>] -->";
            show_error(
                &row,
                &DocumentMedia::Mermaid(source.into()),
                "",
                "Unsupported syntax",
            );
            let label = row
                .last_child()
                .expect("source fallback")
                .downcast::<gtk::Label>()
                .expect("text");
            assert_eq!(label.text(), source);
            assert!(!label.uses_markup());
            assert!(label.is_selectable());
        },
    );
}
