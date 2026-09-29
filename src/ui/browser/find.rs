// SPDX-License-Identifier: MIT

//! 10xer footer find: moves the cursor between name matches in the displayed
//! listing and keeps the matched substrings highlighted without hiding rows.

use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::{Rc, Weak},
};

use gtk::prelude::*;

use super::{BrowserView, ViewState};
use crate::ui::browser_modes::BrowserMode;

/// The last committed find of a window. `n` repeats it after its highlights are
/// dismissed; leaving 10xer mode forgets it.
#[derive(Default)]
pub(super) struct FindState {
    query: String,
    backward: bool,
    highlighted: bool,
}

type Rgb = (u16, u16, u16);

thread_local! {
    static HIGHLIGHT_COLORS: Cell<Option<(Rgb, Rgb)>> = const { Cell::new(None) };
    /// Windows that have committed a find, so a theme change can recolor them.
    static FIND_VIEWS: RefCell<Vec<Weak<ViewState>>> = const { RefCell::new(Vec::new()) };
}

fn rgb(color: &str) -> Option<Rgb> {
    let color = gtk::gdk::RGBA::parse(color).ok()?;
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
    Some((
        channel(color.red()),
        channel(color.green()),
        channel(color.blue()),
    ))
}

/// Pango needs concrete colors, so theme application passes its accent and
/// background here and live highlights are recolored.
pub(in crate::ui) fn apply_theme(accent: &str, background: &str) {
    HIGHLIGHT_COLORS.set(rgb(accent).zip(rgb(background)));
    let views = FIND_VIEWS.with_borrow_mut(|views| {
        views.retain(|view| view.strong_count() > 0);
        views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    });
    for view in views {
        view.refresh_find_highlights();
    }
}

fn register_find_view(state: &Rc<ViewState>) {
    FIND_VIEWS.with_borrow_mut(|views| {
        if !views
            .iter()
            .any(|view| std::ptr::eq(view.as_ptr(), Rc::as_ptr(state)))
        {
            views.push(Rc::downgrade(state));
        }
    });
}

pub(in crate::ui) fn match_ranges(name: &str, query: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // A lowercase expansion keeps the byte span of the character it came from.
    let folded: Vec<(Range<usize>, char)> = name
        .char_indices()
        .flat_map(|(start, character)| {
            let span = start..start + character.len_utf8();
            character
                .to_lowercase()
                .map(move |lower| (span.clone(), lower))
        })
        .collect();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index + needle.len() <= folded.len() {
        let window = &folded[index..index + needle.len()];
        if window
            .iter()
            .zip(&needle)
            .all(|((_, character), wanted)| character == wanted)
        {
            ranges.push(window[0].0.start..window[needle.len() - 1].0.end);
            index += needle.len();
        } else {
            index += 1;
        }
    }
    ranges
}

fn highlight_attributes(text: &str, query: &str) -> Option<gtk::pango::AttrList> {
    let ranges = match_ranges(text, query);
    if ranges.is_empty() {
        return None;
    }
    // A solid accent segment stays visible over the translucent accent of
    // selected rows. Before any theme is applied, bold still marks the match.
    let attributes = gtk::pango::AttrList::new();
    for range in ranges {
        let (start, end) = (range.start as u32, range.end as u32);
        let mut segment: Vec<gtk::pango::Attribute> = match HIGHLIGHT_COLORS.get() {
            Some(((ar, ag, ab), (br, bg, bb))) => vec![
                gtk::pango::AttrColor::new_background(ar, ag, ab).into(),
                gtk::pango::AttrColor::new_foreground(br, bg, bb).into(),
            ],
            None => vec![gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold).into()],
        };
        for attribute in &mut segment {
            attribute.set_start_index(start);
            attribute.set_end_index(end);
        }
        for attribute in segment {
            attributes.insert(attribute);
        }
    }
    Some(attributes)
}

pub(in crate::ui) fn highlight_name(widget: &gtk::Widget, query: Option<&str>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        let attributes = query.and_then(|query| highlight_attributes(&label.text(), query));
        if attributes.is_some() || label.attributes().is_some() {
            label.set_attributes(attributes.as_ref());
        }
    } else if let Some(label) = widget.downcast_ref::<gtk::Inscription>() {
        let attributes = query.and_then(|query| {
            label
                .text()
                .and_then(|text| highlight_attributes(&text, query))
        });
        if attributes.is_some() || label.attributes().is_some() {
            label.set_attributes(attributes.as_ref());
        }
    }
}

impl ViewState {
    pub(in crate::ui) fn find_highlight(&self) -> Option<String> {
        let find = self.find.borrow();
        (find.highlighted && !find.query.is_empty()).then(|| find.query.clone())
    }

    fn refresh_find_highlights(&self) {
        let query = self.find_highlight();
        for column in self.columns.borrow().iter() {
            for bound in column.bound_rows.borrow().iter() {
                if let Some(label) = bound.rename_label.upgrade() {
                    highlight_name(label.upcast_ref(), query.as_deref());
                }
            }
        }
        self.mode_views
            .borrow()
            .visit_name_labels(|label| highlight_name(label, query.as_deref()));
    }
}

impl BrowserView {
    /// Commits `query` and moves the cursor to its next match (previous when
    /// `backward`). A miss leaves the cursor where it is. Returns whether a match
    /// was found.
    pub(in crate::ui) fn find(&self, query: &str, backward: bool, listing_focused: bool) -> bool {
        if query.is_empty() {
            return false;
        }
        register_find_view(&self.state);
        self.state.find.replace(FindState {
            query: query.to_owned(),
            backward,
            highlighted: true,
        });
        self.state.refresh_find_highlights();
        self.find_next(query, backward, listing_focused)
    }

