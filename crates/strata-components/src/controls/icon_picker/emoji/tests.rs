use super::*;
use crate::controls::icon_picker::Choice;

#[test]
fn the_bundled_emoji_up_to_15_1_fill_every_group() {
    let s = PickerSource::emoji(DEFAULT_VERSION);
    assert_eq!(s.items().len(), 1898);
    assert_eq!(s.categories().len(), GROUPS.len());
    for c in 0..GROUPS.len() {
        assert!(!s.members(c).is_empty(), "group {c} is empty");
    }
    // Newer emoji wait for a higher cap: 🫩 (face with bags under eyes) is Emoji 16.0.
    assert!(s.find(&Choice::emoji("🫩")).is_none());
    assert!(
        PickerSource::emoji(16.0)
            .find(&Choice::emoji("🫩"))
            .is_some()
    );
    assert_eq!(
        PickerSource::emoji(f32::INFINITY).items().len(),
        records().len()
    );
}

#[test]
fn bundled_emoji_are_whole_untoned_and_fit_strata_s_64_bytes() {
    for r in records() {
        assert!(!r.glyph.is_empty() && !r.name.is_empty(), "{r:?}");
        assert!(r.glyph.len() <= 64, "{} is {} bytes", r.name, r.glyph.len());
        assert!(!r.glyph.chars().any(char::is_control), "{}", r.name);
        assert!(
            !r.glyph
                .chars()
                .any(|c| ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)),
            "{} has a skin tone",
            r.name
        );
        assert!(r.version.is_finite(), "{} has no version", r.name);
    }
}

#[test]
fn an_emoji_carries_its_cldr_keywords() {
    let s = PickerSource::emoji(DEFAULT_VERSION);
    let grin = &s.items()[s.find(&Choice::emoji("😀")).expect("bundled")];
    assert_eq!(grin.name, "grinning face");
    assert!(grin.keywords.iter().any(|k| k == "happy"));
}
