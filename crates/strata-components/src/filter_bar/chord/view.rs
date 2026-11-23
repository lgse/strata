// SPDX-License-Identifier: MIT

//! The chord's footer pill, and its panel of next keys above it.

use rune::{El, LiveRegion, col, row, text};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use super::state::ChordState;
use crate::filter_bar::keycap::keycap;

/// The pill's key under `key`.
pub fn pill_key(key: &str) -> String {
    format!("{key}-pill")
}

/// The panel's key under `key`.
pub fn panel_key(key: &str) -> String {
    format!("{key}-panel")
}

/// Line `i`'s key under `key`.
pub fn option_key(key: &str, i: usize) -> String {
    format!("{key}-option-{i}")
}

pub fn pill<A: Clone>(look: &Look, key: &str, state: &ChordState<A>) -> Option<El> {
    let map = state.armed()?;
    Some(
        crate::footer::pill(look, &map.mark)
            .key(pill_key(key))
            .role("status")
            .label(map.mark.clone())
            .live_region(LiveRegion::Polite),
    )
}

pub fn panel<A: Clone>(look: &Look, key: &str, state: &ChordState<A>) -> Option<El> {
    let map = state.armed()?;
    let k = &look.kit;
    let n = map.options.len();
    let rows = n.div_ceil(2).max(1);
    let mut columns = row().gap(look.space.lg).items_start();
    for (c, chunk) in map.options.chunks(rows).enumerate() {
        let mut column = col().gap(look.space.xs);
        for (j, o) in chunk.iter().enumerate() {
            let i = c * rows + j;
            column = column.child(
                row()
                    .key(option_key(key, i))
                    .role("listitem")
                    .label(format!("{}: {}", o.keycap, o.label))
                    .position_in_set(i + 1, n)
                    .gap(look.space.sm)
                    .items_center()
                    .child(keycap(look, &o.keycap))
                    .child(
                        text(o.label.clone())
                            .font(k.ty.small)
                            .color(k.c.text_base)
                            .a11y_hidden(),
                    ),
            );
        }
        columns = columns.child(column);
    }
    let content = col()
        .key(panel_key(key))
        .role("list")
        .label(map.name.clone())
        .padding(look.space.sm)
        .preserve_focus_on_press(true)
        .focusable(false)
        .child(columns);
    Some(popover::anchored(
        k,
        strata_kit::transparency::raised_popover(look, content, k.radius.md),
        &pill_key(key),
        Placement::TopEnd,
        look.space.xs,
    ))
}
