// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn thumbnail_metadata_updates_admitted_targets_but_not_unbound_or_recycled_ones() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::tests::thumbnail_metadata_updates_admitted_targets_but_not_unbound_or_recycled_ones",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let first = directory.path().join("a.png");
            let second = directory.path().join("b.png");
            std::fs::write(&first, b"fixture").expect("first file");
            std::fs::write(&second, b"fixture").expect("second file");
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            browser.navigate(Location::local(directory.path()));
            let context = glib::MainContext::default();
            let deadline = Instant::now() + Duration::from_secs(5);
            while browser.entry_at(0, 1).is_none() {
                assert!(Instant::now() < deadline, "directory did not load");
                context.iteration(false);
            }
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
            cancel_metadata(image_id);
            publish_thumbnail_metadata(image_id, &second, &metadata);
            assert_eq!(
                browser.entry_at(0, 1).expect("second").image_dimensions,
                MetadataValue::Unknown
            );
            assert_eq!(observed.get(), 1);
        },
    );
}
