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
    for (name, background, foreground, accent, selection, dark_background) in [
        (
            "washed-out",
            "#191123",
            "#7199cb",
            "#917baa",
            "#a8b1ff",
            "#b18a9b",
        ),
        (
            "light", "#fff9ec", "#372c2d", "#873f94", "#87abcf", "#f2ecdf",
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
            dark_background.into(),
        ));
    }
    for (name, background, foreground, accent, selection, dark_background) in palettes {
        let source = format!(
            "background = '{background}'\nforeground = '{foreground}'\naccent = '{accent}'\nselection = '{selection}'\ndark_background = '{dark_background}'\ngreen = '{accent}'\n"
        );
        let tokens =
            tokens_from_quattro(&name, &source, OmarchyVariant::Contrast).expect("valid palette");
        assert!(validate_tokens(&tokens).is_ok(), "{name}");
        for surface in [
            &tokens.background,
            &tokens.surface,
            &tokens.muted,
            &tokens.highlight,
        ] {
            for (foreground, minimum) in [
                (&tokens.text, 7.0),
                (&tokens.dim_text, 4.5),
                (&tokens.accent, 4.5),
                (&tokens.danger, 4.5),
                (&tokens.border, 3.0),
            ] {
                assert!(
                    contrast(foreground, surface) >= minimum,
                    "{name}: {foreground} on {surface}"
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
            assert!(contrast(syntax.as_ref().expect("resolved syntax"), &tokens.surface) >= 4.5);
        }
        assert_eq!(
            luminance(&tokens.background) > 0.45,
            luminance(&background) > 0.45
        );
    }
}

#[test]
fn variants_place_the_theme_own_surfaces() {
    let source = "mode = 'dark'\nbackground = '#1a1b26'\ndark_background = '#13141c'\nlighter_background = '#24283b'\nforeground = '#a9b1d6'\naccent = '#7aa2f7'\nselection = '#292e42'\nmuted = '#414868'\n";
    let variant = |variant| tokens_from_quattro("tokyo-night", source, variant).expect("palette");
    let layout = |tokens: &ThemeTokens| {
        [
            tokens.background.clone(),
            tokens.surface.clone(),
            tokens.muted.clone(),
            tokens.highlight.clone(),
        ]
    };
    let lighter = variant(OmarchyVariant::Lighter);
    assert_eq!(
        layout(&lighter),
        ["#1a1b26", "#13141c", "#24283b", "#292e42"]
    );
    let darker = variant(OmarchyVariant::Darker);
    assert_eq!(
        layout(&darker),
        ["#13141c", "#13141c", "#24283b", "#292e42"]
    );
    assert_eq!(
        (&darker.text, &darker.accent, &darker.border),
        (&lighter.text, &lighter.accent, &lighter.border)
    );
    let contrast_tokens = variant(OmarchyVariant::Contrast);
    assert_eq!(layout(&contrast_tokens), layout(&darker));
    assert!(contrast(&contrast_tokens.text, &contrast_tokens.highlight) >= 7.0);
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
        let tokens = tokens_from_quattro("limited", &source, OmarchyVariant::Contrast)
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
