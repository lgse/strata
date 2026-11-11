//! The type ramp, scaled by text size, and the interface font.
//!
//! Work package: K2, `docs/work/phase-1-leaves/K2-shape-density-type.md`.
//!
//! Values are Strata's, read at `lgse/strata` `upstream/main` 35ee677ab:
//! `src/ui/preferences/text_size.rs` (8–48 px, default 13), `desktop_scale.rs` (the desktop
//! factor multiplies the size once, rounded to a whole pixel), `src/ui/theme.rs:651` (icons
//! scale by size / 13) and `src/ui/theme/font.rs` (the chosen family, then the app's own).
//!
//! # Text size
//!
//! The base ramp is rune-kit's (`ui` is 13 on 20), which is Strata's at its default 13 px,
//! so the whole ramp and the icon sizes scale by `root / 13`. Sizes scale exactly; line
//! heights round to whole points, so rows and bars built from them stay on whole points. At
//! 13 px exactly the ramp is rune-kit's own, untouched (its `text_small` keeps 19.5).
//!
//! # The interface font
//!
//! strata-kit never lists fonts: the host passes the installed families in as data (from P5,
//! or from Rune once it can list them; Rune issue #114). [`font_choices`] and
//! [`filter_fonts`] build the picker's list from them, and the picker's Default row is
//! filtered by its label like any other ([`font_label_matches`]), as Strata's is. A chosen family goes in front of the
//! app's own sans families; the mono slot stays the app's, since previews keep their
//! monospace font. The app's own are the bundled JetBrains Mono for both slots
//! ([`fonts::FAMILIES`](crate::fonts::FAMILIES), K6), as Strata's are, so Default (no
//! chosen family) puts the bundled font back. The host hands
//! [`Look::font`](crate::look::Look::font) to `Tx::set_font_families` when it changes, and the
//! app's own families when it becomes `None`.
//!
//! Rune parses a family list as CSS does but without escapes: a quoted name ends at the next
//! matching quote. So a name with `"` is quoted with `'`, and a name with both quote kinds
//! (or a control character) can't be named at all: [`quote_family`] returns `None`, and the
//! picker leaves it out.

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

/// The text size Strata draws at: `text_size`, clamped, times the desktop's text scale once,
/// rounded to a whole pixel so hinted glyphs keep their top row. A scale that isn't a
/// positive number is ignored.
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

/// `size` moved `delta` px, clamped: Ctrl++ and Ctrl+− step it by one. Ctrl+0 resets it to
/// [`TEXT_SIZE`]'s default.
pub fn step_text_size(size: u16, delta: i32) -> u16 {
    TEXT_SIZE.stepped(size, delta)
}

/// The ramps and icon sizes at [`BASE_PX`]: W2's, on rune-kit's kit.
fn base() -> &'static (Type, Ramp, IconSizes) {
    static BASE: OnceLock<(Type, Ramp, IconSizes)> = OnceLock::new();
    BASE.get_or_init(|| {
        // `Look::from_kit`, never `Look::fallback`: the fallback applies `LookParams`,
        // which calls back in here, and re-entering a `OnceLock`'s initializer deadlocks.
        let look = Look::from_kit(Kit::neutral(false));
        (look.kit.ty, look.ty, look.icon_size)
    })
}

/// `style` at `factor`: the size exactly, the line height (when set) on whole points. At a
/// factor of 1 the style is returned as it is, so Strata's default text size leaves the base
/// ramp's line heights alone, half points included.
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

/// `name` as one entry of a family list, or `None` when Rune's parser can't take it: empty,
/// with a control character, or with both `"` and `'`.
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

/// The sans list for a chosen family: `name` first, then the app's own list, so text keeps
/// the app's look where the family isn't installed or lacks a glyph.
pub fn family_list(name: &str, app_ui: &str) -> Option<String> {
    let quoted = quote_family(name)?;
    let app_ui = app_ui.trim();
    Some(if app_ui.is_empty() {
        quoted
    } else {
        format!("{quoted}, {app_ui}")
    })
}

/// The families for `font` over the app's `app`: `None` (the app's own) when no family is
/// chosen or it can't be named. The mono slot is always the app's.
pub fn families(font: Option<&str>, app: &Families) -> Option<Families> {
    let ui = family_list(font?, &app.ui)?;
    Some(Families::new(ui, app.mono.clone()))
}

/// The picker's families from the installed ones: trimmed, sorted ignoring case. It
/// deliberately keeps one entry per name ignoring case (fontconfig matches families that
/// way, so `Inter` and `inter` are one family), and deliberately leaves out names
/// [`quote_family`] can't take, since choosing one could never apply. The picker puts a
/// Default row before them.
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

/// Whether a picker row labelled `label` shows for `query`: its label contains the query,
/// ignoring case. The Default row is filtered by its label ("Default") like a family's.
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

/// The type step of [`Look::apply_params`]: the ramps and icons at `root` px, and the
/// families.
pub(crate) fn apply(look: &mut Look, root: f32, font: Option<&str>, app: &Families) {
    let factor = factor(root);
    let (ty, ramp) = scale_ramp(factor);
    look.kit.ty = ty;
    look.ty = ramp;
    look.icon_size = scale_icons(factor);
    look.font = families(font, app);
}
