// SPDX-License-Identifier: MIT

use super::{SourcePalette, ThemeTokens, blend, parse_rgb_channels, resolved_source_palette};
use crate::ui::preferences::OmarchyVariant;

pub(super) fn apply_variant(
    tokens: &mut ThemeTokens,
    source_background: &str,
    variant: OmarchyVariant,
    source_palette: Option<&SourcePalette>,
) {
    let high_contrast = variant == OmarchyVariant::HighContrast;
    let light = high_contrast && luminance(source_background) > 0.45;
    let background_pole = if light { "white" } else { "black" };
    let foreground_pole = if light { "black" } else { "white" };
    let background_limit = if light {
        0.90
    } else if high_contrast {
        0.012
    } else {
        0.025
    };

    // Use the terminal background, not ANSI color8: bright gray can otherwise
    // wash out every surface. Endpoints only adjust luminance, retaining hue.
    tokens.background = (0..=255)
        .map(|step| blend(source_background, background_pole, f64::from(step) / 255.0))
        .find(|color| {
            if light {
                luminance(color) >= background_limit
            } else {
                luminance(color) <= background_limit
            }
        })
        .expect("the luminance endpoint is reachable");
    tokens.surface = blend(
        &tokens.background,
        foreground_pole,
        if high_contrast { 0.055 } else { 0.035 },
    );
    tokens.muted = blend(&tokens.surface, foreground_pole, 0.04);
    tokens.highlight = blend(&tokens.surface, &tokens.highlight, 0.12);
    let backgrounds = [
        &tokens.background,
        &tokens.surface,
        &tokens.muted,
        &tokens.highlight,
    ];
    tokens.text = readable(
        &tokens.text,
        &backgrounds,
        foreground_pole,
        if high_contrast { 7.0 } else { 4.5 },
    );
    tokens.dim_text = readable(&tokens.dim_text, &backgrounds, foreground_pole, 4.5);
    tokens.accent = readable(&tokens.accent, &backgrounds, foreground_pole, 4.5);
    tokens.danger = readable(&tokens.danger, &backgrounds, foreground_pole, 4.5);
    tokens.border = readable(
        &tokens.border,
        &backgrounds,
        foreground_pole,
        if high_contrast { 3.0 } else { 1.5 },
    );

    let palette = resolved_source_palette(tokens, source_palette);
    let backgrounds = [&tokens.background, &tokens.surface];
    let syntax = |color: &str| Some(readable(color, &backgrounds, foreground_pole, 4.5));
    tokens.syntax_keyword = syntax(&palette.statement);
    tokens.syntax_string = syntax(&palette.string);
    tokens.syntax_constant = syntax(&palette.constant);
    tokens.syntax_type = syntax(&palette.type_color);
    tokens.syntax_preprocessor = syntax(&palette.preprocessor);
}

fn readable(color: &str, backgrounds: &[&String], pole: &str, ratio: f64) -> String {
    (0..=255)
        .map(|step| blend(color, pole, f64::from(step) / 255.0))
        .find(|color| {
            backgrounds
                .iter()
                .all(|background| contrast(color, background) >= ratio)
        })
        .expect("variant surfaces leave enough contrast at the foreground endpoint")
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

fn contrast(left: &str, right: &str) -> f64 {
    let left = luminance(left);
    let right = luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

#[cfg(test)]
mod tests;
