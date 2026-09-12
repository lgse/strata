// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{MediaPreviewSize, PreviewContent};

#[test]
fn shared_thumbnail_lookup_is_limited_to_supported_placeholders() {
    assert!(uses_shared_thumbnail(ParseOperation::PreviewImage, 0));
    assert!(uses_shared_thumbnail(ParseOperation::PreviewPdf, 0));
    assert!(!uses_shared_thumbnail(ParseOperation::PreviewPdf, 1));
    assert!(!uses_shared_thumbnail(
        ParseOperation::PreviewMedia(MediaPreviewSize::new(640, 800)),
        0
    ));
}

#[test]
fn full_render_settles_only_after_a_progressive_placeholder() {
    assert_eq!(full_render_settle_delay(false), Duration::ZERO);
    assert_eq!(
        full_render_settle_delay(true),
        PROGRESSIVE_RENDER_SETTLE_DELAY
    );
}

#[test]
fn cancelled_progressive_load_does_not_start_the_full_render() {
    let cancellation = Cancellation::default();
    cancellation.cancel();

    let should_start = glib::MainContext::new().block_on(wait_for_full_render(true, &cancellation));

    assert!(!should_start);
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
        pdf_page: Some(0),
    };
    let pdf_page_1 = PreviewCacheKey {
        path: PathBuf::from("doc.pdf"),
        modified: 300,
        pdf_page: Some(1),
    };
    let page0_content = PreviewContent::Pdf {
        png: vec![10, 20],
        page: 0,
        pages: 2,
    };
    let page1_content = PreviewContent::Pdf {
        png: vec![30, 40, 50],
        page: 1,
        pages: 2,
    };
    cache.insert(pdf_page_0.clone(), page0_content.clone());
    cache.insert(pdf_page_1.clone(), page1_content.clone());
    assert_eq!(cache.get(&pdf_page_0), Some(page0_content));
    assert_eq!(cache.get(&pdf_page_1), Some(page1_content));
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
            pages: 1
        }),
        80
    );
    assert_eq!(
        preview_content_size(&PreviewContent::SandboxedMedia {
            media: SandboxedMedia {
                path: "clip.mp4".into(),
                size: MediaPreviewSize::new(520, 800),
                backend: MediaPreviewBackend::Software
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
