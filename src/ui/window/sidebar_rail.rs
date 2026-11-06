// SPDX-License-Identifier: MIT

//! Switching the sidebar between its full width and the icon rail.

use std::rc::Rc;

use gtk::prelude::*;

use super::{
    SidebarState,
    sidebar::{UPDATE_TEXT_HIDDEN, UPDATE_TEXT_SHOWN},
    sidebar_width::{
        MIN_SIDEBAR_WIDTH, SidebarFit, preferred_sidebar_width, sidebar_minimum_width,
        sidebar_rail_button_size, sidebar_rail_width,
    },
};

fn children(parent: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(parent.first_child(), gtk::Widget::next_sibling)
}

fn apply_rail_to_shell(scroller: &gtk::ScrolledWindow, shell: &gtk::Widget, rail: bool) {
    scroller.set_overlay_scrolling(rail);
    if rail {
        shell.add_css_class("sidebar-rail");
    } else {
        shell.remove_css_class("sidebar-rail");
    }
    if let Some(paned) = shell.parent().and_downcast::<gtk::Paned>() {
        apply_rail_to_paned(&paned, rail);
    }
    scroller.set_width_request(shell_width(rail));
    shell.set_size_request(shell_width(rail), -1);
}

fn shell_width(rail: bool) -> i32 {
    if rail {
        sidebar_rail_width()
    } else {
        MIN_SIDEBAR_WIDTH
    }
}

/// The rail cannot be resized, so its divider stays thin and keeps the plain cursor.
fn apply_rail_to_paned(paned: &gtk::Paned, rail: bool) {
    paned.set_wide_handle(!rail);
    if let Some(handle) = paned_separator(paned) {
        handle.set_cursor_from_name(if rail {
            Some("default")
        } else {
            Some("col-resize")
        });
    }
}

/// Section headers keep their row and follow `sync_section_states` instead.
fn sync_rail_button(button: &gtk::Button, rail: bool) {
    if !button.has_css_class("sidebar-heading-toggle") {
        sync_sidebar_button(button, rail);
    }
}

fn sync_rail_device_child(widget: &gtk::Widget, rail: bool) {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
        sync_sidebar_button(&button, rail);
    } else if widget.has_css_class("sidebar-device-actions") {
        widget.set_visible(!rail);
    }
}

impl SidebarState {
    pub(in crate::ui) fn set_rail(&self, rail: bool) {
        let changed = self.rail.replace(rail) != rail;
        if !changed && !rail {
            return;
        }
        if changed {
            self.notify_rail_changed(rail);
        }
        if let Some(scroller) = self.sidebar_scroller()
            && let Some(shell) = scroller.parent()
        {
            apply_rail_to_shell(&scroller, &shell, rail);
        }
        self.sync_update_notice_rail(rail);
        self.sync_rail_rows();
    }

    fn notify_rail_changed(&self, rail: bool) {
        if let Some(listener) = self.rail_changed.borrow().clone() {
            listener(rail);
        }
    }

    fn sync_update_notice_rail(&self, rail: bool) {
        let Some(slot) = self
            .update_label
            .ancestor(gtk::Stack::static_type())
            .and_downcast::<gtk::Stack>()
        else {
            return;
        };
        slot.set_visible_child_name(if rail {
            UPDATE_TEXT_HIDDEN
        } else {
            UPDATE_TEXT_SHOWN
        });
        let Some(content) = slot.parent().and_downcast::<gtk::Box>() else {
            return;
        };
        content.set_spacing(if rail { 0 } else { 8 });
        if let Some(dot) = content.last_child() {
            dot.set_visible(!rail);
        }
    }

    pub(super) fn sync_rail_rows(&self) {
        let mut widget_child = self.widget.first_child();
        while let Some(child) = widget_child {
            widget_child = child.next_sibling();
            self.sync_rail_child(&child);
        }
        self.sync_empty_pinned_section();
        self.sync_section_states();
        self.sync_keycaps();
    }

    pub(super) fn sync_rail_child(&self, child: &gtk::Widget) {
        let rail = self.rail.get();
        if let Ok(heading) = child.clone().downcast::<gtk::Label>() {
            heading.set_visible(!rail);
        } else if let Ok(button) = child.clone().downcast::<gtk::Button>() {
            sync_rail_button(&button, rail);
        } else if let Ok(revealer) = child.clone().downcast::<gtk::Revealer>() {
            if let Some(inner) = revealer.child() {
                children(&inner).for_each(|row| self.sync_rail_child(&row));
            }
        } else if child.has_css_class("sidebar-device") {
            children(child).for_each(|widget| sync_rail_device_child(&widget, rail));
        }
    }

    /// Runs after the rail engages or releases, however that was decided.
    pub(in crate::ui) fn connect_rail_changed(&self, listener: impl Fn(bool) + 'static) {
        self.rail_changed.replace(Some(Rc::new(listener)));
    }

    /// The user's choice, from the footer button or the saved preference.
    pub(in crate::ui) fn set_compact(&self, compact: bool) {
        if let Some(rail) = self.policy.set_compact(compact) {
            self.apply_rail(rail);
        }
    }

    /// Feeds the fit rule; the caller applies a returned change.
    pub(in crate::ui) fn observe_fit(&self, fit: SidebarFit) -> Option<bool> {
        self.policy.observe(fit)
    }

    /// Shows the rail or the full sidebar and moves the divider with it.
    pub(in crate::ui) fn apply_rail(&self, rail: bool) {
        self.set_rail(rail);
        let Some(shell) = self
            .sidebar_scroller()
            .and_then(|scroller| scroller.parent())
        else {
            return;
        };
        let Some(paned) = shell.parent().and_downcast::<gtk::Paned>() else {
            return;
        };
        if !shell.is_visible() {
            return;
        }
        let target = if rail {
            sidebar_rail_width()
        } else {
            self.saved_width
                .get()
                .unwrap_or_else(preferred_sidebar_width)
                .max(sidebar_minimum_width(&shell))
        };
        if paned.position() != target {
            paned.set_position(target);
        }
    }
}

pub(super) fn sync_sidebar_button(button: &gtk::Button, rail: bool) {
    // Height stays with the theme, so a row is equally tall in both modes.
    button.set_size_request(if rail { sidebar_rail_button_size() } else { -1 }, -1);
    let Some(content) = button.child() else {
        return;
    };
    for widget in children(&content) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && !label.has_css_class("sidebar-keycap")
        {
            sync_rail_label(button, label, rail);
        }
    }
    content.set_halign(if rail {
        gtk::Align::Center
    } else {
        gtk::Align::Fill
    });
}

/// The rail hides row labels, so the button carries the name instead.
fn sync_rail_label(button: &gtk::Button, label: &gtk::Label, rail: bool) {
    button.set_tooltip_text(rail.then_some(label.label().as_str()));
    if rail {
        crate::ui::accessibility::set_stand_in_label(button, &label.label());
    } else {
        crate::ui::accessibility::clear_stand_in_label(button);
    }
    label.set_visible(!rail);
}
pub(super) fn paned_separator(paned: &gtk::Paned) -> Option<gtk::Widget> {
    let mut child = paned.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "separator" {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}
