// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{ModelPalette, PreviewRequestId},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn model_requests_reuse_only_matching_size_and_palette_without_ui_state() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("sample.3mf");
    fs::write(&path, b"injected renderer").expect("model fixture");
    let entry = FileEntry {
        location: Location::local(&path),
        thumbnail_path: None,
        native_name: "sample.3mf".into(),
        display_name: "sample.3mf".into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    };
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let calls = Arc::new(AtomicUsize::new(0));
    for (width, accent, expected_calls) in [
        (200, 0xff0000, 1),
        (200, 0xff0000, 1),
        (200, 0x00ff00, 2),
        (300, 0x00ff00, 3),
        (200, 0xff0000, 3),
    ] {
        let palette = ModelPalette {
            accent,
            surface: 0x101010,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let emit = events.clone();
        let rendered = calls.clone();
        let _handle = provider.load_with_renderer(
            PreviewRequest {
                id: PreviewRequestId(expected_calls as u64),
                entry: entry.clone(),
                text_byte_limit: 1024,
                render_document: false,
                pdf_page: 0,
                media_size: MediaPreviewSize::new(width, 200),
                model_palette: palette,
                archive_password: None,
            },
            Rc::new(move |event| emit.borrow_mut().push(event)),
            move |_, operation, _, _, _| {
                let ParseOperation::PreviewModel(render) = operation else {
                    panic!("model operation")
                };
                assert_eq!(render.format, ModelFormat::ThreeMf);
                assert_eq!(render.palette, palette);
                assert_eq!(render.size, MediaPreviewSize::new(width, 200));
                rendered.fetch_add(1, Ordering::SeqCst);
                Ok(crate::sandbox::ParseOutput {
                    data: vec![1, 2, 3],
                    page: 0,
                    pages: 0,
                })
            },
        );
        context.block_on(async {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                glib::timeout_future(Duration::from_millis(1)).await;
            }
        });
        assert!(matches!(
            events.borrow().as_slice(),
            [PreviewEvent::Ready(Preview {
                content: PreviewContent::Model { .. },
                ..
            })]
        ));
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
    }
}
