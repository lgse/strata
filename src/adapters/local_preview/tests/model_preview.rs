// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{ModelEncoding, ModelMesh, ModelPalette, ModelView, PreviewRequestId},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn model_requests_reuse_only_matching_size_palette_and_view_without_ui_state() {
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
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    };
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let calls = Arc::new(AtomicUsize::new(0));
    for (width, accent, view, expected_calls) in [
        (200, 0xff0000, ModelView::FrontLeft, 1),
        (200, 0xff0000, ModelView::FrontLeft, 1),
        (200, 0x00ff00, ModelView::FrontLeft, 2),
        (300, 0x00ff00, ModelView::FrontLeft, 3),
        (200, 0xff0000, ModelView::FrontLeft, 3),
        (200, 0xff0000, ModelView::FrontRight, 4),
        (200, 0xff0000, ModelView::FrontLeft, 4),
        (200, 0xff0000, ModelView::FrontRight, 4),
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
                model_view: view,
                archive_password: None,
            },
            Rc::new(move |event| emit.borrow_mut().push(event)),
            move |_, operation, _, _, _| {
                let ParseOperation::PreviewModel(render) = operation else {
                    panic!("model operation")
                };
                assert_eq!(render.format, ModelFormat::ThreeMf);
                assert_eq!(render.palette, palette);
                assert_eq!(render.view, view);
                assert_eq!(render.size, MediaPreviewSize::new(width, 200));
                rendered.fetch_add(1, Ordering::SeqCst);
                Ok(crate::sandbox::ParseOutput {
                    data: vec![1, 2, 3],
                    page: 0,
                    pages: 0,
                    text_layer: None,
                    model_mesh: Some(ModelMesh {
                        view,
                        encoding: ModelEncoding::ThreeMf,
                        triangles: 17,
                    }),
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
                content: PreviewContent::Model { mesh: Some(mesh), .. },
                ..
            })] if mesh.view == view && mesh.triangles == 17 && mesh.encoding == ModelEncoding::ThreeMf
        ));
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
    }
}

#[test]
fn gcode_without_a_thumbnail_keeps_text_and_binary_reports_the_gap() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let directory = tempfile::tempdir().expect("fixture directory");
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let palette = ModelPalette {
        accent: 0xff0000,
        surface: 0x101010,
    };

    let text_path = directory.path().join("plate.gcode");
    fs::write(&text_path, b"; header\nG1 X0 ; TOOLPATH-MARKER\n").expect("text gcode");
    let remote = Location::uri(gio::File::for_path(&text_path).uri());
    for (id, location) in [(1, Location::local(&text_path)), (3, remote)] {
        let text_events = load_gcode(
            &context,
            &provider,
            gcode_entry(location, "plate.gcode"),
            palette,
            id,
        );
        assert!(matches!(
            text_events.as_slice(),
            [PreviewEvent::Ready(Preview {
                content: PreviewContent::Text { content, truncated: false },
                ..
            })] if content.contains("TOOLPATH-MARKER")
        ));
    }

    let binary_path = directory.path().join("plate.bgcode");
    let mut binary = b"; bgcode\n".to_vec();
    binary.push(0);
    binary.extend(b"TOOLPATH-MARKER\n");
    fs::write(&binary_path, binary).expect("binary gcode");
    let binary_events = load_gcode(
        &context,
        &provider,
        gcode_entry(Location::local(&binary_path), "plate.bgcode"),
        palette,
        2,
    );
    assert!(matches!(
        binary_events.as_slice(),
        [PreviewEvent::Failed { message, .. }]
            if message == crate::services::model_preview::NO_GCODE_THUMBNAIL
    ));
}

fn gcode_entry(location: Location, name: &str) -> FileEntry {
    FileEntry {
        location,
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

fn load_gcode(
    context: &glib::MainContext,
    provider: &LocalPreviewProvider,
    entry: FileEntry,
    palette: ModelPalette,
    id: u64,
) -> Vec<PreviewEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emit = events.clone();
    let _handle = provider.load_with_renderer(
        PreviewRequest {
            id: PreviewRequestId(id),
            entry,
            text_byte_limit: 1024,
            render_document: false,
            pdf_page: 0,
            media_size: MediaPreviewSize::new(200, 200),
            model_palette: palette,
            model_view: ModelView::FrontLeft,
            archive_password: None,
        },
        Rc::new(move |event| emit.borrow_mut().push(event)),
        |_, operation, _, _, _| {
            let ParseOperation::PreviewModel(render) = operation else {
                panic!("model operation")
            };
            assert_eq!(render.format, ModelFormat::Gcode);
            Err(crate::services::model_preview::NO_GCODE_THUMBNAIL.to_owned())
        },
    );
    context.block_on(async {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while events.borrow().is_empty() && std::time::Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
    });
    events.replace(Vec::new())
}
