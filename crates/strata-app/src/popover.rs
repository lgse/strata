// SPDX-License-Identifier: MIT

//! Non-modal popovers and menus over a full-window dismiss scrim.
//!
//! Uses a scrim instead of `El::modal` so the background stays visible to assistive technology,
//! focus moving outside autohides the popover, and an outside wheel scrolls only the listing via
//! `El::wheel_scrolls` (Rune chains wheel scrolling up ancestors only).

use rune::{El, Key, KeyEvent, PointerPress, Tx, WheelEvent, el};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use crate::{App, slots::Frame};

#[cfg(doc)]
use crate::commands::Holds;

/// A plain popover's surface, mounted in the layer while its part says it is open.
pub(crate) struct Plain {
    /// The surface's key; the scrim is keyed after it.
    pub(crate) key: &'static str,
    pub(crate) anchor: String,
    /// A plain popover or a menu, which differ in their keys and their role.
    pub(crate) kind: Kind,
    /// The accessible name of the surface (a dialog).
    pub(crate) label: String,
    /// The keys of its controls, in order: Tab, Shift+Tab, Up and Down move through them.
    pub(crate) stops: Vec<&'static str>,
    /// Those of [`Self::stops`] that take Up and Down themselves, such as a slider.
    pub(crate) own_arrows: &'static [&'static str],
    pub(crate) listing: Option<&'static str>,
    pub(crate) viewports: Vec<String>,
    /// Closes the popover: the part's own state.
    pub(crate) close: fn(&mut App),
    /// The popover this one was opened from, if any: an outside press or wheel closes it too.
    pub(crate) parent: Option<Parent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Dialog,
    Menu,
}

#[derive(Clone, Copy)]
pub(crate) struct Parent {
    pub(crate) close: fn(&mut App),
    pub(crate) anchor: &'static str,
}

impl Plain {
    /// Pushes the scrim and the surface around `body` onto the layer.
    pub(crate) fn build(self, look: &Look, body: El, frame: &mut Frame<'_, App>) {
        let kit = &look.kit;
        let listing = self.listing.and_then(|key| frame.cx.window_layout(key));
        let closes = Closes {
            own: self.close,
            parent: self.parent.map(|parent| parent.close),
            anchor: self
                .parent
                .map_or_else(|| self.anchor.clone(), |parent| parent.anchor.to_owned()),
        };
        let viewports = self
            .viewports
            .iter()
            .filter_map(|path| Some((path.clone(), frame.cx.window_layout(path)?)))
            .collect();
        frame
            .layer
            .push(scrim(self.key, listing, viewports, closes));
        let (close, stops, own_arrows, kind) = (self.close, self.stops, self.own_arrows, self.kind);
        let role = match kind {
            Kind::Dialog => "dialog",
            Kind::Menu => "menu",
        };
        let body = body
            .key(self.key)
            .role(role)
            .label(self.label)
            .focusable(kind == Kind::Menu)
            .focus_return(self.anchor.clone())
            .on_key(
                move |app: &mut App, event: &KeyEvent, tx: &mut Tx| match kind {
                    Kind::Dialog => keyed(app, event, tx, (close, &stops, own_arrows)),
                    Kind::Menu => menu_keyed(app, event, tx, (close, &stops)),
                },
            )
            .preserve_focus_on_press(true)
            .on_press(|_: &mut App| {})
            .on_wheel(|_: &mut App, _: WheelEvent, _: &mut Tx| true);
        let mut surface = strata_kit::transparency::raised_popover(look, body, kit.radius.lg)
            .border(1.0, look.transparency.frosted_border(kit.c.border_base));
        if look.glow.on {
            surface = surface.shadow(
                0.0,
                look.glow.radius,
                look.glow.color.alpha(look.glow.opacity),
            );
        }
        frame.layer.push(popover::anchored(
            kit,
            surface,
            &self.anchor,
            Placement::Bottom,
            look.space.xs,
        ));
    }
}

#[derive(Clone)]
struct Closes {
    own: fn(&mut App),
    parent: Option<fn(&mut App)>,
    anchor: String,
}

impl Closes {
    fn close(&self, app: &mut App, tx: &mut Tx) {
        (self.own)(app);
        if let Some(parent) = self.parent {
            parent(app);
        }
        tx.focus(&self.anchor, true);
    }
}

