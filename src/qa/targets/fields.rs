// SPDX-License-Identifier: MIT

use gtk::prelude::*;

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
