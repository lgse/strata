// SPDX-License-Identifier: MIT

use std::{fs, time::Duration};

mod remote_preview;

use super::*;
use crate::services::{MediaPreviewSize, PreviewContent};

#[test]
fn renders_requested_pdf_pages_within_the_pixel_budget() {
    let path = std::env::temp_dir().join(format!(
        "strata-preview-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.set_source_rgb(0.2, 0.4, 0.8);
        context.paint().expect("paint PDF page");
        context.show_page().expect("finish first PDF page");
        context.set_source_rgb(0.8, 0.4, 0.2);
        context.paint().expect("paint second PDF page");
        context.show_page().expect("finish second PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    crate::sandbox_helper::run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "1:640x800".to_owned(),
        "software".to_owned(),
    ])
    .expect("render second PDF page");
    let png = fs::read(&output).expect("read rendered page");
    let metadata =
        fs::read_to_string(output_directory.join("result.meta")).expect("read PDF metadata");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);

    assert_eq!(metadata, "1 2");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(
        u32::from_be_bytes(png[16..20].try_into().expect("PNG width bytes")),
        618
    );
    assert_eq!(
        u32::from_be_bytes(png[20..24].try_into().expect("PNG height bytes")),
        800
    );
}

#[test]
fn pdf_preview_emits_a_text_layer_matching_the_rendered_page() {
    let path = std::env::temp_dir().join(format!(
        "strata-preview-text-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.set_source_rgb(0.1, 0.1, 0.1);
        context.set_font_size(24.0);
        context.move_to(72.0, 96.0);
        context
            .show_text("selectable helper text")
            .expect("draw text");
        context.show_page().expect("finish PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    crate::sandbox_helper::run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "0:640x800".to_owned(),
        "software".to_owned(),
    ])
    .expect("render PDF page with text");
    let png = fs::read(&output).expect("read rendered page");
    let png_width = u32::from_be_bytes(png[16..20].try_into().expect("PNG width bytes"));
    let sidecar = fs::read(output_directory.join("result.text")).expect("read text layer");
    let layer: crate::services::PdfTextLayer =
        serde_json::from_slice(&sidecar).expect("parse text layer");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);

    assert_eq!(layer.text.trim_end_matches('\n'), "selectable helper text");
    assert_eq!(layer.glyphs.len(), layer.text.chars().count());
    assert_eq!(layer.width, png_width as f32);
    // The text baseline sits near the top of the page in PNG pixels.
    assert!(layer.glyphs[0][1] > 0.0 && layer.glyphs[0][1] < layer.height * 0.25);
}

#[test]
fn pdf_rendering_fits_the_viewport_width_without_clipping_tall_pages() {
    assert_eq!(
        pdf_render_size(MediaPreviewSize::new(640, 480)),
        PdfRenderSize::new(640, 1_800)
    );
    assert_eq!(
        pdf_render_size(MediaPreviewSize::new(2_000, 480)),
        PdfRenderSize::new(MediaPreviewSize::MAX_EDGE, 1_800)
    );
}

