// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn renders_flowchart_and_sequence_diagrams_without_a_browser() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("diagram.mmd");
    for source in [
        "flowchart LR\n A[Open file] --> B{Supported?}\n B -->|Yes| C[Render diagram]",
        "sequenceDiagram\n participant User\n participant Strata\n User->>Strata: Select Markdown\n Strata-->>User: Render preview",
    ] {
        fs::write(&input, source).expect("diagram");
        let png = mermaid(&input).expect("render Mermaid");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 100, "nonempty rendered content");
    }
    fs::write(&input, "not a diagram").expect("invalid source");
    assert!(mermaid(&input).is_err());
}

#[test]
fn math_renders_fractions_roots_and_ams_without_host_capabilities() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("equation.tex");
    for (source, display) in [
        (r"E=mc^2", false),
        (r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}", true),
        (r"\begin{pmatrix}a&b\\c&d\end{pmatrix}", true),
        (r"\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}", true),
    ] {
        fs::write(&input, source).expect("equation");
        let png = math(&input, display).expect("render equation");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    for source in [
        r"\unknowncommand{x}",
        r"\frac{1}",
        r"\input{/etc/passwd}",
        r"\require{html}",
        r"\href{https://example.test}{x}",
        r"\def\x{\x}\x",
    ] {
        fs::write(&input, source).expect("unsupported equation");
        assert!(math(&input, true).is_err(), "reject {source}");
    }
}

#[test]
fn svg_text_remains_visible_with_generic_font_families() {
    for family in ["sans-serif", "serif", "monospace"] {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"><text x="4" y="24" font-family="{family}" font-size="20">Readable label</text></svg>"#
        );
        let rendered = svg(&source, 800).expect("text SVG");
        let pixels = resvg::tiny_skia::Pixmap::decode_png(&rendered.png).expect("PNG");
        assert!(
            pixels.pixels().iter().any(|pixel| pixel.alpha() != 0),
            "{family} must render text rather than an empty image"
        );
    }
}

#[test]
fn svg_images_ignore_external_resources_and_scripts() {
    let rendered = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><script>alert('never')</script><image href="file:///private.png" width="80" height="60"/><image href="https://example.test/tracker.png" width="80" height="60"/><rect width="30" height="20" fill="#cc0000"/></svg>"##, 800).expect("inert SVG");
    let plain = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="30" height="20" fill="#cc0000"/></svg>"##, 800).expect("plain SVG");
    assert_eq!(
        rendered.png, plain.png,
        "only supported local vector content should render"
    );
    assert!(svg("invalid SVG", 800).is_err());
}

#[test]
fn svg_reports_native_dimensions_and_bounds_the_render() {
    let rendered = svg(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="800"><rect width="1600" height="800" fill="#cc0000"/></svg>"##,
        256,
    )
    .expect("large SVG");
    assert_eq!((rendered.width, rendered.height), (1600, 800));
    let pixels = resvg::tiny_skia::Pixmap::decode_png(&rendered.png).expect("PNG");
    assert_eq!((pixels.width(), pixels.height()), (256, 128));
    let small = svg(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#cc0000"/></svg>"##,
        256,
    )
    .expect("small SVG");
    let pixels = resvg::tiny_skia::Pixmap::decode_png(&small.png).expect("PNG");
    assert_eq!((pixels.width(), pixels.height()), (40, 20));
    assert_eq!(
        svg_dimensions("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 12 6\"/>"),
        Some((12, 6))
    );
}

#[test]
fn svg_font_families_reads_attributes_styles_and_css() {
    let source = concat!(
        r##"<svg xmlns="http://www.w3.org/2000/svg"><style>text { font-family: 'Brand Sans', sans-serif; }</style>"##,
        r#"<text font-family="Title, 'Alt Title'">a</text>"#,
        r##"<text style="font-family:Mono; fill:#000">b</text></svg>"##,
    );
    assert_eq!(
        svg_font_families(source),
        ["Brand Sans", "sans-serif", "Title", "Alt Title", "Mono"].map(str::to_owned)
    );
    assert!(svg_font_families("<svg/>").is_empty());
}

#[test]
fn sniffed_svg_rejected_by_resvg_reaches_the_raster_loaders() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("broken.svg");
    // Sniffs as SVG but is not well-formed XML, so resvg rejects it.
    fs::write(
        &input,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect"#,
    )
    .expect("fixture");
    let source = crate::sandbox_helper::svg_source(&input).expect("sniffed as SVG");
    let resvg_error = svg(&source, 256).err().expect("resvg rejects it");
    // The raster chain decides the outcome; the resvg error must not
    // short-circuit the other loaders.
    if let Err(error) = image(&input, 256) {
        assert_ne!(error, resvg_error);
    }
}

#[test]
fn numeric_character_references_keep_the_full_font_scan() {
    assert!(needs_full_font_scan(r#"<svg><text>&#x1F600;</text></svg>"#));
    assert!(needs_full_font_scan(
        "<svg><text>\u{4e2d}\u{6587}</text></svg>"
    ));
    assert!(!needs_full_font_scan("<svg><text>plain</text></svg>"));
}
