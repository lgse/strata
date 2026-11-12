// SPDX-License-Identifier: MIT

//! The collapsible Pinned and Devices sections of the sidebar.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;

use crate::ui::{motion::animations_enabled, preferences::PreferenceManager};

use super::{SidebarState, sidebar_drop_feedback, sidebar_rows::reparent_in_order};

const DRIVE_SEPARATOR_CLASS: &str = "sidebar-drive-separator";

/// Pinned rows are split across two revealers with the active row kept between
/// them, so collapsing folds the other rows away around the row the user is in.
pub(super) struct PinnedSection {
    pub(super) header: SectionHeader,
    pub(super) upper_revealer: gtk::Revealer,
    pub(super) upper_box: gtk::Box,
    pub(super) lower_revealer: gtk::Revealer,
    pub(super) lower_box: gtk::Box,
    /// Drop gaps and rows in display order, wherever each is currently parented.
    pub(super) items: RefCell<Vec<gtk::Widget>>,
}

impl PinnedSection {
    pub(super) fn collect_items(&self, sidebar: &gtk::Box) {
        let mut items = Vec::new();
        let mut next = self.lower_revealer.next_sibling();
        while let Some(child) = next {
            next = child.next_sibling();
            sidebar.remove(&child);
            self.upper_box.append(&child);
            items.push(child);
        }
        self.items.replace(items);
    }

    pub(super) fn apply_state(&self, collapsed: bool, rail: bool) {
        for revealer in [&self.upper_revealer, &self.lower_revealer] {
            revealer.set_transition_duration(section_transition_duration());
            revealer.set_reveal_child(!collapsed);
        }
        self.header.apply_state(collapsed, rail);
    }
}

pub(super) struct CollapsibleSection {
    pub(super) header: SectionHeader,
    pub(super) revealer: gtk::Revealer,
    pub(super) rows: gtk::Box,
}

impl CollapsibleSection {
    /// Returns the row count without the drop gaps between rows.
    pub(super) fn collect_rows(&self, sidebar: &gtk::Box) -> usize {
        let mut count = 0;
        let mut next = self.revealer.next_sibling();
        while let Some(child) = next {
            next = child.next_sibling();
            sidebar.remove(&child);
            self.rows.append(&child);
            let spacer =
                sidebar_drop_feedback::is_gap(&child) || child.has_css_class(DRIVE_SEPARATOR_CLASS);
            if !spacer {
                count += 1;
            }
        }
        count
    }

    pub(super) fn apply_state(&self, collapsed: bool, rail: bool) {
        self.revealer
            .set_transition_duration(section_transition_duration());
        self.revealer.set_reveal_child(!collapsed);
        self.header.apply_state(collapsed, rail);
    }
}

pub(super) struct SectionHeader {
    pub(super) toggle: gtk::Button,
    pub(super) title: gtk::Label,
    pub(super) rule: gtk::Separator,
    pub(super) chevron: gtk::Image,
    pub(super) badge: gtk::Label,
    pub(super) item_count: Cell<usize>,
}

impl SectionHeader {
    pub(super) fn set_item_count(&self, count: usize) {
        self.item_count.set(count);
        self.badge.set_text(&count.to_string());
        crate::ui::accessibility::set_description(
            &self.toggle,
            Some(&crate::i18n::count("items", count)),
        );
    }

