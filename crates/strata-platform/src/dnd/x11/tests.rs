use super::*;

/// Review round 1: raw events name the physical button, so a left-handed mapping (physical 3
/// is logical 1) drops on physical 3, not 1.
#[test]
fn the_drop_follows_the_pointers_button_mapping() {
    assert!(is_primary(&[1, 2, 3], 1));
    assert!(!is_primary(&[1, 2, 3], 3));
    let left_handed = [3, 2, 1];
    assert!(is_primary(&left_handed, 3));
    assert!(!is_primary(&left_handed, 1));
    // No mapping known: the physical number stands.
    assert!(is_primary(&[], 1));
}

#[test]
fn ask_lists_a_description_per_action() {
    let actions = Actions::COPY | Actions::MOVE | Actions::ASK;
    assert_eq!(descriptions(actions), b"Copy\0Move\0Ask\0".to_vec());
}

/// Review round 1: a picture wider than a 16-bit window size is cropped, not wrapped.
#[test]
fn an_icon_too_wide_for_an_x_window_is_cropped() {
    let (w, h) = (70_000u32, 2u32);
    let image = DragImage::new(vec![255u8; (w * h * 4) as usize], w, h).expect("image");
    let (iw, ih, px) = icon_pixels(&image);
    assert_eq!((iw, ih), (ICON_MAX as u16, 2));
    assert_eq!(
        px.len(),
        ICON_MAX as usize * 2 * 4,
        "rows cropped to the width"
    );
}
