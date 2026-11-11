use super::*;

const QUATTRO: &str = r##"
background = "#0a0f1a"
dark_background = "#070b13"
lighter_background = "#142033"
foreground = "#a8dfff"
accent = "#3ee0e8"
selection = "#14404a"
muted = "#46676d"
color1 = "#f2552c"
magenta = "#e07a95"
green = "#5fd4a8"
orange = "#f5913a"
cyan = "#3ee0e8"
yellow = "#e9c27f"
blue = "#3fa6d4"
color8 = "#ff00ff"
"##;

fn hex(s: &str) -> Rgba8 {
    Rgba8::parse(s).expect("parses")
}

fn quattro(lines: &[(&str, Option<&str>)]) -> String {
    let mut out: Vec<String> = QUATTRO
        .lines()
        .filter(|l| !lines.iter().any(|(k, _)| l.starts_with(&format!("{k} ="))))
        .map(str::to_owned)
        .collect();
    for (k, v) in lines {
        if let Some(v) = v {
            out.push(format!("{k} = \"{v}\""));
        }
    }
    out.join("\n")
}

fn map(src: &str, variant: Variant) -> Colors {
    theme("azure-glow", src, variant).expect("maps").colors
}

/// settings/themes/omarchy: "Panes and headers use Quattro's `dark_background`", "Hover
/// surfaces use `lighter_background`", "Selection uses `selection`", "Borders use `muted`",
/// and "Lighter puts file columns on `background` and the sidebar, headers, and preview on
/// `dark_background`".
#[test]
fn lighter_maps_quattro_roles_as_written() {
    let t = theme(" tokyo-night\n", QUATTRO, Variant::Lighter).expect("maps");
    assert_eq!(t.name, "Tokyo Night");
    assert_eq!(t.id, "tokyo-night");
    assert_eq!(t.source, Source::Omarchy);
    let c = t.colors;
    assert_eq!(c.background, hex("#0a0f1a"));
    assert_eq!(c.surface, hex("#070b13"));
    assert_eq!(c.muted, hex("#142033"));
    assert_eq!(c.highlight, hex("#14404a"));
    assert_eq!(c.border, hex("#46676d"));
    assert_eq!(c.text, hex("#a8dfff"));
    assert_eq!(c.danger, hex("#f2552c"));
    assert_eq!(c.dim_text, blend(c.background, c.text, 0.62));
}

/// settings/themes/omarchy: the fallback bullets for `dark_background`, `lighter_background`,
/// `selection` and `muted`, and "An invalid optional color such as `selection` or `muted` is
/// ignored and its fallback used."
#[test]
fn missing_or_invalid_optional_colours_fall_back_to_blends() {
    let bg = hex("#0a0f1a");
    let (fg, accent) = (hex("#a8dfff"), hex("#3ee0e8"));
    for selection in [None, Some("invalid")] {
        let src = quattro(&[
            ("dark_background", None),
            ("lighter_background", None),
            ("selection", selection),
            ("muted", Some("nope")),
            ("color1", None),
        ]);
        let c = map(&src, Variant::Lighter);
        assert_eq!(c.surface, bg);
        assert_eq!(c.muted, blend(bg, fg, 0.10));
        assert_eq!(c.highlight, blend(bg, accent, 0.25));
        assert_eq!(c.border, blend(bg, fg, 0.36));
        assert_eq!(c.danger, DEFAULT_DANGER);
    }
    // Some themes set lighter_background equal to background, which would hide hover.
    let c = map(
        &quattro(&[("lighter_background", Some("#0a0f1a"))]),
        Variant::Lighter,
    );
    assert_eq!(c.muted, blend(bg, fg, 0.10));
}

/// settings/themes/omarchy: "ANSI `color8` is never read: changing it in `colors.toml` changes
/// no color."
#[test]
fn color8_is_never_read() {
    for variant in [Variant::Lighter, Variant::Darker, Variant::Contrast] {
        let a = map(QUATTRO, variant);
        let b = map(&quattro(&[("color8", Some("#00ff00"))]), variant);
        assert_eq!(a, b);
    }
}

/// settings/themes/omarchy: "Follow Omarchy is shown only when ... `colors.toml` exist[s] with
/// valid `background`, `foreground`, and `accent` colors."
#[test]
fn without_a_valid_background_foreground_and_accent_there_is_no_theme() {
    for key in ["background", "foreground", "accent"] {
        assert!(
            theme("x", &quattro(&[(key, None)]), Variant::Darker).is_none(),
            "{key}"
        );
        assert!(
            theme("x", &quattro(&[(key, Some("bad"))]), Variant::Darker).is_none(),
            "{key}"
        );
    }
    assert!(theme("x", "background = ", Variant::Darker).is_none());
}