#[test]
fn cold_previews_with_shared_thumbnails_match_rendered_cache_hits() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::cold_previews_with_shared_thumbnails_match_rendered_cache_hits",
        || {
            use crate::{
                model::{EntryKind, FileEntry, Location, MetadataValue},
                services::PreviewRequestId,
            };

            let directory = tempfile::tempdir().expect("preview fixture directory");
            for name in ["document.pdf", "image.png"] {
                let path = directory.path().join(name);
                let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 640, 800)
                    .expect("page surface");
                let mut png = Vec::new();
                surface.write_to_png(&mut png).expect("page PNG");
                crate::ui::thumbnail_cache::store(&path, 1, &png);
                let thumbnail = crate::ui::thumbnail_cache::lookup(&path, 1)
                    .expect("shared thumbnail is available");
                assert_ne!(thumbnail, png);

                let request = PreviewRequest {
                    id: PreviewRequestId(1),
                    entry: FileEntry {
                        location: Location::local(&path),
                        thumbnail_path: None,
                        native_name: name.into(),
                        display_name: name.into(),
                        kind: EntryKind::File,
                        size: MetadataValue::Unknown,
                        modified_unix_seconds: MetadataValue::Known(1),
                        mode: MetadataValue::Unknown,
                        recent_unix_seconds: MetadataValue::Unknown,
                        is_hidden: false,
                        image_dimensions: MetadataValue::Unknown,
                        child_count: MetadataValue::Unknown,
                        duration_seconds: MetadataValue::Unknown,
                    },
                    text_byte_limit: 1024,
                    render_document: false,
                    pdf_page: 0,
                    media_size: MediaPreviewSize::new(640, 800),
                    archive_password: None,
                };
                let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
                let context = glib::MainContext::default();
                let _owner = context.acquire().expect("main context owner");
                for cached in [false, true] {
                    let events = Rc::new(RefCell::new(Vec::new()));
                    let events_for_emit = events.clone();
                    let rendered = png.clone();
                    let handle = provider.load_with_renderer(
                        request.clone(),
                        Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
                        move |_, _, _, _, _| {
                            assert!(!cached, "reopening should use the rendered-page cache");
                            Ok(crate::sandbox::ParseOutput {
                                data: rendered,
                                page: 0,
                                pages: 40,
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
                    let events = events.borrow();
                    assert_eq!(events.len(), 1);
                    let PreviewEvent::Ready(preview) = &events[0] else {
                        panic!("PDF preview failed");
                    };
                    assert_eq!(
                        preview.content,
                        if name.ends_with(".pdf") {
                            PreviewContent::Pdf {
                                png: png.clone(),
                                page: 0,
                                pages: 40,
                                text_layer: None,
                            }
                        } else {
                            PreviewContent::Rasterized { png: png.clone() }
                        }
                    );
                    drop(handle);
                }
            }
        },
    );
}

#[test]
fn preview_cache_stores_and_retrieves_entries() {
    let mut cache = PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    };
    let key1 = PreviewCacheKey {
        path: PathBuf::from("test1.png"),
        modified: 100,
        pdf_page: None,
    };
    let content1 = PreviewContent::Rasterized {
        png: vec![1, 2, 3, 4],
    };
    cache.insert(key1.clone(), content1.clone());
    assert_eq!(cache.get(&key1), Some(content1));
    assert_eq!(cache.byte_count, 4);

    let key2 = PreviewCacheKey {
        path: PathBuf::from("test2.txt"),
        modified: 200,
        pdf_page: None,
    };
    let content2 = PreviewContent::Text {
        content: "hello world".to_owned(),
        truncated: false,
    };
    cache.insert(key2.clone(), content2.clone());
    assert_eq!(cache.get(&key2), Some(content2));
    assert_eq!(cache.byte_count, 4 + 11);

    let pdf_page_0 = PreviewCacheKey {
        path: PathBuf::from("doc.pdf"),
        modified: 300,
        pdf_page: Some((0, PdfRenderSize::new(640, 800))),
    };
    let pdf_page_1 = PreviewCacheKey {
        path: PathBuf::from("doc.pdf"),
        modified: 300,
        pdf_page: Some((1, PdfRenderSize::new(640, 800))),
    };
    let page0_content = PreviewContent::Pdf {
        png: vec![10, 20],
        page: 0,
        pages: 2,
        text_layer: None,
    };
    let page1_content = PreviewContent::Pdf {
        png: vec![30, 40, 50],
        page: 1,
        pages: 2,
        text_layer: None,
    };
    cache.insert(pdf_page_0.clone(), page0_content.clone());
    cache.insert(pdf_page_1.clone(), page1_content.clone());
    assert_eq!(cache.get(&pdf_page_0), Some(page0_content));
    assert_eq!(cache.get(&pdf_page_1), Some(page1_content));
    assert_eq!(
        cache.get(&PreviewCacheKey {
            path: PathBuf::from("doc.pdf"),
            modified: 300,
            pdf_page: Some((0, PdfRenderSize::new(800, 1_800))),
        }),
        None,
        "a page rendered for a smaller viewport must not poison a larger preview"
    );
}

