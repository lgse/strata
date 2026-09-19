// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

use super::{ACTION_EXAMPLES, ActionExample, EditorForm};
use crate::{assets, ui::accessibility};

pub(super) fn button() -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&assets::primary_icon(assets::icons::LIBRARY, 15));
    content.append(&gtk::Label::new(Some("Library")));
    let button = gtk::MenuButton::builder().child(&content).build();
    button.add_css_class("settings-action-library-button");
    accessibility::set_label(&button, "Library");
    button.set_tooltip_text(Some("Choose a bundled script template"));
    button
}

pub(super) fn install(button: &gtk::MenuButton, form: Weak<EditorForm>) {
    let body = accessibility::pane_box();
    accessibility::set_label(&body, "Script library");
    let popover = gtk::Popover::builder()
        .child(&body)
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::End)
        .build();
    popover.add_css_class("settings-action-library");
    let (search_field, search, clear) = super::super::search_field("Search templates");
    search_field.add_css_class("action-library-search");
    body.append(&search_field);
    let filters = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    filters.add_css_class("action-library-filters");
    body.append(&filters);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.add_css_class("action-library-list");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(270)
        .build();
    scroll.add_css_class("context-menu-scroll");
    body.append(&scroll);
    let empty = note("No templates match your search.");
    list.append(&empty);
    let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 8);
    confirmation.add_css_class("action-library-footer");
    confirmation.set_visible(false);
    let question = note("");
    confirmation.append(&question);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Keep draft");
    let replace = gtk::Button::with_label("Replace script");
    for button in [&cancel, &replace] {
        button.add_css_class("settings-action-button");
        actions.append(button);
    }
    confirmation.append(&actions);
    body.append(&confirmation);
    let pending = Rc::new(Cell::new(None::<&'static ActionExample>));
    let select = Rc::new({
        let form = form.clone();
        let popover = popover.downgrade();
        let pending = pending.clone();
        let confirmation = confirmation.clone();
        move |example: &'static ActionExample| {
            let (Some(form), Some(popover)) = (form.upgrade(), popover.upgrade()) else {
                return;
            };
            if form.replaces_draft(example.runtime()) {
                pending.set(Some(example));
                question.set_text(&format!(
                    "Replace your {} draft with {}? Code can be undone in the editor.",
                    example.runtime().label(),
                    example.name
                ));
                confirmation.set_visible(true);
            } else {
                popover.popdown();
                form.apply_example(example);
            }
        }
    });
    let rows: Vec<_> = ACTION_EXAMPLES
        .iter()
        .map(|example| {
            let row = template_row(example);
            let select = select.clone();
            row.connect_clicked(move |_| select(example));
            list.append(&row);
            (example, row)
        })
        .collect();
    let rows = Rc::new(rows);
    let category = Rc::new(Cell::new("All"));
    let update: Rc<dyn Fn()> = Rc::new({
        let search = search.downgrade();
        let category = category.clone();
        let rows = rows.clone();
        let confirmation = confirmation.clone();
        let pending = pending.clone();
        move || {
            let Some(search) = search.upgrade() else {
                return;
            };
            let query = search.text().to_lowercase();
            let mut any = false;
            for (example, row) in rows.iter() {
                let matches = matches(example, &query, category.get());
                row.set_visible(matches);
                any |= matches;
            }
            empty.set_visible(!any);
            clear.set_visible(!query.is_empty());
            pending.set(None);
            confirmation.set_visible(false);
        }
    });
    let changed = update.clone();
    search.connect_changed(move |_| changed());
    let mut categories = vec!["All"];
    for example in ACTION_EXAMPLES {
        if !categories.contains(&example.category) {
            categories.push(example.category);
        }
    }
    let mut group = None::<gtk::ToggleButton>;
    for name in categories {
        let chip = gtk::ToggleButton::with_label(name);
        chip.add_css_class("action-library-category");
        chip.set_group(group.as_ref());
        if group.is_none() {
            chip.set_active(true);
            group = Some(chip.clone());
        }
        let category = category.clone();
        let update = update.clone();
        chip.connect_toggled(move |chip| {
            if chip.is_active() {
                category.set(name);
                update();
            }
        });
        filters.append(&chip);
    }
    update();
    let select_first = {
        let rows = rows.clone();
        move || {
            if let Some((_, row)) = rows.iter().find(|(_, row)| row.is_visible()) {
                row.emit_clicked();
            }
        }
    };
    search.connect_activate(move |_| select_first());
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_popover = popover.downgrade();
    let weak_search = search.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Down
            && weak_search.upgrade().is_some_and(|search| {
                search
                    .root()
                    .and_then(|root| root.focus())
                    .is_some_and(|focus| focus == search || focus.is_ancestor(&search))
            })
        {
            if let Some((_, row)) = rows.iter().find(|(_, row)| row.is_visible()) {
                row.grab_focus();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    body.add_controller(keys);
    let weak_popover = popover.downgrade();
    let accept = pending.clone();
    replace.connect_clicked(move |_| {
        if let (Some(example), Some(form), Some(popover)) =
            (accept.get(), form.upgrade(), weak_popover.upgrade())
        {
            popover.popdown();
            form.apply_example(example);
        }
    });
    let keep = pending.clone();
    let weak_confirmation = confirmation.downgrade();
    cancel.connect_clicked(move |_| {
        keep.set(None);
        if let Some(confirmation) = weak_confirmation.upgrade() {
            confirmation.set_visible(false);
        }
    });
    popover.connect_closed(move |_| {
        pending.set(None);
        confirmation.set_visible(false);
    });
    popover.connect_show(move |_| {
        search.grab_focus();
        search.select_region(0, -1);
    });
    button.set_popover(Some(&popover));
}

fn matches(example: &ActionExample, query: &str, category: &str) -> bool {
    if category != "All" && example.category != category {
        return false;
    }
    let text = format!(
        "{} {} {} {} {} {}",
        example.name,
        example.description,
        example.requirements,
        example.category,
        example.inputs,
        example.runtime().label()
    )
    .to_lowercase();
    query.split_whitespace().all(|word| text.contains(word))
}

fn template_row(example: &ActionExample) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(example.name));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("action-library-title");
    let language = gtk::Label::new(Some(example.runtime().label()));
    language.add_css_class("action-library-language");
    language.set_valign(gtk::Align::Center);
    heading.append(&title);
    heading.append(&language);
    content.append(&heading);
    content.append(&note(example.description));
    content.append(&note(example.inputs));
    let requirements = note(&format!("Requires: {}", example.requirements));
    requirements.add_css_class("action-library-requirements");
    content.append(&requirements);
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("column-menu-option");
    button.add_css_class("action-library-template");
    accessibility::set_label(&button, example.name);
    button.set_tooltip_text(Some(&format!(
        "{} · {}",
        example.requirements, example.inputs
    )));
    button
}

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(52);
    label.add_css_class("settings-option-description");
    label
}
