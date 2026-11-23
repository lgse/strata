// SPDX-License-Identifier: MIT

//! Where a menu opened from the keyboard goes, and the collection controller that opens it.

use rune::{Key, KeyEvent};

use crate::collection::element::{
    Ctx, Effect, Flow, Intent, KeyController, KeyInput, PointerInput,
};
use crate::collection::selection::Selection;
use crate::contract::{CollectionGeometry, EntryId, EntrySource, Point, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnchorEdge {
    /// The entry's right edge: Columns and Icons.
    #[default]
    Right,
    /// The entry's centre: List.
    Center,
}

/// A keyboard-opened menu's target and anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardAnchor {
    /// The cursor's entry when it is selected; `None` opens the background menu.
    pub target: Option<EntryId>,
    /// Where the menu opens, in window points.
    pub at: Point,
    pub reveal: Option<usize>,
}

pub fn keyboard_anchor(
    source: &dyn EntrySource,
    selection: &Selection,
    geometry: &dyn CollectionGeometry,
    visible: Rect,
    window: Point,
    edge: AnchorEdge,
) -> KeyboardAnchor {
    let to_window =
        |p: Point, top: f32| Point::new(window.x + p.x - visible.x, window.y + p.y - top);
    let cursor = selection
        .cursor_index(source)
        .filter(|&i| selection.is_selected_at(source, i));
    let Some(index) = cursor else {
        let c = visible.center();
        return KeyboardAnchor {
            target: None,
            at: to_window(c, visible.y),
            reveal: None,
        };
    };
    let r = geometry.rect_of(index);
    let top = if r.y < visible.y {
        r.y
    } else if r.bottom() > visible.bottom() {
        (r.bottom() - visible.h).max(0.0)
    } else {
        visible.y
    };
    let x = match edge {
        AnchorEdge::Right => r.right(),
        AnchorEdge::Center => r.center().x,
    };
    let x = x.clamp(visible.x, visible.right().max(visible.x));
    KeyboardAnchor {
        target: Some(source.id_at(index)),
        at: to_window(Point::new(x, r.center().y), top),
        reveal: (top != visible.y).then_some(index),
    }
}

pub fn menu_chord(ev: &KeyEvent) -> Option<bool> {
    let m = ev.mods;
    match ev.key {
        Key::ContextMenu => Some(!(m.ctrl || m.cmd || m.alt || m.shift)),
        Key::Function(10) if m.shift => Some(!(m.ctrl || m.cmd || m.alt)),
        _ => None,
    }
}

pub struct MenuKeys {
    inner: Option<Box<dyn KeyController>>,
    edge: AnchorEdge,
}

impl MenuKeys {
    /// Menu keys before `inner`, anchoring item menus at `edge`.
    pub fn new(inner: impl KeyController + 'static, edge: AnchorEdge) -> Self {
        Self {
            inner: Some(Box::new(inner)),
            edge,
        }
    }

    /// Menu keys alone, for a collection with no other keyboard controller.
    pub fn alone(edge: AnchorEdge) -> Self {
        Self { inner: None, edge }
    }
}

impl KeyController for MenuKeys {
    fn key(&mut self, input: &KeyInput<'_>, cx: &mut Ctx<'_>) -> Flow {
        match menu_chord(input.event) {
            Some(true) => {
                if !input.repeat {
                    open(cx, self.edge);
                }
                Flow::Handled
            }
            Some(false) => Flow::Handled,
            None => match self.inner.as_mut() {
                Some(k) => k.key(input, cx),
                None => Flow::Pass,
            },
        }
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        self.inner.as_mut().map_or(Flow::Pass, |k| k.tick(cx))
    }

    fn pointer_seen(&mut self, input: &PointerInput) {
        if let Some(k) = self.inner.as_mut() {
            k.pointer_seen(input);
        }
    }

    fn reset(&mut self) {
        if let Some(k) = self.inner.as_mut() {
            k.reset();
        }
    }
}

fn open(cx: &mut Ctx<'_>, edge: AnchorEdge) {
    let window = cx.content_to_window(cx.visible);
    let a = keyboard_anchor(
        cx.source,
        cx.selection(),
        cx.geometry,
        cx.visible,
        Point::new(window.x, window.y),
        edge,
    );
    if let Some(i) = a.reveal {
        cx.emit(Effect::Reveal(i));
    }
    cx.emit(Effect::Intent(Intent::ContextMenu {
        target: a.target,
        anchor: Rect::new(a.at.x, a.at.y, 0.0, 0.0),
    }));
}
