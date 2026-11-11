use std::collections::BTreeSet;

use super::*;
use crate::shape::LookParams;
use crate::theme::color::{Rgba8, blend};
use crate::theme::colors::Key;
use crate::theme::{bundled, bundled_theme};
use crate::tokens::{self, Kind, Value};

fn tokyo() -> Theme {
    bundled_theme("tokyo-night").expect("bundled").clone()
}

/// The theme-coloured core tokens: colours outside the component slots, which inherit.
fn colour_tokens(look: &Look) -> Vec<(&'static str, Value)> {
    tokens::all()
        .filter(|t| {
            t.kind() == Kind::Color
                && !crate::slots::TOKENS
                    .iter()
                    .any(|(id, _)| t.name().starts_with(&format!("{id}.")))
        })
        .map(|t| (t.name(), t.get(look)))
        .collect()
}

/// The colour tokens that change when `key` moves.
fn reach(key: Key) -> BTreeSet<&'static str> {
    let base = tokyo();
    let mut moved = base.clone();
    let was = base.colors.get(key).expect("set");
    let other = blend(
        was,
        if was.r > 128 {
            Rgba8::BLACK
        } else {
            Rgba8::WHITE
        },
        0.5,
    );
    moved.colors.set(key, Some(other));
    let (a, b) = (colour_tokens(&base.look()), colour_tokens(&moved.look()));
    a.iter()
        .zip(&b)
        .filter(|(x, y)| x.1 != y.1)
        .map(|(x, _)| x.0)
        .collect()
}

fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
    names.iter().copied().collect()
}

/// Each role a theme file sets lands on exactly the tokens `map.rs` documents. Background,
/// text and accent seed `Kit::from_palette`, whose derived tokens move too, so for those three
/// the documented tokens are a subset.
#[test]
fn each_role_reaches_the_tokens_the_map_lists() {
    let exact: [(Key, &[&str]); 11] = [
        (
            Key::Surface,
            &[
                "color.bg_deep",
                "color.layer_01",
                "color.layer_02",
                "color.layer_03",
                "color.layer_04",
                "color.surface_raised",
                "color.surface_raised_base",
                "color.surface_weak",
            ],
        ),
        (
            Key::Danger,
            &[
                "color.danger.bg",
                "color.danger.fg",
                "color.danger.border",
                "color.danger.icon",
                "syntax.deleted",
            ],
        ),
        (
            Key::Muted,
            &[
                "color.bg_button_neutral",
                "color.surface_base",
                "color.surface_disabled",
                "fill.muted",
            ],
        ),
        (Key::Highlight, &["fill.highlight"]),
        (
            Key::Border,
            &[
                "color.border_muted",
                "color.border_base",
                "color.border_strong",
                "color.border_weak",
                "color.border_disabled",
                "color.std_border",
            ],
        ),
        (
            Key::DimText,
            &[
                "color.text_muted",
                "color.text_weak",
                "color.text_faint",
                "color.icon_muted",
                "color.icon_faint",
                "color.icon_weaker",
                "color.std_icon",
                "syntax.comment",
            ],
        ),
        (Key::SyntaxKeyword, &["syntax.keyword", "syntax.operator"]),
        (
            Key::SyntaxString,
            &["syntax.string", "syntax.link", "syntax.regex"],
        ),
        (Key::SyntaxConstant, &["syntax.number"]),
        (Key::SyntaxType, &["syntax.type", "syntax.attribute"]),
        (Key::SyntaxPreprocessor, &["syntax.preprocessor"]),
    ];
    for (key, names) in exact {
        assert_eq!(reach(key), set(names), "{}", key.name());
    }

    let at_least: [(Key, &[&str]); 3] = [
        (
            Key::Background,
            &[
                "color.page",
                "color.bg_base",
                "color.text_inverse",
                "color.text_contrast",
                "color.text_invert_strong",
                "color.icon_inverse",
                "color.icon_contrast",
                "color.std_icon_invert",
                "color.submit_fg",
                "color.submit_icon",
            ],
        ),
        (
            Key::Text,
            &[
                "color.text_base",
                "color.text_strong",
                "color.std_text",
                "color.icon_base",
                "color.std_icon_strong",
                "syntax.function",
                "syntax.property",
                "syntax.parameter",
                "syntax.punctuation",
                "syntax.variable",
            ],
        ),
        (
            Key::Accent,
            &[
                "color.bg_accent",
                "color.text_accent",
                "color.icon_accent",
                "color.border_active",
                "color.border_focus",
                "color.border_hover",
                "color.hover",
                "color.surface_hover",
                "color.surface_raised_hover",
                "color.pressed",
                "color.info.bg",
                "color.info.fg",
                "color.info.border",
                "color.info.icon",
                "glow.color",
                "syntax.heading",
            ],
        ),
    ];
    for (key, names) in at_least {
        let moved = reach(key);
        let missing: Vec<_> = set(names).difference(&moved).copied().collect();
        assert!(missing.is_empty(), "{} misses {missing:?}", key.name());
    }
    // The accent doesn't reach the dim tone, and the text doesn't reach the fills.
    assert!(!reach(Key::Accent).contains("color.text_muted"));
    assert!(!reach(Key::Text).contains("fill.highlight"));
}