#[test]
fn pdf_renders_wait_for_the_active_renderer_and_resume_in_order() {
    let context = glib::MainContext::new();
    context.block_on(async {
        let first = request_pdf_render_permit()
            .acquire()
            .await
            .expect("first PDF render permit");
        let mut second = request_pdf_render_permit();
        let mut third = request_pdf_render_permit();

        assert!(
            second
                .receive
                .as_mut()
                .expect("second receiver")
                .try_recv()
                .expect("second receiver open")
                .is_none()
        );
        assert!(
            third
                .receive
                .as_mut()
                .expect("third receiver")
                .try_recv()
                .expect("third receiver open")
                .is_none()
        );

        drop(first);
        let second = second.acquire().await.expect("second PDF render permit");
        assert!(
            third
                .receive
                .as_mut()
                .expect("third receiver")
                .try_recv()
                .expect("third receiver open")
                .is_none()
        );
        drop(second);
        drop(third.acquire().await.expect("third PDF render permit"));
    });

    PDF_RENDER_QUEUE.with(|queue| {
        let queue = queue.borrow();
        assert_eq!(queue.running, 0);
        assert!(queue.queued.is_empty());
    });
}

#[test]
fn dropping_a_queued_pdf_render_removes_it_without_consuming_a_slot() {
    let context = glib::MainContext::new();
    context.block_on(async {
        let first = request_pdf_render_permit()
            .acquire()
            .await
            .expect("first PDF render permit");
        let cancelled = request_pdf_render_permit();
        drop(cancelled);
        drop(first);
    });

    PDF_RENDER_QUEUE.with(|queue| {
        let queue = queue.borrow();
        assert_eq!(queue.running, 0);
        assert!(queue.queued.is_empty());
    });
}

#[test]
fn cancelled_in_flight_document_renders_keep_the_permit_and_emit_no_stale_events() {
    use crate::{
        model::{EntryKind, FileEntry, Location, MetadataValue},
        services::PreviewRequestId,
    };

    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));

    for (filename, succeeds) in [
        ("cancelled.pdf", false),
        ("cancelled.pdf", true),
        ("cancelled.xlsx", false),
        ("cancelled.xlsx", true),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let events_for_emit = events.clone();
        let (started, receive_started) = oneshot::channel();
        let (finish, receive_finish) = std::sync::mpsc::channel();
        let handle = provider.load_with_renderer(
            PreviewRequest {
                id: PreviewRequestId(1),
                entry: FileEntry {
                    location: Location::local(filename),
                    thumbnail_path: None,
                    native_name: filename.into(),
                    display_name: filename.into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    recent_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                },
                text_byte_limit: 1024,
                render_document: false,
                pdf_page: 1,
                media_size: MediaPreviewSize::new(640, 800),
                archive_password: None,
            },
            Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
            move |_, _, _, _, cancellation| {
                started.send(()).expect("notify renderer started");
                receive_finish
                    .recv_timeout(Duration::from_secs(10))
                    .expect("release renderer");
                assert!(cancellation.is_cancelled());
                if succeeds {
                    Ok(crate::sandbox::ParseOutput {
                        data: br#"{"rows":[["a"],["1"]],"truncated":false}"#.to_vec(),
                        page: 1,
                        pages: 2,
                        text_layer: None,
                    })
                } else {
                    Err("late renderer error".into())
                }
            },
        );
        context.block_on(async {
            receive_started.await.expect("renderer started");
            drop(handle);
            let mut next = request_pdf_render_permit();
            assert!(
                next.receive
                    .as_mut()
                    .expect("next receiver")
                    .try_recv()
                    .expect("next receiver open")
                    .is_none(),
                "cancellation must not release the permit before the helper exits"
            );
            finish.send(()).expect("finish cancelled renderer");
            drop(next.acquire().await.expect("next renderer can start"));
        });
        assert!(
            events.borrow().is_empty(),
            "cancelled load emitted an event"
        );
    }
}

