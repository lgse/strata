// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::ui::tenxer_mode::Chord;

#[derive(Clone)]
pub(super) struct ChordPanel {
    popover: glib::WeakRef<gtk::Popover>,
    options: glib::WeakRef<gtk::Grid>,
}

impl ChordPanel {
    pub(super) fn attach(pill: &gtk::Label) -> Self {
        let options = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .build();
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .autohide(false)
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::End)
            .can_focus(false)
            .focusable(false)
            .child(&options)
            .build();
        popover.add_css_class("shortcut-popover");
        popover.add_css_class("shortcut-chord-popover");
        popover.set_offset(0, -6);
        popover.set_parent(pill);
        let weak_popover = popover.downgrade();
        pill.connect_destroy(move |_| {
            if let Some(popover) = weak_popover.upgrade()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });
        Self {
            popover: popover.downgrade(),
            options: options.downgrade(),
        }
    }

    pub(super) fn show(
        &self,
        pill: &gtk::Label,
        chord: Chord,
        rows: &[(String, String)],
        armed: Rc<Cell<Option<Chord>>>,
    ) {
        let (Some(popover), Some(options)) = (self.popover.upgrade(), self.options.upgrade())
        else {
            return;
        };
        fill(&options, rows);
        // A pill that was hidden has no allocation to anchor to yet.
        let popover = popover.downgrade();
        pill.add_tick_callback(move |pill, _| {
            let Some(popover) = popover.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if armed.get() != Some(chord) {
                return glib::ControlFlow::Break;
            }
            if pill.width() <= 0 {
                return glib::ControlFlow::Continue;
            }
            popover.popup();
            glib::ControlFlow::Break
        });
    }

    pub(super) fn hide(&self) {
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
        if let Some(options) = self.options.upgrade() {
            fill(&options, &[]);
        }
    }

    #[cfg(test)]
    pub(super) fn shown_options(&self) -> Option<Vec<(String, String)>> {
        let popover = self.popover.upgrade()?;
        let options = self.options.upgrade()?;
        if !popover.is_visible() {
            return None;
        }
        let mut rows = Vec::new();
        let mut child = options.first_child();
        while let Some(key) = child {
            let action = key.next_sibling()?;
            let text = |widget: &gtk::Widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .map(|label| label.text().to_string())
                    .unwrap_or_default()
            };
            rows.push((text(&key), text(&action)));
            child = action.next_sibling();
        }
        Some(rows)
    }
}

fn fill(grid: &gtk::Grid, options: &[(String, String)]) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
    let rows = options.len().div_ceil(2).max(1);
    for (index, (key, action)) in options.iter().enumerate() {
        let row = (index % rows) as i32;
        let column = (index / rows * 2) as i32;
        let keycap = gtk::Label::new(Some(key));
        keycap.add_css_class("sidebar-keycap");
        keycap.set_halign(gtk::Align::End);
        if column > 0 {
            keycap.set_margin_start(16);
        }
        let label = gtk::Label::new(Some(action));
        label.set_xalign(0.0);
        grid.attach(&keycap, column, row, 1, 1);
        grid.attach(&label, column + 1, row, 1, 1);
    }
}
