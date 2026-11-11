use super::*;
use crate::shape::LookParams;
use crate::slots;
use crate::typography::{TEXT_SIZE, line_of};

fn look(text_size: u16, frame: u8, content: u8) -> Look {
    Look::fallback(false).with_params(&LookParams {
        text_size,
        frame_density: frame,
        content_density: content,
        ..LookParams::default()
    })
}

fn sizes() -> impl Iterator<Item = u16> {
    TEXT_SIZE.min..=TEXT_SIZE.max
}

#[test]
fn frame_density_grows_every_bar_in_lock_step_and_nothing_else() {
    for size in sizes() {
        let base = look(size, 0, 4);
        for inset in 1..=FRAME.max {
            let grown = look(size, inset, 4);
            let d = 2.0 * f32::from(inset);
            assert_eq!(
                slots::tabs::tab_height(&grown),
                slots::tabs::tab_height(&base) + d
            );
            assert_eq!(grown.row.sidebar, base.row.sidebar + d);
            assert_eq!(grown.frame.bar, base.frame.bar, "the inset is padding");
            assert_eq!(grown.frame.control, base.frame.control);
            assert_eq!(
                (grown.row.list, grown.row.columns, grown.row.palette),
                (base.row.list, base.row.columns, base.row.palette)
            );
            assert_eq!(grown.content, base.content);
        }
    }
}

#[test]
fn content_density_leaves_the_frame_alone() {
    for size in sizes() {
        let base = look(size, 6, 4);
        for pad in 0..=CONTENT.max {
            let other = look(size, 6, pad);
            assert_eq!(other.frame, base.frame);
            assert_eq!(other.row.sidebar, base.row.sidebar);
            assert_eq!(
                slots::tabs::tab_height(&other),
                slots::tabs::tab_height(&base)
            );
            assert_eq!(other.row.palette, base.row.palette);
        }
    }
}

#[test]
fn rows_fit_their_label_and_icon_at_every_size_and_density() {
    for size in sizes() {
        for frame in 0..=FRAME.max {
            for pad in 0..=CONTENT.max {
                let l = look(size, frame, pad);
                let label = line_of(l.kit.ty.ui_compact);
                let icon = l.icon_size.md;
                let p = 2.0 * f32::from(pad);
                let at = format!("{size} px, frame {frame}, content {pad}");
                for row in [l.row.list, l.row.columns] {
                    assert!(row >= label + p && row >= icon + p, "{at}: {row}");
                }
                let inset = 2.0 * l.frame.inset;
                assert!(
                    l.row.sidebar >= label.max(icon).max(l.frame.control) + inset,
                    "{at}"
                );
                assert!(l.frame.bar >= l.frame.control, "{at}");
                let path = line_of(l.kit.ty.small);
                assert!(l.row.palette >= label + 2.0 + path, "{at}");
            }
        }
    }
}

#[test]
fn rows_and_bars_never_shrink_as_density_or_text_grows() {
    for size in sizes() {
        for pad in 1..=CONTENT.max {
            let (a, b) = (look(size, 0, pad - 1), look(size, 0, pad));
            assert!(b.row.list >= a.row.list, "{size} px, content {pad}");
            assert!(b.row.columns >= a.row.columns, "{size} px, content {pad}");
        }
    }
    for size in TEXT_SIZE.min + 1..=TEXT_SIZE.max {
        for pad in [0, 4, 8, 16] {
            let (a, b) = (look(size - 1, 4, pad), look(size, 4, pad));
            let at = format!("{size} px, content {pad}");
            assert!(b.row.list >= a.row.list, "{at}");
            assert!(b.row.sidebar >= a.row.sidebar, "{at}");
            assert!(b.row.palette >= a.row.palette, "{at}");
            assert!(b.frame.bar >= a.frame.bar, "{at}");
        }
    }
}

#[test]
fn every_size_is_on_whole_points() {
    for size in sizes() {
        for pad in 0..=CONTENT.max {
            let l = look(size, 3, pad);
            let r = l.row;
            for x in [
                r.list,
                r.columns,
                r.sidebar,
                r.palette,
                l.frame.bar,
                l.frame.control,
            ] {
                assert_eq!(x.fract(), 0.0, "{size} px, content {pad}: {x}");
            }
        }
    }
}

#[test]
fn densities_beyond_their_end_load_and_step_as_the_end() {
    let (end, beyond) = (look(13, FRAME.max, CONTENT.max), look(13, 200, 200));
    assert_eq!(end.frame, beyond.frame);
    assert_eq!(end.content, beyond.content);
    assert_eq!(end.row, beyond.row);
    for scale in [FRAME, CONTENT] {
        assert_eq!(scale.stepped(scale.max, 1), scale.max);
        assert_eq!(scale.stepped(0, -1), 0);
        assert_eq!(
            scale.stepped(200, -1),
            scale.max - 1,
            "clamps before stepping"
        );
    }
}

#[test]
fn icons_spacing_grows_with_the_pad_and_never_goes_negative() {
    let mut last = icons_spacing(0.0);
    for pad in 0..=CONTENT.max {
        let s = icons_spacing(f32::from(pad));
        for (x, prev) in [
            (s.grid_pad, last.grid_pad),
            (s.child_margin, last.child_margin),
            (s.card_v, last.card_v),
            (s.card_h, last.card_h),
        ] {
            assert!(x >= 0.0 && x >= prev, "content {pad}");
        }
        last = s;
    }
}
