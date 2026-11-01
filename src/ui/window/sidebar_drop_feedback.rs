// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use gtk::{glib, prelude::*};

#[cfg(test)]
mod tests;

pub(super) fn row_gap(label: Option<&str>) -> gtk::Box {
    let gap = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(if label.is_some() {
            gtk::AccessibleRole::Group
        } else {
            gtk::AccessibleRole::Presentation
        })
        .build();
    gap.add_css_class("sidebar-row-gap");
    if let Some(label) = label {
        crate::ui::accessibility::set_label(&gap, label);
    }
    gap
}

pub(super) fn is_gap(widget: &gtk::Widget) -> bool {
    widget.has_css_class("sidebar-row-gap")
}

#[derive(Default)]
pub(super) struct SidebarDropFeedback {
    active: RefCell<Option<ActiveSite>>,
    sidebar: glib::WeakRef<gtk::Box>,
}

struct ActiveSite {
    owner: glib::WeakRef<gtk::Widget>,
    indicator: Option<(glib::WeakRef<gtk::Widget>, &'static str)>,
    cancel: Option<Box<dyn FnOnce()>>,
}

impl ActiveSite {
    fn clear_indicator(&mut self) {
        if let Some((widget, class)) = self.indicator.take()
            && let Some(widget) = widget.upgrade()
        {
            widget.remove_css_class(class);
        }
        if let Some(owner) = self.owner.upgrade() {
            owner.remove_css_class("reorder-before");
            owner.remove_css_class("reorder-after");
        }
    }
}

impl SidebarDropFeedback {
    pub(super) fn new(sidebar: &gtk::Box) -> Self {
        Self {
            sidebar: sidebar.downgrade(),
            ..Self::default()
        }
    }

    pub(super) fn enter(&self, owner: &gtk::Widget) {
        self.enter_with_cancel(owner, None);
        self.set_inserting(true);
    }

    pub(super) fn enter_folder(&self, owner: &gtk::Widget, cancel: impl FnOnce() + 'static) {
        self.enter_with_cancel(owner, Some(Box::new(cancel)));
        self.set_inserting(false);
    }

    fn set_inserting(&self, inserting: bool) {
        if let Some(sidebar) = self.sidebar.upgrade() {
            if inserting {
                sidebar.add_css_class("sidebar-inserting");
            } else {
                sidebar.remove_css_class("sidebar-inserting");
            }
        }
    }

    fn enter_with_cancel(&self, owner: &gtk::Widget, cancel: Option<Box<dyn FnOnce()>>) {
        if self.is_current(owner) {
            return;
        }
        self.reset();
        self.active.replace(Some(ActiveSite {
            owner: owner.downgrade(),
            indicator: None,
            cancel,
        }));
        if let Some(sidebar) = self.sidebar.upgrade() {
            sidebar.add_css_class("sidebar-drag-active");
        }
    }

    pub(super) fn is_current(&self, owner: &gtk::Widget) -> bool {
        self.active
            .borrow()
            .as_ref()
            .is_some_and(|site| site.owner.upgrade().as_ref() == Some(owner))
    }

    pub(super) fn pin(&self, gap: &gtk::Widget) {
        self.show(gap, gap, "eligible", None);
    }

    pub(super) fn folder(&self, row: &gtk::Widget, hovered: bool) {
        if !self.is_current(row) {
            return;
        }
        self.set_inserting(false);
        if hovered {
            self.show(row, row, "drop-destination", None);
        } else if let Some(site) = self.active.borrow_mut().as_mut()
            && site.owner.upgrade().as_ref() == Some(row)
        {
            site.clear_indicator();
        }
    }

    pub(super) fn reorder(&self, row: &gtk::Widget, after: bool) {
        let class = if after {
            "reorder-after"
        } else {
            "reorder-before"
        };
        if is_gap(row) {
            self.pin(row);
            return;
        }
        let gap = if after {
            row.next_sibling()
        } else {
            row.prev_sibling()
        };
        if let Some(gap) = gap.filter(is_gap) {
            self.show(row, &gap, "eligible", Some(class));
        }
    }

    fn show(
        &self,
        owner: &gtk::Widget,
        indicator: &gtk::Widget,
        class: &'static str,
        row_class: Option<&str>,
    ) {
        let mut active = self.active.borrow_mut();
        let Some(site) = active
            .as_mut()
            .filter(|site| site.owner.upgrade().as_ref() == Some(owner))
        else {
            return;
        };
        site.clear_indicator();
        if let Some(row_class) = row_class {
            owner.add_css_class(row_class);
        }
        indicator.add_css_class(class);
        site.indicator = Some((indicator.downgrade(), class));
    }

    pub(super) fn leave(&self, owner: &gtk::Widget) {
        if self.is_current(owner) {
            self.reset();
        }
    }

    pub(super) fn reset(&self) {
        if let Some(mut site) = self.active.take() {
            site.clear_indicator();
            if let Some(cancel) = site.cancel.take() {
                cancel();
            }
        }
        if let Some(sidebar) = self.sidebar.upgrade() {
            sidebar.remove_css_class("sidebar-drag-active");
            sidebar.remove_css_class("sidebar-inserting");
        }
    }
}
