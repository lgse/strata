// SPDX-License-Identifier: MIT

//! The Menu key and Shift+F10 in a view: the context menu of what GTK's
//! `open_focused_context_menu` (`src/ui/browser/view_filter.rs`) opens one for.
//!
//! - **Which menu.** The cursor's entry when it is selected: the menu of that entry, or of
//!   the selection it is in (`menus::open` reads GTK's `context_entries`). Otherwise, with a
//!   cursor outside the selection or none at all, the folder's menu.
//! - **Where.** An entry's menu opens at the entry, vertically centred: at its right edge in
//!   Columns and Icons and at its centre in List (`item_menu_target`). The folder's opens at
//!   the middle of the listing (`folder_menu_target`). C21's `keyboard_anchor` holds both.
//! - **Why a controller.** Where the menu opens is the collection's geometry, which only a
//!   controller is handed. So the key is read after every stage of the chain ahead of the
//!   view's own keys, where GTK's `context_menu_command` sits in the middle of it.

use strata_components::{
    collection::element::{Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerInput},
    context_menu::{AnchorEdge, keyboard_anchor, menu_chord},
    contract::{Point, Rect},
};
use strata_desktop::keys::bindings::CONTEXT_MENU;

use super::press::{self, Origin};

struct ViewKeys {
    inner: Option<Box<dyn KeyController>>,
    edge: AnchorEdge,
}

/// A view's keyboard controller with the context menu's keys ahead of it. A view wraps the
/// controller of each collection that takes its keys, once: the menu opens through the
/// `Intent::ContextMenu` a secondary press sends, so the view's handler for that is the one
/// way a menu opens. `edge` is where on the cursor's entry it opens.
pub(crate) fn view_keys(
    inner: Option<Box<dyn KeyController>>,
    edge: AnchorEdge,
) -> Box<dyn KeyController> {
    Box::new(ViewKeys { inner, edge })
}

/// Sends what a secondary press sends, for the entry and the point `keyboard_anchor` gives.
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
    if let Some(index) = anchor.reveal {
        cx.emit(Effect::Reveal(index));
    }
    cx.emit(Effect::Intent(Intent::ContextMenu {
        target: anchor.target,
        anchor: Rect::new(anchor.at.x, anchor.at.y, 0.0, 0.0),
    }));
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
        // The same keys with anything else held open nothing, as on GTK. Left to Rune, the
        // Menu key would become a secondary press at the focused element's corner.
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
