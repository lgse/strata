// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::ui::theme::{
    Theme, ThemeTokens, azure_tokens, blend, builtins, canonical_color, color_to_hex,
    custom_theme_id, merge_builtin_and_custom_themes, omarchy::contrast, slugify, title_case_slug,
    validate_tokens,
};

#[test]
fn bundled_catalog_is_valid_unique_and_alphabetical() {
    let themes = builtins();

    let mut ids = HashSet::new();
    let mut previous_name = String::new();
    for theme in &themes {
        assert!(
            ids.insert(theme.id.as_str()),
            "bundled theme IDs must be unique"
        );
        assert!(
            validate_tokens(&theme.tokens).is_ok(),
            "{} must contain valid theme tokens",
            theme.tokens.name
        );
        let name = theme.tokens.name.to_lowercase();
        assert!(previous_name <= name, "bundled themes must be alphabetical");
        previous_name = name;
    }

    for removed in [
        "apprentice",
        "brogrammer",
        "codeschool",
        "everforest-dark-medium",
        "everforest-light-soft",
        "gruvbox-dark-medium",
        "gruvbox-dark-soft",
        "gruvbox-light-medium",
        "gruvbox-light-soft",
        "jellybeans",
        "shades-of-purple",
        "xcode-dusk",
    ] {
        assert!(!ids.contains(removed), "{removed} should not be bundled");
    }
}

type Color = fn(&ThemeTokens) -> &str;

/// Text roles, the backgrounds they sit on, and the contrast floor each pair must clear.
const READABILITY_CHECKS: [(&str, Color, &str, Color, f64); 4] = [
    (
        "text",
        |tokens| &tokens.text,
        "background",
        |tokens| &tokens.background,
        4.5,
    ),
    (
        "text",
        |tokens| &tokens.text,
        "surface",
        |tokens| &tokens.surface,
        3.0,
    ),
    (
        "dim text",
        |tokens| &tokens.dim_text,
        "background",
        |tokens| &tokens.background,
        4.5,
    ),
    (
        "dim text",
        |tokens| &tokens.dim_text,
        "surface",
        |tokens| &tokens.surface,
        3.0,
    ),
];

#[test]
fn bundled_themes_keep_text_readable() {
    let themes = builtins();
    assert!(!themes.is_empty());
    let failures = themes
        .iter()
        .flat_map(readability_failures)
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn readability_failures(theme: &Theme) -> Vec<String> {
    READABILITY_CHECKS
        .iter()
        .filter_map(|(name, color, role, background, floor)| {
            let ratio = contrast(color(&theme.tokens), background(&theme.tokens));
            (ratio < *floor).then(|| {
                format!(
                    "{} {name} on {role} is {ratio:.2}:1, below the {floor}:1 floor",
                    theme.id
                )
            })
        })
        .collect()
}

#[test]
fn custom_themes_replace_bundled_themes_with_the_same_id() {
    let builtin = Theme {
        id: "dracula".to_owned(),
        tokens: azure_tokens(),
        custom: false,
    };
    let mut custom = builtin.clone();
    custom.tokens.name = "My Dracula".to_owned();
    custom.custom = true;

    let themes = merge_builtin_and_custom_themes(vec![builtin], vec![custom]);

    assert_eq!(themes.len(), 1);
    assert!(themes[0].custom);
    assert_eq!(themes[0].tokens.name, "My Dracula");
}

#[test]
fn names_become_safe_config_file_slugs() {
    assert_eq!(slugify("  Rosé / Pine!  "), "ros-pine");
    assert_eq!(slugify("Ocean  Blue"), "ocean-blue");
}

#[test]
fn custom_theme_ids_fall_back_and_skip_taken_ids() {
    let cases: [(&str, &[&str], &str); 5] = [
        ("Ocean Blue", &["ocean-blue"], "ocean-blue-2"),
        ("日本語", &[], "custom-theme"),
        ("!!!", &[], "custom-theme"),
        ("日本語", &["custom-theme"], "custom-theme-2"),
        (
            "日本語",
            &["custom-theme", "custom-theme-2"],
            "custom-theme-3",
        ),
    ];
    for (name, taken, expected) in cases {
        assert_eq!(
            custom_theme_id(name, |id| taken.contains(&id)),
            expected,
            "{name:?}"
        );
    }
}

#[test]
fn omarchy_slugs_become_display_names() {
    assert_eq!(title_case_slug("tokyo-night"), "Tokyo Night");
}

#[test]
fn colors_can_be_blended_into_semantic_tokens() {
    for (start, end) in [
        ("#000000", "#ffffff"),
        ("rgb(0,0,0)", "rgb(255,255,255)"),
        ("#000", "#fff"),
        ("black", "white"),
    ] {
        assert_eq!(blend(start, end, 0.5), "#808080", "{start} to {end}");
    }
}

#[test]
fn gtk_color_formats_canonicalize_for_persistence_and_scheme_xml() {
    for (value, css, opaque) in [
        ("rgb(153,193,241)", Some("#99c1f1"), "#99c1f1"),
        ("#fff", Some("#ffffff"), "#ffffff"),
        ("#81A1C1", Some("#81a1c1"), "#81a1c1"),
        ("rebeccapurple", Some("#663399"), "#663399"),
        ("#000aaafff", Some("#00aaff"), "#00aaff"),
        ("#0000aaaaffff", Some("#00aaff"), "#00aaff"),
        ("#0000aaaaffff8888", Some("#00aaff88"), "#00aaff"),
        ("rgba(0, 170, 255, 0.5)", Some("#00aaff80"), "#00aaff"),
        ("0x7aa2f7", None, "0x7aa2f7"),
    ] {
        assert_eq!(canonical_color(value).as_deref(), css, "{value}");
        assert_eq!(color_to_hex(value), opaque, "{value}");
    }
}
