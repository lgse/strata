// SPDX-License-Identifier: MIT

use super::{SourcePalette, ThemeTokens, blend, parse_rgb_channels, resolved_source_palette};
use crate::ui::preferences::OmarchyVariant;

/// Quattro's `mode` decides when present; otherwise the background's brightness does.
pub(super) fn is_light(background: &str, mode: Option<&str>) -> bool {
    match mode {
        Some("light") => true,
        Some("dark") => false,
        _ => luminance(background) > 0.45,
    }
}

/// Lighter is the plain mapping. Darker moves the file columns onto the pane color,
/// and Contrast raises foregrounds on that darker layout.
pub(super) fn apply_variant(
    tokens: &mut ThemeTokens,
    light: bool,
    variant: OmarchyVariant,
    source_palette: Option<&SourcePalette>,
) {
    if variant == OmarchyVariant::Lighter {
        return;
    }
    tokens.background = tokens.surface.clone();
    if variant == OmarchyVariant::Contrast {
        apply_contrast(tokens, light, source_palette);
    }
}

/// Keeps the theme's surfaces and raises foregrounds to their contrast targets.
fn apply_contrast(tokens: &mut ThemeTokens, light: bool, source_palette: Option<&SourcePalette>) {
    let (background_pole, foreground_pole) = if light {
        ("white", "black")
    } else {
        ("black", "white")
    };
    // A mid-tone surface is darkened or lightened only when even the foreground
    // pole could not reach the text target on it.
    for surface in [
        &mut tokens.background,
        &mut tokens.surface,
        &mut tokens.muted,
        &mut tokens.highlight,
    ] {
        *surface = toward(surface, background_pole, |color| {
            contrast(color, foreground_pole) >= 7.0
        });
    }
    let backgrounds = [
        &tokens.background,
        &tokens.surface,
        &tokens.muted,
        &tokens.highlight,
    ];
    tokens.text = readable(&tokens.text, &backgrounds, foreground_pole, 7.0);
    tokens.dim_text = readable(&tokens.dim_text, &backgrounds, foreground_pole, 4.5);
    tokens.accent = readable(&tokens.accent, &backgrounds, foreground_pole, 4.5);
    tokens.danger = readable(&tokens.danger, &backgrounds, foreground_pole, 4.5);
    tokens.border = readable(&tokens.border, &backgrounds, foreground_pole, 3.0);

    let palette = resolved_source_palette(tokens, source_palette);
    let backgrounds = [&tokens.background, &tokens.surface];
    let syntax = |color: &str| Some(readable(color, &backgrounds, foreground_pole, 4.5));
    tokens.syntax_keyword = syntax(&palette.statement);
    tokens.syntax_string = syntax(&palette.string);
    tokens.syntax_constant = syntax(&palette.constant);
    tokens.syntax_type = syntax(&palette.type_color);
    tokens.syntax_preprocessor = syntax(&palette.preprocessor);
}

/// Moves `color` toward `pole` until `done` holds; only luminance changes, so hue is kept.
fn toward(color: &str, pole: &str, done: impl Fn(&str) -> bool) -> String {
    (0..=255)
        .map(|step| blend(color, pole, f64::from(step) / 255.0))
        .find(|color| done(color))
        .unwrap_or_else(|| pole.to_owned())
}

pub(super) fn readable(color: &str, backgrounds: &[&String], pole: &str, ratio: f64) -> String {
    (0..=255)
        .map(|step| blend(color, pole, f64::from(step) / 255.0))
        .find(|color| {
            backgrounds
                .iter()
                .all(|background| contrast(color, background) >= ratio)
        })
        .expect("the foreground endpoint reaches the contrast floor")
}

fn luminance(color: &str) -> f64 {
    let channels = parse_rgb_channels(color)
        .expect("validated theme color")
        .map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
    channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
}

pub(super) fn contrast(left: &str, right: &str) -> f64 {
    let left = luminance(left);
    let right = luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

#[cfg(test)]
mod tests;
