// SPDX-License-Identifier: MIT

use super::*;

struct RenderedPdfPage {
    png: Vec<u8>,
    sidecar: Vec<u8>,
}

impl RenderedPdfPage {
    fn dimensions(&self) -> (u32, u32) {
        let be_u32 = |range: std::ops::Range<usize>, what: &str| {
            u32::from_be_bytes(self.png[range].try_into().expect(what))
        };
        (
            be_u32(16..20, "PNG width bytes"),
            be_u32(20..24, "PNG height bytes"),
        )
    }
}

fn pdf_fixture_path(stem: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{stem}-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ))
}

fn write_letter_pdf(path: &Path, draw: impl FnOnce(&cairo::Context)) {
    let surface = cairo::PdfSurface::new(612.0, 792.0, path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        draw(&context);
    }
    surface.finish();
}

/// Renders `page_spec` through the sandbox helper and reads the PNG plus the
/// named sidecar before removing every fixture file.
fn render_pdf_page(
    path: PathBuf,
    page_spec: &str,
    sidecar: &str,
    expectation: &str,
) -> RenderedPdfPage {
    let output_directory = path.with_extension("output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    crate::sandbox_helper::run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        page_spec.to_owned(),
        "software".to_owned(),
    ])
    .expect(expectation);
    let png = fs::read(&output).expect("read rendered page");
    let sidecar = fs::read(output_directory.join(sidecar)).expect("read PDF sidecar");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);
    RenderedPdfPage { png, sidecar }
}

#[test]
fn renders_requested_pdf_pages_within_the_pixel_budget() {
    let path = pdf_fixture_path("strata-preview");
    write_letter_pdf(&path, |context| {
        context.set_source_rgb(0.2, 0.4, 0.8);
        context.paint().expect("paint PDF page");
        context.show_page().expect("finish first PDF page");
        context.set_source_rgb(0.8, 0.4, 0.2);
        context.paint().expect("paint second PDF page");
        context.show_page().expect("finish second PDF page");
    });

    let rendered = render_pdf_page(path, "1:640x800", "result.meta", "render second PDF page");
    let metadata = String::from_utf8(rendered.sidecar.clone()).expect("read PDF metadata");

    assert_eq!(metadata, "1 2");
    assert!(rendered.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(rendered.dimensions(), (618, 800));
}

#[test]
fn pdf_preview_emits_a_text_layer_matching_the_rendered_page() {
    let path = pdf_fixture_path("strata-preview-text");
    write_letter_pdf(&path, |context| {
        context.set_source_rgb(0.1, 0.1, 0.1);
        context.set_font_size(24.0);
        context.move_to(72.0, 96.0);
        context
            .show_text("selectable helper text")
            .expect("draw text");
        context.show_page().expect("finish PDF page");
    });

    let rendered = render_pdf_page(
        path,
        "0:640x800",
        "result.text",
        "render PDF page with text",
    );
    let (png_width, _) = rendered.dimensions();
    let layer: crate::services::PdfTextLayer =
        serde_json::from_slice(&rendered.sidecar).expect("parse text layer");

    assert_eq!(layer.text.trim_end_matches('\n'), "selectable helper text");
    assert_eq!(layer.glyphs.len(), layer.text.chars().count());
    assert_eq!(layer.width, png_width as f32);
    // The text baseline sits near the top of the page in PNG pixels.
    let baseline = layer.glyphs[0][1];
    assert!(baseline > 0.0 && baseline < layer.height * 0.25);
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