/// settings/themes/omarchy: "A Quattro color in a GTK-only form, such as `accent =
/// "#7a7a2af7f"`, loads as `#rrggbb`".
#[test]
fn gtk_only_forms_load_canonical() {
    let c = map(
        &quattro(&[("accent", Some("#7a7a2af7f"))]),
        Variant::Lighter,
    );
    assert_eq!(c.accent.hex(), "#7aa2f7");
}

/// settings/themes/omarchy: "Code previews take keyword, string, constant, type, and
/// preprocessor colors from Quattro's `magenta`, `green`, `orange`, `cyan`, and `yellow`",
/// "Without the named colors, `color5`, `color2`, `color9`, and `color3` supply" them, and
/// "An unparsable Quattro syntax color, such as `green = "invalid"`, counts as missing and its
/// fallback applies".
#[test]
fn syntax_colours_come_from_named_then_numbered_terminal_colours() {
    let s = map(QUATTRO, Variant::Lighter).syntax;
    assert_eq!(
        [s.keyword, s.string, s.constant, s.type_, s.preprocessor],
        ["#e07a95", "#5fd4a8", "#f5913a", "#3ee0e8", "#e9c27f"].map(|h| Some(hex(h)))
    );

    let legacy = quattro(&[
        ("magenta", None),
        ("green", Some("invalid")),
        ("orange", None),
        ("cyan", None),
        ("color5", Some("#111111")),
        ("color2", Some("#222222")),
        ("color9", Some("#333333")),
        ("color3", Some("#444444")),
    ]);
    let s = map(&legacy, Variant::Lighter).syntax;
    assert_eq!(
        [s.keyword, s.string, s.constant, s.type_],
        ["#111111", "#222222", "#333333", "#444444"].map(|h| Some(hex(h)))
    );
    // Without the terminal palette (no green), yellow still sets preprocessor; nothing else
    // fills in, so the look blends the rest from the accent.
    let bare = quattro(&[
        ("magenta", None),
        ("green", None),
        ("orange", None),
        ("cyan", None),
    ]);
    let s = map(&bare, Variant::Lighter).syntax;
    assert_eq!((s.keyword, s.preprocessor), (None, Some(hex("#e9c27f"))));
}

/// settings/themes/omarchy: "Darker also puts file columns on `dark_background`", and "Lighter
/// and Darker keep the theme's colors as authored, so a light palette stays light under
/// Darker."
#[test]
fn darker_moves_the_columns_onto_the_pane_colour_and_keeps_the_rest() {
    let (lighter, darker) = (
        map(QUATTRO, Variant::Lighter),
        map(QUATTRO, Variant::Darker),
    );
    assert_eq!(darker.background, lighter.surface);
    assert_eq!(
        Colors {
            background: lighter.surface,
            ..lighter
        },
        darker
    );

    let light = r##"background = "#f4f1ea"
dark_background = "#e8e3d8"
foreground = "#3a3530"
accent = "#2f6fb0"
"##;
    let t = theme("paper", light, Variant::Darker).expect("maps");
    assert!(!t.look().kit.dark, "a light palette stays light");
}

fn surfaces(c: &Colors) -> [Rgba8; 4] {
    [c.background, c.surface, c.muted, c.highlight]
}

/// settings/themes/omarchy: "Contrast uses the Darker layout and raises text to 7:1",
/// "Contrast raises dim text, accent, and danger to 4.5:1, and borders to 3:1, against those
/// four surfaces", and "Contrast raises code-preview syntax colors to 4.5:1 against the window
/// and pane surfaces."
#[test]
fn contrast_raises_every_foreground_to_its_target() {
    let darker = map(QUATTRO, Variant::Darker);
    let c = map(QUATTRO, Variant::Contrast);
    assert_eq!(
        surfaces(&c),
        surfaces(&darker),
        "the dark palette's surfaces are kept"
    );
    for s in surfaces(&c) {
        assert!(contrast(c.text, s) >= 7.0);
        for fg in [c.dim_text, c.accent, c.danger] {
            assert!(contrast(fg, s) >= 4.5);
        }
        assert!(contrast(c.border, s) >= 3.0);
    }
    let syn = c.syntax;
    for fg in [
        syn.keyword,
        syn.string,
        syn.constant,
        syn.type_,
        syn.preprocessor,
    ] {
        let fg = fg.expect("set");
        assert!(contrast(fg, c.background) >= 4.5 && contrast(fg, c.surface) >= 4.5);
    }
}

