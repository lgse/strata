// SPDX-License-Identifier: MIT

use rune::{El, row, text};
use strata_kit::look::Look;

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
