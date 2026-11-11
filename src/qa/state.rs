// SPDX-License-Identifier: MIT

//! Reads the application's state from its own widget tree, as AT-SPI would expose it.

use std::{collections::HashMap, fmt::Write as _};

use gtk::{graphene, prelude::*};

pub(crate) use super::protocol::{
    EntryObservation, FieldState, Observation, collapse_whitespace, contains, empty_app_state,
    field_matches, inside, middle_visible, on_screen, within_viewport, within_window,
};
use super::{backend::Gtk, protocol::EntryRow, targets};
use crate::ui::accessibility_qa::{self as names, Mark};

pub(crate) fn read_field(widget: &gtk::Widget) -> Option<FieldState> {
    let editable = widget.dynamic_cast_ref::<gtk::Editable>()?;
    Some(FieldState {
        name: targets::accessible_name(widget),
        text: editable.text().to_string(),
        caret: editable.position(),
        selection: editable
            .selection_bounds()
            .filter(|(start, end)| start != end)
            .map(|(start, end)| [start.min(end), start.max(end)]),
        focused: targets::editable_focused(widget),
        showing: widget.is_mapped(),
    })
}

pub(crate) type Scene = super::protocol::Scene<gtk::Widget>;
pub(crate) type PaneWidgets = super::protocol::PaneNodes<gtk::Widget>;

pub(crate) fn scan(backend: &Gtk, window: &gtk::Window) -> Scene {
    super::protocol::scan(backend, window.upcast_ref())
}

pub(crate) fn rendered_bounds(widget: &gtk::Widget) -> Option<graphene::Rect> {
    if !widget.is_mapped() || !widget.is_visible() {
        return None;
    }
    let native = widget.native()?;
    let bounds = widget.compute_bounds(&native)?;
    let sane = |extent: f32| extent > 0.0 && extent < 100_000.0;
    (sane(bounds.width()) && sane(bounds.height())).then_some(bounds)
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            children.push(current);
        }
        stack.extend(children.into_iter().rev());
        found.push(widget);
    }
    found
}

fn address(widget: &gtk::Widget) -> usize {
    widget.as_ptr() as usize
}

struct Row {
    item: gtk::ListItem,
    name: String,
}

/// The row widget carrying a list item's focus and selected states is the parent of its child.
fn rows() -> HashMap<usize, Row> {
    names::entries()
        .into_iter()
        .filter_map(|(item, naming)| {
            let row = item.child()?.parent()?;
            let name = naming.label.filter(|name| !name.is_empty())?;
            Some((address(&row), Row { item, name }))
        })
        .collect()
}

pub(crate) fn row_name(widget: &gtk::Widget) -> Option<String> {
    names::entries().into_iter().find_map(|(item, naming)| {
        let row = item.child()?.parent()?;
        (row == *widget)
            .then_some(naming.label)
            .flatten()
            .filter(|name| !name.is_empty())
    })
}

pub(crate) fn row_description(widget: &gtk::Widget) -> Option<String> {
    names::entries().into_iter().find_map(|(item, naming)| {
        let row = item.child()?.parent()?;
        (row == *widget).then_some(naming.description).flatten()
    })
}

pub(crate) fn row_selected(widget: &gtk::Widget) -> bool {
    names::entries().into_iter().any(|(item, _)| {
        item.child()
            .and_then(|child| child.parent())
            .is_some_and(|row| row == *widget)
            && item.is_selected()
    })
}

/// The row a widget is, when a list item recorded as an entry is bound to it.
pub(crate) fn row(widget: &gtk::Widget) -> Option<EntryRow<gtk::Widget>> {
    names::entries().into_iter().find_map(|(item, naming)| {
        let row = item.child()?.parent()?;
        let name = naming.label.filter(|name| !name.is_empty())?;
        (row == *widget).then(|| EntryRow {
            node: row,
            name,
            selected: item.is_selected(),
            focused: widget.has_focus(),
        })
    })
}

