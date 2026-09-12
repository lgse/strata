// SPDX-License-Identifier: MIT

use super::columns::ColumnSpan;
use super::*;

impl ViewState {
    pub(super) fn column_span(&self, depth: usize) -> Option<ColumnSpan> {
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        let left = columns[..depth]
            .iter()
            .map(column_width)
            .fold(0, i32::saturating_add);
        Some(ColumnSpan {
            left: f64::from(left),
            right: f64::from(left.saturating_add(column_width(column))),
            total: columns.iter().map(column_width).map(f64::from).sum(),
        })
    }

    fn focused_column_span(&self) -> Option<ColumnSpan> {
        let count = self.columns.borrow().len();
        let depth = self
            .browser
            .active_depth()
            .filter(|depth| *depth < count)
            .or_else(|| count.checked_sub(1))?;
        self.column_span(depth)
    }
}

fn column_width(column: &ColumnView) -> i32 {
    column
        .shell
        .width()
        .max(column.shell.width_request())
        .max(COLUMN_WIDTH)
}

impl BrowserView {
    pub(in crate::ui) fn preview_occupied_width(&self, available: i32) -> i32 {
        if self.view_mode() != BrowserMode::Columns {
            return single_pane_preview_reservation(available);
        }
        self.state
            .columns
            .borrow()
            .iter()
            .map(column_width)
            .fold(0, i32::saturating_add)
    }

    pub(in crate::ui) fn preview_navigation_width(&self, available: i32) -> i32 {
        self.state
            .focused_column_span()
            .map_or(COLUMN_WIDTH, |span| {
                (span.width() + span.peek_space(f64::from(available))) as i32
            })
    }

    pub(in crate::ui) fn preview_last_column_width(&self) -> i32 {
        self.state
            .columns
            .borrow()
            .last()
            .map_or(COLUMN_WIDTH, column_width)
    }

    pub(in crate::ui) fn preserve_columns_after_preview(&self, available: i32) {
        if self.view_mode() != BrowserMode::Columns {
            return;
        }
        let offset = self.state.scroller.hadjustment().value();
        let occupied = self.preview_occupied_width(available);
        let gap = (offset + f64::from(available - occupied)).ceil().max(0.0) as i32;
        self.state.columns_widget.set_margin_end(gap);
        self.state.horizontal_scroll_generation.set(
            self.state
                .horizontal_scroll_generation
                .get()
                .saturating_add(1),
        );
    }

    pub(in crate::ui) fn clear_preview_scroll_space(&self) {
        self.state.columns_widget.set_margin_end(0);
    }

    pub(in crate::ui) fn bind_preview_scrolling(&self, preview: &gtk::Revealer) {
        let weak = self.downgrade();
        let weak_preview = preview.downgrade();
        let generation = Rc::new(Cell::new(0u64));
        let changed_generation = generation.clone();
        preview.connect_reveal_child_notify(move |_| {
            changed_generation.set(changed_generation.get().saturating_add(1));
        });
        let last_page_size = Cell::new(self.state.scroller.hadjustment().page_size());
        self.state
            .scroller
            .hadjustment()
            .connect_changed(move |adjustment| {
                let page_changed =
                    last_page_size.replace(adjustment.page_size()) != adjustment.page_size();
                let generation_id = generation.get();
                let generation = generation.clone();
                let weak = weak.clone();
                let weak_preview = weak_preview.clone();
                // GtkViewport must finish allocating before its scroll value is changed.
                glib::idle_add_local_once(move || {
                    let Some(view) = weak.upgrade() else { return };
                    if generation.get() != generation_id || view.view_mode() != BrowserMode::Columns
                    {
                        return;
                    }
                    let is_open = weak_preview
                        .upgrade()
                        .is_some_and(|preview| preview.reveals_child());
                    let adjustment = view.state.scroller.hadjustment();
                    if is_open {
                        view.state.horizontal_scroll_generation.set(
                            view.state
                                .horizontal_scroll_generation
                                .get()
                                .saturating_add(1),
                        );
                        if let Some(span) = view.state.focused_column_span() {
                            let target = span.reveal_target(
                                adjustment.value(),
                                adjustment.page_size(),
                                adjustment.lower(),
                                adjustment.upper(),
                            );
                            let fully_visible = span.left >= adjustment.value()
                                && span.right <= adjustment.value() + adjustment.page_size();
                            if view.state.columns_widget.margin_end() == 0
                                || !fully_visible
                                || target >= adjustment.value()
                            {
                                adjustment.set_value(target);
                            }
                        }
                    } else if page_changed {
                        // Closing can update the range before GTK expands the viewport.
                        let maximum_gap = (adjustment.page_size() as i32
                            - view.preview_last_column_width())
                        .max(0);
                        let gap = view.state.columns_widget.margin_end().min(maximum_gap);
                        view.state.columns_widget.set_margin_end(gap);
                    }
                });
            });
        let weak = self.downgrade();
        self.connect_view_mode_changed(move |_| {
            if let Some(view) = weak.upgrade() {
                view.clear_preview_scroll_space();
            }
        });
    }
}
