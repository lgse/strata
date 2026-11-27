// SPDX-License-Identifier: MIT

use rune::{Color, El, Live, el};
use strata_kit::look::Look;

/// How the band looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandLook {
    /// The fill.
    pub fill: Color,
    /// The outline's colour.
    pub border: Color,
    /// The outline's width, in points.
    pub border_width: f32,
    /// The corner radius, in points.
    pub radius: f32,
}

impl BandLook {
    /// Strata's `.file-marquee` (`src/style.css`, `upstream/main` 35ee677ab) in the Look's tokens.
    pub fn from_look(look: &Look) -> Self {
        let selected = rune_kit::TableStyle::from_kit(&look.kit).selected;
        Self {
            fill: selected.alpha(selected.a * 0.58),
            border: look.kit.c.text_accent,
            border_width: 1.0,
            radius: look.round(4.0),
        }
    }
}

/// The band's key, so in-process tests can find it while it shows.
pub const KEY: &str = "marquee-band";

/// The band, keyed `KEY`, out of flow at the top-left of its parent and placed by `rect` (`[x, y, w.
pub fn band_view(look: &Look, rect: &Live<[f32; 4]>) -> El {
    let style = BandLook::from_look(look);
    el().key(KEY)
        .a11y_hidden()
        .absolute(0.0, 0.0)
        .fill(style.fill)
        .border(style.border_width, style.border)
        .radius(style.radius)
        .live_rect(rect)
}
