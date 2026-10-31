// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Capture {
    pub(crate) x: i64,
    pub(crate) y: i64,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<[u8; 3]>,
}

impl Capture {
    fn pixel(&self, column: usize, row: usize) -> [u8; 3] {
        self.pixels[row * self.width + column]
    }
}

/// PIL's `convert("L")`: ITU-R 601-2 luma in 16-bit fixed point.
pub(crate) fn luma([red, green, blue]: [u8; 3]) -> u8 {
    let weighted =
        u32::from(red) * 19_595 + u32::from(green) * 38_470 + u32::from(blue) * 7_471 + 0x8000;
    u8::try_from(weighted >> 16).unwrap_or(u8::MAX)
}

pub(crate) fn text_position(capture: &Capture) -> (Option<f64>, u64) {
    let mut weights = Vec::with_capacity(capture.height);
    for row in 0..capture.height {
        let mut values: Vec<u8> = (0..capture.width)
            .map(|column| luma(capture.pixel(column, row)))
            .collect();
        let row_values = values.clone();
        values.sort_unstable();
        let background = values.get(values.len() / 2).copied().unwrap_or_default();
        weights.push(
            row_values
                .iter()
                .map(|value| u64::from(value.abs_diff(background)))
                .sum::<u64>(),
        );
    }
    let contrast: u64 = weights.iter().sum();
    if contrast == 0 {
        return (None, 0);
    }
    let moment: u64 = weights
        .iter()
        .enumerate()
        .map(|(row, weight)| row as u64 * weight)
        .sum();
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel sums stay far below 2^52, as Python's float division assumes"
    )]
    let position = moment as f64 / contrast as f64;
    (Some(position), contrast)
}

pub(crate) fn differs_beside_pointer(
    before: &Capture,
    after: &Capture,
    pointer: (i64, i64),
) -> bool {
    if (before.width, before.height) != (after.width, after.height) {
        return true;
    }
    let (left, top) = (pointer.0 - before.x - 2, pointer.1 - before.y - 2);
    let (right, bottom) = (pointer.0 - before.x + 32, pointer.1 - before.y + 32);
    (0..before.height).any(|row| {
        (0..before.width).any(|column| {
            let (x, y) = (column as i64, row as i64);
            let masked = left <= x && x <= right && top <= y && y <= bottom;
            !masked && before.pixel(column, row) != after.pixel(column, row)
        })
    })
}
