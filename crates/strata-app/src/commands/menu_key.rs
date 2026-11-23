// SPDX-License-Identifier: MIT

use strata_components::{
    collection::element::{
        CollectionState, Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerInput,
    },
    context_menu::{AnchorEdge, keyboard_anchor, menu_chord},
    contract::{Point, Rect},
};
use strata_desktop::keys::bindings::CONTEXT_MENU;

use super::press::{self, Origin};

struct ViewKeys {
    inner: Option<Box<dyn KeyController>>,
    edge: AnchorEdge,
}

pub(crate) fn view_keys(collection: &mut CollectionState, edge: AnchorEdge) {
    let keyboard = &mut collection.controllers.keyboard;
    let inner = keyboard.take();
    *keyboard = Some(Box::new(ViewKeys { inner, edge }));
}

fn where_drawn(cx: &Ctx<'_>, index: usize, revealed: Point, window: Point) -> Option<Point> {
    let row = cx.geometry.rect_of(index);
    let shows = row.bottom() > cx.visible.y && row.y < cx.visible.bottom();
    let middle = row.center().y.clamp(cx.visible.y, cx.visible.bottom());
    shows.then(|| Point::new(revealed.x, window.y + middle - cx.visible.y))
}

fn open(cx: &mut Ctx<'_>, edge: AnchorEdge) {
    let window = cx.content_to_window(cx.visible);
    let window = Point::new(window.x, window.y);
    let anchor = keyboard_anchor(
        cx.source,
        cx.selection(),
        cx.geometry,
        cx.visible,
        window,
        edge,
    );
    let at = match anchor.reveal {
        Some(index) => where_drawn(cx, index, anchor.at, window),
        None => Some(anchor.at),
    };
    if let Some(at) = at {
        cx.emit(Effect::Intent(Intent::ContextMenu {
            target: anchor.target,
            anchor: Rect::new(at.x, at.y, 0.0, 0.0),
        }));
    }
}

impl KeyController for ViewKeys {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        let origin = Origin {
            physical: input.physical,
            base: input.base,
        };
        if press::read(input.event, origin).is(CONTEXT_MENU) {
            // A held key opens one menu.
            if !input.repeat {
                open(cx, self.edge);
            }
            return Flow::Handled;
        }
        if menu_chord(input.event).is_some() {
            return Flow::Handled;
        }
        match self.inner.as_mut() {
            Some(inner) => inner.key(input, cx),
            None => Flow::Pass,
        }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        match self.inner.as_mut() {
            Some(inner) => inner.tick(cx),
            None => Flow::Pass,
        }
    }

    fn pointer_seen(&mut self, input: &PointerInput) {
        if let Some(inner) = self.inner.as_mut() {
            inner.pointer_seen(input);
        }
    }

    fn reset(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.reset();
        }
    }
}
