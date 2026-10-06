// SPDX-License-Identifier: MIT

use super::*;

mod acceptance;
mod column_widths;
mod filtered_preview;
mod image_conversion;
mod keyboard;
mod sizing;

pub(super) fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = widget_with_class(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
