// SPDX-License-Identifier: MIT

use super::index::{DirKind, DirParams};

pub(super) fn difference(dir: &DirParams, size: i32, scale: i32) -> i32 {
    let scaled = size * scale;
    match dir.kind {
        DirKind::Fixed => (scaled - dir.size * dir.scale).abs(),
        DirKind::Scalable => {
            if scaled < dir.min * dir.scale {
                dir.min * dir.scale - scaled
            } else if size > dir.max * dir.scale {
                scaled - dir.max * dir.scale
            } else {
                0
            }
        }
        DirKind::Threshold => {
            let min = (dir.size - dir.threshold) * dir.scale;
            let max = (dir.size + dir.threshold) * dir.scale;
            if scaled < min {
                min - scaled
            } else if scaled > max {
                scaled - max
            } else {
                0
            }
        }
    }
}

pub(super) fn better(
    a: &DirParams,
    da: i32,
    b: &DirParams,
    db: i32,
    size: i32,
    scale: i32,
) -> bool {
    if da == 0 {
        if db != 0 {
            return true;
        }
    } else {
        if a.size >= size && b.size < size {
            return true;
        }
        if a.size < size && b.size >= size {
            return false;
        }
        if da != db {
            return da < db;
        }
    }
    if (a.scale == scale) != (b.scale == scale) {
        return a.scale == scale;
    }
    let scalable = |d: &DirParams| d.kind == DirKind::Scalable;
    if scalable(a) != scalable(b) {
        return !scalable(a);
    }
    (size * scale - a.size * a.scale).abs() <= (size * scale - b.size * b.scale).abs()
}