#[test]
fn cancelled_archive_listings_emit_no_stale_events() {
    use crate::{
        model::{EntryKind, FileEntry, Location, MetadataValue},
        services::PreviewRequestId,
    };

    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_for_emit = events.clone();
    let (started, receive_started) = oneshot::channel();
    let (finish, receive_finish) = std::sync::mpsc::channel();
    let handle = provider.load_with_renderer(
        PreviewRequest {
            id: PreviewRequestId(1),
            entry: FileEntry {
                location: Location::local("cancelled.zip"),
                thumbnail_path: None,
                native_name: "cancelled.zip".into(),
                display_name: "cancelled.zip".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                mode: MetadataValue::Unknown,
                recent_unix_seconds: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                is_hidden: false,
            },
            text_byte_limit: 1024,
            render_document: false,
            pdf_page: 0,
            media_size: MediaPreviewSize::new(640, 800),
            archive_password: None,
        },
        Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
        move |_, _, _, _, cancellation| {
            started.send(()).expect("notify renderer started");
            receive_finish
                .recv_timeout(Duration::from_secs(10))
                .expect("release renderer");
            assert!(cancellation.is_cancelled());
            Ok(crate::sandbox::ParseOutput {
                data: b"{\"status\":\"open\",\"entries\":[],\"message\":null}".to_vec(),
                page: 0,
                pages: 0,
                text_layer: None,
            })
        },
    );
    context.block_on(async {
        receive_started.await.expect("renderer started");
        drop(handle);
        finish.send(()).expect("finish cancelled renderer");
        glib::timeout_future(Duration::from_millis(50)).await;
    });
    assert!(
        events.borrow().is_empty(),
        "cancelled load emitted an event"
    );
}

#[test]
fn preview_cache_evicts_the_least_recent_entry() {
    let mut cache = PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    };
    let keys: Vec<_> = (0..=MAX_PREVIEW_CACHE_ENTRIES)
        .map(|index| PreviewCacheKey {
            path: PathBuf::from(format!("image-{index}.png")),
            modified: index as i64,
            pdf_page: None,
        })
        .collect();

    for key in &keys[..MAX_PREVIEW_CACHE_ENTRIES] {
        cache.insert(key.clone(), PreviewContent::Rasterized { png: vec![0] });
    }
    assert!(cache.get(&keys[0]).is_some());
    cache.insert(
        keys[MAX_PREVIEW_CACHE_ENTRIES].clone(),
        PreviewContent::Rasterized { png: vec![0] },
    );

    assert!(cache.get(&keys[0]).is_some());
    assert!(cache.get(&keys[1]).is_none());
    assert_eq!(cache.entries.len(), MAX_PREVIEW_CACHE_ENTRIES);
    assert_eq!(cache.byte_count, MAX_PREVIEW_CACHE_ENTRIES);
}

#[test]
fn replacing_a_preview_cache_entry_updates_its_byte_count() {
    let mut cache = PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    };
    let key = PreviewCacheKey {
        path: PathBuf::from("image.png"),
        modified: 1,
        pdf_page: None,
    };

    cache.insert(key.clone(), PreviewContent::Rasterized { png: vec![0; 8] });
    cache.insert(key, PreviewContent::Rasterized { png: vec![0; 3] });

    assert_eq!(cache.byte_count, 3);
    assert_eq!(cache.entries.len(), 1);
}

#[test]
fn active_media_requests_are_never_retained_by_the_preview_cache() {
    let mut cache = PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    };
    let key = PreviewCacheKey {
        path: PathBuf::from("clip.mp4"),
        modified: 1,
        pdf_page: None,
    };
    let content = PreviewContent::SandboxedMedia {
        media: SandboxedMedia {
            path: "clip.mp4".into(),
            size: MediaPreviewSize::new(520, 800),
            backend: MediaPreviewBackend::Software,
            input_owner: None,
        },
    };
    cache.insert(key.clone(), content.clone());

    assert_eq!(cache.get(&key), None);
    assert!(cache.entries.is_empty());
    assert_eq!(cache.byte_count, 0);
}

