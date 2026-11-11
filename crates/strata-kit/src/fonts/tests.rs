use rune::text::TextSystem;
use rune::{Family, TextStyle};

use super::*;
use crate::look::Look;

/// The big-endian integer of `N` bytes at `at`.
fn be<const N: usize>(data: &[u8], at: usize) -> u32 {
    data[at..at + N]
        .iter()
        .fold(0, |n, &b| (n << 8) | u32::from(b))
}

/// The bytes of the sfnt table tagged `tag`.
fn table<'a>(face: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
    let count = be::<2>(face, 4) as usize;
    (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&rec| &face[rec..rec + 4] == tag)
        .map(|rec| {
            let (offset, len) = (
                be::<4>(face, rec + 8) as usize,
                be::<4>(face, rec + 12) as usize,
            );
            &face[offset..offset + len]
        })
        .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
}

/// Every Windows-platform (UTF-16BE) string with name ID `id` in the `name` table.
fn names(face: &[u8], id: u32) -> Vec<String> {
    let name = table(face, b"name");
    let (count, strings) = (be::<2>(name, 2) as usize, be::<2>(name, 4) as usize);
    (0..count)
        .map(|i| 6 + 12 * i)
        .filter(|&rec| be::<2>(name, rec) == 3 && be::<2>(name, rec + 6) == id)
        .map(|rec| {
            let (len, at) = (
                be::<2>(name, rec + 8) as usize,
                be::<2>(name, rec + 10) as usize,
            );
            let raw = &name[strings + at..strings + at + len];
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_be_bytes(c))
                .collect();
            String::from_utf16(&units).expect("UTF-16 name")
        })
        .collect()
}

/// The `wght` axis's (min, max) in the `fvar` table.
fn wght_axis(face: &[u8]) -> (f32, f32) {
    let fvar = table(face, b"fvar");
    let (axes, count, size) = (
        be::<2>(fvar, 4) as usize,
        be::<2>(fvar, 8) as usize,
        be::<2>(fvar, 10) as usize,
    );
    let fixed = |at| be::<4>(fvar, at) as i32 as f32 / 65536.0;
    (0..count)
        .map(|i| axes + size * i)
        .find(|&rec| &fvar[rec..rec + 4] == b"wght")
        .map(|rec| (fixed(rec + 4), fixed(rec + 12)))
        .expect("a wght axis")
}

/// Every weight the look's type ramps use.
fn ramp_weights() -> Vec<f32> {
    let look = Look::fallback(false);
    let t = look.kit.ty;
    [
        t.ui,
        t.ui_medium,
        t.ui_compact,
        t.small,
        t.small_medium,
        t.body,
        t.body_medium,
        t.text_small,
        t.text_small_medium,
        t.text_xsmall,
        t.text_xsmall_medium,
        t.mono,
        look.ty.title,
        look.ty.heading,
    ]
    .map(|s| s.weight)
    .to_vec()
}

// The weight test, which measures through `rune::text`'s shared state, lives alone in
// `tests/fonts_weight.rs`: see there.

/// The face's family (name ID 1) and typographic family (16, when present) are the name
/// [`FAMILIES`] gives Rune, and its `wght` axis spans every weight the ramps use.
#[test]
fn the_bundled_face_is_named_as_rune_matches_it_and_spans_the_ramps_weights() {
    let family = names(JETBRAINS_MONO, 1);
    assert!(!family.is_empty(), "no family name");
    for name in family.iter().chain(&names(JETBRAINS_MONO, 16)) {
        assert_eq!(name, FAMILY);
    }
    let (min, max) = wght_axis(JETBRAINS_MONO);
    for weight in ramp_weights() {
        assert!(
            (min..=max).contains(&weight),
            "{weight} outside {min}–{max}"
        );
    }
}

/// Text in both slots and by name shapes in the bundled face: monospace at JetBrains Mono's
/// 600-unit advance (on 1000 units per em). A proportional fallback gives `i` and `M`
/// different advances, and DejaVu Sans Mono's is 602. It measures on its own text system, not
/// through `rune::text`'s shared state, so parallel tests can't change what it measures.
#[test]
fn text_shapes_in_jetbrains_mono_not_a_fallback() {
    let mut text = TextSystem::new(FONTS, FAMILIES);
    let style = TextStyle::new(100.0, 400.0);
    for (slot, style) in [
        ("sans", style),
        ("mono", style.mono()),
        ("named", style.family(Family::Named(FAMILY))),
    ] {
        for run in ["iiiiiiiiii", "MMMMMMMMMM"] {
            let (w, _) = text.measure(run, style, 4.0);
            assert_eq!(w, 600.0, "{slot} {run}");
        }
    }
}
