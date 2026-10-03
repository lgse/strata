// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use crate::{
    assets,
    ui::{accessibility, controls::pane_header_action},
};

pub(super) struct CompactProgress {
    pub(super) root: gtk::Box,
    pub(super) title: gtk::Label,
    pub(super) status: gtk::Label,
    pub(super) info: gtk::Label,
    pub(super) destination: gtk::Label,
    pub(super) meta: gtk::Label,
    pub(super) progress: gtk::ProgressBar,
    pub(super) cancel: gtk::Button,
    overlay: gtk::Overlay,
    dock: gtk::ScrolledWindow,
    list: gtk::Box,
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
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("file-operation-card");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.append(&assets::primary_icon(icon, 14));
        let title = wrapped_label(22);
        title.set_hexpand(true);
        title.add_css_class("job-name");
        header.append(&title);
        let status = gtk::Label::builder().xalign(1.0).build();
        status.add_css_class("job-status");
        header.append(&status);
        let cancel = gtk::Button::new();
        pane_header_action(&cancel);
        cancel.add_css_class("progress-cancel");
        cancel.set_child(Some(&assets::primary_icon(assets::icons::X, 14)));
        cancel.set_tooltip_text(Some("Cancel operation"));
        accessibility::set_label(&cancel, "Cancel operation");
        header.append(&cancel);
        root.append(&header);
        let info = wrapped_label(36);
        info.add_css_class("job-meta");
        root.append(&info);
        let destination = wrapped_label(36);
        destination.add_css_class("job-meta");
        root.append(&destination);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("modal-progress");
        root.append(&progress);
        let meta = wrapped_label(36);
        meta.add_css_class("job-meta");
        root.append(&meta);
        list.append(&root);
        Self {
            root,
            title,
            status,
            info,
            destination,
            meta,
            progress,
            cancel,
            overlay: overlay.clone(),
            dock,
            list,
        }
    }

    pub(super) fn remove(&self) {
        if self.root.parent().as_ref() == Some(self.list.upcast_ref()) {
            self.list.remove(&self.root);
        }
        if self.list.first_child().is_none() && self.dock.parent().is_some() {
            self.overlay.remove_overlay(&self.dock);
        }
    }
}
