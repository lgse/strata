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
fn svg_images_ignore_external_resources_and_scripts() {
    let png = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><script>alert('never')</script><image href="file:///private.png" width="80" height="60"/><image href="https://example.test/tracker.png" width="80" height="60"/><rect width="30" height="20" fill="#cc0000"/></svg>"##).expect("inert SVG");
    let plain = svg(r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="30" height="20" fill="#cc0000"/></svg>"##).expect("plain SVG");
    assert_eq!(
        png, plain,
        "only supported local vector content should render"
    );
    assert!(svg("invalid SVG").is_err());
}
