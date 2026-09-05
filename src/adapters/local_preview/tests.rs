// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use super::*;
use crate::services::PreviewContent;

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
        "1".to_owned(),
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
}

#[test]
fn preview_cache_stores_and_retrieves_entries() {
    let mut cache = PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    };
    let key1 = PreviewCacheKey {
        path: PathBuf::from("/tmp/test1.png"),
        modified: Some(100),
        pdf_page: None,
    };
    let content1 = PreviewContent::Rasterized {
        png: vec![1, 2, 3, 4],
    };
    cache.insert(key1.clone(), content1.clone());
    assert_eq!(cache.get(&key1), Some(content1));
    assert_eq!(cache.byte_count, 4);

    let key2 = PreviewCacheKey {
        path: PathBuf::from("/tmp/test2.txt"),
        modified: Some(200),
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
        path: PathBuf::from("/tmp/doc.pdf"),
        modified: Some(300),
        pdf_page: Some(0),
    };
    let pdf_page_1 = PreviewCacheKey {
        path: PathBuf::from("/tmp/doc.pdf"),
        modified: Some(300),
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
        preview_content_size(&PreviewContent::SandboxedMedia { data: vec![0; 50] }),
        50
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
