// SPDX-License-Identifier: MIT

use rune::{Color, El, LineCap, Shape, canvas, memo};
use strata_kit::look::Look;

pub fn checkerboard(key: impl Into<String>, size: [f32; 2], look: &Look) -> El {
    let square = look.space.md.max(1.0);
    let light = look.transparency.opaque(look.kit.c.surface_raised);
    let dark = look.kit.c.surface_base;
    let version = (
        size[0].to_bits(),
        size[1].to_bits(),
        square.to_bits(),
        light,
        dark,
    );
    memo(key, version, move || board(size, square, light, dark))
}

fn board(size: [f32; 2], square: f32, light: Color, dark: Color) -> El {
    let (cols, rows) = (
        (size[0] / square).ceil().max(0.0) as usize,
        (size[1] / square).ceil().max(0.0) as usize,
    );
    let mut shape = Shape::new();
    for r in 0..rows {
        let top = r as f32 * square;
        // The last row and column stop at the board's edge.
        let height = square.min(size[1] - top);
        let y = top + height / 2.0;
        // Dark squares are where row + column is odd.
        for c in ((r + 1) % 2..cols).step_by(2) {
            let x = c as f32 * square;
            let end = (x + square).min(size[0]);
            shape = shape.line([x, y], [end, y], height, LineCap::Butt, dark);
        }
    }
    canvas(shape)
        .size(size[0], size[1])
        .fill(light)
        .a11y_hidden()
        .inert()
}
