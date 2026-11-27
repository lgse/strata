// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn pixels_are_premultiplied_and_swapped_to_bgra() {
    let half_red = [255, 0, 0, 128];
    assert_eq!(bgra_premultiplied(&half_red), vec![0, 0, 128, 128]);
    assert_eq!(
        bgra_premultiplied(&[10, 20, 30, 255]),
        vec![30, 20, 10, 255]
    );
    assert_eq!(bgra_premultiplied(&[10, 20, 30, 0]), vec![0, 0, 0, 0]);
}

#[test]
fn a_buffer_pads_to_a_size_its_scale_divides() {
    let img = DragImage::new(vec![255u8; 3 * 5 * 4], 3, 5)
        .expect("image")
        .scale(2.0);
    let (w, h, px, hot) = buffer_for(&img, 2);
    assert_eq!((w, h), (4, 6));
    assert_eq!(px.len(), (4 * 6 * 4) as usize);
    assert_eq!(&px[12..16], &[0, 0, 0, 0], "the padded column is clear");
    assert_eq!(&px[0..4], &[255, 255, 255, 255]);
    // The hotspot, the centre (1.5, 2.5) px = (0.75, 1.25) pt, in scale-2 logical pixels.
    assert_eq!(hot, (1, 1));
}

#[test]
fn unpremultiply_srgb_restores_straight_rgba_and_clears_transparent() {
    let mut px = vec![
        128, 64, 32, 128, // ~50% alpha premultiplied
        10, 20, 30, 255, // opaque
        50, 60, 70, 0, // transparent with stray RGB
    ];
    unpremultiply_srgb(&mut px);
    assert_eq!(&px[0..4], &[255, 128, 64, 128]);
    assert_eq!(&px[4..8], &[10, 20, 30, 255]);
    assert_eq!(&px[8..12], &[0, 0, 0, 0]);

    // Round-trip into Wayland/X11 premultiplied BGRA must premultiply only once.
    assert_eq!(&bgra_premultiplied(&px[0..4]), &[32, 64, 128, 128]);
}

#[test]
fn snapshot_without_root_fill_renders_transparent_background_and_straight_rgba() {
    fn view(_: &(), _: &mut Cx<()>) -> El {
        rune::el()
            .size(32.0, 32.0)
            .center()
            .child(rune::el().size(16.0, 16.0).fill(rune::Color {
                r: 1.0,
                g: 0.25,
                b: 0.5,
                a: 0.5,
            }))
    }

    let img = DragIcon::snapshot((), view, (32.0, 32.0), 1.0, true)
        .hotspot(10.0, 12.0)
        .produce()
        .expect("snapshot rendered");
    let (w, h) = img.size();
    let pixel = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
    };

    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert_eq!(
            pixel(x, y),
            [0, 0, 0, 0],
            "corner ({x}, {y}) must be fully transparent, not opaque black"
        );
    }

    let center = pixel(w / 2, h / 2);
    assert!(
        center[3].abs_diff(128) <= 3,
        "center alpha should be ~50%, got {center:?}"
    );
    assert!(
        center[0] >= 245,
        "center red must be un-premultiplied straight RGBA (~255), got {center:?}"
    );
    assert_eq!(img.hotspot, (10.0, 12.0));
}

#[test]
fn an_image_of_the_wrong_size_is_refused() {
    assert!(DragImage::new(vec![0u8; 15], 2, 2).is_none());
    assert!(DragImage::new(Vec::new(), 0, 0).is_none());
}
