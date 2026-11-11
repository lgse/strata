//! GTK 4's size matching (`theme_dir_size_difference` and `compare_dir_size_matches` in
//! `gtkicontheme.c`), kept rule for rule so a lookup picks the file GTK picked.

use super::index::{DirKind, DirParams};

/// How far a directory's sizes are from `size` at `scale`, in device pixels: 0 inside the
/// directory's range.
pub(super) fn difference(dir: &DirParams, size: i32, scale: i32) -> i32 {
    let scaled = size * scale;
    match dir.kind {
        DirKind::Fixed => (scaled - dir.size * dir.scale).abs(),
        DirKind::Scalable => {
            if scaled < dir.min * dir.scale {
                dir.min * dir.scale - scaled
            // GTK compares the unscaled size with the scaled `MaxSize` here (the spec compares
            // the scaled one), so at scale 2 a Scalable directory covers up to twice its
            // `MaxSize` in device pixels. Kept, as the owner ruled, to pick what GTK picked.
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

/// Whether `a` (at difference `da`) is a better match than `b` (at `db`) for `size` at `scale`.
/// A full tie goes to `a`, which the resolver passes as the later directory, as GTK does.
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
        // Scaling an icon down looks better than scaling one up. GTK compares unscaled sizes.
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
