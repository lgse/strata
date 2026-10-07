// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::theme::{builtins, tokens_from_quattro, validate_tokens};

#[test]
fn variants_keep_semantic_pairs_readable_for_dark_light_and_limited_palettes() {
    let mut palettes: Vec<_> = builtins()
        .into_iter()
        .map(|theme| {
            let tokens = theme.tokens;
            (
                tokens.name,
                tokens.background,
                tokens.text,
                tokens.accent,
                tokens.highlight,
                tokens.surface,
            )
        })
        .collect();
    for (name, background, foreground, accent, selection, shadow) in [
        (
            "washed-out",
            "#191123",
            "#7199cb",
            "#917baa",
            "#a8b1ff",
            "#b18a9b",
        ),
        (
            "light", "#fff9ec", "#372c2d", "#873f94", "#87abcf", "#929292",
        ),
        (
            "monochrome",
            "#777777",
            "#777777",
            "#777777",
            "#777777",
            "#777777",
        ),
        ("black", "black", "black", "black", "black", "black"),
        ("white", "white", "white", "white", "white", "white"),
    ] {
        palettes.push((
            name.into(),
            background.into(),
            foreground.into(),
            accent.into(),
            selection.into(),
            shadow.into(),
        ));
    }
    for (name, background, foreground, accent, selection, shadow) in palettes {
        let source = format!(
            "background = '{background}'\nforeground = '{foreground}'\naccent = '{accent}'\nselection = '{selection}'\ncolor8 = '{shadow}'\ngreen = '{accent}'\n"
        );
        for variant in [OmarchyVariant::Darker, OmarchyVariant::HighContrast] {
            let tokens = tokens_from_quattro(&name, &source, variant).expect("valid palette");
            assert!(validate_tokens(&tokens).is_ok(), "{name} {variant:?}");
            let text_ratio = if variant == OmarchyVariant::HighContrast {
                7.0
            } else {
                4.5
            };
            for surface in [
                &tokens.background,
                &tokens.surface,
                &tokens.muted,
                &tokens.highlight,
            ] {
                for (foreground, minimum) in [
                    (&tokens.text, text_ratio),
                    (&tokens.dim_text, 4.5),
                    (&tokens.accent, 4.5),
                    (&tokens.danger, 4.5),
                    (
                        &tokens.border,
                        if variant == OmarchyVariant::HighContrast {
                            3.0
                        } else {
                            1.5
                        },
                    ),
                ] {
                    assert!(
                        contrast(foreground, surface) >= minimum,
                        "{name} {variant:?}: {foreground} on {surface}"
                    );
                }
            }
            for syntax in [
                &tokens.syntax_keyword,
                &tokens.syntax_string,
                &tokens.syntax_constant,
                &tokens.syntax_type,
                &tokens.syntax_preprocessor,
            ] {
                assert!(
                    contrast(syntax.as_ref().expect("resolved syntax"), &tokens.surface) >= 4.5
                );
            }
            if variant == OmarchyVariant::Darker {
                assert!(luminance(&tokens.background) <= luminance(&background));
                assert!(luminance(&tokens.background) < 0.03);
            } else {
                assert_eq!(
                    luminance(&tokens.background) > 0.45,
                    luminance(&background) > 0.45
                );
            }
        }
    }
}

#[test]
fn darker_uses_terminal_background_without_discarding_a_readable_accent() {
    let source =
        "background = '#101020'\nforeground = '#eeeeee'\naccent = '#99aaff'\ncolor8 = '#999999'\n";
    let original =
        tokens_from_quattro("native", source, OmarchyVariant::Original).expect("original palette");
    let darker =
        tokens_from_quattro("native", source, OmarchyVariant::Darker).expect("darker palette");
    assert!(luminance(&darker.surface) < luminance(&original.background));
    assert_eq!(darker.background, "#101020");
    assert_eq!(darker.accent, original.accent);
}

#[test]
fn incomplete_and_invalid_optional_colors_use_readable_fallbacks() {
    for optional in [
        "",
        "selection = 'invalid'\ncolor8 = 'invalid'\ngreen = 'invalid'\n",
    ] {
        let source = format!(
            "background = '#554433'\nforeground = '#554433'\naccent = '#554433'\n{optional}"
        );
        let tokens = tokens_from_quattro("limited", &source, OmarchyVariant::HighContrast)
            .expect("limited palette");
        assert!(validate_tokens(&tokens).is_ok());
        assert!(contrast(&tokens.text, &tokens.surface) >= 7.0);
    }
    assert!(
        tokens_from_quattro(
            "invalid",
            "background = 'invalid'\nforeground = 'white'\naccent = 'red'",
            OmarchyVariant::Darker
        )
        .is_none()
    );
}
