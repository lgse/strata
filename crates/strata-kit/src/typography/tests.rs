use super::*;
use crate::shape::LookParams;

fn at_size(text_size: u16) -> Look {
    Look::fallback(false).with_params(&LookParams {
        text_size,
        ..LookParams::default()
    })
}

fn styles(look: &Look) -> Vec<TextStyle> {
    let t = look.kit.ty;
    vec![
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
}

#[test]
fn the_desktop_scale_multiplies_once_and_rounds_to_a_whole_pixel() {
    assert_eq!(root_px(10, 1.1818), 12.0);
    assert_eq!(root_px(13, 1.25), 16.0);
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(root_px(13, bad), 13.0, "scale {bad}");
    }
}

#[test]
fn text_sizes_outside_the_range_are_clamped() {
    assert_eq!(root_px(4, 1.0), root_px(TEXT_SIZE.min, 1.0));
    assert_eq!(root_px(200, 1.0), root_px(TEXT_SIZE.max, 1.0));
    assert_eq!(step_text_size(TEXT_SIZE.max, 1), TEXT_SIZE.max);
    assert_eq!(step_text_size(TEXT_SIZE.min, -1), TEXT_SIZE.min);
    assert_eq!(step_text_size(13, 1), 14);
    assert_eq!(step_text_size(13, -1), 12);
}

#[test]
fn the_ramp_scales_together_and_keeps_its_order() {
    let base = styles(&at_size(13));
    for size in TEXT_SIZE.min..=TEXT_SIZE.max {
        let scaled = styles(&at_size(size));
        let f = factor(f32::from(size));
        for (a, b) in base.iter().zip(&scaled) {
            assert!(
                (b.size - a.size * f).abs() < 1e-4,
                "{size} px: {a:?} → {b:?}"
            );
            if size != 13 {
                assert_eq!(b.line_height.fract(), 0.0, "{size} px: whole-point line");
            }
            assert_eq!(
                (a.weight, a.tracking, a.family),
                (b.weight, b.tracking, b.family)
            );
        }
        for i in 0..base.len() {
            for j in 0..base.len() {
                if base[i].size < base[j].size {
                    assert!(scaled[i].size < scaled[j].size, "{size} px keeps the order");
                }
            }
        }
    }
}

#[test]
fn the_default_text_size_leaves_kits_ramp_untouched() {
    let neutral = Look::from_kit(rune_kit::Kit::neutral(false));
    let applied = neutral.clone().with_params(&LookParams::default());
    assert_eq!(styles(&applied), styles(&neutral));
    assert_eq!(applied.kit.ty.text_small.line_height, 19.5);
}

#[test]
fn icons_follow_the_text_size_and_thumbnails_do_not() {
    let (small, large) = (at_size(8), at_size(48));
    assert!(large.icon_size.md > small.icon_size.md);
    assert!(large.icon_size.sm < large.icon_size.md && large.icon_size.md < large.icon_size.lg);
    assert_eq!(large.tiles, small.tiles);
    assert_eq!(large.thumbnails, small.thumbnails);
}

#[test]
fn a_chosen_family_goes_before_the_apps_and_leaves_mono_alone() {
    let app = Families::new("Inter, sans-serif", "IBM Plex Mono");
    let chosen = families(Some("DejaVu Serif"), &app).expect("nameable");
    assert_eq!(chosen.ui, "\"DejaVu Serif\", Inter, sans-serif");
    assert_eq!(chosen.mono, app.mono);
    assert_eq!(families(None, &app), None);
}

/// settings/themes/interface-font: by default a chosen family leads the bundled font, the
/// previews' mono slot stays bundled, and Default leaves the app's (bundled) families.
#[test]
fn by_default_a_chosen_family_leads_the_bundled_font_and_mono_stays_bundled() {
    let with = |font: Option<&str>| {
        Look::fallback(false)
            .with_params(&LookParams {
                font: font.map(str::to_owned),
                ..LookParams::default()
            })
            .font
    };
    let chosen = with(Some("Fira Sans")).expect("nameable");
    assert_eq!(chosen.ui, "\"Fira Sans\", JetBrains Mono");
    assert_eq!(chosen.mono, "JetBrains Mono");
    assert_eq!(with(None), None);
}

#[test]
fn a_name_with_one_kind_of_quote_takes_the_other() {
    assert_eq!(
        quote_family("Other \"Quoted\" Sans").as_deref(),
        Some("'Other \"Quoted\" Sans'")
    );
    assert_eq!(
        quote_family("Bob's Sans").as_deref(),
        Some("\"Bob's Sans\"")
    );
    assert_eq!(
        quote_family("Back\\slash Sans").as_deref(),
        Some("\"Back\\slash Sans\"")
    );
}

#[test]
fn a_name_the_parser_cant_take_keeps_the_apps_families() {
    let app = Families::new("Inter", "Inter");
    for name in ["Both \"and' Sans", "", "  ", "Tab\tSans"] {
        assert_eq!(families(Some(name), &app), None, "{name:?}");
    }
}

#[test]
fn the_picker_sorts_ignoring_case_without_duplicates() {
    let installed = [
        "noto Sans",
        "DejaVu Sans",
        "Inter",
        "inter",
        " Cantarell ",
        "Both \"and' Sans",
        "Noto Sans",
    ]
    .map(String::from);
    let choices = font_choices(installed);
    let lower: Vec<String> = choices.iter().map(|c| c.to_lowercase()).collect();
    assert_eq!(lower, ["cantarell", "dejavu sans", "inter", "noto sans"]);
    assert!(choices.contains(&"Cantarell".to_owned()), "trimmed");
}

#[test]
fn the_search_keeps_names_containing_the_query_ignoring_case() {
    let choices = font_choices(["DejaVu Sans", "Noto Serif", "Inter"].map(String::from));
    assert_eq!(filter_fonts(&choices, "SANS"), ["DejaVu Sans"]);
    assert_eq!(filter_fonts(&choices, "er"), ["Inter", "Noto Serif"]);
    assert_eq!(filter_fonts(&choices, "").len(), 3);
    assert!(filter_fonts(&choices, "mono").is_empty());
    assert!(font_label_matches("Default", "FAULT"));
    assert!(!font_label_matches("Default", "sans"));
}
