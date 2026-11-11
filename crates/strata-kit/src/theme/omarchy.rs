//! Omarchy Quattro: the current Omarchy theme as a Strata theme, in the variant the user
//! picked.
//!
//! Strata reads `~/.local/state/omarchy/current/theme.name` and
//! `current/theme/colors.toml`, watches them, and remembers the variant per Omarchy theme.
//! Here both files arrive as their contents. The mapping is Strata's (`src/ui/theme/omarchy.rs`
//! and `omarchy_state.rs` at lgse/strata 35ee677ab):
//!
//! - `background` is the window; `dark_background` (else `background`) the panes and
//!   headers; `lighter_background` the hover surface, unless missing or equal to
//!   `background`, when it is `background` 10 % toward `foreground`; `selection` (else 25 %
//!   toward `accent`) the selection; `muted` (else 36 % toward `foreground`) the borders; dim
//!   text is 62 % toward `foreground`; danger is `color1`, else Strata's default. ANSI
//!   `color8` is never read.
//! - Syntax colours are `magenta`, `green`, `orange`, `cyan` and `yellow`, else the older
//!   `color5`, `color2`, `color9` and `color3`. A colour still missing comes from the
//!   terminal palette (`magenta` or `blue`, `green`, `orange` or `yellow`, `cyan` or `blue`,
//!   `yellow`, all present), else from the accent and text.
//! - A value that doesn't parse as a colour counts as missing. Without a valid `background`,
//!   `foreground` and `accent` there is no theme, and Strata hides Follow Omarchy.
//!
//! [`Variant::Darker`] moves the file columns onto `dark_background`. [`Variant::Contrast`]
//! does that, then raises text to 7:1 against the window, pane, hover and selection
//! surfaces, dim text, accent and danger to 4.5:1, borders to 3:1, and syntax colours to
//! 4.5:1 against the window and pane. A surface moves (toward black, or white on a light
//! palette) only when even the opposite pole can't reach 7:1 on it. Light is `mode = "light"`,
//! dark `mode = "dark"`, else a background luminance above 0.45.

use toml::de::{DeTable, DeValue};

use super::color::{Rgba8, blend, contrast, luminance, readable, toward};
use super::colors::{Colors, DEFAULT_DANGER, Source, SourceColors, Theme};
use super::syntax::{Palette, resolve_over};

#[cfg(test)]
mod tests;

/// Strata's "Omarchy variant" setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Variant {
    /// File columns on `background`; the sidebar, headers and preview on `dark_background`.
    Lighter,
    /// The whole window on `dark_background`. Strata's default.
    #[default]
    Darker,
    /// Darker, with foregrounds raised to contrast targets.
    Contrast,
}

/// The Omarchy theme named `name` (the contents of `theme.name`) with the colours in
/// `colors_toml` (the contents of `colors.toml`), or `None` without a valid background,
/// foreground and accent.
pub fn theme(name: &str, colors_toml: &str, variant: Variant) -> Option<Theme> {
    let doc = DeTable::parse(colors_toml).ok()?;
    let values = doc.get_ref();
    let get = |key: &str| match values.get(key).map(|v| v.get_ref()) {
        Some(DeValue::String(s)) => Rgba8::parse(s),
        _ => None,
    };
    let background = get("background")?;
    let text = get("foreground")?;
    let accent = get("accent")?;
    let mut colors = Colors {
        background,
        surface: get("dark_background").unwrap_or(background),
        text,
        accent,
        danger: get("color1").unwrap_or(DEFAULT_DANGER),
        muted: get("lighter_background")
            .filter(|&l| l != background)
            .unwrap_or_else(|| blend(background, text, 0.10)),
        highlight: get("selection").unwrap_or_else(|| blend(background, accent, 0.25)),
        border: get("muted").unwrap_or_else(|| blend(background, text, 0.36)),
        dim_text: blend(background, text, 0.62),
        syntax: SourceColors {
            keyword: get("magenta").or_else(|| get("color5")),
            string: get("green").or_else(|| get("color2")),
            constant: get("orange").or_else(|| get("color9")),
            type_: get("cyan").or_else(|| get("color3")),
            preprocessor: get("yellow"),
        },
    };
    let mode = match values.get("mode").map(|v| v.get_ref()) {
        Some(DeValue::String(s)) => Some(s.as_ref()),
        _ => None,
    };
    let light = is_light(background, mode);
    let palette = (|| {
        Some(Palette {
            statement: get("magenta").or_else(|| get("blue"))?,
            string: get("green")?,
            constant: get("orange").or_else(|| get("yellow"))?,
            type_: get("cyan").or_else(|| get("blue"))?,
            preprocessor: get("yellow")?,
        })
    })();

    if variant != Variant::Lighter {
        colors.background = colors.surface;
    }
    if variant == Variant::Contrast {
        raise_contrast(&mut colors, light, palette);
    }
    // Strata resolves the left-out syntax colours from the terminal palette as it applies
    // the theme; resolving them here gives the same colours.
    if let Some(p) = palette {
        let s = &mut colors.syntax;
        s.keyword.get_or_insert(p.statement);
        s.string.get_or_insert(p.string);
        s.constant.get_or_insert(p.constant);
        s.type_.get_or_insert(p.type_);
        s.preprocessor.get_or_insert(p.preprocessor);
    }

    let id = name.trim();
    Some(Theme {
        id: id.to_owned(),
        name: title_case(id),
        colors,
        source: Source::Omarchy,
    })
}

/// `mode` decides when it says `light` or `dark`; otherwise the background's luminance.
fn is_light(background: Rgba8, mode: Option<&str>) -> bool {
    match mode {
        Some("light") => true,
        Some("dark") => false,
        _ => luminance(background) > 0.45,
    }
}

fn raise_contrast(c: &mut Colors, light: bool, palette: Option<Palette>) {
    let (bg_pole, fg_pole) = if light {
        (Rgba8::WHITE, Rgba8::BLACK)
    } else {
        (Rgba8::BLACK, Rgba8::WHITE)
    };
    for surface in [
        &mut c.background,
        &mut c.surface,
        &mut c.muted,
        &mut c.highlight,
    ] {
        *surface = toward(*surface, bg_pole, |s| contrast(s, fg_pole) >= 7.0);
    }
    let surfaces = [c.background, c.surface, c.muted, c.highlight];
    c.text = readable(c.text, &surfaces, fg_pole, 7.0);
    c.dim_text = readable(c.dim_text, &surfaces, fg_pole, 4.5);
    c.accent = readable(c.accent, &surfaces, fg_pole, 4.5);
    c.danger = readable(c.danger, &surfaces, fg_pole, 4.5);
    c.border = readable(c.border, &surfaces, fg_pole, 3.0);

    let p = resolve_over(c, palette);
    let panes = [c.background, c.surface];
    let raise = |x| Some(readable(x, &panes, fg_pole, 4.5));
    c.syntax = SourceColors {
        keyword: raise(p.statement),
        string: raise(p.string),
        constant: raise(p.constant),
        type_: raise(p.type_),
        preprocessor: raise(p.preprocessor),
    };
}

/// `tokyo-night` as `Tokyo Night`.
fn title_case(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
