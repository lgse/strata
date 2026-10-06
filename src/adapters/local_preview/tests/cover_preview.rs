// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{ModelPalette, PreviewRequestId},
};

#[test]
fn comic_and_epub_requests_render_covers_instead_of_archive_trees() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let directory = tempfile::tempdir().expect("fixture directory");
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));

    for (extension, format) in [
        ("cbz", crate::sandbox::CoverFormat::Cbz),
        ("cbr", crate::sandbox::CoverFormat::Cbr),
        ("epub", crate::sandbox::CoverFormat::Epub),
    ] {
        let name = format!("sample.{extension}");
        let path = directory.path().join(&name);
        fs::write(&path, b"injected renderer").expect("cover fixture");
        let entry = FileEntry {
            location: Location::local(&path),
            thumbnail_path: None,
            native_name: name.clone().into(),
            display_name: name,
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Known(1),
            mode: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            is_hidden: false,
            recent_uri: None,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let emit = events.clone();
        let _handle = provider.load_with_renderer(
            PreviewRequest {
                id: PreviewRequestId(1),
                entry,
                text_byte_limit: 1024,
                render_document: false,
                pdf_page: 0,
                media_size: MediaPreviewSize::new(800, 800),
                model_palette: ModelPalette::default(),
                archive_password: None,
            },
            Rc::new(move |event| emit.borrow_mut().push(event)),
            move |_, operation, _, _, _| {
                assert_eq!(operation, ParseOperation::PreviewCover(format));
                Ok(crate::sandbox::ParseOutput {
                    data: vec![1, 2, 3],
                    page: 0,
                    pages: 0,
                    text_layer: None,
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
                content: PreviewContent::Rasterized { .. },
                ..
            })]
        ));
    }
}
