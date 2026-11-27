// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use rune::TextStyle;
use rune_kit::{Kit, Type};

use crate::look::{Families, IconSizes, Look, Ramp};
use crate::shape::Steps;

#[cfg(test)]
mod tests;

/// The interface text size in px.
pub const TEXT_SIZE: Steps = Steps {
    min: 8,
    max: 48,
    step: 1,
    default: 13,
};

/// The text size the base ramp and icon sizes are drawn at.
pub const BASE_PX: f32 = 13.0;

pub fn root_px(text_size: u16, scale: f32) -> f32 {
    let size = f32::from(TEXT_SIZE.clamp(text_size));
    if !(scale.is_finite() && scale > 0.0) {
        return size;
    }
    (size * scale).round().max(1.0)
}

/// The factor a root size puts on the base ramp and icon sizes.
pub fn factor(root_px: f32) -> f32 {
    root_px / BASE_PX
}

pub fn step_text_size(size: u16, delta: i32) -> u16 {
    TEXT_SIZE.stepped(size, delta)
}

fn base() -> &'static (Type, Ramp, IconSizes) {
    static BASE: OnceLock<(Type, Ramp, IconSizes)> = OnceLock::new();
    BASE.get_or_init(|| {
        let look = Look::from_kit(Kit::neutral(false));
        (look.kit.ty, look.ty, look.icon_size)
    })
}

pub fn scale_style(style: TextStyle, factor: f32) -> TextStyle {
    if factor == 1.0 {
        return style;
    }
    TextStyle {
        size: style.size * factor,
        line_height: if style.line_height > 0.0 {
            (style.line_height * factor).round().max(1.0)
        } else {
            0.0
        },
        ..style
    }
}

/// The line a style takes: its line height, or Rune's 1.25 × size when it sets none.
pub fn line_of(style: TextStyle) -> f32 {
    if style.line_height > 0.0 {
        style.line_height
    } else {
        (style.size * 1.25).round()
    }
}

/// rune-kit's ramp and Strata's at `factor`.
pub fn scale_ramp(factor: f32) -> (Type, Ramp) {
    let (ty, ramp, _) = *base();
    let s = |style| scale_style(style, factor);
    (
        Type {
            ui: s(ty.ui),
            ui_medium: s(ty.ui_medium),
            ui_compact: s(ty.ui_compact),
            small: s(ty.small),
            small_medium: s(ty.small_medium),
            body: s(ty.body),
            body_medium: s(ty.body_medium),
            text_small: s(ty.text_small),
            text_small_medium: s(ty.text_small_medium),
            text_xsmall: s(ty.text_xsmall),
            text_xsmall_medium: s(ty.text_xsmall_medium),
            mono: s(ty.mono),
        },
        Ramp {
            title: s(ramp.title),
            heading: s(ramp.heading),
        },
    )
}

/// Toolbar and row icon sizes at `factor`, on whole points.
pub fn scale_icons(factor: f32) -> IconSizes {
    let (_, _, icons) = *base();
    let s = |x: f32| (x * factor).round().max(1.0);
    IconSizes {
        sm: s(icons.sm),
        md: s(icons.md),
        lg: s(icons.lg),
    }
}

pub fn quote_family(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    match (name.contains('"'), name.contains('\'')) {
        (false, _) => Some(format!("\"{name}\"")),
        (true, false) => Some(format!("'{name}'")),
        (true, true) => None,
    }
}

pub fn family_list(name: &str, app_ui: &str) -> Option<String> {
    let quoted = quote_family(name)?;
    let app_ui = app_ui.trim();
    Some(if app_ui.is_empty() {
        quoted
    } else {
        format!("{quoted}, {app_ui}")
    })
}

pub fn families(font: Option<&str>, app: &Families) -> Option<Families> {
    let ui = family_list(font?, &app.ui)?;
    Some(Families::new(ui, app.mono.clone()))
}

pub fn font_choices(installed: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names: Vec<(String, String)> = installed
        .into_iter()
        .filter(|name| quote_family(name).is_some())
        .map(|name| {
            let name = name.trim().to_owned();
            (name.to_lowercase(), name)
        })
        .collect();
    names.sort();
    names.dedup_by(|a, b| a.0 == b.0);
    names.into_iter().map(|(_, name)| name).collect()
}

pub fn font_label_matches(label: &str, query: &str) -> bool {
    label.to_lowercase().contains(&query.trim().to_lowercase())
}

/// The choices that [`font_label_matches`] `query`. An empty query keeps them all.
pub fn filter_fonts<'a>(choices: &'a [String], query: &str) -> Vec<&'a str> {
    choices
        .iter()
        .filter(|name| font_label_matches(name, query))
        .map(String::as_str)
        .collect()
}

pub(crate) fn apply(look: &mut Look, root: f32, font: Option<&str>, app: &Families) {
    let factor = factor(root);
    let (ty, ramp) = scale_ramp(factor);
    look.kit.ty = ty;
    look.ty = ramp;
    look.icon_size = scale_icons(factor);
    look.font = families(font, app);
}
