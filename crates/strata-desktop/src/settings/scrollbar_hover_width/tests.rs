// SPDX-License-Identifier: MIT

use crate::settings::{
    DEFAULT_SCROLLBAR_HOVER_WIDTH, MAX_SCROLLBAR_HOVER_WIDTH, MIN_SCROLLBAR_HOVER_WIDTH,
    Preferences, ScrollbarHoverWidth, parse_preferences,
};

#[test]
fn scrollbar_hover_width_clamps_on_construction_and_deserialization() {
    assert_eq!(DEFAULT_SCROLLBAR_HOVER_WIDTH, 8);
    assert_eq!(
        ScrollbarHoverWidth::default().get(),
        DEFAULT_SCROLLBAR_HOVER_WIDTH
    );
    assert_eq!(ScrollbarHoverWidth::new(0).get(), MIN_SCROLLBAR_HOVER_WIDTH);
    assert_eq!(
        ScrollbarHoverWidth::new(MIN_SCROLLBAR_HOVER_WIDTH - 1).get(),
        MIN_SCROLLBAR_HOVER_WIDTH
    );
    assert_eq!(ScrollbarHoverWidth::new(16).get(), 16);
    assert_eq!(
        ScrollbarHoverWidth::new(MAX_SCROLLBAR_HOVER_WIDTH + 40).get(),
        MAX_SCROLLBAR_HOVER_WIDTH
    );

    for (raw, expected) in [
        ("-10", MIN_SCROLLBAR_HOVER_WIDTH),
        ("0", MIN_SCROLLBAR_HOVER_WIDTH),
        ("5", MIN_SCROLLBAR_HOVER_WIDTH),
        ("6", 6),
        ("18", 18),
        ("24", 24),
        ("999", MAX_SCROLLBAR_HOVER_WIDTH),
    ] {
        let toml =
            format!("mode = \"theme\"\ntheme = \"tokyo-night\"\nscrollbar_hover_width = {raw}\n");
        let parsed = parse_preferences(&toml).expect("parse clamped scrollbar width");
        assert_eq!(parsed.scrollbar_hover_width.get(), expected, "raw={raw}");
    }
}

#[test]
fn scrollbar_hover_preferences_round_trip_and_salvage_invalid_types() {
    let defaults = Preferences::default();
    assert!(defaults.scrollbar_expand_on_hover);
    assert_eq!(defaults.scrollbar_hover_width, ScrollbarHoverWidth::DEFAULT);
    assert_eq!(defaults.scrollbar_hover_width.get(), 8);

    let omitted = parse_preferences("mode = \"theme\"\ntheme = \"tokyo-night\"\n")
        .expect("parse omitted scrollbar preferences");
    assert!(omitted.scrollbar_expand_on_hover);
    assert_eq!(omitted.scrollbar_hover_width, ScrollbarHoverWidth::DEFAULT);
    assert_eq!(omitted.scrollbar_hover_width.get(), 8);

    let custom = Preferences {
        scrollbar_expand_on_hover: false,
        scrollbar_hover_width: ScrollbarHoverWidth::new(18),
        ..Preferences::default()
    };
    let serialized = toml::to_string(&custom).expect("serialize preferences");
    let round_tripped = parse_preferences(&serialized).expect("round-trip preferences");
    assert!(!round_tripped.scrollbar_expand_on_hover);
    assert_eq!(round_tripped.scrollbar_hover_width.get(), 18);

    let explicit_false = parse_preferences(
        "mode = \"theme\"\ntheme = \"tokyo-night\"\nscrollbar_expand_on_hover = false\n",
    )
    .expect("parse explicit false scrollbar preference");
    assert!(!explicit_false.scrollbar_expand_on_hover);
    assert_eq!(
        explicit_false.scrollbar_hover_width,
        ScrollbarHoverWidth::DEFAULT
    );

    let salvaged = parse_preferences(
        "mode = \"theme\"\ntheme = \"tokyo-night\"\nscrollbar_expand_on_hover = \"yes\"\nscrollbar_hover_width = \"wide\"\n",
    )
    .expect("salvage invalid scrollbar preference types");
    assert!(salvaged.scrollbar_expand_on_hover);
    assert_eq!(salvaged.scrollbar_hover_width, ScrollbarHoverWidth::DEFAULT);
    assert_eq!(salvaged.scrollbar_hover_width.get(), 8);
}