/// A theme sets colours only: with the same parameters, every length, number and flag
/// matches the fallback's, so K2's tokens survive a theme change.
#[test]
fn a_theme_changes_only_colours() {
    let params = LookParams {
        text_size: 16,
        frame_density: 4,
        element_glow: false,
        ..LookParams::default()
    };
    for theme in bundled().iter().step_by(7) {
        let themed = theme.look().with_params(&params);
        let plain = Look::fallback(themed.kit.dark).with_params(&params);
        for t in tokens::all().filter(|t| t.kind() != Kind::Color) {
            assert_eq!(
                t.get(&themed),
                t.get(&plain),
                "{} on {}",
                t.name(),
                theme.id
            );
        }
    }
}

/// The WP's test: "Every bundled theme parses and maps, with no missing token." Every
/// colour token reads a real colour, and the appearance agrees with Strata's own dark rule.
#[test]
fn every_bundled_theme_maps_in_its_own_appearance() {
    for theme in bundled() {
        let look = theme.look();
        assert_eq!(look.kit.dark, theme.dark_background(), "{}", theme.id);
        for (name, v) in colour_tokens(&look) {
            let Value::Color(c) = v else { unreachable!() };
            assert!(
                [c.r, c.g, c.b, c.a].iter().all(|x| x.is_finite()),
                "{name} on {}",
                theme.id
            );
        }
        assert_eq!(look.kit.c.bg_base, theme.colors.background.color());
        assert_eq!(look.fill.highlight, theme.colors.highlight.color());
    }
}

/// A translucent theme colour keeps its opacity in the tokens drawn from it: the stylesheet's
/// `alpha(@strata_border, 0.6)` is 60 % of the border's own alpha, as GTK multiplies.
#[test]
fn a_translucent_theme_colour_scales_its_own_opacity() {
    let src = r##"name = "Glass"
background = "#0c1a2b"
surface = "#122438"
text = "#c9deed"
accent = "#4fd6ff"
muted = "#1e3a52"
highlight = "#244d68"
border = "rgba(49, 91, 117, 0.5)"
dim_text = "#6f8da3"
"##;
    let theme = crate::theme::parse_custom("glass", src).expect("loads");
    let look = theme.look();
    let border = theme.colors.border.color();
    assert!((border.a - 0.5).abs() < 0.01);
    let kit = &look.kit;
    assert_eq!(kit.c.border_base, kit.alpha(border, border.a * 0.60));
    assert_ne!(kit.c.border_base, kit.alpha(border, 0.60));
    assert_eq!(kit.c.std_border, border, "the border itself is as written");
}
