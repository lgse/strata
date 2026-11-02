// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

use super::{ShortcutFooter, catalog::category_buttons, reference::ReferenceArea};
use crate::ui::keys::{
    KeyPress,
    bindings::{
        DOWN, END_KEY, ENTER_ANY_SHIFT, ESCAPE_ANY, FOCUS_NEXT, FOCUS_PREVIOUS, HOME_KEY, LEFT,
        PAGE_DOWN_KEY, PAGE_UP_KEY, REFERENCE, REFERENCE_CATEGORIES, REFERENCE_LIST,
        REFERENCE_SEARCH, RIGHT, SPACE, UP,
    },
};

const SHIFT: gdk::ModifierType = gdk::ModifierType::SHIFT_MASK;

/// Which reference area holds keyboard focus.
struct AreaFocus {
    search: bool,
    categories: bool,
    results: bool,
}

impl AreaFocus {
    fn area(&self) -> Option<ReferenceArea> {
        if self.search {
            Some(ReferenceArea::Search)
        } else if self.categories {
            Some(ReferenceArea::Categories)
        } else if self.results {
            Some(ReferenceArea::Results)
        } else {
            None
        }
    }

    /// The results scroll from the list itself, or from search with vertical keys.
    fn scrolls_results(&self, press: &KeyPress) -> bool {
        if self.results {
            return true;
        }
        self.search && search_scroll_key(press)
    }
}

fn search_scroll_key(press: &KeyPress) -> bool {
    press.is_any(&[UP.ignoring(SHIFT), DOWN.ignoring(SHIFT)])
        || press.is_any(&[PAGE_UP_KEY.ignoring(SHIFT), PAGE_DOWN_KEY.ignoring(SHIFT)])
}

fn focus_within(focus: Option<&gtk::Widget>, widget: &impl IsA<gtk::Widget>) -> bool {
    focus.is_some_and(|focus| {
        focus == widget.upcast_ref::<gtk::Widget>() || focus.is_ancestor(widget)
    })
}

fn category_step(press: &KeyPress) -> Option<isize> {
    if press.is_any(&[
        UP.ignoring(SHIFT).or_alternates(),
        LEFT.ignoring(SHIFT).or_alternates(),
    ]) {
        Some(-1)
    } else if press.is_any(&[
        DOWN.ignoring(SHIFT).or_alternates(),
        RIGHT.ignoring(SHIFT).or_alternates(),
    ]) {
        Some(1)
    } else if press.is(HOME_KEY.ignoring(SHIFT)) {
        Some(-100)
    } else if press.is(END_KEY.ignoring(SHIFT)) {
        Some(100)
    } else {
        None
    }
}

enum ScrollMotion {
    Step(f64),
    Page(f64),
    Start,
    End,
}

fn scroll_motion(press: &KeyPress) -> Option<ScrollMotion> {
    if press.is_any(&[UP.ignoring(SHIFT).or_alternates(), LEFT.ignoring(SHIFT)]) {
        Some(ScrollMotion::Step(-1.0))
    } else if press.is_any(&[DOWN.ignoring(SHIFT).or_alternates(), RIGHT.ignoring(SHIFT)]) {
        Some(ScrollMotion::Step(1.0))
    } else if press.is(PAGE_UP_KEY.ignoring(SHIFT)) {
        Some(ScrollMotion::Page(-1.0))
    } else if press.is(PAGE_DOWN_KEY.ignoring(SHIFT)) {
        Some(ScrollMotion::Page(1.0))
    } else if press.is(HOME_KEY.ignoring(SHIFT)) {
        Some(ScrollMotion::Start)
    } else if press.is(END_KEY.ignoring(SHIFT)) {
        Some(ScrollMotion::End)
    } else {
        None
    }
}

/// Line and page steps, falling back to fractions of the page when unset.
fn scroll_steps(adjustment: &gtk::Adjustment, page: f64) -> (f64, f64) {
    let step = if adjustment.step_increment() >= 1.0 {
        adjustment.step_increment()
    } else {
        page / 10.0
    };
    let page_step = if adjustment.page_increment() >= 1.0 {
        adjustment.page_increment()
    } else {
        page
    };
    (step, page_step)
}

impl ShortcutFooter {
    pub(in crate::ui) fn reference_is_open(&self) -> bool {
        self.session.is_open()
    }

    pub(in crate::ui) fn handle_key(&self, press: &KeyPress) -> Option<glib::Propagation> {
        let reference_open = self.session.is_open();
        if self.press_toggles_reference(press, reference_open) {
            if reference_open {
                self.session.close();
            } else {
                self.session.open(&self.root);
            }
            return Some(glib::Propagation::Stop);
        }
        if !reference_open {
            return None;
        }
        if press.is(ESCAPE_ANY) {
            self.session.close();
            return Some(glib::Propagation::Stop);
        }
        if self.focus_area_for(press) {
            return Some(glib::Propagation::Stop);
        }
        Some(self.handle_reference_key(press))
    }

