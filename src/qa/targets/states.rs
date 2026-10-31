// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{super::state, editable_focused};
use crate::ui::accessibility_qa as names;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct States {
    pub(crate) selected: bool,
    pub(crate) focused: bool,
    pub(crate) checked: bool,
    pub(crate) pressed: bool,
    pub(crate) sensitive: bool,
    pub(crate) selectable: bool,
    pub(crate) editable: bool,
    pub(crate) focusable: bool,
    pub(crate) busy: bool,
    /// Outlined as the result Enter in a filter field opens.
    pub(crate) marked: bool,
}

pub(crate) fn states(widget: &gtk::Widget) -> States {
    // Native toggles report their own state, so a recorded one cannot go stale.
    let recorded = names::naming(widget);
    let checked = if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
        check.is_active()
    } else if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
        switch.is_active()
    } else {
        recorded
            .as_ref()
            .and_then(|naming| naming.checked)
            .unwrap_or(false)
    };
    // GTK's AT-SPI bridge reports an active switch as pressed as well as checked.
    let pressed = if let Some(toggle) = widget.downcast_ref::<gtk::ToggleButton>() {
        toggle.is_active()
    } else if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
        switch.is_active()
    } else {
        recorded
            .as_ref()
            .and_then(|naming| naming.pressed)
            .unwrap_or(false)
    };
    States {
        // A list box row reports its own selection, as GTK's AT-SPI bridge
        // does, and so does a notebook's tab.
        selected: widget.downcast_ref::<gtk::ListBoxRow>().map_or_else(
            || {
                recorded
                    .as_ref()
                    .and_then(|naming| naming.selected)
                    .or_else(|| notebook_tab_selected(widget))
                    .unwrap_or_else(|| state::row_selected(widget) || list_row_selected(widget))
            },
            gtk::ListBoxRow::is_selected,
        ),
        focused: if widget.is::<gtk::Editable>() {
            editable_focused(widget)
        } else {
            widget.has_focus()
        },
        checked,
        pressed,
        sensitive: widget.is_sensitive(),
        selectable: widget.downcast_ref::<gtk::ListBoxRow>().map_or_else(
            || {
                recorded
                    .as_ref()
                    .is_some_and(|naming| naming.selected.is_some())
            },
            gtk::ListBoxRow::is_selectable,
        ),
        editable: widget
            .dynamic_cast_ref::<gtk::Editable>()
            .map(EditableExt::is_editable)
            .or_else(|| {
                widget
                    .downcast_ref::<gtk::TextView>()
                    .map(gtk::TextView::is_editable)
            })
            .unwrap_or(false),
        focusable: widget.is_focusable(),
        busy: recorded
            .as_ref()
            .and_then(|naming| naming.busy)
            .unwrap_or(false),
        // An entry target is the list's row; the row's child carries the mark.
        marked: std::iter::once(widget.clone())
            .chain(widget.first_child())
            .any(|widget| widget.has_css_class(crate::ui::ENTER_TARGET_CLASS)),
    }
}

fn list_row_selected(widget: &gtk::Widget) -> bool {
    matches!(
        widget.accessible_role(),
        gtk::AccessibleRole::ListItem | gtk::AccessibleRole::GridCell | gtk::AccessibleRole::Row
    ) && widget.state_flags().contains(gtk::StateFlags::SELECTED)
}

fn notebook_tab_selected(widget: &gtk::Widget) -> Option<bool> {
    if widget.accessible_role() != gtk::AccessibleRole::Tab {
        return None;
    }
    let label = widget.first_child()?;
    let notebook = std::iter::successors(widget.parent(), WidgetExt::parent)
        .find_map(|ancestor| ancestor.downcast::<gtk::Notebook>().ok())?;
    let page = (0..notebook.n_pages()).find(|page| {
        notebook
            .nth_page(Some(*page))
            .and_then(|child| notebook.tab_label(&child))
            .is_some_and(|tab| tab == label)
    })?;
    Some(notebook.current_page() == Some(page))
}
