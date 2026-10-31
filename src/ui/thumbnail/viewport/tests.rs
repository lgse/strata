// SPDX-License-Identifier: MIT

use super::*;

mod folder_counts;

fn metadata_fixture(names: &[&str]) -> (tempfile::TempDir, Rc<Browser>) {
    let directory = tempfile::tempdir().expect("fixture");
    for name in names {
        std::fs::write(directory.path().join(name), b"fixture").expect("media file");
    }
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    browser.navigate(Location::local(directory.path()));
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while browser.entry_at(0, names.len() - 1).is_none() {
        assert!(Instant::now() < deadline, "directory did not load");
        context.iteration(false);
    }
    (directory, browser)
}

#[test]
fn show_thumbnails_changes_preserve_pending_metadata() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::tests::show_thumbnails_changes_preserve_pending_metadata",
        || {
            let (_directory, browser) =
                metadata_fixture(&["a.mp4", "b.mp4", "c.mp4", "d.mp4", "e.mp4"]);
            let manager = crate::ui::preferences::PreferenceManager::shared();
            let cases: &[(bool, &[bool], bool)] = &[
                (true, &[], false),
                (true, &[false], false),
                (true, &[false, true], false),
                (false, &[true], false),
                (false, &[true], true),
            ];
            let metadata = crate::sandbox::metadata::MediaMetadata {
                dimensions: Some((1920, 1080)),
                duration: Some(12.7),
                ..Default::default()
            };
            for (position, &(initial, changes, cached)) in cases.iter().enumerate() {
                manager.set_show_thumbnails(initial);
                let entry = browser.entry_at(0, position).expect("media entry");
                if cached {
                    let texture = gdk::MemoryTexture::new(
                        1,
                        1,
                        gdk::MemoryFormat::R8g8b8a8,
                        &glib::Bytes::from_owned(vec![120, 80, 40, 255]),
                        4,
                    )
                    .upcast();
                    remember_thumbnail_for_test(&entry, texture);
                }
                let image = ThumbnailSlot::new(18);
                set_thumbnail_or_icon(&image, &entry, crate::assets::icons::DOCUMENTS, 18, 32);
                request_metadata(
                    &image,
                    &image,
                    &browser,
                    0,
                    position,
                    entry.location.clone(),
                    true,
                );
                for &enabled in changes {
                    manager.set_show_thumbnails(enabled);
                }
                publish_thumbnail_metadata(
                    image.as_ptr() as usize,
                    entry.location.native_path().expect("media path"),
                    &metadata,
                );
                let updated = browser.entry_at(0, position).expect("updated entry");
                assert_eq!(
                    updated.image_dimensions,
                    MetadataValue::Known((1920, 1080)),
                    "dimensions lost: initial={initial}, changes={changes:?}, cached={cached}",
                );
                assert_eq!(
                    updated.duration_seconds,
                    MetadataValue::Known(13),
                    "duration lost: initial={initial}, changes={changes:?}, cached={cached}",
                );
                cancel_thumbnails_in(image.upcast_ref());
            }
        },
    );
}

#[test]
fn thumbnail_metadata_updates_admitted_targets_but_not_unbound_or_recycled_ones() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::tests::thumbnail_metadata_updates_admitted_targets_but_not_unbound_or_recycled_ones",
        || {
            let (directory, browser) = metadata_fixture(&["a.png", "b.png"]);
            let first = directory.path().join("a.png");
            let second = directory.path().join("b.png");
            let context = glib::MainContext::default();
            let deadline = Instant::now() + Duration::from_secs(5);
            let image = ThumbnailSlot::new(64);
            let image_id = image.as_ptr() as usize;
            request_metadata(
                &image,
                &image,
                &browser,
                0,
                0,
                Location::local(&first),
                true,
            );
            while REFRESH_PENDING.with(Cell::get) {
                assert!(Instant::now() < deadline, "viewport admission did not run");
                context.iteration(false);
            }
            let observed = Rc::new(Cell::new(0));
            let events = observed.clone();
            browser.observe(move |event| {
                if matches!(event, crate::app::BrowserEvent::MetadataFilled { .. }) {
                    events.set(events.get() + 1);
                }
            });
            let metadata = crate::sandbox::metadata::MediaMetadata {
                dimensions: Some((1920, 1080)),
                ..Default::default()
            };
            publish_thumbnail_metadata(image_id, &first, &metadata);
            assert_eq!(
                browser.entry_at(0, 0).expect("first").image_dimensions,
                MetadataValue::Known((1920, 1080))
            );
            assert_eq!(observed.get(), 1);
            set_thumbnail_or_icon(
                &image,
                &browser.entry_at(0, 1).expect("recycled entry"),
                crate::assets::icons::PICTURES,
                18,
                32,
            );
            let stale_metadata = crate::sandbox::metadata::MediaMetadata {
                dimensions: Some((640, 480)),
                ..Default::default()
            };
            publish_thumbnail_metadata(image_id, &first, &stale_metadata);
            assert_eq!(
                browser.entry_at(0, 0).expect("first").image_dimensions,
                MetadataValue::Known((1920, 1080))
            );
            assert_eq!(observed.get(), 1);
            request_metadata(
                &image,
                &image,
                &browser,
                0,
                1,
                Location::local(&second),
                true,
            );
            publish_thumbnail_metadata(image_id, &first, &metadata);
            assert_eq!(
                browser.entry_at(0, 1).expect("second").image_dimensions,
                MetadataValue::Unknown
            );
            cancel_thumbnails_in(image.upcast_ref());
            publish_thumbnail_metadata(image_id, &second, &metadata);
            assert_eq!(
                browser.entry_at(0, 1).expect("second").image_dimensions,
                MetadataValue::Unknown
            );
            assert_eq!(observed.get(), 1);
        },
    );
}
