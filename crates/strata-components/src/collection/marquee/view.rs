//! The band's drawing: one element, placed by a live rect so a move or a scroll moves it
//! without a view commit. Look and Kit tokens only.

use rune::{Color, El, Live, el};
use strata_kit::look::Look;

/// How the band looks.
///
/// Used by: the C1e adapter, and hosts that draw the band themselves.
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
    /// Strata's `.file-marquee` (`src/style.css`, `upstream/main` 35ee677ab) in the Look's
    /// tokens. Its fill is the selection highlight at 0.58 opacity; here, the selection colour
    /// the collection draws selected rows in (rune-kit's table selection) at 0.58 of its own
    /// opacity, so the band reads as a lighter selection over the rows it crosses. Its 1 px
    /// outline is the accent: Kit's `text_accent`, which the collection's drop highlight also
    /// uses. Its 4 px radius scales with the Look's roundness.
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

/// The band's key, so in-process tests can find it while it shows. Keyed elements get a group
/// role in Rune's accessibility tree, so the band also leaves the tree ([`El::a11y_hidden`]).
pub const KEY: &str = "marquee-band";

/// The band, keyed [`KEY`], out of flow at the top-left of its parent and placed by `rect`
/// (`[x, y, w, h]` in the parent's points). The parent is the layer the collection draws its
/// marquee in, which follows the scroll, so `rect` is in content coordinates: fill it from
/// `MarqueeMachine::drawn`. It has no handlers, so it takes no input, and it is hidden from
/// assistive technology: it's decoration, and the status line speaks the count it selects.
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
