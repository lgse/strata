//! The chord's footer pill, and its panel of next keys above it.

use rune::{El, LiveRegion, col, row, text};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use super::state::ChordState;
use crate::filter_bar::keycap::keycap;

/// The pill's fill and border: the accent at these opacities, as Strata's
/// `.shortcut-footer-chord-pill` draws them (`src/style.css`: `alpha(@strata_accent, 0.14)`
/// and a 1 px `alpha(@strata_accent, 0.4)` border). No Kit or strata-kit token holds them.
const PILL_FILL_ALPHA: f32 = 0.14;
const PILL_BORDER_ALPHA: f32 = 0.4;

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

/// The armed chord's pill (`g-`), for the footer beside the `10X` pill (C19), or `None` with
/// nothing armed: the accent on a faint accent tint with a fainter accent border, Strata's
/// `.shortcut-footer-chord-pill`. A polite live region, so assistive technology says the
/// chord as it arms.
pub fn pill<A: Clone>(look: &Look, key: &str, state: &ChordState<A>) -> Option<El> {
    let map = state.armed()?;
    let k = &look.kit;
    let accent = k.c.text_accent;
    Some(
        row()
            .key(pill_key(key))
            .role("status")
            .label(map.mark.clone())
            .live_region(LiveRegion::Polite)
            .px(look.space.sm)
            .center()
            .radius(look.frame.control)
            .fill(k.alpha(accent, PILL_FILL_ALPHA))
            .border(1.0, k.alpha(accent, PILL_BORDER_ALPHA))
            .child(
                text(map.mark.clone())
                    .font(k.ty.small_medium)
                    .color(accent)
                    .a11y_hidden(),
            ),
    )
}

/// The armed chord's panel, for the window's overlay layer (`rune_kit::root`), or `None`:
/// above the pill, its right edges with the pill's, listing each line as a keycap beside its
/// action, down two columns. It takes neither focus nor the keyboard, so the next key still
/// reaches the chord, and a press on it leaves the keyboard where it was. A list to assistive
/// technology, each line named "keycap: action".
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
        popover::raised(k, content, k.radius.md),
        &pill_key(key),
        Placement::TopEnd,
        look.space.xs,
    ))
}
