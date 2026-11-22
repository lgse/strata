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
fn edge_grid_samples_uniform_quadrants_and_rejects_invalid_buffers() {
    let uniform = frame(64, 36, |_, _| [10, 20, 30]);
    let grid = EdgeGrid::sample(&uniform, 64, 36).expect("uniform grid");
    assert!(grid.cells.iter().all(|cell| *cell == [10, 20, 30]));

    let quadrants = frame(640, 360, |x, y| match (x < 320, y < 180) {
        (true, true) => [255, 0, 0],
        (false, true) => [0, 255, 0],
        (true, false) => [0, 0, 255],
        (false, false) => [255, 255, 0],
    });
    let grid = EdgeGrid::sample(&quadrants, 640, 360).expect("quadrant grid");
    assert_eq!(grid.cells[0], [255, 0, 0], "top-left");
    assert_eq!(grid.cells[GRID_WIDTH - 1], [0, 255, 0], "top-right");
    assert_eq!(
        grid.cells[(GRID_HEIGHT - 1) * GRID_WIDTH],
        [0, 0, 255],
        "bottom-left"
    );
    assert_eq!(grid.cells[CELLS - 1], [255, 255, 0], "bottom-right");

    assert!(EdgeGrid::sample(&frame(1, 1, |_, _| [7, 8, 9]), 1, 1).is_some());
    assert!(
        EdgeGrid::sample(&[0; 8], 2, 2).is_none(),
        "short buffers are rejected"
    );
    assert!(EdgeGrid::sample(&[], 0, 0).is_none());
    assert!(EdgeGrid::sample(&[0; 16], 0, 4).is_none());
    assert!(EdgeGrid::sample(&[0; 16], 4, 0).is_none());
}

#[test]
fn ambient_glow_snaps_first_frame_eases_subsequent_throttles_and_clears() {
    let mut glow = AmbientGlow::new();
    assert!(!glow.is_lit());

    let red = EdgeGrid {
        cells: [[200, 0, 0]; CELLS],
    };
    assert!(glow.update(&red, 1.0), "first frame updates immediately");
    assert!(glow.is_lit());
    assert_eq!(
        glow.pixels().size(),
        (GRID_WIDTH as u32, GRID_HEIGHT as u32)
    );
    assert!(
        glow.shown_rgb().iter().all(|c| *c == [200, 0, 0]),
        "first frame snaps without ramping from black"
    );

    let blue = EdgeGrid {
        cells: [[0, 0, 200]; CELLS],
    };
    assert!(
        !glow.update(&blue, 1.0 + UPDATE_INTERVAL * 0.5),
        "updates within UPDATE_INTERVAL are throttled"
    );
    assert!(glow.shown_rgb().iter().all(|c| *c == [200, 0, 0]));

    assert!(
        glow.update(&blue, 1.0 + UPDATE_INTERVAL),
        "update after UPDATE_INTERVAL applies exponential smoothing"
    );
    let expected_r = (200.0 + (0.0 - 200.0) * SMOOTHING).round() as u8;
    let expected_b = (0.0 + (200.0 - 0.0) * SMOOTHING).round() as u8;
    assert!(
        glow.shown_rgb()
            .iter()
            .all(|c| *c == [expected_r, 0, expected_b]),
        "got {:?}",
        glow.shown_rgb()[0]
    );

    glow.clear();
    assert!(!glow.is_lit());
    assert!(
        glow.update(&blue, 1.0 + UPDATE_INTERVAL * 1.1),
        "after clear, next frame snaps immediately without waiting for throttle"
    );
    assert!(glow.is_lit());
    assert!(glow.shown_rgb().iter().all(|c| *c == [0, 0, 200]));
}