#[test]
fn preview_content_size_computes_accurately() {
    assert_eq!(
        preview_content_size(&PreviewContent::Rasterized { png: vec![0; 100] }),
        100
    );
    assert_eq!(
        preview_content_size(&PreviewContent::Pdf {
            png: vec![0; 80],
            page: 0,
            pages: 1,
            text_layer: None,
        }),
        80
    );
    assert_eq!(
        preview_content_size(&PreviewContent::SandboxedMedia {
            media: SandboxedMedia {
                path: "clip.mp4".into(),
                size: MediaPreviewSize::new(520, 800),
                backend: MediaPreviewBackend::Software,
                input_owner: None,
            },
        }),
        0
    );
    assert_eq!(
        preview_content_size(&PreviewContent::Text {
            content: "12345".to_owned(),
            truncated: false
        }),
        5
    );
    assert_eq!(preview_content_size(&PreviewContent::Unsupported), 0);
}

#[test]
fn uncertain_file_names_resolve_their_preview_from_the_content() {
    use crate::{
        model::{EntryKind, FileEntry, Location, MetadataValue},
        services::PreviewRequestId,
    };

    let _main_context = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let directory = tempfile::tempdir().expect("preview fixture directory");
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");

    for (name, bytes, expected) in [
        (
            "some notes",
            &b"hello\nworld\n"[..],
            PreviewContent::Text {
                content: "hello\nworld\n".to_owned(),
                truncated: false,
            },
        ),
        (
            "some data",
            &[0_u8, 159, 146, 150, 0, 255, 0, 1][..],
            PreviewContent::Unsupported,
        ),
    ] {
        let path = directory.path().join(name);
        fs::write(&path, bytes).expect("preview fixture");
        let request = PreviewRequest {
            id: PreviewRequestId(1),
            entry: FileEntry {
                location: Location::local(&path),
                thumbnail_path: None,
                native_name: name.into(),
                display_name: name.into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
            },
            text_byte_limit: 1024,
            render_document: false,
            pdf_page: 0,
            media_size: MediaPreviewSize::new(640, 800),
            archive_password: None,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let events_for_emit = events.clone();
        let _handle = provider.load_with_renderer(
            request,
            Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
            |_, _, _, _, _| -> Result<crate::sandbox::ParseOutput, String> {
                panic!("text and unsupported files must not reach the sandbox")
            },
        );
        context.block_on(async {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while events.borrow().is_empty() && std::time::Instant::now() < deadline {
                glib::timeout_future(Duration::from_millis(1)).await;
            }
        });

        let events = events.borrow();
        assert_eq!(events.len(), 1, "{name}");
        let PreviewEvent::Ready(preview) = &events[0] else {
            panic!("{name} preview failed");
        };
        assert_eq!(preview.content, expected, "{name}");
    }
}

#[test]
fn archives_list_member_trees_as_preview_content() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::archives_list_member_trees_as_preview_content",
        || {
            use crate::{
                model::{EntryKind, FileEntry, Location, MetadataValue},
                services::{ArchiveDirectory, ArchiveNode, ArchivePreviewTree, PreviewRequestId},
            };

            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("sample.zip");
            {
                let mut writer =
                    zip::ZipWriter::new(fs::File::create(&path).expect("create zip fixture"));
                let _ = writer.start_file("a.txt", zip::write::SimpleFileOptions::default());
                let _ = std::io::Write::write_all(&mut writer, b"hello");
                let _ = writer.start_file("folder/b.txt", zip::write::SimpleFileOptions::default());
                let _ = std::io::Write::write_all(&mut writer, b"abc");
                writer.finish().expect("write zip fixture");
            }

            let request = PreviewRequest {
                id: PreviewRequestId(1),
                entry: FileEntry {
                    location: Location::local(&path),
                    thumbnail_path: None,
                    native_name: "sample.zip".into(),
                    display_name: "sample.zip".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    recent_unix_seconds: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                },
                text_byte_limit: 1024,
                render_document: false,
                pdf_page: 0,
                media_size: MediaPreviewSize::new(640, 800),
                archive_password: None,
            };
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("main context owner");
            let events = Rc::new(RefCell::new(Vec::new()));
            let events_for_emit = events.clone();
            let _handle = provider.load_with_renderer(
                request,
                Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
                move |path, operation, _, _, _| {
                    let crate::sandbox::ParseOperation::ArchiveList { format, password } =
                        operation
                    else {
                        panic!("archive previews must request an archive listing");
                    };
                    let cancelled = std::sync::atomic::AtomicBool::new(false);
                    let result = crate::adapters::local_operations::list_archive_entries_direct(
                        path,
                        format,
                        password.as_ref().map(crate::services::SecretString::expose),
                        &cancelled,
                    );
                    Ok(crate::sandbox::ParseOutput {
                        data: crate::adapters::local_operations::encode_archive_result(&result),
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

            let events = events.borrow();
            assert_eq!(events.len(), 1);
            let PreviewEvent::Ready(preview) = &events[0] else {
                panic!("archive preview failed");
            };
            let PreviewContent::Archive { tree } = &preview.content else {
                panic!("expected an archive tree, got {:?}", preview.content);
            };
            assert_eq!(
                tree,
                &ArchivePreviewTree {
                    root: ArchiveDirectory {
                        name: String::new(),
                        children: vec![
                            ArchiveNode::Directory(ArchiveDirectory {
                                name: "folder".to_owned(),
                                children: vec![ArchiveNode::File {
                                    name: "b.txt".to_owned(),
                                    size: 3,
                                }],
                            }),
                            ArchiveNode::File {
                                name: "a.txt".to_owned(),
                                size: 5,
                            },
                        ],
                    },
                    file_count: 2,
                }
            );
        },
    );
}

fn preview_archive(path: &std::path::Path, name: &str) -> Vec<PreviewEvent> {
    preview_archive_with_password(path, name, None)
}

fn preview_archive_with_password(
    path: &std::path::Path,
    name: &str,
    password: Option<&str>,
) -> Vec<PreviewEvent> {
    use crate::{
        model::{EntryKind, FileEntry, Location, MetadataValue},
        services::PreviewRequestId,
    };

    let request = PreviewRequest {
        id: PreviewRequestId(1),
        entry: FileEntry {
            location: Location::local(path),
            thumbnail_path: None,
            native_name: name.into(),
            display_name: name.into(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            is_hidden: false,
        },
        text_byte_limit: 1024,
        render_document: false,
        pdf_page: 0,
        media_size: MediaPreviewSize::new(640, 800),
        archive_password: password
            .map(|password| crate::services::SecretString::new(password.to_owned())),
    };
    let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_for_emit = events.clone();
    let _handle = provider.load_with_renderer(
        request,
        Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
        move |path, operation, _, _, _| {
            let crate::sandbox::ParseOperation::ArchiveList { format, password } = operation else {
                panic!("archive previews must request an archive listing");
            };
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let result = crate::adapters::local_operations::list_archive_entries_direct(
                path,
                format,
                password.as_ref().map(crate::services::SecretString::expose),
                &cancelled,
            );
            Ok(crate::sandbox::ParseOutput {
                data: crate::adapters::local_operations::encode_archive_result(&result),
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
    events.borrow().clone()
}

#[test]
fn tar_gz_archives_list_member_trees_as_preview_content() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::tar_gz_archives_list_member_trees_as_preview_content",
        || {
            use crate::services::{ArchiveDirectory, ArchiveNode, ArchivePreviewTree};

            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("docs.tar.gz");
            {
                let file = fs::File::create(&path).expect("create tar.gz fixture");
                let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
                    file,
                    flate2::Compression::default(),
                ));
                let mut header = tar::Header::new_gnu();
                header.set_mode(0o644);
                header.set_size(2);
                header.set_entry_type(tar::EntryType::Regular);
                builder
                    .append_data(&mut header, "inner/readme.txt", &b"hi"[..])
                    .expect("write tar.gz fixture");
                let encoder = builder.into_inner().expect("finish tar.gz fixture");
                encoder.finish().expect("finish gzip trailer");
            }
            let events = preview_archive(&path, "docs.tar.gz");
            assert_eq!(events.len(), 1);
            let PreviewEvent::Ready(preview) = &events[0] else {
                panic!("archive preview failed");
            };
            let PreviewContent::Archive { tree } = &preview.content else {
                panic!("expected an archive tree, got {:?}", preview.content);
            };
            assert_eq!(
                tree,
                &ArchivePreviewTree {
                    root: ArchiveDirectory {
                        name: String::new(),
                        children: vec![ArchiveNode::Directory(ArchiveDirectory {
                            name: "inner".to_owned(),
                            children: vec![ArchiveNode::File {
                                name: "readme.txt".to_owned(),
                                size: 2,
                            }],
                        })],
                    },
                    file_count: 1,
                }
            );
        },
    );
}

#[test]
fn corrupt_archives_report_a_failed_preview() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::corrupt_archives_report_a_failed_preview",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("fake.zip");
            fs::write(&path, b"not a zip archive").expect("write corrupt fixture");
            let events = preview_archive(&path, "fake.zip");
            assert_eq!(events.len(), 1);
            let PreviewEvent::Failed { message, .. } = &events[0] else {
                panic!("expected a failed archive preview, got {:?}", events[0]);
            };
            assert_eq!(message, "This file is not a valid archive or is damaged.");
        },
    );
}

fn encrypted_archive_fixture(
    directory: &tempfile::TempDir,
    name: &str,
    format: crate::services::ArchiveFormat,
) -> std::path::PathBuf {
    let source = directory.path().join("folder");
    fs::create_dir_all(&source).expect("create fixture source");
    if !source.join("item.txt").exists() {
        fs::write(source.join("item.txt"), b"contents").expect("write fixture source");
    }
    let path = directory.path().join(name);
    crate::adapters::local_operations::write_compression_fixture(
        &path,
        &[source],
        format,
        Some("s3cret"),
    )
    .expect("write encrypted fixture");
    path
}

#[test]
fn encrypted_archive_preview_prompts_for_a_password() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::encrypted_archive_preview_prompts_for_a_password",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(
                &directory,
                "secret.zip",
                crate::services::ArchiveFormat::Zip,
            );
            let events = preview_archive(&zip, "secret.zip");
            assert_eq!(events.len(), 1);
            let PreviewEvent::NeedsPassword { entry, .. } = &events[0] else {
                panic!("expected a password request, got {:?}", events[0]);
            };
            assert_eq!(entry.native_name, "secret.zip");

            let seven_z = encrypted_archive_fixture(
                &directory,
                "secret.7z",
                crate::services::ArchiveFormat::SevenZ,
            );
            let events = preview_archive(&seven_z, "secret.7z");
            assert_eq!(events.len(), 1);
            assert!(
                matches!(&events[0], PreviewEvent::NeedsPassword { .. }),
                "expected a password request, got {:?}",
                events[0]
            );
        },
    );
}

