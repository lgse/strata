// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    row: u32,
    column: usize,
    offset: usize,
}

#[derive(Clone, Copy)]
struct Range {
    anchor: Point,
    focus: Point,
}

impl Range {
    fn ordered(self) -> (Point, Point) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
}

struct BoundCell {
    label: glib::WeakRef<gtk::Label>,
    item: glib::WeakRef<gtk::ListItem>,
    column: usize,
    highlight: RefCell<Option<(usize, usize, Vec<u16>)>>,
}

pub(super) struct Selection {
    table: Rc<TableState>,
    bound: RefCell<Vec<BoundCell>>,
    range: Cell<Option<Range>>,
    dragging: Cell<bool>,
    pointer: Cell<(f64, f64)>,
    pressed_link: RefCell<Option<(glib::WeakRef<gtk::Label>, String)>>,
}

impl Selection {
    pub(super) fn new(table: Rc<TableState>) -> Rc<Self> {
        Rc::new(Self {
            table,
            bound: RefCell::new(Vec::new()),
            range: Cell::new(None),
            dragging: Cell::new(false),
            pointer: Cell::new((0.0, 0.0)),
            pressed_link: RefCell::new(None),
        })
    }

    pub(super) fn bind(&self, label: &gtk::Label, item: &gtk::ListItem, column: usize) {
        let mut bound = self.bound.borrow_mut();
        bound.retain(|cell| cell.label.upgrade().is_some_and(|other| other != *label));
        bound.push(BoundCell {
            label: label.downgrade(),
            item: item.downgrade(),
            column,
            highlight: RefCell::new(None),
        });
        drop(bound);
        label.set_attributes(None);
        if self.range.get().is_some() {
            self.highlight();
        }
    }

    pub(super) fn clear(&self) {
        self.range.set(None);
        self.dragging.set(false);
        self.pressed_link.borrow_mut().take();
        self.highlight();
    }

    fn row(&self, position: u32) -> Option<usize> {
        let item = self
            .table
            .model
            .item(position)?
            .downcast::<glib::BoxedAnyObject>()
            .ok()?;
        let index = *item.borrow::<usize>();
        Some(index)
    }

    fn text(&self, point: Point) -> &str {
        self.row(point.row)
            .and_then(|index| self.table.rows[index].get(point.column))
            .map_or("", |cell| cell.text.as_str())
    }

    pub(super) fn copy_text(&self) -> Option<String> {
        let (start, end) = self.range.get()?.ordered();
        if start == end {
            return None;
        }
        let mut output = String::new();
        for row in start.row..=end.row {
            let first = if row == start.row { start.column } else { 0 };
            let last = if row == end.row {
                end.column
            } else {
                self.row(row)
                    .map_or(0, |index| self.table.rows[index].len().saturating_sub(1))
            };
            if row != start.row {
                output.push('\n');
            }
            for column in first..=last {
                if column != first {
                    output.push('\t');
                }
                let point = Point {
                    row,
                    column,
                    offset: 0,
                };
                let text = self.text(point);
                let from = if (row, column) == (start.row, start.column) {
                    start.offset
                } else {
                    0
                };
                let to = if (row, column) == (end.row, end.column) {
                    end.offset
                } else {
                    text.chars().count()
                };
                output.extend(text.chars().skip(from).take(to.saturating_sub(from)));
            }
        }
        Some(output)
    }

