// SPDX-License-Identifier: MIT

use gtk::prelude::*;
use std::{rc::Rc, time::Duration};

mod completion;
#[cfg(test)]
mod tests;

use crate::{
    assets,
    ui::{accessibility, controls::pane_header_action},
};

pub(super) struct CompactProgress {
    pub(super) root: gtk::Box,
    pub(super) title: gtk::Label,
    pub(super) status: gtk::Label,
    pub(super) info: gtk::Label,
    pub(super) count: gtk::Label,
    pub(super) destination: gtk::Label,
    pub(super) meta: gtk::Label,
    pub(super) progress: gtk::ProgressBar,
    pub(super) cancel: gtk::Button,
    pub(super) complete: gtk::Button,
    pub(super) pin: gtk::ToggleButton,
    icon: gtk::Image,
    completion: Rc<completion::Completion>,
}

fn dock(overlay: &gtk::Overlay) -> (gtk::ScrolledWindow, gtk::Box) {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.has_css_class("file-operation-dock")
            && let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>()
            && let Some(viewport) = scroll.child().and_downcast::<gtk::Viewport>()
            && let Some(list) = viewport.child().and_downcast::<gtk::Box>()
        {
            return (scroll.clone(), list);
        }
    }
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    // Shadows must fit inside the viewport, not in the scroller's clipped padding.
    list.set_margin_start(16);
    list.set_margin_end(16);
    list.set_margin_top(16);
    list.set_margin_bottom(16);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .propagate_natural_width(true)
        .max_content_height(320)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(12)
        .margin_bottom(48)
        .build();
    scroll.add_css_class("file-operation-dock");
    overlay.add_overlay(&scroll);
    (scroll, list)
}

fn wrapped_label(width: i32) -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(width)
        .build()
}

impl CompactProgress {
    pub(super) fn new(overlay: &gtk::Overlay, icon: &str) -> Self {
        let (dock, list) = dock(overlay);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("file-operation-card");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = assets::primary_icon(icon, 14);
        header.append(&icon);
        let title = wrapped_label(22);
        title.set_hexpand(true);
        title.add_css_class("job-name");
        header.append(&title);
        let status = gtk::Label::builder().xalign(1.0).build();
        status.add_css_class("job-status");
        let pin = gtk::ToggleButton::new();
        pane_header_action(&pin);
        pin.add_css_class("progress-card-action");
        pin.set_child(Some(&assets::primary_icon(assets::icons::PIN, 14)));
        pin.set_tooltip_text(Some("Keep notification"));
        accessibility::set_label(&pin, "Keep notification");
        pin.set_visible(false);
        header.append(&pin);
        let cancel = gtk::Button::new();
        pane_header_action(&cancel);
        cancel.add_css_class("progress-cancel");
        cancel.add_css_class("progress-card-action");
        cancel.set_child(Some(&assets::primary_icon(assets::icons::X, 14)));
        cancel.set_tooltip_text(Some("Cancel operation"));
        accessibility::set_label(&cancel, "Cancel operation");
        header.append(&cancel);
        root.append(&header);
        let details = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let info = wrapped_label(42);
        info.set_hexpand(true);
        info.add_css_class("job-meta");
        info.add_css_class("job-description");
        details.append(&info);
        let count = gtk::Label::builder()
            .xalign(1.0)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        count.add_css_class("job-meta");
        details.append(&count);
        root.append(&details);
        let destination = wrapped_label(42);
        destination.add_css_class("job-meta");
        destination.set_margin_bottom(4);
        root.append(&destination);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("modal-progress");
        root.append(&progress);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let complete = gtk::Button::with_label("Complete");
        complete.add_css_class("job-action");
        complete.add_css_class("progress-complete");
        complete.set_cursor_from_name(Some("pointer"));
        accessibility::set_label(&complete, "Dismiss completed notification");
        complete.set_visible(false);
        footer.append(&complete);
        let meta = wrapped_label(42);
        meta.set_hexpand(true);
        meta.add_css_class("job-meta");
        footer.append(&meta);
        status.set_valign(gtk::Align::Start);
        status.set_margin_start(8);
        footer.append(&status);
        root.append(&footer);
        list.append(&root);
        let completion = completion::Completion::new(completion::Widgets {
            root: root.downgrade(),
            overlay: overlay.downgrade(),
            dock: dock.downgrade(),
            list: list.downgrade(),
            meta: meta.downgrade(),
            status: status.downgrade(),
            progress: progress.downgrade(),
        });
        completion.bind(&root, &cancel, &complete, &pin);
        Self {
            root,
            title,
            status,
            info,
            count,
            destination,
            meta,
            progress,
            cancel,
            complete,
            pin,
            icon,
            completion,
        }
    }

    pub(super) fn set_cancel_action(&self, action: Rc<dyn Fn()>) {
        self.completion.set_cancel(action);
    }

    pub(super) fn completed(&self, title: &str) {
        self.complete_after(title, Duration::from_secs(5));
    }

    fn complete_after(&self, title: &str, duration: Duration) {
        if self.root.parent().is_none() {
            return;
        }
        self.title.set_text(title);
        assets::set_primary_icon(&self.icon, assets::icons::CHECK);
        self.complete.set_visible(true);
        self.pin.set_visible(true);
        self.cancel.set_visible(true);
        self.cancel.set_sensitive(true);
        self.cancel.set_tooltip_text(Some("Dismiss notification"));
        accessibility::set_label(&self.cancel, "Dismiss notification");
        self.completion.complete(duration);
    }

    pub(super) fn remove(&self) {
        self.completion.dismiss();
    }
}