    /// Repeats the last committed find, reversed for **N**. `None` when there is
    /// no query to repeat.
    pub(in crate::ui) fn repeat_find(&self, reverse: bool, listing_focused: bool) -> Option<bool> {
        let (query, backward) = {
            let mut find = self.state.find.borrow_mut();
            if find.query.is_empty() {
                return None;
            }
            find.highlighted = true;
            (find.query.clone(), find.backward != reverse)
        };
        self.state.refresh_find_highlights();
        Some(self.find_next(&query, backward, listing_focused))
    }

    pub(in crate::ui) fn find_query(&self) -> Option<String> {
        let find = self.state.find.borrow();
        (!find.query.is_empty()).then(|| find.query.clone())
    }

    /// Dismisses retained highlights; the query stays repeatable. Returns whether
    /// any were showing.
    pub(in crate::ui) fn dismiss_find_highlight(&self) -> bool {
        let dismissed = std::mem::replace(&mut self.state.find.borrow_mut().highlighted, false);
        if dismissed {
            self.state.refresh_find_highlights();
        }
        dismissed
    }

    pub(in crate::ui) fn clear_find(&self) {
        let had_highlight = self.state.find.replace(FindState::default()).highlighted;
        if had_highlight {
            self.state.refresh_find_highlights();
        }
    }

    /// Moves the cursor by `direction` in the displayed listing without taking
    /// keyboard focus, for **Up** / **Down** in a footer prompt.
    pub(in crate::ui) fn step_cursor_unfocused(&self, direction: i32) {
        if self.step_filter_results(direction, 1, false) {
            return;
        }
        let Some(depth) = self.focused_listing_depth() else {
            return;
        };
        let Some(order) = self.displayed_order(depth) else {
            return;
        };
        let current = self.cursor_index(depth, &order);
        let index = match current {
            Some(index) => index
                .saturating_add_signed(direction as isize)
                .min(order.len() - 1),
            None if direction < 0 => order.len() - 1,
            None => 0,
        };
        if Some(index) != current {
            self.place_found_cursor(depth, order[index], &order, false);
        }
    }

    fn find_next(&self, query: &str, backward: bool, listing_focused: bool) -> bool {
        if let Some(target) = self.filter_target()
            && target.results_view().is_some()
        {
            let Some(results) = target.results() else {
                return false;
            };
            let Some(hits) = target.hits() else {
                return false;
            };
            let len = results.len();
            if len == 0 {
                return false;
            }
            let current = hits.cursor.map(|position| position as usize);
            let found = (1..=len)
                .map(|step| match (current, backward) {
                    (Some(index), false) => (index + step) % len,
                    (Some(index), true) => (index + len - step % len) % len,
                    (None, false) => step - 1,
                    (None, true) => len - step,
                })
                .find(|&index| !match_ranges(&results[index].name, query).is_empty());
            let Some(index) = found else {
                return false;
            };
            self.place_found_result(index as u32, listing_focused);
            return true;
        }
        let Some(depth) = self.focused_listing_depth() else {
            return false;
        };
        let Some(order) = self.displayed_order(depth) else {
            return false;
        };
        let len = order.len();
        let current = self.cursor_index(depth, &order);
        let found = (1..=len)
            .map(|step| match (current, backward) {
                (Some(index), false) => (index + step) % len,
                (Some(index), true) => (index + len - step % len) % len,
                (None, false) => step - 1,
                (None, true) => len - step,
            })
            .map(|index| order[index])
            .find(|&position| {
                self.state
                    .browser
                    .entry_at(depth, position)
                    .is_some_and(|entry| !match_ranges(&entry.display_name, query).is_empty())
            });
        let Some(position) = found else {
            return false;
        };
        self.place_found_cursor(depth, position, &order, listing_focused);
        true
    }

    fn cursor_index(&self, depth: usize, order: &[usize]) -> Option<usize> {
        let (_, position, _) = self
            .state
            .browser
            .focused_item()
            .filter(|(focused_depth, _, _)| *focused_depth == depth)?;
        order.iter().position(|&candidate| candidate == position)
    }

    /// `take_focus` lets the listing's keyboard focus follow the cursor; a footer
    /// prompt keeps it otherwise.
    fn place_found_cursor(&self, depth: usize, position: usize, order: &[usize], take_focus: bool) {
        self.keyboard_navigation();
        let keep_focus = !take_focus;
        self.state.cursor_keeps_focus.set(keep_focus);
        self.state
            .mode_views
            .borrow()
            .set_cursor_keeps_focus(keep_focus);
        self.state
            .browser
            .place_cursor(depth, position, Some(order));
        self.state.cursor_keeps_focus.set(false);
        self.state.mode_views.borrow().set_cursor_keeps_focus(false);
        let target = if self.view_mode() == BrowserMode::Columns {
            let columns = self.state.columns.borrow();
            columns.get(depth).and_then(|column| {
                Some((
                    column.list.clone().upcast::<gtk::Widget>(),
                    column.map.view_position(position)?,
                ))
            })
        } else {
            self.state.mode_views.borrow().cursor_view(depth, position)
        };
        let Some((view, view_position)) = target else {
            return;
        };
        let flags = if take_focus {
            gtk::ListScrollFlags::FOCUS
        } else {
            gtk::ListScrollFlags::NONE
        };
        if let Some(list) = view.downcast_ref::<gtk::ListView>() {
            list.scroll_to(view_position, flags, None);
        } else if let Some(grid) = view.downcast_ref::<gtk::GridView>() {
            grid.scroll_to(view_position, flags, None);
        }
    }
}

#[cfg(test)]
mod tests;
