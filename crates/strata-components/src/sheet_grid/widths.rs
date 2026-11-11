//! Column widths, by Strata's rules (`measure_columns` and the fit in `widget_with_height`):
//! each column wants the widest of its first 64 rows' text, and while nobody has resized one,
//! the columns share out the viewport's spare width by how much more than 160 points each
//! wants.

use super::model::Sheet;

/// The narrowest a column gets by dragging its divider (Strata's last-column drag floor).
pub const MIN_WIDTH: f32 = 48.0;
/// The widest a dragged column gets.
pub const MAX_WIDTH: f32 = 4000.0;
const SAMPLE_ROWS: usize = 64;
const SAMPLE_LINES: usize = 8;
const SAMPLE_CHARS: usize = 128;
const WANT_MIN: f32 = 72.0;
const WANT_MAX: f32 = 640.0;
const SHARE_FLOOR: f32 = 160.0;

/// What each column wants: its widest sampled line by `measure`, plus the cell's padding (and
/// the sort caret's room in a title), between 72 and 640 points.
pub(crate) fn wanted(sheet: &Sheet, pad: f32, measure: impl Fn(&str, bool) -> f32) -> Vec<f32> {
    let mut out = vec![WANT_MIN; sheet.columns()];
    for (i, row) in sheet.rows.iter().take(SAMPLE_ROWS).enumerate() {
        let title = i == 0 && sheet.header;
        // Strata pads cells by 36 px and titles by 40: the padding both sides, plus a caret.
        let extra = 2.0 * pad + if title { 20.0 } else { 16.0 };
        for (j, cell) in row.iter().enumerate() {
            for line in cell.lines().take(SAMPLE_LINES) {
                let line: String = line.chars().take(SAMPLE_CHARS).collect();
                let w = (measure(&line, title) + extra).min(WANT_MAX);
                out[j] = out[j].max(w);
            }
        }
    }
    out
}

/// The widths for `available` points: each column at least the smaller of what it wants and
/// 160, the spare shared by what each wants beyond that, the last column taking what rounding
/// leaves, so together they fill `available` once it holds every column's floor.
pub(crate) fn fit(wanted: &[f32], available: f32) -> Vec<f32> {
    let floors: Vec<f32> = wanted.iter().map(|w| w.min(SHARE_FLOOR)).collect();
    let extra = (available - floors.iter().sum::<f32>()).max(0.0);
    let weights: Vec<f32> = wanted
        .iter()
        .zip(&floors)
        .map(|(w, f)| (w - f).max(1.0))
        .collect();
    let total: f32 = weights.iter().sum::<f32>().max(1.0);
    let mut left = extra;
    let n = wanted.len();
    floors
        .iter()
        .zip(&weights)
        .enumerate()
        .map(|(j, (f, w))| {
            let share = if j + 1 == n {
                left
            } else {
                (extra * w / total).floor()
            };
            left -= share;
            f + share
        })
        .collect()
}
