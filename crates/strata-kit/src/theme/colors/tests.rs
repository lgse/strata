use super::*;

fn theme_on(background: &str) -> Theme {
    let c = Rgba8::parse(background).expect("parses");
    Theme {
        id: "t".into(),
        name: "T".into(),
        colors: Colors {
            background: c,
            surface: c,
            text: Rgba8::BLACK,
            accent: Rgba8::BLACK,
            danger: DEFAULT_DANGER,
            muted: c,
            highlight: c,
            border: c,
            dim_text: Rgba8::BLACK,
            syntax: SourceColors::default(),
        },
        source: Source::Custom,
    }
}

/// settings/themes: "A theme is filed under Light when its background's relative luminance
/// exceeds 0.4, including backgrounds written as `rgb()` or `#fff`."
#[test]
fn the_light_filter_reads_the_background_in_any_form() {
    for light in ["#fff", "rgb(255, 255, 255)", "#e0e0e0"] {
        assert!(theme_on(light).is_light(), "{light}");
    }
    for dark in ["#000", "#a0a0a0", "rgb(16, 16, 30)"] {
        assert!(!theme_on(dark).is_light(), "{dark}");
    }
}

/// settings/themes: "A theme gets Mocha file icons when its background's (299R + 587G +
/// 114B) ÷ 1000 is below 128, otherwise Latte."
#[test]
fn a_dark_background_is_below_128_by_strata_weights() {
    assert!(theme_on("#7f7f7f").dark_background());
    assert!(!theme_on("#808080").dark_background());
    // The two rules differ: mid grey is neither dark here nor Light in the library.
    let grey = theme_on("#999999");
    assert!(!grey.dark_background() && !grey.is_light());
}

#[test]
fn a_location_counts_lines_and_characters_from_one() {
    let src = "a = 1\nnamé = \"x\"\n";
    assert_eq!(Location::of(src, 0), Location { line: 1, column: 1 });
    let at = src.find('"').expect("a quote");
    assert_eq!(Location::of(src, at), Location { line: 2, column: 8 });
}
