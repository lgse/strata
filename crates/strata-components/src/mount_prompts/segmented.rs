// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Color, Cursor, El, Tx, row, text};
use strata_kit::look::Look;

const SEGMENT_RADIUS: f32 = 4.0;
/// Minimum height of each segment option.
const SEGMENT_HEIGHT: f32 = 28.0;

/// Configuration for [`build`].
pub(crate) struct SegmentedSpec<'a> {
    pub key: String,
    pub group_label: String,
    pub labels: &'a [&'a str],
    pub selected: usize,
    pub disabled: bool,
}

struct SegmentItem<'a> {
    index: usize,
    count: usize,
    label: &'a str,
    active: bool,
    disabled: bool,
}

type OnPick<S> = Rc<dyn Fn(&mut S, usize, &mut Tx)>;

/// Outer track and inner option radii at `look`'s corner roundness.
pub(crate) fn radii(look: &Look) -> (f32, f32) {
    (look.kit.radius.md, look.round(SEGMENT_RADIUS))
}

pub(crate) fn build<S: 'static>(
    look: &Look,
    spec: SegmentedSpec<'_>,
    on_pick: impl Fn(&mut S, usize, &mut Tx) + 'static,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (track_radius, _) = radii(look);
    let count = spec.labels.len().max(1);
    let on_pick: OnPick<S> = Rc::new(on_pick);
    let items = spec.labels.iter().enumerate().map(|(index, &label)| {
        let item = SegmentItem {
            index,
            count,
            label,
            active: spec.selected == index,
            disabled: spec.disabled,
        };
        segment(look, item, &on_pick)
    });
    row()
        .key(spec.key)
        .role("radiogroup")
        .label(spec.group_label)
        .focus_group()
        .w_full()
        .equal_columns(count)
        .padding(2.0)
        .radius(track_radius)
        .fill(alpha(look, c.bg_base, 0.25))
        .border(1.0, alpha(look, c.std_border, 0.45))
        .children(items)
}

fn segment<S: 'static>(look: &Look, item: SegmentItem<'_>, on_pick: &OnPick<S>) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (_, segment_radius) = radii(look);
    let active_enabled = item.active && !item.disabled;
    let (fill, ink) = if active_enabled {
        (alpha(look, c.bg_accent, 0.22), c.text_accent)
    } else {
        (Color::TRANSPARENT, c.text_muted)
    };
    let mut label = text(item.label.to_owned())
        .font(kit.ty.ui)
        .color(ink)
        .truncate()
        .a11y_hidden();
    if !item.disabled && !item.active {
        label = label.group_color(c.text_base);
    }
    let seg = row()
        .key(format!("segment{}", item.index))
        .grid_cell(item.index)
        .role("radio")
        .label(item.label.to_owned())
        .checked(item.active)
        .focus_selected(item.active)
        .position_in_set(item.index + 1, item.count)
        .disabled(item.disabled)
        .focusable(!item.disabled)
        .min_h(SEGMENT_HEIGHT)
        .px(look.space.lg)
        .py(3.0)
        .center()
        .radius(segment_radius)
        .fill(fill)
        .token_motion(kit.motion.control)
        .child(label);
    if item.disabled {
        return seg.layer_opacity(0.4);
    }
    let hover_fill = if item.active {
        alpha(look, c.bg_accent, 0.26)
    } else {
        c.hover
    };
    let on_pick = on_pick.clone();
    let index = item.index;
    seg.hover_group()
        .hover(hover_fill)
        .press(alpha(look, c.bg_accent, 0.28))
        .focus_ring(2.0, -2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| on_pick(s, index, tx))
}

fn alpha(look: &Look, color: Color, factor: f32) -> Color {
    look.kit.alpha(color, color.a * factor)
}
