// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{ModelPalette, ModelView, PreviewRequestId},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn route_requests_bypass_text_and_cache_by_palette() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("sample.GPX");
    fs::write(&path, b"<gpx/>").expect("route fixture");
    let entry = FileEntry {
        location: Location::local(&path),
        thumbnail_path: None,
        native_name: "sample.GPX".into(),
        display_name: "sample.GPX".into(),
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
    for (accent, expected_calls) in [(0xff0000, 1), (0xff0000, 1), (0x00ff00, 2), (0xff0000, 2)] {
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
                media_size: MediaPreviewSize::new(400, 300),
                model_palette: palette,
                model_view: ModelView::default(),
                archive_password: None,
            },
            Rc::new(move |event| emit.borrow_mut().push(event)),
            move |_, operation, _, _, _| {
                let ParseOperation::PreviewRoute(render) = operation else {
                    panic!("route operation")
                };
                assert_eq!(render.format, RouteFormat::Gpx);
                assert_eq!(render.palette, palette);
                rendered.fetch_add(1, Ordering::SeqCst);
                let preview = RoutePreview {
                    png: resvg::tiny_skia::Pixmap::new(800, 600)
                        .expect("allocate route image")
                        .encode_png()
                        .expect("encode route image"),
                    profile: None,
                    distance_m: 1000.0,
                    duration_seconds: None,
                    elevation: None,
                    points: 2,
                    samples: vec![
                        crate::services::route_preview::RouteSample {
                            position: [0.0, 0.0],
                            distance_m: 0.0,
                            elevation_y: None,
                            starts_segment: true,
                        },
                        crate::services::route_preview::RouteSample {
                            position: [1.0, 1.0],
                            distance_m: 1000.0,
                            elevation_y: None,
                            starts_segment: false,
                        },
                    ],
                };
                Ok(crate::sandbox::ParseOutput {
                    data: serde_json::to_vec(&preview).expect("serialize route preview"),
                    page: 0,
                    pages: 0,
                    text_layer: None,
                    model_mesh: None,
                })
            },
        );
        context.block_on(async {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                glib::timeout_future(Duration::from_millis(1)).await;
            }
        });
        assert!(
            matches!(events.borrow().as_slice(), [PreviewEvent::Ready(Preview {
            content: PreviewContent::Route { preview }, ..
        })] if preview.distance_m == 1000.0)
        );
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
    }
}
