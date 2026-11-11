//! A keycap chip that draws its key as given, on Strata's keycap look (`.sidebar-keycap`:
//! the text colour on the base fill, a 1 px border, small corners, bold small type).
//!
//! Not rune-kit's `Keybind`: it upper-cases a single character ("g" draws G), with no way to
//! keep the case (Rune draft U-C18-1). In 10xer mode `g` and `G` (Shift+g) are different
//! keys, and Strata shows a letter as it is typed.

use rune::{El, row, text};
use strata_kit::look::Look;

/// The chip for `key` ("g", "G", "1–9", "Space", "Shift"). It is decorative: the row it sits
/// in names the key for assistive technology.
pub fn keycap(look: &Look, key: &str) -> El {
    let k = &look.kit;
    row()
        .px(look.space.xs)
        .center()
        .radius(k.radius.sm)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .a11y_hidden()
        .child(
            text(key.to_owned())
                .font(k.ty.small_medium)
                .color(k.c.text_base),
        )
}
