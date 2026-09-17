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
        let png = svg(&source).expect("text SVG");
        let pixels = resvg::tiny_skia::Pixmap::decode_png(&png).expect("PNG");
        assert!(
            pixels.pixels().iter().any(|pixel| pixel.alpha() != 0),
            "{family} must render text rather than an empty image"
        );
    }
}

#[test]
fn svg_images_ignore_external_resources_and_scripts() {
    let png = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><script>alert('never')</script><image href="file:///private.png" width="80" height="60"/><image href="https://example.test/tracker.png" width="80" height="60"/><rect width="30" height="20" fill="#cc0000"/></svg>"##).expect("inert SVG");
    let plain = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="30" height="20" fill="#cc0000"/></svg>"##).expect("plain SVG");
    assert_eq!(
        png, plain,
        "only supported local vector content should render"
    );
    assert!(svg("invalid SVG").is_err());
}
