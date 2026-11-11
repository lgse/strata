use super::*;

fn parse(s: &str) -> String {
    Rgba8::parse(s)
        .unwrap_or_else(|| panic!("{s:?} parses"))
        .hex()
}

/// settings/themes: "A custom file with a GTK-only color form, such as 9-digit hex, loads with
/// colors as `#rrggbb`, or `#rrggbbaa` when translucent".
#[test]
fn gtk_only_hex_forms_load_canonical() {
    assert_eq!(parse("#7a7a2af7f"), "#7aa2f7");
    assert_eq!(parse("#7a7aa2a2f7f7"), "#7aa2f7");
    assert_eq!(parse("#7a7aa2a2f7f78080"), "#7aa2f780");
    assert_eq!(parse("#7AA2F7"), "#7aa2f7");
    assert_eq!(parse("#7aa2f7ff"), "#7aa2f7");
    assert_eq!(parse("#7aa2f780"), "#7aa2f780");
}

/// settings/themes: "A custom theme whose colors are written as `rgb()`, short hex, or color
/// names keeps syntax colors in code previews": each form reads as the same colour.
#[test]
fn css_forms_name_the_same_colour() {
    for form in [
        "#f00",
        "#ff0000",
        "red",
        "RED",
        " red ",
        "rgb(255, 0, 0)",
        "rgb(255 0 0)",
        "rgb(100%, 0%, 0%)",
        "rgba(255, 0, 0, 1)",
        "hsl(0, 100%, 50%)",
        "hsl(360deg 100% 50%)",
    ] {
        assert_eq!(parse(form), "#ff0000", "{form}");
    }
    assert_eq!(parse("#f008"), "#ff000088");
    assert_eq!(parse("rgba(255, 0, 0, 0.5)"), "#ff000080");
    assert_eq!(parse("rgb(255 0 0 / 50%)"), "#ff000080");
    assert_eq!(parse("hsl(120, 100%, 25%)"), "#008000");
    assert_eq!(parse("transparent"), "#00000000");
    assert_eq!(parse("rebeccapurple"), "#663399");
}

#[test]
fn malformed_colours_do_not_parse() {
    for bad in [
        "",
        "#",
        "#zz",
        "#12345",
        "#1234567",
        "nope",
        "rgb(1,2)",
        "rgb(a,b,c)",
        "hsl(1,2,3)",
        "rgb(1,2,3",
        "#7aa2f7 trailing",
        "lightgoldenrod",
    ] {
        assert_eq!(Rgba8::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_colour_reaches_rune_unchanged() {
    let c = Rgba8::parse("#7aa2f780").expect("parses").color();
    assert_eq!(c.hex(), "#7aa2f780");
}

/// settings/themes: "Loading a bundled theme whose catalog dim text misses those floors blends
/// it toward its text color in 1/255 steps, stopping at the first that passes. Tokyo Night's
/// #737aa2 loads as #767da6."
#[test]
fn readable_stops_at_the_first_step_that_passes_as_strata_does() {
    let hex = |s| Rgba8::parse(s).expect("parses");
    let (bg, surface, text) = (hex("#16161e"), hex("#1f2335"), hex("#c0caf5"));
    let dim = readable(hex("#737aa2"), &[bg], text, 4.5);
    let dim = readable(dim, &[surface], text, 3.0);
    assert_eq!(dim.hex(), "#767da6");
    // Already readable: unchanged.
    assert_eq!(readable(text, &[bg], Rgba8::WHITE, 4.5), text);
    // Nothing reaches 21:1 on mid grey: the pole.
    assert_eq!(
        readable(hex("#777777"), &[hex("#777777")], Rgba8::WHITE, 21.0),
        Rgba8::WHITE
    );
}

#[test]
fn contrast_is_wcag() {
    assert!((contrast(Rgba8::BLACK, Rgba8::WHITE) - 21.0).abs() < 1e-9);
    assert!((contrast(Rgba8::WHITE, Rgba8::WHITE) - 1.0).abs() < 1e-9);
}
