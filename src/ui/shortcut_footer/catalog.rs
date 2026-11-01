// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::ui::{
    keys::{self, Keys, bindings},
    shortcut_reference::{ReferenceScope, Shortcut},
};

/// The reference's results, categories and their shared selection.
pub(super) struct ReferenceView {
    pub(super) reference: gtk::Box,
    pub(super) categories: gtk::Box,
    pub(super) search: gtk::Entry,
    pub(super) scroll: gtk::ScrolledWindow,
    pub(super) selected: Rc<RefCell<String>>,
    pub(super) scope: Rc<Cell<ReferenceScope>>,
}

#[derive(Clone)]
pub(super) struct WeakReferenceView {
    reference: glib::WeakRef<gtk::Box>,
    categories: glib::WeakRef<gtk::Box>,
    search: glib::WeakRef<gtk::Entry>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    selected: Rc<RefCell<String>>,
    scope: Rc<Cell<ReferenceScope>>,
}

impl ReferenceView {
    pub(super) fn downgrade(&self) -> WeakReferenceView {
        WeakReferenceView {
            reference: self.reference.downgrade(),
            categories: self.categories.downgrade(),
            search: self.search.downgrade(),
            scroll: self.scroll.downgrade(),
            selected: self.selected.clone(),
            scope: self.scope.clone(),
        }
    }
}

impl WeakReferenceView {
    pub(super) fn upgrade(&self) -> Option<ReferenceView> {
        Some(ReferenceView {
            reference: self.reference.upgrade()?,
            categories: self.categories.upgrade()?,
            search: self.search.upgrade()?,
            scroll: self.scroll.upgrade()?,
            selected: self.selected.clone(),
            scope: self.scope.clone(),
        })
    }

    pub(super) fn reference_alive(&self) -> bool {
        self.reference.upgrade().is_some()
    }
}

/// What the results list shows.
pub(super) struct ReferenceQuery<'a> {
    pub(super) scope: ReferenceScope,
    pub(super) selected: &'a str,
    pub(super) query: &'a str,
    pub(super) compact: bool,
}

impl ReferenceQuery<'_> {
    fn shows(&self, title: &str) -> bool {
        self.selected == "All" || self.selected == title
    }
}

pub(super) fn reference_note() -> String {
    use bindings::{
        DOWN, ESCAPE, FOCUS_NEXT, REFERENCE_CATEGORIES, REFERENCE_LIST, REFERENCE_SEARCH, UP,
    };
    let label = |shown| keys::label(&shown);
    rust_i18n::t!(
        "%{category_keys} categories · %{search_keys} search · %{list_keys} list · %{move_keys} move · %{cycle_keys} cycle · %{close_keys} close",
        category_keys = label(Keys::One(REFERENCE_CATEGORIES.shown())),
        search_keys = label(Keys::One(REFERENCE_SEARCH.shown())),
        list_keys = label(Keys::One(REFERENCE_LIST.shown())),
        move_keys = label(Keys::Any(crate::ui::keys::shown![
            DOWN.alternate(1),
            UP.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0)
        ])),
        cycle_keys = label(Keys::One(FOCUS_NEXT.shown())),
        close_keys = label(Keys::One(ESCAPE.shown()))
    )
    .into_owned()
}

/// Keeps each hint, with the separator that follows it, on one line so the note wraps
/// only after a "·", never inside a hint such as "j / k / ↑ / ↓ move".
pub(super) fn hint_note_attributes(note: &str) -> gtk::pango::AttrList {
    const SEPARATOR: &str = " · ";
    let attributes = gtk::pango::AttrList::new();
    let mut start = 0;
    while start < note.len() {
        let end = note[start..].find(SEPARATOR).map_or(note.len(), |offset| {
            start + offset + SEPARATOR.trim_end().len()
        });
        attributes.insert(crate::ui::controls::no_break_attribute(start..end));
        start = end + 1;
    }
    attributes
}

