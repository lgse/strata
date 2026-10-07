// SPDX-License-Identifier: MIT

use super::*;

fn frame(width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = color_at(x, y);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    pixels
}

#[test]
fn samples_follow_the_frame_regions_they_cover() {
    let uniform = frame(64, 36, |_, _| [10, 20, 30]);
    let grid = sample(&uniform, 64, 36).expect("grid");
    assert!(grid.cells.iter().all(|cell| *cell == [10, 20, 30]));

    let halves = frame(
        640,
        360,
        |x, _| if x < 320 { [255, 0, 0] } else { [0, 0, 255] },
    );
    let grid = sample(&halves, 640, 360).expect("grid");
    for row in 0..GRID_HEIGHT {
        let cells = &grid.cells[row * GRID_WIDTH..(row + 1) * GRID_WIDTH];
        assert_eq!(cells[0], [255, 0, 0], "row {row} left edge");
        assert_eq!(cells[GRID_WIDTH - 1], [0, 0, 255], "row {row} right edge");
    }
    let bands = frame(
        640,
        360,
        |_, y| if y < 180 { [0, 255, 0] } else { [255, 255, 0] },
    );
    let grid = sample(&bands, 640, 360).expect("grid");
    assert!(
        grid.cells[..GRID_WIDTH]
            .iter()
            .all(|cell| *cell == [0, 255, 0])
    );
    assert!(
        grid.cells[CELLS - GRID_WIDTH..]
            .iter()
            .all(|cell| *cell == [255, 255, 0])
    );

    assert!(sample(&frame(1, 1, |_, _| [7, 7, 7]), 1, 1).is_some());
    assert!(sample(&[0; 8], 2, 2).is_none(), "short buffers are refused");
    assert!(sample(&[], 0, 0).is_none());
}
