// SPDX-License-Identifier: MIT

//! Recording a new shortcut: while a capture is armed, every key pressed in
//! its window reaches it, as pressed, before any handler sees the key.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{
    glib::{self, Propagation},
    prelude::*,
};

use super::KeyPress;

struct Armed {
    id: u64,
    anchor: glib::WeakRef<gtk::Widget>,
    handler: Rc<dyn Fn(&KeyPress)>,
}

thread_local! {
    static ARMED: RefCell<Vec<Armed>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

#[must_use = "the capture ends when this is dropped"]
pub(crate) struct Capture(u64);

impl Drop for Capture {
    fn drop(&mut self) {
        disarm(|current| current.id == self.0);
    }
}

/// Removes the captures `ended` selects, dropping their handlers only once the
/// slot is free again, since a handler may own another capture.
fn disarm(ended: impl Fn(&Armed) -> bool) {
    let removed: Vec<Armed> = ARMED.with(|armed| {
        let (removed, kept) = armed.take().into_iter().partition(&ended);
        armed.replace(kept);
        removed
    });
    drop(removed);
}

/// Sends `handler` every press but a lone modifier made in `anchor`'s window
/// while `anchor` is mapped, ahead of every other key handler, replacing any
/// earlier capture in that window.
pub(crate) fn capture(
    anchor: &impl IsA<gtk::Widget>,
    handler: impl Fn(&KeyPress) + 'static,
) -> Capture {
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    let anchor = anchor.as_ref();
    let window = topmost(anchor);
    disarm(|current| {
        current
            .anchor
            .upgrade()
            .is_none_or(|other| topmost(&other) == window)
    });
    ARMED.with(|armed| {
        armed.borrow_mut().push(Armed {
            id,
            anchor: anchor.downgrade(),
            handler: Rc::new(handler),
        });
    });
    Capture(id)
}

pub(super) fn deliver(
    controller: &gtk::EventControllerKey,
    press: &KeyPress,
) -> Option<Propagation> {
    if press.is_modifier() || ARMED.with(|armed| armed.borrow().is_empty()) {
        return None;
    }
    let window = topmost(&controller.widget()?);
    let handler = ARMED.with(|armed| {
        armed.borrow().iter().find_map(|armed| {
            let anchor = armed.anchor.upgrade()?;
            (anchor.is_mapped() && topmost(&anchor) == window).then(|| armed.handler.clone())
        })
    })?;
    handler(press);
    Some(Propagation::Stop)
}

fn topmost(widget: &gtk::Widget) -> gtk::Widget {
    let mut top = widget.clone();
    while let Some(parent) = top.parent() {
        top = parent;
    }
    top
}
