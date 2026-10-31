// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::super::state;
use crate::ui::accessibility_qa as names;

/// Falls back to a picture's alternative text, which GTK reports as its description.
pub(crate) fn accessible_description(widget: &gtk::Widget) -> String {
    names::naming(widget)
        .and_then(|naming| naming.description)
        .or_else(|| {
            matches!(
                widget.accessible_role(),
                gtk::AccessibleRole::ListItem
                    | gtk::AccessibleRole::GridCell
                    | gtk::AccessibleRole::Cell
            )
            .then(|| state::row_description(widget))
            .flatten()
        })
        .or_else(|| {
            widget
                .downcast_ref::<gtk::Picture>()
                .and_then(gtk::Picture::alternative_text)
                .map(|text| text.to_string())
        })
        .or_else(|| {
            // GtkMenuButton describes its inner toggle button by its own description.
            widget
                .is::<gtk::ToggleButton>()
                .then(|| widget.parent().and_downcast::<gtk::MenuButton>())
                .flatten()
                .map(|menu_button| accessible_description(menu_button.upcast_ref()))
        })
        .unwrap_or_default()
}

/// AT-SPI role names, lowercase with spaces, as AT-SPI spells them.
pub(crate) fn role_name(widget: &gtk::Widget) -> &'static str {
    use gtk::AccessibleRole as Role;
    if widget.is::<gtk::ScrolledWindow>() {
        return "scroll pane";
    }
    // GTK's AT-SPI bridge reports a scale as a slider.
    if widget.is::<gtk::Scale>() {
        return "slider";
    }
    // And a password entry as password text.
    if widget.is::<gtk::PasswordEntry>() {
        return "password text";
    }
    match widget.accessible_role() {
        Role::AlertDialog | Role::Alert => "alert",
        Role::Button => "button",
        Role::Caption => "caption",
        Role::Cell | Role::GridCell => "table cell",
        Role::Checkbox => "check box",
        Role::ColumnHeader => "column header",
        Role::ComboBox => "combo box",
        Role::Dialog => "dialog",
        Role::Grid | Role::Table => "table",
        Role::Generic | Role::TabPanel => "panel",
        Role::Group => "grouping",
        Role::Heading => "heading",
        Role::Img => "image",
        Role::Input | Role::SearchBox => "entry",
        Role::Label => "label",
        Role::Link => "link",
        Role::List => "list",
        Role::ListBox => "list box",
        Role::ListItem => "list item",
        Role::Menu => "menu",
        Role::MenuBar => "menu bar",
        Role::MenuItem => "menu item",
        Role::MenuItemCheckbox => "check menu item",
        Role::MenuItemRadio => "radio menu item",
        Role::Meter => "level bar",
        Role::ProgressBar => "progress bar",
        Role::Radio => "radio button",
        Role::Row => "table row",
        Role::RowHeader => "row header",
        Role::Scrollbar => "scroll bar",
        Role::Separator => "separator",
        Role::Slider => "slider",
        Role::SpinButton => "spin button",
        Role::Status => "status bar",
        Role::Switch | Role::ToggleButton => "toggle button",
        Role::Tab => "page tab",
        Role::TabList => "page tab list",
        Role::TextBox => "text",
        Role::Toolbar => "tool bar",
        Role::Tooltip => "tool tip",
        Role::Tree => "tree",
        Role::TreeGrid => "tree table",
        Role::TreeItem => "tree item",
        Role::Window => "frame",
        _ => "filler",
    }
}

/// Roles whose accessible name GTK builds from their children's names.
fn named_from_content(widget: &gtk::Widget) -> bool {
    use gtk::AccessibleRole as Role;
    matches!(
        widget.accessible_role(),
        Role::Button
            | Role::Cell
            | Role::Checkbox
            | Role::ColumnHeader
            | Role::GridCell
            | Role::Heading
            | Role::Link
            | Role::MenuItem
            | Role::MenuItemCheckbox
            | Role::MenuItemRadio
            | Role::Option
            | Role::Radio
            | Role::Row
            | Role::RowHeader
            | Role::Switch
            | Role::Tab
            | Role::ToggleButton
            | Role::Tooltip
            | Role::TreeItem
    )
}

/// Tooltips never name a widget here; a control that needs a name gets an explicit label.
pub(crate) fn accessible_name(widget: &gtk::Widget) -> String {
    let mut naming = Vec::new();
    if let Some(name) = own_name(widget, &mut naming) {
        return name;
    }
    if named_from_content(widget) {
        return content_name(widget, &mut naming);
    }
    String::new()
}

/// `naming` guards against labelled-by relations that lead back to a widget being named.
fn own_name(widget: &gtk::Widget, naming: &mut Vec<gtk::Widget>) -> Option<String> {
    let recorded = |widget: &gtk::Widget| {
        names::naming(widget)
            .and_then(|naming| naming.label)
            .filter(|label| !label.is_empty())
    };
    // GTK's AT-SPI bridge reports a scrolled window's label on the text view
    // it scrolls, and the scrolled window itself unnamed.
    if widget.is::<gtk::ScrolledWindow>() && recorded(widget).is_some() {
        return Some(String::new());
    }
    // A button's text label names it through the `labelled-by` relation GTK
    // gives it, which comes before a label set on the button itself, unless
    // the relation was replaced or reset.
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().is_some()
        && names::labelled_by(widget).is_empty()
        && !names::labelled_by_reset(widget)
        && let Some(label) = button.child().and_downcast::<gtk::Label>()
    {
        return Some(label.text().to_string());
    }
    if let Some(label) = recorded(widget) {
        return Some(label);
    }
    if widget.is::<gtk::TextView>()
        && let Some(label) = widget
            .parent()
            .filter(|parent| parent.is::<gtk::ScrolledWindow>())
            .and_then(|parent| recorded(&parent))
    {
        return Some(label);
    }
    let labels = names::labelled_by(widget);
    if !labels.is_empty() && !naming.contains(widget) {
        naming.push(widget.clone());
        let mut parts = Vec::new();
        for label in &labels {
            let part = if label == widget {
                content_name(widget, naming)
            } else if naming.contains(label) {
                String::new()
            } else {
                own_name(label, naming).unwrap_or_else(|| content_name(label, naming))
            };
            if !part.is_empty() {
                parts.push(part);
            }
        }
        naming.pop();
        return Some(parts.join(" "));
    }
    // GtkMenuButton labels its inner toggle button with its own name.
    if widget.is::<gtk::ToggleButton>()
        && let Some(menu_button) = widget.parent().and_downcast::<gtk::MenuButton>()
    {
        return own_name(menu_button.upcast_ref(), naming);
    }
    if matches!(
        widget.accessible_role(),
        gtk::AccessibleRole::ListItem | gtk::AccessibleRole::GridCell | gtk::AccessibleRole::Cell
    ) && let Some(name) = state::row_name(widget)
    {
        return Some(name);
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        return Some(label.text().to_string());
    }
    widget
        .downcast_ref::<gtk::Inscription>()
        .map(|inscription| {
            inscription
                .text()
                .map(|text| text.to_string())
                .unwrap_or_default()
        })
}

fn content_name(widget: &gtk::Widget, naming: &mut Vec<gtk::Widget>) -> String {
    let mut parts = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        // A popover is its own surface, not part of the button that opens it.
        if !current.is_visible() || current.is::<gtk::Native>() {
            continue;
        }
        let part = own_name(&current, naming).unwrap_or_else(|| content_name(&current, naming));
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts.join(" ")
}
