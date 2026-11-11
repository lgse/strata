//! Tile metrics and the caption cut, measured in Rune's own fonts.

mod perf;
mod runtime;

use strata_kit::look::Look;

use super::caption::{CaptionCache, cut};
use super::metrics::TileMetrics;

const LONG: &str = "2026-10-07-release-candidate-build-final-v3-with-a-very-long-tail.tar.gz";

/// The thumbnail slot keeps the tile size at any text size; the caption's reserved lines grow
/// with it. (The metrics take no name or details at all: a tile's height never follows them.)
#[test]
fn a_tiles_slot_ignores_the_text_size_and_its_caption_follows_it() {
    let look = Look::fallback(false);
    let a = TileMetrics::new(&look, 64.0, 2, true);
    assert_eq!(a.slot, 64.0);
    let mut big = Look::fallback(false);
    big.kit.ty.ui_compact.size *= 2.0;
    big.kit.ty.ui_compact.line_height *= 2.0;
    let b = TileMetrics::new(&big, 64.0, 2, true);
    assert_eq!(b.slot, 64.0, "the thumbnail doesn't follow the text size");
    assert!(b.height > a.height, "the caption does");
}

#[test]
fn sizes_off_the_ladder_are_held_to_its_ends() {
    let look = Look::fallback(false);
    assert_eq!(TileMetrics::new(&look, 8.0, 2, true).slot, 32.0);
    assert_eq!(TileMetrics::new(&look, 999.0, 2, true).slot, 256.0);
    assert_eq!(TileMetrics::new(&look, f32::NAN, 2, true).slot, 64.0);
}

#[test]
fn rows_hold_more_tiles_as_the_width_grows_up_to_the_density_cap() {
    let mut look = Look::fallback(false);
    let m = TileMetrics::new(&look, 32.0, 2, true);
    assert_eq!(m.per_line(0.0), 1);
    assert_eq!(m.per_line(m.min_cell * 3.0 + 1.0), 3);
    assert_eq!(m.per_line(100_000.0), 20, "Compact caps at 20");
    look.content.pad = 8.0;
    let airy = TileMetrics::new(&look, 32.0, 2, true);
    assert_eq!(airy.per_line(100_000.0), 16, "Airy caps at 16");
}

#[test]
fn a_name_that_fits_is_whole_and_a_long_one_keeps_both_ends() {
    let font = Look::fallback(false).kit.ty.ui_compact;
    let short = cut("notes.md", font, 100.0, 2);
    assert!(!short.cut);
    assert_eq!(&*short.text, "notes.md");
    let long = cut(LONG, font, 100.0, 2);
    assert!(long.cut);
    assert!(long.text.contains('…'));
    assert!(long.text.starts_with("2026"));
    assert!(
        long.text.ends_with(".tar.gz"),
        "the end that tells names apart stays"
    );
    let line = strata_kit::typography::line_of(font);
    assert!(long.size.1 <= 2.0 * line + 0.5, "it fits its two lines");
    // One line holds less of it.
    let one = cut(LONG, font, 100.0, 1);
    assert!(one.text.chars().count() < long.text.chars().count());
}

#[test]
fn the_caption_cache_stays_bounded() {
    let cache = CaptionCache::default();
    let font = Look::fallback(false).kit.ty.ui_compact;
    for i in 0..10_000 {
        cache.get(&format!("file-{i}.txt"), font, 100.0, 2);
    }
    assert!(cache.len() <= 2 * 4096);
    let a = cache.get("file-9999.txt", font, 100.0, 2);
    assert_eq!(&*a.text, "file-9999.txt", "a recent cut is still there");
}

#[test]
fn a_middle_press_autoscrolls_from_no_tile_or_a_tiles_blank_space_only() {
    use crate::collection::element::{Ctx, Hit, PointerInput, PointerPhase, Region};
    use crate::collection::selection::Selection;
    use crate::contract::{Point, Rect, UniformRows};
    use crate::scroll::AutoscrollSettings;
    use rune::{Mods, PointerButton};

    let src = crate::contract::ListSource::new(Vec::new());
    let geometry = UniformRows::new(0, 20.0, 100.0);
    let mut sel = Selection::new();
    let cx = Ctx::new(&src, &geometry, Rect::default(), 0.0, &mut sel);
    let press = |index, region| PointerInput {
        phase: PointerPhase::Down,
        button: PointerButton::Middle,
        point: Point::default(),
        window: Point::default(),
        travel: Point::default(),
        hit: Hit { index, region },
        mods: Mods::default(),
        clicks: 1,
        cancelled: false,
        now: 0.0,
    };
    let s = AutoscrollSettings::default();
    let bg = |index, region| super::autoscroll_background(&press(index, region), &cx, &s);
    assert!(bg(None, Region::Gutter), "no tile");
    assert!(bg(Some(3), Region::Gutter), "a tile's blank space");
    assert!(!bg(Some(3), Region::Content), "its thumbnail");
    assert!(!bg(Some(3), Region::Name), "its name");
}