fn scrim(
    key: &str,
    listing: Option<[f32; 4]>,
    viewports: Vec<(String, [f32; 4])>,
    closes: Closes,
) -> El {
    let (on_secondary, on_wheel) = (closes.clone(), closes.clone());
    el().key(format!("{key}-scrim"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .on_background_press_tx(move |app: &mut App, tx: &mut Tx| closes.close(app, tx))
        .on_secondary_press(move |app: &mut App, _: PointerPress, tx: &mut Tx| {
            on_secondary.close(app, tx);
        })
        .on_wheel(move |app: &mut App, wheel: WheelEvent, tx: &mut Tx| {
            on_wheel.close(app, tx);
            !listing.is_some_and(|rect| inside(rect, wheel.x, wheel.y))
        })
        .children(
            viewports
                .into_iter()
                .map(|(path, rect)| hand_off(path, rect)),
        )
}

fn hand_off(path: String, [x, y, w, h]: [f32; 4]) -> El {
    el().absolute(x, y)
        .w(w)
        .h(h)
        .wheel_scrolls(path)
        .on_wheel(|_: &mut App, _: WheelEvent, _: &mut Tx| false)
}

fn inside([x, y, w, h]: [f32; 4], px: f32, py: f32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

pub(crate) fn focus_left(tx: &Tx, anchor: &str, stops: &[&str], child: &[&str]) -> bool {
    tx.focused().is_some_and(|focused| {
        focused != anchor && !stops.contains(&focused) && !child.contains(&focused)
    })
}

fn keyed(
    app: &mut App,
    event: &KeyEvent,
    tx: &mut Tx,
    (close, stops, own_arrows): (fn(&mut App), &[&'static str], &[&'static str]),
) -> bool {
    let at = tx
        .focused()
        .and_then(|focused| stops.iter().position(|stop| *stop == focused));
    let last = stops.len().saturating_sub(1);
    let next = match event.key {
        Key::Escape => {
            close(app);
            return true;
        }
        Key::Tab if stops.is_empty() => return false,
        Key::Tab => match (at, event.mods.shift) {
            (Some(at), false) if at == last => 0,
            (Some(at), false) => at + 1,
            (Some(0), true) | (None, true) => last,
            (Some(at), true) => at - 1,
            (None, false) => 0,
        },
        Key::Up | Key::Down => {
            let Some(at) = at else {
                return false;
            };
            if own_arrows.contains(&stops[at]) {
                return false;
            }
            if event.key == Key::Down {
                (at + 1).min(last)
            } else {
                at.saturating_sub(1)
            }
        }
        _ => return false,
    };
    tx.focus(stops[next], true);
    true
}

fn menu_keyed(
    app: &mut App,
    event: &KeyEvent,
    tx: &mut Tx,
    (close, stops): (fn(&mut App), &[&'static str]),
) -> bool {
    if event.key == Key::Escape {
        close(app);
        return true;
    }
    let Some(step) = menu_step(event) else {
        return false;
    };
    let Some(last) = stops.len().checked_sub(1) else {
        return true;
    };
    let at = tx
        .focused()
        .and_then(|focused| stops.iter().position(|stop| *stop == focused));
    let next = match (step, at) {
        (MenuStep::First, _) | (MenuStep::Next, None) => 0,
        (MenuStep::Last, _) | (MenuStep::Back, None) => last,
        (MenuStep::Next, Some(at)) if at == last => 0,
        (MenuStep::Next, Some(at)) => at + 1,
        (MenuStep::Back, Some(0)) => last,
        (MenuStep::Back, Some(at)) => at - 1,
    };
    tx.focus(stops[next], true);
    true
}

#[derive(Clone, Copy)]
enum MenuStep {
    Next,
    Back,
    First,
    Last,
}

fn menu_step(event: &KeyEvent) -> Option<MenuStep> {
    Some(match event.key {
        Key::Down => MenuStep::Next,
        Key::Up => MenuStep::Back,
        Key::Tab if event.mods.shift => MenuStep::Back,
        Key::Tab => MenuStep::Next,
        Key::Home => MenuStep::First,
        Key::End => MenuStep::Last,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
