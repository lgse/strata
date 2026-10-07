// SPDX-License-Identifier: MIT

//! Selection decisions use displayed positions. Adapters translate the returned anchor to their
//! source identity, and own hit testing, focus, slow-click rename and navigation. Keyboard and
//! marquee changes enter through the selection model; their snapshot is the next pointer baseline.

use gtk::{gdk, glib, prelude::*};
use std::{cell::Cell, rc::Rc};

pub(super) struct SelectionChange {
    pub(super) selected: gtk::Bitset,
    pub(super) anchor: Option<u32>,
    pub(super) preserved_group: bool,
}

pub(super) fn pointer_selection(
    previous: &gtk::Bitset,
    position: u32,
    anchor: Option<u32>,
    multiple: bool,
    modifiers: gdk::ModifierType,
) -> SelectionChange {
    let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    let preserved_group =
        multiple && !control && !shift && previous.contains(position) && previous.size() > 1;
    let selected = if !multiple {
        gtk::Bitset::new_range(position, 1)
    } else if shift {
        let anchor = anchor.unwrap_or(position);
        gtk::Bitset::new_range(anchor.min(position), anchor.abs_diff(position) + 1)
    } else if control {
        let selected = previous.copy();
        if !selected.remove(position) {
            selected.add(position);
        }
        selected
    } else if preserved_group {
        previous.copy()
    } else {
        gtk::Bitset::new_range(position, 1)
    };
    SelectionChange {
        selected,
        anchor: if shift { anchor } else { Some(position) },
        preserved_group,
    }
}

/// A modified press may become a marquee/drag, so claim only on release. Cancellation never
/// activates. The short-lived release token distinguishes pointer activation from keyboard GTK
/// activation; a generation prevents deferred cleanup from clearing a newer press.
#[derive(Clone, Default)]
pub(super) struct PointerSequence {
    activation: Rc<Cell<Option<bool>>>,
    generation: Rc<Cell<u64>>,
}

impl PointerSequence {
    pub(super) fn install(&self, gesture: &gtk::GestureClick) {
        let released = self.clone();
        gesture.connect_released(move |gesture, _, _, _| {
            if released.activation.get() == Some(false) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
            let sequence = released.clone();
            let generation = sequence.generation.get();
            glib::idle_add_local_once(move || {
                if sequence.generation.get() == generation {
                    sequence.activation.set(None);
                }
            });
        });
        let cancelled = self.clone();
        gesture.connect_cancel(move |_, _| {
            cancelled
                .generation
                .set(cancelled.generation.get().wrapping_add(1));
            cancelled.activation.set(None);
        });
    }

    pub(super) fn press(&self, modifiers: gdk::ModifierType) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.activation.set(Some(!modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
        )));
    }

    pub(super) fn activation(&self) -> Option<bool> {
        self.activation.get()
    }
}

#[cfg(test)]
mod tests;
