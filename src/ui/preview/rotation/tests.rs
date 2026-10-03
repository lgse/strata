// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

use super::RotatedPaintable;

fn paintable(width: i32, height: i32) -> (gdk::MemoryTexture, RotatedPaintable) {
    let stride = width as usize * 4;
    let texture = gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &glib::Bytes::from_owned(vec![0u8; stride * height as usize]),
        stride,
    );
    let rotated = RotatedPaintable::new(&texture);
    (texture, rotated)
}

#[test]
fn starts_unrotated_with_inner_extent() {
    let (_texture, rotated) = paintable(40, 20);
    assert_eq!(rotated.quarter_turns(), 0);
    assert_eq!(rotated.intrinsic_width(), 40);
    assert_eq!(rotated.intrinsic_height(), 20);
}

#[test]
fn quarter_turns_swap_the_reported_extent() {
    let (_texture, rotated) = paintable(40, 20);
    rotated.rotate_cw();
    assert_eq!(rotated.quarter_turns(), 1);
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (20, 40)
    );
    rotated.rotate_cw();
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (40, 20)
    );
}

#[test]
fn opposite_turns_cancel_each_other() {
    let (_texture, rotated) = paintable(40, 20);
    rotated.rotate_cw();
    rotated.rotate_ccw();
    assert_eq!(rotated.quarter_turns(), 0);
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (40, 20)
    );
}

#[test]
fn four_turns_restore_the_original_orientation() {
    let (_texture, rotated) = paintable(40, 20);
    for _ in 0..4 {
        rotated.rotate_cw();
    }
    assert_eq!(rotated.quarter_turns(), 0);
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (40, 20)
    );
    for _ in 0..4 {
        rotated.rotate_ccw();
    }
    assert_eq!(rotated.quarter_turns(), 0);
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (40, 20)
    );
}

#[test]
fn dropping_the_source_keeps_the_image_alive() {
    let rotated = {
        let (_texture, rotated) = paintable(40, 20);
        rotated
    };
    rotated.rotate_cw();
    assert_eq!(
        (rotated.intrinsic_width(), rotated.intrinsic_height()),
        (20, 40)
    );
}