/// A tile's frame and name show hover only while `RowCx::hover_live` is on: off after a key
/// moves the cursor (keyboard mode), until the pointer moves. A bare tile follows it itself.
#[test]
fn a_tile_shows_no_hover_in_keyboard_mode() {
    use crate::collection::element::{RowCx, RowState};
    use crate::contract::{Entry, EntryId, EntryKind};

    let look = Look::fallback(false);
    let entry = Entry::new(EntryId::new(1), "a.txt", EntryKind::File, "");
    let rcx = |hover_live, selected| RowCx {
        index: 0,
        entry,
        state: RowState {
            selected,
            ..RowState::default()
        },
        look: &look,
        scope: "grid",
        hover_live,
    };
    for selected in [false, true] {
        assert!(
            super::tile::frame_look(&rcx(true, selected)).2.is_some(),
            "{selected}"
        );
        assert!(
            super::tile::frame_look(&rcx(false, selected)).2.is_none(),
            "{selected}"
        );
    }
    assert!(super::tile::name_look(&rcx(true, false)).2.is_some());
    assert!(super::tile::name_look(&rcx(false, false)).2.is_none());
}

/// A cut keeps whole grapheme clusters at both ends: no combining mark is left without its
/// letter, and no ZWJ emoji sequence is split, at any caption width.
#[test]
fn a_cut_never_splits_a_grapheme_cluster() {
    use unicode_segmentation::UnicodeSegmentation;

    let font = Look::fallback(false).kit.ty.ui_compact;
    let combining = "e\u{301}".repeat(30);
    let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let name = format!("{combining}{family}{family}{family}{combining}.txt");
    let whole: Vec<&str> = name.graphemes(true).collect();
    let mut cuts = 0;
    for width in (40..240).step_by(3) {
        let c = cut(&name, font, width as f32, 2);
        if !c.cut {
            continue;
        }
        cuts += 1;
        let (head, tail) = c.text.split_once('…').expect("an ellipsis");
        let head: Vec<&str> = head.graphemes(true).collect();
        let tail: Vec<&str> = tail.graphemes(true).collect();
        assert_eq!(head, whole[..head.len()], "the head at {width}: {}", c.text);
        assert_eq!(
            tail,
            whole[whole.len() - tail.len()..],
            "the tail at {width}: {}",
            c.text
        );
    }
    assert!(cuts > 10, "the name was cut at most widths");
}

/// A details line wider than the card is cut to the card's inner width as it draws, so a
/// point on the tile's margin beside it is blank space, not the line.
#[test]
fn a_long_details_line_hits_only_inside_the_card() {
    use super::caption::Cut;
    use crate::collection::element::Region;
    use crate::contract::Point;

    let look = Look::fallback(false);
    let m = TileMetrics::new(&look, 64.0, 2, true);
    let cell_w = m.min_cell;
    let cut = Cut {
        text: "a".into(),
        cut: false,
        size: (8.0, m.name_line),
    };
    let y = m.details_top() + m.details_line / 2.0;
    let wide = 10.0 * cell_w;
    assert_eq!(
        super::regions::region(&m, cell_w, &cut, wide, Point::new(cell_w / 2.0, y)),
        Region::Content
    );
    assert_eq!(
        super::regions::region(&m, cell_w, &cut, wide, Point::new(m.margin / 2.0, y)),
        Region::Gutter,
        "the tile's margin"
    );
}

/// The filter's Enter target is outlined in the accent (Strata's `.enter-target`), as a
/// selected frame is outlined in its own ring, unless the cursor ring is drawn over it.
#[test]
fn the_enter_target_frame_is_outlined_in_the_accent() {
    use crate::collection::element::{RowCx, RowMarks, RowState};
    use crate::contract::{Entry, EntryId, EntryKind};

    let look = Look::fallback(false);
    let entry = Entry::new(EntryId::new(1), "a.txt", EntryKind::File, "");
    let marks = RowMarks {
        enter_target: true,
        ..RowMarks::default()
    };
    let rcx = |marks, cursor_shown| RowCx {
        index: 0,
        entry,
        state: RowState {
            marks,
            cursor_shown,
            ..RowState::default()
        },
        look: &look,
        scope: "grid",
        hover_live: false,
    };
    assert_eq!(
        super::tile::frame_look(&rcx(marks, false)).1,
        Some((1.0, look.kit.c.text_accent))
    );
    assert_eq!(
        super::tile::frame_look(&rcx(RowMarks::default(), false)).1,
        None
    );
    assert_ne!(
        super::tile::frame_look(&rcx(marks, true)).1,
        Some((1.0, look.kit.c.text_accent)),
        "the cursor ring wins"
    );
}
