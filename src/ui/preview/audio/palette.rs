// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, prelude::*};

use crate::ui::theme::{ThemeManager, ThemeTokens};

#[derive(Clone, Copy, PartialEq)]
pub(in crate::ui::preview) struct Palette {
    pub(in crate::ui::preview) accent: gdk::RGBA,
    pub(in crate::ui::preview) accent_bright: gdk::RGBA,
    pub(in crate::ui::preview) peak: gdk::RGBA,
    pub(in crate::ui::preview) text: gdk::RGBA,
    pub(in crate::ui::preview) dim: gdk::RGBA,
    pub(in crate::ui::preview) background: gdk::RGBA,
}

thread_local! {
    static PALETTE: Cell<Option<Palette>> = const { Cell::new(None) };
    static SURFACES: RefCell<Vec<glib::WeakRef<gtk::Widget>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn mix(from: gdk::RGBA, to: gdk::RGBA, amount: f32) -> gdk::RGBA {
    let blend = |a: f32, b: f32| a + (b - a) * amount;
    gdk::RGBA::new(
        blend(from.red(), to.red()),
        blend(from.green(), to.green()),
        blend(from.blue(), to.blue()),
        blend(from.alpha(), to.alpha()),
    )
}

pub(in crate::ui::preview) fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(
        color.red(),
        color.green(),
        color.blue(),
        color.alpha() * alpha,
    )
}

fn from_tokens(tokens: &ThemeTokens) -> Option<Palette> {
    let parse = |value: &str| gdk::RGBA::parse(value).ok();
    let accent = parse(&tokens.accent)?;
    let text = parse(&tokens.text)?;
    Some(Palette {
        accent,
        accent_bright: mix(accent, text, 0.4),
        peak: mix(accent, text, 0.7),
        text,
        dim: parse(&tokens.dim_text).unwrap_or(text),
        background: parse(&tokens.background)?,
    })
}

pub(in crate::ui::preview) fn palette() -> Palette {
    if let Some(palette) = PALETTE.get() {
        return palette;
    }
    let palette = from_tokens(&ThemeManager::shared().appearance_tokens()).unwrap_or(Palette {
        accent: gdk::RGBA::BLACK,
        accent_bright: gdk::RGBA::BLACK,
        peak: gdk::RGBA::BLACK,
        text: gdk::RGBA::BLACK,
        dim: gdk::RGBA::BLACK,
        background: gdk::RGBA::WHITE,
    });
    PALETTE.set(Some(palette));
    palette
}

pub(in crate::ui::preview) fn follow_theme(widget: &impl IsA<gtk::Widget>) {
    SURFACES.with_borrow_mut(|surfaces| {
        surfaces.retain(|surface| surface.upgrade().is_some());
        surfaces.push(widget.upcast_ref::<gtk::Widget>().downgrade());
    });
}

pub(in crate::ui) fn apply_theme(tokens: &ThemeTokens) {
    if let Some(palette) = from_tokens(tokens) {
        PALETTE.set(Some(palette));
    }
    SURFACES.with_borrow_mut(|surfaces| {
        surfaces.retain(|surface| {
            let Some(surface) = surface.upgrade() else {
                return false;
            };
            surface.queue_draw();
            true
        });
    });
}