#[test]
fn encrypted_zip_preview_unlocks_with_the_correct_password() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::encrypted_zip_preview_unlocks_with_the_correct_password",
        || {
            use crate::services::{ArchiveDirectory, ArchiveNode, ArchivePreviewTree};

            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(
                &directory,
                "secret.zip",
                crate::services::ArchiveFormat::Zip,
            );
            let events = preview_archive_with_password(&zip, "secret.zip", Some("s3cret"));
            assert_eq!(events.len(), 1);
            let PreviewEvent::Ready(preview) = &events[0] else {
                panic!("expected an unlocked archive, got {:?}", events[0]);
            };
            let PreviewContent::Archive { tree } = &preview.content else {
                panic!("expected an archive tree, got {:?}", preview.content);
            };
            assert_eq!(tree.file_count, 1);
            assert_eq!(
                tree,
                &ArchivePreviewTree {
                    root: ArchiveDirectory {
                        name: String::new(),
                        children: vec![ArchiveNode::Directory(ArchiveDirectory {
                            name: "folder".to_owned(),
                            children: vec![ArchiveNode::File {
                                name: "item.txt".to_owned(),
                                size: 8,
                            }],
                        })],
                    },
                    file_count: 1,
                }
            );
        },
    );
}

#[test]
fn encrypted_zip_preview_rejects_incorrect_and_empty_passwords() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::encrypted_zip_preview_rejects_incorrect_and_empty_passwords",
        || {
            use crate::services::INCORRECT_ARCHIVE_PASSWORD;

            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(
                &directory,
                "secret.zip",
                crate::services::ArchiveFormat::Zip,
            );
            for password in ["wrong", ""] {
                let events = preview_archive_with_password(&zip, "secret.zip", Some(password));
                assert_eq!(events.len(), 1);
                let PreviewEvent::Failed { message, .. } = &events[0] else {
                    panic!(
                        "expected an incorrect-password failure, got {:?}",
                        events[0]
                    );
                };
                assert_eq!(message, INCORRECT_ARCHIVE_PASSWORD);
            }
        },
    );
}

