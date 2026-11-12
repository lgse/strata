// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::collection::BoundResult;

pub(super) fn configure(
    view: &gtk::Widget,
    scroll: &gtk::ScrolledWindow,
    bound: Rc<RefCell<Vec<BoundResult>>>,
) {
    scroll.set_hscrollbar_policy(gtk::PolicyType::Automatic);
    scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
    crate::ui::browser_modes::wheel_scrolls_sideways(scroll);
    let Some(grid) = view.downcast_ref::<gtk::GridView>() else {
        return;
    };
    let viewport = scroll.downgrade();
    let previous = Cell::new((0, 0));
    grid.add_tick_callback(move |grid, _| {
        let Some(scroll) = viewport.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let row = bound.borrow().iter().find_map(|item| {
            item.widget
                .upgrade()
                .map(|widget| widget.height())
                .filter(|height| *height > 0)
        });
        if let Some(row) = row {
            let size = (scroll.height(), row);
            if previous.replace(size) != size && size.0 > 0 {
                let rows = (size.0 / row).clamp(1, 200) as u32;
                if grid.max_columns() != rows {
                    grid.set_max_columns(rows);
                }
            }
        }
        glib::ControlFlow::Continue
    });
}
