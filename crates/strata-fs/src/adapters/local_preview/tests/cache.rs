// SPDX-License-Identifier: MIT

use super::*;

fn empty_cache() -> PreviewCache {
    PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    }
}

fn cache_key(path: &str, modified: i64) -> PreviewCacheKey {
    PreviewCacheKey {
        path: PathBuf::from(path),
        modified,
        pdf_page: None,
        model: None,
        route: None,
    }
}

fn pdf_page_key(page: i32, size: PdfRenderSize) -> PreviewCacheKey {
    PreviewCacheKey {
        pdf_page: Some((page, size)),
        ..cache_key("doc.pdf", 300)
    }
}

fn pdf_page(png: Vec<u8>, page: i32, pages: i32) -> PreviewContent {
    PreviewContent::Pdf {
        png,
        page,
        pages,
        text_layer: None,
    }
}

fn text(content: &str) -> PreviewContent {
    PreviewContent::Text {
        content: content.to_owned(),
        truncated: false,
    }
}

fn sandboxed_clip() -> PreviewContent {
    PreviewContent::SandboxedMedia {
        media: SandboxedMedia {
            path: "clip.mp4".into(),
            size: MediaPreviewSize::new(520, 800),
            backend: MediaPreviewBackend::Software,
            input_owner: None,
            audio_only: false,
        },
    }
}

fn assert_cache_holds(cache: &PreviewCache, entries: usize, bytes: usize) {
    assert_eq!(
        (cache.entries.len(), cache.byte_count),
        (entries, bytes),
        "(entries, bytes)"
    );
}

/// Writes one PNG page into the shared thumbnail cache and returns the page,
/// which differs from the thumbnail the cache now holds.
fn store_shared_thumbnail(path: &Path) -> Vec<u8> {
    let surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, 640, 800).expect("page surface");
    let mut png = Vec::new();
    surface.write_to_png(&mut png).expect("page PNG");
    crate::adapters::thumbnail_cache::store(path, 1, &png);
    let thumbnail =
        crate::adapters::thumbnail_cache::lookup(path, 1).expect("shared thumbnail is available");
    assert_ne!(thumbnail, png);
    png
}

fn expected_rendered_content(name: &str, png: &[u8]) -> PreviewContent {
    if name.ends_with(".pdf") {
        pdf_page(png.to_vec(), 0, 40)
    } else {
        PreviewContent::Rasterized { png: png.to_vec() }
    }
}

fn load_rendered_page(
    provider: &LocalPreviewProvider,
    context: &glib::MainContext,
    request: &PreviewRequest,
    png: &[u8],
    cached: bool,
) -> Vec<PreviewEvent> {
    let (events, emit) = event_recorder();
    let rendered = png.to_vec();
    let handle = provider.load_with_renderer(request.clone(), emit, move |_, _, _, _, _| {
        assert!(!cached, "reopening should use the rendered-page cache");
        Ok(parse_output(rendered, 0, 40))
    });
    wait_for_first_event(context, &events);
    let events = events.borrow().clone();
    drop(handle);
    events
}

#[test]
fn cold_previews_with_shared_thumbnails_match_rendered_cache_hits() {
    crate::test_support::isolated_test(
        "adapters::local_preview::tests::cache::cold_previews_with_shared_thumbnails_match_rendered_cache_hits",
        || {
            let directory = tempfile::tempdir().expect("preview fixture directory");
            for name in ["document.pdf", "image.png"] {
                let path = directory.path().join(name);
                let _fixture =
                    crate::util::ui_thread::allow(crate::util::ui_thread::Exception::TestSetup);
                let png = store_shared_thumbnail(&path);

                let mut request = preview_request(crate::model::Location::local(&path), name);
                request.entry.modified_unix_seconds = crate::model::MetadataValue::Known(1);
                let provider = software_provider();
                let context = glib::MainContext::default();
                let _owner = context.acquire().expect("main context owner");
                for cached in [false, true] {
                    let events = load_rendered_page(&provider, &context, &request, &png, cached);
                    assert_eq!(events.len(), 1);
                    let PreviewEvent::Ready(preview) = &events[0] else {
                        panic!("PDF preview failed");
                    };
                    assert_eq!(preview.content, expected_rendered_content(name, &png));
                }
            }
        },
    );
}

#[test]
fn preview_cache_stores_and_retrieves_entries() {
    let mut cache = empty_cache();
    let image = cache_key("test1.png", 100);
    let image_content = PreviewContent::Rasterized {
        png: vec![1, 2, 3, 4],
    };
    cache.insert(image.clone(), image_content.clone());
    assert_eq!(cache.get(&image), Some(image_content));
    assert_eq!(cache.byte_count, 4);

    let notes = cache_key("test2.txt", 200);
    cache.insert(notes.clone(), text("hello world"));
    assert_eq!(cache.get(&notes), Some(text("hello world")));
    assert_eq!(cache.byte_count, 4 + 11);

    let viewport = PdfRenderSize::new(640, 800);
    let pages = [
        (pdf_page_key(0, viewport), pdf_page(vec![10, 20], 0, 2)),
        (pdf_page_key(1, viewport), pdf_page(vec![30, 40, 50], 1, 2)),
    ];
    for (key, content) in &pages {
        cache.insert(key.clone(), content.clone());
    }
    for (key, content) in pages {
        assert_eq!(cache.get(&key), Some(content));
    }
    assert_eq!(
        cache.get(&pdf_page_key(0, PdfRenderSize::new(800, 1_800))),
        None,
        "a page rendered for a smaller viewport must not poison a larger preview"
    );
}

#[test]
fn preview_cache_evicts_the_least_recent_entry() {
    let mut cache = empty_cache();
    let keys: Vec<_> = (0..=MAX_PREVIEW_CACHE_ENTRIES)
        .map(|index| cache_key(&format!("image-{index}.png"), index as i64))
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
    assert_cache_holds(&cache, MAX_PREVIEW_CACHE_ENTRIES, MAX_PREVIEW_CACHE_ENTRIES);
}

#[test]
fn replacing_a_preview_cache_entry_updates_its_byte_count() {
    let mut cache = empty_cache();
    let key = cache_key("image.png", 1);

    cache.insert(key.clone(), PreviewContent::Rasterized { png: vec![0; 8] });
    cache.insert(key, PreviewContent::Rasterized { png: vec![0; 3] });

    assert_cache_holds(&cache, 1, 3);
}

#[test]
fn active_media_requests_are_never_retained_by_the_preview_cache() {
    let mut cache = empty_cache();
    let key = cache_key("clip.mp4", 1);
    cache.insert(key.clone(), sandboxed_clip());

    assert_eq!(cache.get(&key), None);
    assert!(cache.entries.is_empty());
    assert_eq!(cache.byte_count, 0);
}

#[test]
fn preview_content_size_computes_accurately() {
    for (content, size) in [
        (PreviewContent::Rasterized { png: vec![0; 100] }, 100),
        (pdf_page(vec![0; 80], 0, 1), 80),
        (sandboxed_clip(), 0),
        (text("12345"), 5),
        (PreviewContent::Unsupported, 0),
    ] {
        assert_eq!(preview_content_size(&content), size, "{content:?}");
    }
}