/// settings/themes/omarchy: "Contrast keeps the theme's surfaces and selection unless pure
/// white, or black on a light palette, falls below 7:1 on one. That surface then moves toward
/// black, or white on a light palette, until it reaches 7:1", and "Contrast keeps a palette
/// light with `mode = "light"` ... and dark with `mode = "dark"`."
#[test]
fn contrast_moves_only_a_mid_tone_surface_and_follows_mode() {
    let mid = r##"background = "#8a8a8a"
dark_background = "#101010"
foreground = "#dddddd"
accent = "#3ee0e8"
mode = "dark"
"##;
    let c = map(mid, Variant::Lighter);
    let raised = map(mid, Variant::Contrast);
    // Darker moved the columns onto #101010, where white already reaches 7:1; the mid-tone
    // hover blend from #8a8a8a moves toward black until it does.
    assert_eq!(raised.surface, c.surface);
    assert_ne!(raised.muted, c.muted);
    assert!(contrast(raised.muted, Rgba8::WHITE) >= 7.0);
    assert!(raised.muted.r < c.muted.r);

    let as_light = mid.replace("mode = \"dark\"", "mode = \"light\"");
    let light = map(&as_light, Variant::Contrast);
    assert!(contrast(light.text, light.background) >= 7.0);
    assert!(
        luminance(light.text) < luminance(light.background),
        "dark text on light"
    );
}

/// settings/themes/omarchy: "Without `mode`, or with any value other than `light` or `dark`,
/// Contrast treats a Quattro `background` with relative luminance above 0.45 as light."
#[test]
fn without_mode_the_background_decides() {
    assert!(is_light(hex("#c0c0c0"), None));
    assert!(is_light(hex("#c0c0c0"), Some("sepia")));
    assert!(!is_light(hex("#a0a0a0"), None));
    assert!(!is_light(hex("#ffffff"), Some("dark")));
    assert!(is_light(hex("#000000"), Some("light")));
}

/// Strata's extreme-palette check (`src/ui/theme/omarchy/tests.rs` at lgse/strata 35ee677ab):
/// Contrast over every bundled theme read as a Quattro palette, and over washed-out, light,
/// monochrome, all-black and all-white palettes. settings/themes/omarchy: "Contrast raises
/// text to 7:1", "dim text, accent, and danger to 4.5:1, and borders to 3:1, against those four
/// surfaces", "code-preview syntax colors to 4.5:1 against the window and pane surfaces", and a
/// palette keeps its appearance. Strata's `validate_tokens` check becomes a check that every
/// colour token in the finished look is finite.
#[test]
fn contrast_keeps_every_pair_readable_on_dark_light_and_extreme_palettes() {
    let mut palettes: Vec<(String, [Rgba8; 5])> = crate::theme::bundled()
        .iter()
        .map(|t| {
            let c = &t.colors;
            (
                t.id.clone(),
                [c.background, c.text, c.accent, c.highlight, c.surface],
            )
        })
        .collect();
    for (name, colours) in [
        (
            "washed-out",
            ["#191123", "#7199cb", "#917baa", "#a8b1ff", "#b18a9b"],
        ),
        (
            "light",
            ["#fff9ec", "#372c2d", "#873f94", "#87abcf", "#f2ecdf"],
        ),
        ("monochrome", ["#777777"; 5]),
        ("black", ["black"; 5]),
        ("white", ["white"; 5]),
    ] {
        palettes.push((name.to_owned(), colours.map(hex)));
    }
    for (name, [background, foreground, accent, selection, dark_background]) in palettes {
        let src = format!(
            "background = '{}'\nforeground = '{}'\naccent = '{}'\nselection = '{}'\n\
             dark_background = '{}'\ngreen = '{}'\n",
            background.hex(),
            foreground.hex(),
            accent.hex(),
            selection.hex(),
            dark_background.hex(),
            accent.hex(),
        );
        let c = map(&src, Variant::Contrast);
        let look = theme(&name, &src, Variant::Contrast).expect("maps").look();
        for t in crate::tokens::all() {
            if let crate::tokens::Value::Color(v) = t.get(&look) {
                assert!(
                    [v.r, v.g, v.b, v.a].iter().all(|x| x.is_finite()),
                    "{name}: {}",
                    t.name()
                );
            }
        }
        for s in surfaces(&c) {
            for (fg, floor) in [
                (c.text, 7.0),
                (c.dim_text, 4.5),
                (c.accent, 4.5),
                (c.danger, 4.5),
                (c.border, 3.0),
            ] {
                assert!(
                    contrast(fg, s) >= floor,
                    "{name}: {} on {}",
                    fg.hex(),
                    s.hex()
                );
            }
        }
        let syn = c.syntax;
        for fg in [
            syn.keyword,
            syn.string,
            syn.constant,
            syn.type_,
            syn.preprocessor,
        ] {
            let fg = fg.expect("resolved");
            assert!(
                contrast(fg, c.surface) >= 4.5,
                "{name}: syntax {}",
                fg.hex()
            );
        }
        assert_eq!(
            luminance(c.background) > 0.45,
            luminance(background) > 0.45,
            "{name} keeps its appearance"
        );
    }
}