#[test]
fn corrupt_encrypted_zip_preview_reports_a_failed_preview() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::corrupt_encrypted_zip_preview_reports_a_failed_preview",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            let zip = encrypted_archive_fixture(
                &directory,
                "secret.zip",
                crate::services::ArchiveFormat::Zip,
            );
            let bytes = fs::read(&zip).expect("read fixture");
            fs::write(
                &zip,
                &bytes[..bytes.len().saturating_sub(64).max(bytes.len() / 2)],
            )
            .expect("truncate central directory");
            let events = preview_archive(&zip, "secret.zip");
            assert_eq!(events.len(), 1);
            let PreviewEvent::Failed { message, .. } = &events[0] else {
                panic!("expected a failed archive preview, got {:?}", events[0]);
            };
            assert_eq!(message, "This file is not a valid archive or is damaged.");
        },
    );
}

#[test]
fn unsupported_archive_preview_reports_unsupported_format() {
    crate::test_support::gtk_test(
        "adapters::local_preview::tests::unsupported_archive_preview_reports_unsupported_format",
        || {
            use crate::{
                model::{EntryKind, FileEntry, Location, MetadataValue},
                services::PreviewRequestId,
            };

            let directory = tempfile::tempdir().expect("preview fixture directory");
            let path = directory.path().join("sample.zip");
            fs::write(&path, b"placeholder").expect("write placeholder");
            let request = PreviewRequest {
                id: PreviewRequestId(1),
                entry: FileEntry {
                    location: Location::local(&path),
                    thumbnail_path: None,
                    native_name: "sample.zip".into(),
                    display_name: "sample.zip".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    recent_unix_seconds: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                },
                text_byte_limit: 1024,
                render_document: false,
                pdf_page: 0,
                media_size: MediaPreviewSize::new(640, 800),
                archive_password: None,
            };
            let provider = LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software));
            let context = glib::MainContext::default();
            let _owner = context.acquire().expect("main context owner");
            let events = Rc::new(RefCell::new(Vec::new()));
            let events_for_emit = events.clone();
            let _handle = provider.load_with_renderer(
                request,
                Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
                move |_, operation, _, _, _| {
                    let crate::sandbox::ParseOperation::ArchiveList { .. } = operation else {
                        panic!("archive previews must request an archive listing");
                    };
                    let listing = crate::adapters::local_operations::ArchiveListing {
                        status:
                            crate::adapters::local_operations::ArchiveListingStatus::Unsupported,
                        entries: Vec::new(),
                    };
                    Ok(crate::sandbox::ParseOutput {
                        data: crate::adapters::local_operations::encode_archive_result(&Ok(
                            listing,
                        )),
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

            let events = events.borrow();
            assert_eq!(events.len(), 1);
            let PreviewEvent::Failed { message, .. } = &events[0] else {
                panic!(
                    "expected an unsupported-format failure, got {:?}",
                    events[0]
                );
            };
            assert_eq!(message, crate::adapters::ARCHIVE_UNSUPPORTED_MESSAGE);
        },
    );
}
