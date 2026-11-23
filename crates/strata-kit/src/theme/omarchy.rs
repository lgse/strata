// SPDX-License-Identifier: MIT

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
