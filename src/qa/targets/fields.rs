// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};

use super::{
    super::{protocol::FieldRef, state},
    LookupError, Memory, accessible_name, presented, role_name,
};

/// A GTK editable has focus itself or, for an entry, through its inner text.
pub(crate) fn editable_focused(widget: &gtk::Widget) -> bool {
    let Some(focus) = widget
        .root()
        .and_then(|root| root.focus())
        .filter(WidgetExt::has_focus)
    else {
        return false;
    };
    focus == *widget
        || widget
            .dynamic_cast_ref::<gtk::Editable>()
            .and_then(EditableExt::delegate)
            .is_some_and(|delegate| delegate.upcast::<gtk::Widget>() == focus)
}

fn is_field(widget: &gtk::Widget) -> bool {
    role_name(widget) == "text"
        && widget
            .dynamic_cast_ref::<gtk::Editable>()
            .is_some_and(EditableExt::is_editable)
}

pub(crate) fn editable_fields(window: &gtk::Window) -> usize {
    presented(window.upcast_ref())
        .iter()
        .filter(|widget| is_field(widget) && state::rendered_bounds(widget).is_some())
        .count()
}

pub(crate) fn editable_texts(window: &gtk::Window) -> Vec<String> {
    presented(window.upcast_ref())
        .iter()
        .filter(|widget| is_field(widget) && state::rendered_bounds(widget).is_some())
        .filter_map(text_content)
        .collect()
}

pub(crate) fn focused_field(presented: &[gtk::Widget]) -> Option<gtk::Widget> {
    presented
        .iter()
        .find(|widget| {
            is_field(widget) && editable_focused(widget) && state::rendered_bounds(widget).is_some()
        })
        .cloned()
}

pub(crate) fn field(
    window: &gtk::Window,
    memory: &Memory,
    field: &FieldRef,
) -> Result<Option<gtk::Widget>, LookupError> {
    if let Some(remembered) = &field.of {
        return memory
            .get(remembered)
            .map(glib::WeakRef::upgrade)
            .ok_or_else(|| {
                LookupError::Broken(format!("no field was remembered as {remembered:?}"))
            });
    }
    let everything = presented(window.upcast_ref());
    Ok(match &field.name {
        Some(name) => everything.into_iter().find(|widget| {
            is_field(widget)
                && state::rendered_bounds(widget).is_some()
                && accessible_name(widget) == *name
        }),
        None => focused_field(&everything),
    })
}

pub(crate) fn text_content(widget: &gtk::Widget) -> Option<String> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        return Some(label.text().to_string());
    }
    if let Some(inscription) = widget.downcast_ref::<gtk::Inscription>() {
        return inscription.text().map(|text| text.to_string());
    }
    if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
        let buffer = view.buffer();
        let (start, end) = buffer.bounds();
        return Some(buffer.text(&start, &end, false).to_string());
    }
    widget
        .dynamic_cast_ref::<gtk::Editable>()
        .map(|editable| editable.text().to_string())
}
