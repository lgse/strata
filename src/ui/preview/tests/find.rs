// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn source_find_tracks_incremental_text_and_resets_for_another_file() {
    crate::test_support::gtk_test(
        "ui::preview::tests::find::source_find_tracks_incremental_text_and_resets_for_another_file",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.show(entry("source.rs"), None);
            let content = format!("{}// tail ÉCLAIR\n", "// padding line\n".repeat(600));
            {
                let pending = provider.0.borrow();
                let pending = &pending[0];
                (pending.emit)(PreviewEvent::Ready(Preview {
                    request_id: pending.request.id,
                    entry: pending.request.entry.clone(),
                    content_type: "text/x-rust".into(),
                    content: PreviewContent::Text {
                        content,
                        truncated: false,
                    },
                }));
            }
            assert!(
                preview
                    .state
                    .source_preview
                    .virtual_state
                    .borrow()
                    .is_none()
            );
            preview.state.find_button.emit_clicked();
            preview.state.find.entry.set_text("éclair");
            let buffer = preview.state.source_preview.view.buffer();
            assert!(buffer.selection_bounds().is_none());
            while glib::MainContext::default().iteration(false) {}
            let (start, end) = buffer
                .selection_bounds()
                .expect("find selects the later-loaded match");
            assert_eq!(buffer.text(&start, &end, true), "ÉCLAIR");
            preview.show(entry("other.txt"), None);
            assert!(!preview.state.find.widget.is_visible());
            assert!(preview.state.find.entry.text().is_empty());
            preview.close();
        },
    );
}
