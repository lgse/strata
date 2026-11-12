// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gdk, pango};
use vte4::prelude::*;

use crate::ui::{
    preferences::PreferenceManager,
    theme::{ThemeManager, ThemeTokens},
};

pub(super) fn bind(terminal: &vte4::Terminal, preferences: &Rc<PreferenceManager>) {
    let preferences = preferences.clone();
    ThemeManager::shared().bind_theme_preference(
        terminal,
        move |manager| {
            (
                manager.appearance_tokens(),
                preferences.interface_font(),
                preferences.interface_scale(),
            )
        },
        |widget, (tokens, family, scale)| {
            let Some(terminal) = widget.downcast_ref::<vte4::Terminal>() else {
                return;
            };
            apply(terminal, &tokens);
            let mut font = pango::FontDescription::new();
            font.set_family(family.as_deref().unwrap_or("monospace"));
            font.set_absolute_size(13.0 * scale * f64::from(pango::SCALE));
            terminal.set_font(Some(&font));
        },
    );
}

fn apply(terminal: &vte4::Terminal, tokens: &ThemeTokens) {
    let parse = |value: &str| gdk::RGBA::parse(value).ok();
    let (Some(text), Some(background), Some(accent), Some(highlight)) = (
        parse(&tokens.text),
        parse(&tokens.background),
        parse(&tokens.accent),
        parse(&tokens.highlight),
    ) else {
        return;
    };
    let palette = [
        &tokens.background,
        &tokens.danger,
        tokens.syntax_string.as_ref().unwrap_or(&tokens.accent),
        tokens.syntax_constant.as_ref().unwrap_or(&tokens.highlight),
        &tokens.accent,
        tokens.syntax_keyword.as_ref().unwrap_or(&tokens.accent),
        tokens.syntax_type.as_ref().unwrap_or(&tokens.accent),
        &tokens.text,
    ]
    .map(|value| parse(value).unwrap_or(text));
    let bright = palette.map(|color| blend(color, text));
    let colors: Vec<_> = palette.iter().chain(bright.iter()).collect();
    terminal.set_colors(Some(&text), Some(&background), &colors);
    terminal.set_color_cursor(Some(&accent));
    terminal.set_color_cursor_foreground(Some(&background));
    terminal.set_color_highlight(Some(&highlight));
    terminal.set_color_highlight_foreground(Some(&text));
}

fn blend(color: gdk::RGBA, text: gdk::RGBA) -> gdk::RGBA {
    let mix = |a: f32, b: f32| a * 0.8 + b * 0.2;
    gdk::RGBA::new(
        mix(color.red(), text.red()),
        mix(color.green(), text.green()),
        mix(color.blue(), text.blue()),
        1.0,
    )
}
