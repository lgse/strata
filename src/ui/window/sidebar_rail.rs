// SPDX-License-Identifier: MIT

//! Switching the sidebar between its full width and the icon rail.

use std::rc::Rc;

use gtk::prelude::*;

use super::{SidebarState, sidebar_width::{MIN_SIDEBAR_WIDTH, SidebarFit, preferred_sidebar_width, sidebar_minimum_width, sidebar_rail_button_size, sidebar_rail_width}};

impl SidebarState {
    pub(in crate::ui) fn set_rail(&self, rail: bool) {
        let changed = self.rail.replace(rail) != rail;
        if !changed && !rail {
            return;
        }
        if changed && let Some(listener) = self.rail_changed.borrow().clone() {
            listener(rail);
        }
        if let Some(scroller) = self.sidebar_scroller()
            && let Some(shell) = scroller.parent()
        {
            scroller.set_overlay_scrolling(rail);
            if rail {
                shell.add_css_class("sidebar-rail");
            } else {
                shell.remove_css_class("sidebar-rail");
            }
            if let Some(paned) = shell.parent().and_downcast::<gtk::Paned>() {
                paned.set_wide_handle(!rail);
                if let Some(handle) = paned_separator(&paned) {
                    handle.set_cursor_from_name(if rail {
                        Some("default")
                    } else {
                        Some("col-resize")
                    });
                }
            }
            scroller.set_width_request(if rail {
                sidebar_rail_width()
            } else {
                MIN_SIDEBAR_WIDTH
            });
            shell.set_size_request(
                if rail {
                    sidebar_rail_width()
                } else {
                    MIN_SIDEBAR_WIDTH
                },
                -1,
            );
        }
        let text = self.update_label.parent();
        if let Some(text) = &text {
            text.set_visible(!rail);
        }
        if let Some(content) = text
            .and_then(|text| text.parent())
            .and_downcast::<gtk::Box>()
        {
            content.set_spacing(if rail { 0 } else { 8 });
            content.set_halign(if rail {
                gtk::Align::Center
            } else {
                gtk::Align::Fill
            });
            if let Some(dot) = content.first_child() {
                dot.set_visible(!rail);
            }
        }
        self.sync_rail_rows();
    }

    pub(super) fn sync_rail_rows(&self) {
        let rail = self.rail.get();
        let size = if rail { sidebar_rail_button_size() } else { -1 };
        if let Some(notice) = self
            .update_label
            .ancestor(gtk::Button::static_type())
            .and_downcast::<gtk::Button>()
        {
            notice.set_size_request(if rail { sidebar_rail_width() } else { -1 }, size);
        }
        let mut widget_child = self.widget.first_child();
        while let Some(child) = widget_child {
            widget_child = child.next_sibling();
            self.sync_rail_child(&child);
        }
        self.sync_section_states();
        self.sync_keycaps();
    }

    pub(super) fn sync_rail_child(&self, child: &gtk::Widget) {
        let rail = self.rail.get();
        if let Ok(heading) = child.clone().downcast::<gtk::Label>() {
            heading.set_visible(!rail);
        } else if let Ok(button) = child.clone().downcast::<gtk::Button>() {
            if button.has_css_class("sidebar-heading-toggle") {
                button.set_visible(!rail);
            } else {
                sync_sidebar_button(&button, rail);
            }
        } else if let Ok(revealer) = child.clone().downcast::<gtk::Revealer>() {
            if let Some(inner) = revealer.child() {
                let mut row_child = inner.first_child();
                while let Some(row) = row_child {
                    row_child = row.next_sibling();
                    self.sync_rail_child(&row);
                }
            }
        } else if child.has_css_class("sidebar-device") {
            let mut dev_child = child.first_child();
            while let Some(w) = dev_child {
                dev_child = w.next_sibling();
                if let Ok(button) = w.clone().downcast::<gtk::Button>() {
                    sync_sidebar_button(&button, rail);
                } else if w.has_css_class("sidebar-device-actions") {
                    w.set_visible(!rail);
                }
            }
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
    let size = if rail { sidebar_rail_button_size() } else { -1 };
    button.set_size_request(size, size);
    if let Some(content) = button.child() {
        let mut child = content.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                if label.has_css_class("sidebar-keycap") {
                    continue;
                }
                button.set_tooltip_text(rail.then_some(label.label().as_str()));
                if rail {
                    crate::ui::accessibility::set_stand_in_label(button, &label.label());
                } else {
                    crate::ui::accessibility::clear_stand_in_label(button);
                }
                label.set_visible(!rail);
            }
        }
        content.set_halign(if rail {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        });
    }
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
