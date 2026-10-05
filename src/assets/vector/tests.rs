// SPDX-License-Identifier: MIT

use super::*;

fn pixels(surface: &cairo::ImageSurface) -> Vec<u32> {
    let mut pixels = Vec::new();
    surface
        .with_data(|data| {
            pixels = data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| u32::from_ne_bytes(*pixel))
                .collect();
        })
        .expect("surface data");
    pixels
}

#[test]
fn vector_pixels_preserve_color_transparency_and_bounded_output() {
    let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="12" height="24" fill="red" fill-opacity="0.5"/></svg>"#;
    let image = surface(source, 24).expect("vector surface");
    let data = pixels(&image);
    assert_eq!(
        data[12 * 24 + 4],
        0x80800000,
        "native-endian premultiplied red"
    );
    assert_eq!(data[12 * 24 + 20], 0, "untouched area stays transparent");
    let texture = crate::assets::texture_from_surface(&image).expect("GTK memory texture");
    let mut downloaded = vec![0; 24 * 24 * 4];
    gtk::gdk::prelude::TextureExtManual::download(&texture, &mut downloaded, 24 * 4);
    let downloaded: Vec<_> = downloaded
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| u32::from_ne_bytes(*pixel))
        .collect();
    assert_eq!(
        downloaded, data,
        "GTK receives the same premultiplied color and alpha"
    );
    for invalid in [0, -1, 769, i32::MAX] {
        assert!(surface(source, invalid).is_none());
    }
    assert!(surface("not SVG", 24).is_none());
    let at_limit = format!("{source}{}", " ".repeat(64 * 1024 - source.len()));
    assert!(surface(&at_limit, 24).is_some());
    assert!(surface(&format!("{at_limit} "), 24).is_none());
}

#[test]
fn vector_icons_never_resolve_external_or_embedded_images() {
    let directory = tempfile::tempdir().expect("external image fixture");
    let embedded = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="red"/></svg>"#;
    let external = directory.path().join("external.svg");
    std::fs::write(&external, embedded).expect("external SVG");
    let encoded: String = embedded
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect();
    for href in [
        external.to_string_lossy().into_owned(),
        format!("data:image/svg+xml,{encoded}"),
    ] {
        let href = href.replace('&', "&amp;").replace('"', "&quot;");
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="24" height="24"><image width="24" height="24" xlink:href="{href}"/></svg>"#
        );
        assert!(
            pixels(&surface(&source, 24).expect("bounded vector tree"))
                .iter()
                .all(|pixel| *pixel == 0),
            "image references must remain unresolved"
        );
    }
}