pub(super) fn category_buttons(categories: &gtk::Box) -> Vec<gtk::Button> {
    let mut buttons = Vec::new();
    let mut child = categories.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let mut item = row.first_child();
        while let Some(widget) = item {
            item = widget.next_sibling();
            if let Ok(button) = widget.downcast::<gtk::Button>() {
                buttons.push(button);
            }
        }
    }
    buttons
}

fn append_category_row(categories: &gtk::Box) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_vexpand(false);
    categories.append(&row);
    row
}

pub(super) fn reflow_categories(categories: &gtk::Box, compact: bool, width: i32) {
    let buttons = category_buttons(categories);
    let focused = buttons.iter().position(|button| button.has_focus());
    for button in &buttons {
        if let Some(row) = button.parent().and_downcast::<gtk::Box>() {
            row.remove(button);
        }
    }
    while let Some(row) = categories.first_child() {
        categories.remove(&row);
    }
    pack_category_rows(categories, &buttons, compact, width);
    if let Some(index) = focused {
        buttons[index].grab_focus();
    }
}

/// Fills rows left to right; outside compact mode each button gets its own row.
fn pack_category_rows(categories: &gtk::Box, buttons: &[gtk::Button], compact: bool, width: i32) {
    let mut row = append_category_row(categories);
    let mut occupied = 0;
    for button in buttons {
        let natural = button.measure(gtk::Orientation::Horizontal, -1).1;
        let needed = if occupied == 0 {
            natural
        } else {
            occupied + 8 + natural
        };
        let overflows = !compact || needed > width;
        if occupied > 0 && overflows {
            row = append_category_row(categories);
            occupied = natural;
        } else {
            occupied = needed;
        }
        row.append(button);
    }
}

pub(super) fn rebuild_reference(view: &ReferenceView) {
    let categories = &view.categories;
    while let Some(child) = categories.first_child() {
        categories.remove(&child);
    }
    let sections = crate::ui::shortcut_reference::reference_sections(view.scope.get());
    if !sections
        .iter()
        .any(|section| section.title == view.selected.borrow().as_str())
    {
        *view.selected.borrow_mut() = String::from("All");
    }
    let total: usize = sections.iter().map(|section| section.rows.len()).sum();
    for (name, count) in std::iter::once(("All", total)).chain(
        sections
            .iter()
            .map(|section| (section.title, section.rows.len())),
    ) {
        let button = category_button(view, name, count);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_vexpand(false);
        row.append(&button);
        categories.append(&row);
    }
    if categories.has_css_class("compact") {
        let width = categories
            .root()
            .and_downcast::<gtk::Window>()
            .map_or(400, |window| (window.width() - 104).max(1));
        reflow_categories(categories, true, width);
    }
    render_reference(
        &view.reference,
        &ReferenceQuery {
            scope: view.scope.get(),
            selected: &view.selected.borrow(),
            query: &view.search.text(),
            compact: view.reference.has_css_class("compact"),
        },
    );
}

fn category_button(view: &ReferenceView, name: &'static str, count: usize) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("shortcut-reference-category");
    let compact = view.categories.has_css_class("compact");
    if compact {
        button.add_css_class("action-library-category");
    }
    let is_selected = *view.selected.borrow() == name;
    if is_selected {
        button.add_css_class("selected");
    }
    button.update_state(&[gtk::accessible::State::Selected(Some(is_selected))]);
    button.set_child(Some(&category_line(name, count, compact)));
    button.set_hexpand(!compact);
    button.set_halign(if compact {
        gtk::Align::Start
    } else {
        gtk::Align::Fill
    });
    let weak_view = view.downgrade();
    button.connect_clicked(move |button| {
        *weak_view.selected.borrow_mut() = name.to_owned();
        if let Some(view) = weak_view.upgrade() {
            select_category(&view, button, name);
        }
    });
    button
}

