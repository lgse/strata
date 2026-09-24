// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn selection_changes_and_close_cancel_raw_metadata() {
    crate::test_support::gtk_test(
        "ui::preview::tests::raw_metadata::selection_changes_and_close_cancel_raw_metadata",
        || {
            let drawer = PreviewDrawer::new(Rc::new(NoopPreviewProvider), false);
            let cancellation = || {
                drawer
                    .state
                    .raw_metadata_load
                    .borrow()
                    .as_ref()
                    .expect("pending RAW metadata request")
                    .0
                    .clone()
            };
            let mut entry = media_size::entry("missing-metadata.NEF.2");
            entry.display_name = "missing-metadata.NEF".into();
            entry.thumbnail_path = entry.location.native_path().map(ToOwned::to_owned);
            entry.location = Location::uri("trash:///missing-metadata.NEF.2");
            drawer.show(entry.clone(), None);
            let first_load = cancellation();
            assert!(drawer.state.raw_details_scroll.is_visible());
            drawer
                .state
                .show_after_focus_change(media_size::entry("next-metadata.NEF"), None);
            assert!(first_load.is_cancelled());
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
            media_size::wait_until(|| {
                drawer
                    .state
                    .current
                    .borrow()
                    .as_ref()
                    .is_some_and(|entry| entry.native_name == "next-metadata.NEF")
            });
            let second_load = cancellation();
            drawer
                .state
                .show_after_focus_change(media_size::entry("plain.png"), None);
            assert!(second_load.is_cancelled());
            media_size::wait_until(|| {
                drawer
                    .state
                    .current
                    .borrow()
                    .as_ref()
                    .is_some_and(|entry| entry.native_name == "plain.png")
            });
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
            assert!(!drawer.state.raw_details_scroll.is_visible());
            drawer.show(entry, None);
            let closing_load = cancellation();
            drawer.close();
            assert!(closing_load.is_cancelled());
            while glib::MainContext::default().iteration(false) {}
            assert!(!drawer.state.raw_details_scroll.is_visible());
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
            drawer.show(media_size::entry("plain.png"), None);
            assert!(!drawer.state.raw_details_scroll.is_visible());
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
        },
    );
}