    /// A key that types, as 10xer's ~ does, toggles only where it would not type.
    fn press_toggles_reference(&self, press: &KeyPress, reference_open: bool) -> bool {
        if press.text().is_some() {
            self.typed_key_toggles(press, reference_open)
        } else {
            press.is(REFERENCE)
        }
    }

    fn focus_area_for(&self, press: &KeyPress) -> bool {
        if press.is(REFERENCE_CATEGORIES) {
            self.focus_category()
        } else if press.is(REFERENCE_SEARCH) {
            self.focus_search()
        } else if press.is(REFERENCE_LIST) {
            self.focus_results()
        } else {
            false
        }
    }

    fn area_focus(&self) -> AreaFocus {
        let focus = self.search.root().and_then(|root| root.focus());
        AreaFocus {
            search: focus_within(focus.as_ref(), &self.search),
            categories: focus_within(focus.as_ref(), &self.sidebar),
            results: focus_within(focus.as_ref(), &self.scroll),
        }
    }

    fn handle_reference_key(&self, press: &KeyPress) -> glib::Propagation {
        let focus = self.area_focus();
        if press.is_any(&[FOCUS_NEXT, FOCUS_PREVIOUS]) {
            let current = focus.area().unwrap_or_else(|| self.active_area.get());
            self.cycle_area(current, press.is(FOCUS_PREVIOUS));
            return glib::Propagation::Stop;
        }
        if focus.categories
            && let Some(result) = self.category_key(press)
        {
            return result;
        }
        if focus.scrolls_results(press) && self.scroll_reference(press) {
            return glib::Propagation::Stop;
        }
        if focus.search {
            return glib::Propagation::Proceed;
        }
        // No browsing shortcut may operate on files while the reference is open.
        glib::Propagation::Stop
    }

    fn cycle_area(&self, current: ReferenceArea, backwards: bool) {
        match (current, backwards) {
            (ReferenceArea::Search, false) | (ReferenceArea::Results, true) => {
                self.focus_category();
            }
            (ReferenceArea::Categories, false) | (ReferenceArea::Search, true) => {
                self.focus_results();
            }
            (ReferenceArea::Results, false) | (ReferenceArea::Categories, true) => {
                self.focus_search();
            }
        }
    }

    fn category_key(&self, press: &KeyPress) -> Option<glib::Propagation> {
        if let Some(direction) = category_step(press) {
            self.move_category(direction);
            return Some(glib::Propagation::Stop);
        }
        (press.is(ENTER_ANY_SHIFT) || press.is(SPACE.ignoring(SHIFT)))
            .then_some(glib::Propagation::Proceed)
    }

    pub(super) fn category_buttons(&self) -> Vec<gtk::Button> {
        category_buttons(&self.categories)
    }

    fn focus_search(&self) -> bool {
        let focused = self.search.grab_focus();
        if focused {
            self.active_area.set(ReferenceArea::Search);
        }
        focused
    }

    fn focus_results(&self) -> bool {
        let focused = self.scroll.grab_focus();
        if focused {
            self.active_area.set(ReferenceArea::Results);
        }
        focused
    }

    fn focus_category(&self) -> bool {
        let focused = self
            .category_buttons()
            .into_iter()
            .find(|button| button.has_css_class("selected"))
            .is_some_and(|button| button.grab_focus());
        if focused {
            self.active_area.set(ReferenceArea::Categories);
        }
        focused
    }

    fn move_category(&self, delta: isize) {
        let buttons = self.category_buttons();
        let Some(index) = buttons
            .iter()
            .position(|button| button.has_focus())
            .or_else(|| {
                buttons
                    .iter()
                    .position(|button| button.has_css_class("selected"))
            })
        else {
            return;
        };
        let next = (index as isize + delta).clamp(0, buttons.len() as isize - 1) as usize;
        buttons[next].grab_focus();
        buttons[next].emit_clicked();
    }

    fn scroll_reference(&self, press: &KeyPress) -> bool {
        // h and l do not scroll; j and k still match below as alternates.
        let press = &press.untyped();
        let adjustment = self.scroll.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let (step, page_step) = scroll_steps(&adjustment, page);
        let delta = match scroll_motion(press) {
            Some(ScrollMotion::Step(direction)) => direction * step,
            Some(ScrollMotion::Page(direction)) => direction * page_step,
            Some(ScrollMotion::Start) => {
                adjustment.set_value(adjustment.lower());
                return true;
            }
            Some(ScrollMotion::End) => {
                adjustment.set_value((adjustment.upper() - page).max(adjustment.lower()));
                return true;
            }
            None => return false,
        };
        let limit = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value((adjustment.value() + delta).clamp(adjustment.lower(), limit));
        true
    }

    fn typed_key_toggles(&self, press: &KeyPress, reference_open: bool) -> bool {
        if !press.is(REFERENCE) {
            return false;
        }
        if reference_open {
            !(press.used_fallback() && self.typing_has_focus())
        } else {
            !self.prompt_has_focus() && !self.typing_has_focus()
        }
    }

    fn typing_has_focus(&self) -> bool {
        self.root
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| crate::ui::focus_navigation::accepts_typing(&focus))
    }
}
