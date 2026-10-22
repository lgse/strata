// SPDX-License-Identifier: MIT

use super::*;

fn show_text(
    preview: &PreviewDrawer,
    provider: &Provider,
    name: &str,
    content_type: &str,
    content: String,
) {
    preview.show(entry(name), None);
    let pending = provider.0.borrow();
    let pending = pending.last().expect("text preview request");
    (pending.emit)(PreviewEvent::Ready(Preview {
        request_id: pending.request.id,
        entry: pending.request.entry.clone(),
        content_type: content_type.into(),
        content: PreviewContent::Text {
            content,
            truncated: false,
        },
    }));
}

fn source_text(view: &sourceview5::View) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

#[test]
fn long_line_highlighted_sources_stay_virtual_and_files_never_share_a_view() {
    crate::test_support::gtk_test(
        "ui::preview::tests::source::long_line_highlighted_sources_stay_virtual_and_files_never_share_a_view",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            let source = &preview.state.source_preview;

            let minified = "var a=function(b){return b};".repeat(20_000);
            show_text(&preview, &provider, "min.js", "text/javascript", minified);
            assert!(source.virtual_state.borrow().is_some());
            assert!(source.scroll.borrow().is_none());

            show_text(
                &preview,
                &provider,
                "first.rs",
                "text/x-rust",
                "fn first() {}\n".into(),
            );
            assert!(source.virtual_state.borrow().is_none());
            let first = source.view.borrow().clone();
            assert_eq!(source_text(&first), "fn first() {}\n");

            show_text(
                &preview,
                &provider,
                "second.rs",
                "text/x-rust",
                "fn second() {}\n".into(),
            );
            let second = source.view.borrow().clone();
            assert_ne!(first, second);
            assert_eq!(source_text(&second), "fn second() {}\n");
            assert_eq!(source_text(&first), "fn first() {}\n");
            preview.close();
        },
    );
}
