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

/// A 3×5 picture for a scale-2 surface pads to 4×6, so no `invalid_size` reaches winit's
/// connection; the padding is clear.
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
fn an_image_of_the_wrong_size_is_refused() {
    assert!(DragImage::new(vec![0u8; 15], 2, 2).is_none());
    assert!(DragImage::new(Vec::new(), 0, 0).is_none());
}