    // Native label selections compete for primary-clipboard ownership. Pango
    // attributes let every bound cell show one shared selection instead.
    fn highlight(&self) {
        for cell in self.bound.borrow().iter() {
            let (Some(label), Some(item)) = (cell.label.upgrade(), cell.item.upgrade()) else {
                continue;
            };
            let text = label.text();
            let length = text.chars().count();
            let (mut from, mut to) = (0, 0);
            if let Some(range) = self.range.get() {
                let (start, end) = range.ordered();
                let location = (item.position(), cell.column);
                if location >= (start.row, start.column) && location <= (end.row, end.column) {
                    from = if location == (start.row, start.column) {
                        start.offset.min(length)
                    } else {
                        0
                    };
                    to = if location == (end.row, end.column) {
                        end.offset.min(length)
                    } else {
                        length
                    };
                }
            }
            let mut colors = Vec::new();
            if from < to {
                #[expect(
                    deprecated,
                    reason = "GTK exposes named theme colors through StyleContext"
                )]
                let context = label.style_context();
                #[expect(
                    deprecated,
                    reason = "GTK exposes named theme colors through StyleContext"
                )]
                for color in [
                    context.lookup_color("theme_accent"),
                    context.lookup_color("theme_bg"),
                ]
                .into_iter()
                .flatten()
                {
                    colors.extend([
                        (color.red() * 65535.0) as u16,
                        (color.green() * 65535.0) as u16,
                        (color.blue() * 65535.0) as u16,
                    ]);
                }
            }
            let next = (from, to, colors);
            if cell.highlight.borrow().as_ref() == Some(&next) {
                continue;
            }
            let attrs = gtk::pango::AttrList::new();
            if next.2.len() == 6 {
                for (index, rgb) in next.2.as_chunks::<3>().0.iter().enumerate() {
                    let mut attr = if index == 0 {
                        gtk::pango::AttrColor::new_background(rgb[0], rgb[1], rgb[2])
                    } else {
                        gtk::pango::AttrColor::new_foreground(rgb[0], rgb[1], rgb[2])
                    };
                    attr.set_start_index(byte_offset(&text, from) as u32);
                    attr.set_end_index(byte_offset(&text, to) as u32);
                    attrs.insert(attr);
                }
            }
            replace_highlight(&label, (from < to).then_some(&attrs));
            cell.highlight.replace(Some(next));
        }
    }

    fn hit(&self, view: &gtk::ColumnView, x: f64, y: f64) -> Option<(Point, gtk::Label)> {
        let mut nearest = None;
        let mut distance = f64::INFINITY;
        for cell in self.bound.borrow().iter() {
            let (Some(label), Some(item)) = (cell.label.upgrade(), cell.item.upgrade()) else {
                continue;
            };
            if !label.is_mapped() || item.position() == gtk::INVALID_LIST_POSITION {
                continue;
            }
            let Some(bounds) = label.compute_bounds(view) else {
                continue;
            };
            let left = f64::from(bounds.x());
            let top = f64::from(bounds.y());
            let dx = (left - x)
                .max(0.0)
                .max(x - left - f64::from(bounds.width()));
            let dy = (top - y).max(0.0).max(y - top - f64::from(bounds.height()));
            let candidate = dy * 100000.0 + dx;
            if candidate >= distance {
                continue;
            }
            distance = candidate;
            let (ox, oy) = label.layout_offsets();
            let layout = label.layout();
            let (_, index, trailing) = layout.xy_to_index(
                ((x - left - f64::from(ox)) * f64::from(gtk::pango::SCALE)) as i32,
                ((y - top - f64::from(oy)) * f64::from(gtk::pango::SCALE)) as i32,
            );
            let text = label.text();
            let offset = text
                .get(..index.max(0) as usize)
                .map_or(0, |s| s.chars().count())
                + trailing.max(0) as usize;
            nearest = Some((
                Point {
                    row: item.position(),
                    column: cell.column,
                    offset: offset.min(text.chars().count()),
                },
                label,
            ));
        }
        nearest
    }

    pub(super) fn install(self: &Rc<Self>, view: &gtk::ColumnView, scroll: &gtk::ScrolledWindow) {
        let outside_click = Rc::new(RefCell::new(
            None::<(glib::WeakRef<gtk::Widget>, gtk::EventControllerLegacy)>,
        ));
        let controller_for_map = outside_click.clone();
        let weak_state = Rc::downgrade(self);
        view.connect_map(move |view| {
            let Some(root) = view
                .root()
                .and_then(|root| root.dynamic_cast::<gtk::Widget>().ok())
            else {
                return;
            };
            let click = gtk::EventControllerLegacy::new();
            click.set_name(Some("table-selection-dismiss"));
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak_state = weak_state.clone();
            // Raw presses are not cancelled when a descendant claims a drag.
            click.connect_event(move |_, event| {
                if event.event_type() == gtk::gdk::EventType::ButtonPress
                    && event
                        .downcast_ref::<gtk::gdk::ButtonEvent>()
                        .is_some_and(|event| event.button() == 1)
                    && let Some(state) = weak_state.upgrade()
                {
                    state.clear();
                    for cell in state.bound.borrow().iter() {
                        if let Some(label) = cell.label.upgrade() {
                            label.select_region(0, 0);
                        }
                    }
                }
                glib::Propagation::Proceed
            });
            root.add_controller(click.clone());
            controller_for_map.replace(Some((root.downgrade(), click)));
        });
        view.connect_unmap(move |_| {
            if let Some((root, click)) = outside_click.borrow_mut().take()
                && let Some(root) = root.upgrade()
            {
                root.remove_controller(&click);
            }
        });
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = self.clone();
        let weak_view = view.downgrade();
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(view) = weak_view.upgrade() else {
                return;
            };
            let in_body = view
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|mut widget| {
                    loop {
                        if widget.css_name() == "header" {
                            return false;
                        }
                        if widget.has_css_class("preview-document-table-cell")
                            || widget.css_name() == "cell"
                        {
                            return true;
                        }
                        let Some(parent) = widget.parent() else {
                            return false;
                        };
                        widget = parent;
                    }
                });
            if !in_body {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            let Some((point, label)) = state.hit(&view, x, y) else {
                return;
            };
            state.clear();
            state.pressed_link.replace(
                label
                    .current_uri()
                    .map(|uri| (label.downgrade(), uri.to_string())),
            );
            label.grab_focus();
            label.select_region(0, 0);
            state.range.set(Some(Range {
                anchor: point,
                focus: point,
            }));
            state.pointer.set((x, y));
            state.dragging.set(true);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let state = self.clone();
        let weak_view = view.downgrade();
        drag.connect_drag_update(move |gesture, dx, dy| {
            if !state.dragging.get() {
                return;
            }
            let (Some(view), Some((x, y))) = (weak_view.upgrade(), gesture.start_point()) else {
                return;
            };
            state.pointer.set((x + dx, y + dy));
            state.extend(&view);
        });
        let state = self.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            state.dragging.set(false);
            if let Some((label, uri)) = state.pressed_link.borrow_mut().take()
                && dx.abs() < 4.0
                && dy.abs() < 4.0
                && let Some(label) = label.upgrade()
                && label.current_uri().as_deref() == Some(uri.as_str())
                && crate::services::has_web_scheme(&uri)
            {
                super::super::virtual_preview::open_web_link(&uri, &label);
            }
        });
        let state = self.clone();
        drag.connect_cancel(move |_, _| {
            state.dragging.set(false);
            state.pressed_link.borrow_mut().take();
        });
        view.add_controller(drag);

        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = self.clone();
        let weak_view = view.downgrade();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gtk::gdk::Key::c | gtk::gdk::Key::C)
                && let (Some(text), Some(view)) = (state.copy_text(), weak_view.upgrade())
            {
                view.clipboard().set_text(&text);
                return glib::Propagation::Stop;
            }
            if matches!(
                key,
                gtk::gdk::Key::Escape
                    | gtk::gdk::Key::Left
                    | gtk::gdk::Key::Right
                    | gtk::gdk::Key::Up
                    | gtk::gdk::Key::Down
                    | gtk::gdk::Key::Home
                    | gtk::gdk::Key::End
            ) || (modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gtk::gdk::Key::a | gtk::gdk::Key::A))
            {
                state.clear();
            }
            glib::Propagation::Proceed
        });
        view.add_controller(key);
        let state = self.clone();
        let weak_scroll = scroll.downgrade();
        view.add_tick_callback(move |view, _| {
            if state.dragging.get()
                && let Some(scroll) = weak_scroll.upgrade()
            {
                let (x, y) = state.pointer.get();
                for (coordinate, extent, adjustment) in [
                    (x, view.width(), scroll.hadjustment()),
                    (y, view.height(), scroll.vadjustment()),
                ] {
                    let delta = if coordinate < 24.0 {
                        -12.0
                    } else if coordinate > f64::from(extent) - 24.0 {
                        12.0
                    } else {
                        0.0
                    };
                    if delta != 0.0 {
                        adjustment.set_value((adjustment.value() + delta).clamp(
                            adjustment.lower(),
                            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                        ));
                    }
                }
                state.extend(view);
            }
            if state.range.get().is_some() {
                state.highlight();
            }
            glib::ControlFlow::Continue
        });
    }

    fn extend(&self, view: &gtk::ColumnView) {
        let (x, y) = self.pointer.get();
        if let (Some(mut range), Some((point, _))) = (self.range.get(), self.hit(view, x, y)) {
            range.focus = point;
            self.range.set(Some(range));
            self.highlight();
        }
    }
}

fn replace_highlight(label: &gtk::Label, attributes: Option<&gtk::pango::AttrList>) {
    // GTK can retain removed attributes in its cached Pango layout. Rebuild
    // from the original markup so shrinking/clearing cannot leave stale colors.
    let markup = label.uses_markup();
    let source = label.label();
    label.set_attributes(None);
    label.set_text("");
    if markup {
        label.set_markup(&source);
    } else {
        label.set_text(&source);
    }
    label.set_attributes(attributes);
}

fn byte_offset(text: &str, characters: usize) -> usize {
    text.char_indices()
        .nth(characters)
        .map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests;
