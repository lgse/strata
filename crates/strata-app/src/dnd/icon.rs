// SPDX-License-Identifier: MIT

//! Drag preview icon snapshot (`drag_preview_icon` in `src/ui/browser/clipboard.rs`):
//! renders the dragged file's resolved file-type icon on a transparent surface with a subtle
//! drop shadow, plus stacked rear copies and a contrasting count pill when dragging multiple items.

use rune::{Color, Cx, El, Icon, TextStyle, el, row, text};
use strata_core::model::FileEntry;
use strata_kit::{icons::IconName, look::Look};
use strata_platform::dnd::DragIcon;

#[cfg(test)]
mod tests;

const ICON_PX: f32 = 32.0;
const SINGLE_CANVAS: f32 = 48.0;
const SINGLE_FRONT: f32 = (SINGLE_CANVAS - ICON_PX) / 2.0;
const MULTI_FRONT: f32 = 6.0;
const MULTI_BASE_CANVAS: f32 = 56.0;
const EXTRA_DIGIT_WIDTH: f32 = 8.0;
const BADGE_HEIGHT: f32 = 20.0;
const BADGE_PAD_X: f32 = 5.0;
const BADGE_MARGIN: f32 = 4.0;
const SHADOW_OFFSET_Y: f32 = 1.0;
const SHADOW_SIGMA: f32 = 1.5;
const SHADOW_ALPHA: f32 = 0.34;
const STACK_LAYERS: [(f32, f32); 2] = [(5.0, 0.32), (2.5, 0.60)];

#[derive(Clone)]
struct PreviewSpec {
    icon: &'static Icon,
    count: usize,
    icon_color: Color,
    shadow_color: Color,
    badge_fill: Color,
    badge_border: Color,
    badge_ink: Color,
    badge_font: TextStyle,
    canvas: (f32, f32),
    front: (f32, f32),
}

impl PreviewSpec {
    fn from_entries(look: &Look, entries: &[FileEntry]) -> Self {
        let count = entries.len().max(1);
        let key = entries
            .first()
            .map(crate::entry_icon::entry_icon)
            .unwrap_or(crate::entry_icon::DOCUMENTS);
        let icon = look
            .icon_named(key)
            .unwrap_or_else(|| look.icon(IconName::File));
        let (canvas, front) = layout_for(count);
        let badge_fill = look.kit.c.bg_accent;
        Self {
            icon,
            count,
            icon_color: look.kit.c.icon_accent,
            shadow_color: look.kit.c.text_base.alpha(SHADOW_ALPHA),
            badge_fill,
            badge_border: look.kit.c.surface_raised,
            badge_ink: look.ink_on(badge_fill),
            badge_font: TextStyle {
                weight: 600.0,
                ..look.kit.ty.small
            },
            canvas,
            front,
        }
    }

    fn hotspot(&self) -> (f32, f32) {
        (self.front.0 + ICON_PX / 2.0, self.front.1 + ICON_PX / 2.0)
    }
}

fn layout_for(count: usize) -> ((f32, f32), (f32, f32)) {
    if count > 1 {
        let digits = count.to_string().len();
        let extra = digits.saturating_sub(2) as f32 * EXTRA_DIGIT_WIDTH;
        (
            (MULTI_BASE_CANVAS + extra, MULTI_BASE_CANVAS),
            (MULTI_FRONT, MULTI_FRONT),
        )
    } else {
        ((SINGLE_CANVAS, SINGLE_CANVAS), (SINGLE_FRONT, SINGLE_FRONT))
    }
}

fn build_preview(spec: &PreviewSpec, _: &mut Cx<PreviewSpec>) -> El {
    let mut root = el().size(spec.canvas.0, spec.canvas.1);
    if spec.count > 1 {
        for (offset, opacity) in STACK_LAYERS {
            root = root.child(
                rune::icon(spec.icon, ICON_PX)
                    .color(spec.icon_color)
                    .opacity(opacity)
                    .pin(
                        Some(spec.front.0 + offset),
                        Some(spec.front.1 + offset),
                        None,
                        None,
                    ),
            );
        }
    }
    let shadow = rune::icon(spec.icon, ICON_PX)
        .icon_one_color()
        .color(spec.shadow_color)
        .blur(SHADOW_SIGMA)
        .pin(
            Some(spec.front.0),
            Some(spec.front.1 + SHADOW_OFFSET_Y),
            None,
            None,
        );
    let front = rune::icon(spec.icon, ICON_PX).color(spec.icon_color).pin(
        Some(spec.front.0),
        Some(spec.front.1),
        None,
        None,
    );
    root = root.child(shadow).child(front);
    if spec.count > 1 {
        root = root.child(count_badge(spec));
    }
    root
}

fn count_badge(spec: &PreviewSpec) -> El {
    row()
        .min_w(BADGE_HEIGHT)
        .h(BADGE_HEIGHT)
        .px(BADGE_PAD_X)
        .center()
        .fill(spec.badge_fill)
        .border(1.0, spec.badge_border)
        .radius(BADGE_HEIGHT / 2.0)
        .child(
            text(spec.count.to_string())
                .font(spec.badge_font)
                .color(spec.badge_ink),
        )
        .pin(None, None, Some(BADGE_MARGIN), Some(BADGE_MARGIN))
}

/// Builds the platform drag icon snapshot for `entries` in `look` at display `scale`.
pub(crate) fn drag_icon(look: &Look, entries: &[FileEntry], scale: f32) -> DragIcon {
    let spec = PreviewSpec::from_entries(look, entries);
    let size = spec.canvas;
    let (hx, hy) = spec.hotspot();
    DragIcon::snapshot(spec, build_preview, size, scale, look.kit.dark).hotspot(hx, hy)
}
