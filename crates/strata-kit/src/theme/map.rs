//! A theme onto a [`Look`]: step 1 of the look's build (`look.rs`).
//!
//! The base is `Kit::from_palette_focus` on the background, text and accent, with the focus
//! ring in the accent (Strata's `outline: 2px solid @strata_accent`). Then each semantic
//! token takes the Strata role its widgets draw with. Translucent tokens go through
//! `Kit::alpha` at the opacities Strata's stylesheet uses (`src/style.css` at lgse/strata
//! 35ee677ab), so a window that blends in linear light still matches. Each opacity is a share
//! of the colour's own, as GTK's `alpha()` multiplies, so a translucent theme colour stays
//! translucent.
//!
//! | Kit tokens | Value | Strata's use |
//! | --- | --- | --- |
//! | `page`, `bg_base` | background | the window, the file columns |
//! | `bg_deep` | surface | headers, tab strip, sidebar: rune-kit's chrome bands |
//! | `layer_01`, `surface_raised`, `surface_raised_base`, `surface_weak` | surface | popovers, menus, dialogs, the preview |
//! | `layer_02`–`layer_04` | `from_palette` on the surface: its `layer_01`–`layer_03` | steps above the surface they sit on, which may be darker or lighter than the background |
//! | `bg_button_neutral`, `surface_base` | muted | form controls, switch troughs |
//! | `surface_disabled` | muted at 30 % | `:disabled` |
//! | `bg_accent`, `text_accent`, `icon_accent`, `border_active`, `border_focus` | accent | |
//! | `hover`, `surface_hover`, `surface_raised_hover` | accent at 15 % | row, menu and header-action `:hover` |
//! | `pressed` | accent at 22 % | `:active`, `:checked` |
//! | `border_muted`, `border_base`, `border_strong` | border at 24, 60, 80 % | inner dividers, pane and popover edges, strong edges |
//! | `border_weak`, `border_disabled` | border at 54, 38 % | entries, disabled entries |
//! | `border_hover` | accent at 68 % | `entry:hover` |
//! | `std_border` | border | |
//! | `text_base`, `text_strong`, `std_text`, `icon_base`, `std_icon_strong` | text | |
//! | `text_muted`, `text_weak`, `text_faint`, `icon_muted`, `icon_faint`, `icon_weaker`, `std_icon` | dim text | Strata's one secondary tone |
//! | `text_inverse`, `text_contrast`, `text_invert_strong`, `icon_inverse`, `icon_contrast`, `std_icon_invert`, `submit_fg`, `submit_icon` | background | text on accent fills |
//! | `danger` | danger; fill 14 %, border 32 % | `.action-dialog-symbol.danger` |
//! | `info` | accent; fill 10 %, border 42 % | Strata's notices |
//!
//! The rest keep `from_palette`'s values: `success` and `warning` (Strata has no such
//! colours), the knobs, the scrim, the depth sheen, the contrast overlays, `surface_float` and
//! the elevation shadows, whose rings follow `border_base`. Kit's type, radii and motion are
//! left alone; K2 sets the first two.
//!
//! Strata extensions: `fill.muted` and `fill.highlight` take muted and highlight;
//! `glow.color` takes the accent (Strata's `@strata_glow`); the syntax colours come from
//! [`super::syntax`]. `glow.on` and `animations` are the user's switches, set by
//! [`crate::shape::LookParams`].
//!
//! Gaps where Kit has no slot: text selection (Kit's `selection()` is the accent at 30 %;
//! Strata selects with the opaque accent and background-coloured text), and the Switch, whose
//! on track rune-kit fills with `bg_accent` where Strata tints it at 25 % under an accent
//! knob.
//!
//! The appearance is `from_palette`'s: dark when the text is lighter than the background. It
//! agrees with Strata's own rule ([`Theme::dark_background`]) on every bundled theme; a
//! custom theme with light text on a light-ish background could tell them apart.

use rune_kit::{Kit, PaletteFocus, PaletteSeed, StateColors};