    /// The rail keeps the header's row so toggling moves no icon, leaving only
    /// the chevron to fold the section.
    pub(super) fn apply_state(&self, collapsed: bool, rail: bool) {
        self.title.set_visible(!rail);
        self.rule.set_visible(!rail);
        if let Some(content) = self.toggle.child() {
            content.set_halign(if rail {
                gtk::Align::Center
            } else {
                gtk::Align::Fill
            });
        }
        self.toggle
            .set_tooltip_text(rail.then(|| self.title.label()).as_deref());
        if collapsed {
            self.toggle.remove_css_class("expanded");
        } else {
            self.toggle.add_css_class("expanded");
        }
        // Swapped rather than rotated: a CSS-rotated icon is not pixel-snapped,
        // so it drifts off the eject column under GL and fractional scaling.
        let icon = if collapsed {
            crate::assets::icons::CHEVRON_RIGHT
        } else {
            crate::assets::icons::CHEVRON_DOWN
        };
        crate::assets::set_primary_icon(&self.chevron, icon);
        self.badge
            .set_visible(!rail && collapsed && self.item_count.get() > 0);
        self.toggle
            .update_state(&[gtk::accessible::State::Expanded(Some(!collapsed))]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarSection {
    Pinned,
    Devices,
}

impl SidebarSection {
    pub(super) fn title(self) -> &'static str {
        match self {
            SidebarSection::Pinned => "PINNED",
            SidebarSection::Devices => "DEVICES",
        }
    }

    pub(super) fn is_collapsed(self, manager: &PreferenceManager) -> bool {
        match self {
            SidebarSection::Pinned => manager.sidebar_pinned_collapsed(),
            SidebarSection::Devices => manager.sidebar_devices_collapsed(),
        }
    }

    pub(super) fn set_collapsed(self, manager: &PreferenceManager, collapsed: bool) {
        match self {
            SidebarSection::Pinned => manager.set_sidebar_pinned_collapsed(collapsed),
            SidebarSection::Devices => manager.set_sidebar_devices_collapsed(collapsed),
        }
    }
}

pub(super) fn section_transition_duration() -> u32 {
    if animations_enabled() { 200 } else { 0 }
}

impl SidebarState {
    pub(super) fn append_separator(&self) {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sidebar-separator");
        separator.add_css_class("sidebar-section-separator");
        self.widget.append(&separator);
    }

    pub(super) fn append_drive_separator(&self) {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class(DRIVE_SEPARATOR_CLASS);
        self.widget.append(&separator);
    }

    pub(super) fn append_section_header(self: &Rc<Self>, section: SidebarSection) -> SectionHeader {
        let title = crate::i18n::tr(section.title());
        let label = gtk::Label::new(Some(&title));
        label.add_css_class("sidebar-heading");
        label.set_xalign(0.0);
        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("sidebar-heading-rule");
        rule.set_hexpand(true);
        rule.set_valign(gtk::Align::Center);
        let badge = gtk::Label::new(None);
        badge.add_css_class("sidebar-heading-count");
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 14);
        chevron.add_css_class("sidebar-heading-chevron");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&label);
        content.append(&rule);
        content.append(&badge);
        content.append(&chevron);
        let toggle = gtk::Button::builder()
            .child(&content)
            .has_frame(false)
            .focus_on_click(false)
            .build();
        toggle.add_css_class("sidebar-heading-toggle");
        toggle.set_cursor_from_name(Some("pointer"));
        crate::ui::accessibility::set_label(&toggle, &title);
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let manager = &state.preference_manager;
                section.set_collapsed(manager, !section.is_collapsed(manager));
            }
        });
        self.widget.append(&toggle);
        SectionHeader {
            toggle,
            title: label,
            rule,
            chevron,
            badge,
            item_count: Cell::new(0),
        }
    }

    pub(super) fn append_section_revealer(&self, collapsed: bool) -> (gtk::Revealer, gtk::Box) {
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(section_transition_duration())
            .reveal_child(!collapsed)
            .child(&rows)
            .build();
        revealer.add_css_class("sidebar-section-revealer");
        self.widget.append(&revealer);
        (revealer, rows)
    }

    pub(super) fn append_pinned_section(self: &Rc<Self>, item_count: usize) -> PinnedSection {
        let header = self.append_section_header(SidebarSection::Pinned);
        header.set_item_count(item_count);
        let collapsed = item_count > 0 && self.section_collapsed(SidebarSection::Pinned);
        header.apply_state(collapsed, self.rail.get());
        let (upper_revealer, upper_box) = self.append_section_revealer(collapsed);
        let (lower_revealer, lower_box) = self.append_section_revealer(collapsed);
        PinnedSection {
            header,
            upper_revealer,
            upper_box,
            lower_revealer,
            lower_box,
            items: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn append_devices_section(self: &Rc<Self>) -> CollapsibleSection {
        let header = self.append_section_header(SidebarSection::Devices);
        let (revealer, rows) =
            self.append_section_revealer(self.section_collapsed(SidebarSection::Devices));
        CollapsibleSection {
            header,
            revealer,
            rows,
        }
    }

    pub(super) fn section_collapsed(&self, section: SidebarSection) -> bool {
        section.is_collapsed(&self.preference_manager)
    }

    pub(super) fn sync_section_states(&self) {
        let rail = self.rail.get();
        if let Some(section) = self.pinned_section.borrow().as_ref() {
            let collapsed = section.header.item_count.get() > 0
                && self.section_collapsed(SidebarSection::Pinned);
            section.apply_state(collapsed, rail);
        }
        if let Some(section) = self.devices_section.borrow().as_ref() {
            section.apply_state(self.section_collapsed(SidebarSection::Devices), rail);
        }
    }

    /// Keeps the active pinned row between the two revealers, with the gaps
    /// and rows before it in the upper one and the rest in the lower one.
    pub(super) fn sync_pinned_section_partition(&self) {
        let guard = self.pinned_section.borrow();
        let Some(section) = guard.as_ref() else {
            return;
        };
        let items = section.items.borrow();
        let active = self.browser().active_location();
        let active_index = active.and_then(|active| {
            let rows = self.place_rows.borrow();
            items.iter().position(|item| {
                rows.iter().any(|(location, row)| {
                    *location == active && row.upcast_ref::<gtk::Widget>() == item
                })
            })
        });
        let (upper, detached, lower) = match active_index {
            Some(index) => (&items[..index], items.get(index), &items[index + 1..]),
            None => (&items[..], None, &items[items.len()..]),
        };
        reparent_in_order(&section.upper_box, upper, None);
        reparent_in_order(&section.lower_box, lower, None);
        if let Some(row) = detached {
            reparent_in_order(
                &self.widget,
                std::slice::from_ref(row),
                Some(section.upper_revealer.upcast_ref()),
            );
        }
    }
}
