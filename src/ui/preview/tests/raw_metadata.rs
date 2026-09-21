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
            let entry = media_size::entry("missing-metadata.NEF");
            drawer.show(entry.clone(), None);
            let first_load = cancellation();
            assert!(drawer.state.raw_details_scroll.is_visible());
            drawer
                .state
                .show_after_focus_change(media_size::entry("plain.png"), None);
            assert!(first_load.is_cancelled());
            assert!(!drawer.state.raw_details_scroll.is_visible());
            drawer.show(entry, None);
            let second_load = cancellation();
            drawer.close();
            assert!(second_load.is_cancelled());
            while glib::MainContext::default().iteration(false) {}
            assert!(!drawer.state.raw_details_scroll.is_visible());
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
            drawer.show(media_size::entry("plain.png"), None);
            assert!(!drawer.state.raw_details_scroll.is_visible());
            assert!(drawer.state.raw_metadata_load.borrow().is_none());
        },
    );
}