use super::colors::{Colors, Theme};
use super::syntax;
use crate::look::{Fills, Look};

#[cfg(test)]
mod tests;

impl Theme {
    /// The theme's look, before the user's shape, density and type settings: apply
    /// [`crate::shape::LookParams`] with [`Look::with_params`], then the user's overrides.
    pub fn look(&self) -> Look {
        look(&self.colors)
    }

    /// The theme's kit, as [`Theme::look`] holds it.
    pub fn kit(&self) -> Kit {
        kit(&self.colors)
    }
}

pub(crate) fn look(colors: &Colors) -> Look {
    let mut look = Look::from_kit(kit(colors));
    let (theme, extra) = syntax::themes(&look.kit, colors);
    look.syntax = theme;
    look.syntax_extra = extra;
    look.glow.color = colors.accent.color();
    look.fill = Fills {
        muted: colors.muted.color(),
        highlight: colors.highlight.color(),
    };
    look
}

pub(crate) fn kit(colors: &Colors) -> Kit {
    let bg = colors.background.color();
    let surface = colors.surface.color();
    let text = colors.text.color();
    let accent = colors.accent.color();
    let danger = colors.danger.color();
    let muted = colors.muted.color();
    let border = colors.border.color();
    let dim = colors.dim_text.color();

    let mut kit = Kit::from_palette_focus(
        PaletteSeed {
            bg,
            fg: text,
            accent,
        },
        PaletteFocus::Accent,
    );
    let raised = Kit::from_palette(PaletteSeed {
        bg: surface,
        fg: text,
        accent,
    })
    .c;
    let a = |c: rune::Color, x: f32| kit.alpha(c, c.a * x);
    let mut p = kit.c;

    p.page = bg;
    p.bg_base = bg;
    p.bg_deep = surface;
    p.layer_01 = surface;
    p.layer_02 = raised.layer_01;
    p.layer_03 = raised.layer_02;
    p.layer_04 = raised.layer_03;
    p.surface_raised = surface;
    p.surface_raised_base = surface;
    p.surface_weak = surface;

    p.bg_button_neutral = muted;
    p.surface_base = muted;
    p.surface_disabled = a(muted, 0.30);

    p.bg_accent = accent;
    p.text_accent = accent;
    p.icon_accent = accent;
    p.border_active = accent;
    p.border_focus = accent;
    p.hover = a(accent, 0.15);
    p.surface_hover = p.hover;
    p.surface_raised_hover = p.hover;
    p.pressed = a(accent, 0.22);

    p.border_muted = a(border, 0.24);
    p.border_base = a(border, 0.60);
    p.border_strong = a(border, 0.80);
    p.border_weak = a(border, 0.54);
    p.border_disabled = a(border, 0.38);
    p.border_hover = a(accent, 0.68);
    p.std_border = border;

    for c in [
        &mut p.text_base,
        &mut p.text_strong,
        &mut p.std_text,
        &mut p.icon_base,
        &mut p.std_icon_strong,
    ] {
        *c = text;
    }
    for c in [
        &mut p.text_muted,
        &mut p.text_weak,
        &mut p.text_faint,
        &mut p.icon_muted,
        &mut p.icon_faint,
        &mut p.icon_weaker,
        &mut p.std_icon,
    ] {
        *c = dim;
    }
    for c in [
        &mut p.text_inverse,
        &mut p.text_contrast,
        &mut p.text_invert_strong,
        &mut p.icon_inverse,
        &mut p.icon_contrast,
        &mut p.std_icon_invert,
        &mut p.submit_fg,
        &mut p.submit_icon,
    ] {
        *c = bg;
    }

    p.danger = StateColors {
        bg: a(danger, 0.14),
        fg: danger,
        border: a(danger, 0.32),
        icon: danger,
    };
    p.info = StateColors {
        bg: a(accent, 0.10),
        fg: accent,
        border: a(accent, 0.42),
        icon: accent,
    };

    kit.c = p;
    kit
}