fn category_line(name: &str, count: usize, compact: bool) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(&crate::i18n::tr(name)));
    label.set_hexpand(!compact);
    label.set_xalign(0.0);
    line.append(&label);
    let amount = gtk::Label::new(Some(&count.to_string()));
    amount.add_css_class("shortcut-reference-count");
    amount.set_visible(!compact);
    line.append(&amount);
    line
}

fn select_category(view: &ReferenceView, button: &gtk::Button, name: &str) {
    for category in category_buttons(&view.categories) {
        category.remove_css_class("selected");
        category.update_state(&[gtk::accessible::State::Selected(Some(false))]);
    }
    button.add_css_class("selected");
    button.update_state(&[gtk::accessible::State::Selected(Some(true))]);
    render_reference(
        &view.reference,
        &ReferenceQuery {
            scope: view.scope.get(),
            selected: name,
            query: &view.search.text(),
            compact: view.reference.has_css_class("compact"),
        },
    );
    view.scroll.vadjustment().set_value(0.0);
}

pub(super) fn render_reference(reference: &gtk::Box, query: &ReferenceQuery<'_>) {
    while let Some(child) = reference.first_child() {
        reference.remove(&child);
    }
    let folded = crate::ui::shortcut_reference::search_folded(query.query.trim());
    let mut found = false;
    for section in crate::ui::shortcut_reference::reference_sections(query.scope) {
        if !query.shows(section.title) {
            continue;
        }
        let rows: Vec<_> = section
            .rows
            .into_iter()
            .filter(|shortcut| shortcut.matches(&folded))
            .collect();
        if !rows.is_empty() {
            found = true;
            append_section(reference, section.title, &rows, query.compact);
        }
    }
    if found {
        reference.set_vexpand(false);
    } else {
        show_no_matches(reference);
    }
}

fn show_no_matches(reference: &gtk::Box) {
    let empty = gtk::Label::new(Some(&crate::i18n::tr("No matching shortcuts")));
    empty.add_css_class("shortcut-reference-note");
    empty.set_wrap(true);
    empty.set_justify(gtk::Justification::Center);
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_vexpand(true);
    empty.set_hexpand(true);
    reference.set_vexpand(true);
    reference.append(&empty);
}

fn append_section(parent: &gtk::Box, title: &str, shortcuts: &[Shortcut], compact: bool) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 16);
    section.append(&section_heading(title, shortcuts.len()));
    let grid = gtk::Grid::new();
    grid.set_column_spacing(48);
    grid.set_row_spacing(12);
    for (index, shortcut) in shortcuts.iter().enumerate() {
        let (column, row) = if compact {
            (0, index as i32)
        } else {
            ((index % 2) as i32, (index / 2) as i32)
        };
        grid.attach(&shortcut_row(shortcut, compact), column, row, 1, 1);
    }
    section.append(&grid);
    parent.append(&section);
}

fn section_heading(title: &str, count: usize) -> gtk::Box {
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&crate::i18n::tr(title).to_uppercase()));
    label.add_css_class("shortcut-reference-heading");
    heading.append(&label);
    let count = gtk::Label::new(Some(&count.to_string()));
    count.add_css_class("shortcut-reference-count");
    heading.append(&count);
    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.set_hexpand(true);
    divider.set_valign(gtk::Align::Center);
    heading.append(&divider);
    heading
}

fn shortcut_row(shortcut: &Shortcut, compact: bool) -> gtk::Box {
    let row = gtk::Box::new(
        if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        },
        6,
    );
    row.add_css_class("shortcut-reference-row");
    let action = gtk::Label::builder()
        .label(shortcut.action_text())
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    action.add_css_class("shortcut-reference-description");
    if compact {
        action.set_max_width_chars(22);
    }
    row.append(&action);
    let keys = gtk::Label::new(Some(&shortcut.key_text()));
    keys.add_css_class("shortcut-reference-key");
    keys.set_halign(gtk::Align::Start);
    keys.set_wrap(compact);
    keys.set_max_width_chars(if compact { 22 } else { 32 });
    row.append(&keys);
    row
}
