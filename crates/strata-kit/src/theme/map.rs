// SPDX-License-Identifier: MIT

use rune_kit::{Kit, PaletteFocus, PaletteSeed, StateColors};

use super::colors::{Colors, Theme};
use super::syntax;
use crate::look::{Fills, Look};

#[cfg(test)]
mod tests;

impl Theme {
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