/// The rows an entry list shows, in the order the list keeps them.
pub(crate) fn container_rows(container: &gtk::Widget) -> Vec<EntryRow<gtk::Widget>> {
    let rows = rows();
    let mut found = Vec::new();
    let mut occupied: HashMap<[i64; 4], String> = HashMap::new();
    for widget in descendants(container) {
        let Some(row) = rows.get(&address(&widget)) else {
            continue;
        };
        let Some(bounds) = rendered_bounds(&widget) else {
            continue;
        };
        if !row_label_matches(&widget, &row.name) {
            continue;
        }
        // GtkListView can give a recycled row its new name before its bounds
        // catch up, so two names share one box mid-refresh; `_entries_in`
        // then reports nothing rather than a wrong order.
        let key = [bounds.x(), bounds.y(), bounds.width(), bounds.height()]
            .map(|value| f64::from(value).floor() as i64);
        match occupied.get(&key) {
            Some(prior) if *prior != row.name => return Vec::new(),
            _ => {
                occupied.insert(key, row.name.clone());
            }
        }
        found.push(EntryRow {
            name: row.name.clone(),
            selected: row.item.is_selected(),
            focused: widget.has_focus(),
            node: widget,
        });
    }
    found
}

/// False while a recycled row's label still shows the previous name.
fn row_label_matches(row: &gtk::Widget, name: &str) -> bool {
    let presented = targets::presented(row);
    // List metadata may precede the name; the thumbnail identifies its caption.
    let name_cell = presented
        .iter()
        .find(|widget| targets::role_name(widget) == "image")
        .and_then(WidgetExt::parent);
    let label = name_cell
        .and_then(|cell| {
            targets::presented(&cell)
                .into_iter()
                .find(|widget| targets::role_name(widget) == "label")
        })
        .or_else(|| {
            presented
                .into_iter()
                .find(|widget| targets::role_name(widget) == "label")
        });
    label.is_none_or(|label| targets::accessible_name(&label) == name)
}

pub(crate) fn is_ready(window: &gtk::Window) -> bool {
    window.is_mapped()
        && descendants(window.upcast_ref()).iter().any(|widget| {
            names::naming(widget).is_some_and(|naming| naming.mark == Some(Mark::EntryContainer))
                && rendered_bounds(widget).is_some()
        })
}

const MAX_TREE_DEPTH: usize = 64;

pub(crate) fn dump(window: &gtk::Window) -> String {
    let rows = rows();
    let mut output = String::new();
    if let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) {
        let _ = writeln!(
            output,
            "focus: {} role={:?} name={:?}{}",
            focus.type_().name(),
            targets::role_name(&focus),
            targets::accessible_name(&focus),
            if focus.is_mapped() {
                ""
            } else {
                " (not mapped)"
            }
        );
    }
    dump_widget(window.upcast_ref(), 0, &rows, &mut output);
    output
}

fn dump_widget(
    widget: &gtk::Widget,
    depth: usize,
    rows: &HashMap<usize, Row>,
    output: &mut String,
) {
    if !widget.is_mapped() || depth > MAX_TREE_DEPTH {
        return;
    }
    let _ = write!(
        output,
        "{:indent$}{} role={:?}",
        "",
        widget.type_().name(),
        targets::role_name(widget),
        indent = depth * 2
    );
    let name = targets::accessible_name(widget);
    if !name.is_empty() {
        let _ = write!(output, " name={name:?}");
    }
    if rows
        .get(&address(widget))
        .is_some_and(|row| row.item.is_selected())
    {
        output.push_str(" [selected]");
    }
    if let Some(description) = names::naming(widget)
        .and_then(|naming| naming.description)
        .filter(|text| !text.is_empty())
    {
        let _ = write!(output, " description={description:?}");
    }
    if let Some(text) = widget_text(widget).filter(|text| *text != name) {
        let _ = write!(output, " text={text:?}");
    }
    if widget.has_focus() {
        output.push_str(" [focused]");
    }
    if let Some(bounds) = rendered_bounds(widget) {
        let _ = write!(
            output,
            " ({:.0},{:.0} {:.0}x{:.0})",
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height()
        );
    }
    output.push('\n');
    let mut child = widget.first_child();
    while let Some(current) = child {
        dump_widget(&current, depth + 1, rows, output);
        child = current.next_sibling();
    }
}

fn widget_text(widget: &gtk::Widget) -> Option<String> {
    let text = if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        label.label().to_string()
    } else if let Some(inscription) = widget.downcast_ref::<gtk::Inscription>() {
        inscription
            .text()
            .map(|text| text.to_string())
            .unwrap_or_default()
    } else {
        widget.downcast_ref::<gtk::Text>()?.text().to_string()
    };
    (!text.is_empty()).then_some(text)
}
