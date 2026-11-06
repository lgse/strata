// SPDX-License-Identifier: MIT

use super::{
    INDETERMINATE_PROGRESS_INTERVAL, TRANSFER_ITEM_TEXT_WIDTH_CHARS,
    TRANSFER_RATE_TEXT_WIDTH_CHARS, presentation,
};
use crate::ui::blur::BlurBin;
use crate::ui::controls::{modal_layout, progress_summary};
use crate::ui::modal::{ModalHost, modal_layer, remember_modal_focus_for_listing};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

/// The untranslated icon and copy an operation presents its progress under.
pub(super) struct ProgressHeading {
    pub(super) icon: String,
    pub(super) title: String,
    pub(super) subtitle: String,
}

impl ProgressHeading {
    pub(super) fn new(icon: &str, title: &str, subtitle: &str) -> Self {
        Self {
            icon: icon.to_owned(),
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
        }
    }
}

pub(in crate::ui::browser) struct FileProgressView {
    pub(super) layer: RefCell<Option<gtk::Box>>,
    pub(super) icon_name: String,
    pub(super) cancel_label: String,
    pub(super) compact: Option<Rc<crate::ui::progress_dock::CompactProgress>>,
    pub(super) overlay: gtk::Overlay,
    pub(super) blurred_root: Option<BlurBin>,
    pub(super) progress: gtk::ProgressBar,
    pub(super) status: gtk::Label,
    pub(super) title: gtk::Label,
    pub(super) subtitle: gtk::Label,
    pub(super) cancel: gtk::Button,
    pub(super) cancel_action: Option<Rc<dyn Fn()>>,
    pub(super) status_row: gtk::Box,
    pub(super) transfer_header: gtk::Box,
    pub(super) transfer_footer: gtk::Box,
    pub(super) transfer_percent: gtk::Label,
    pub(super) transfer_bytes: gtk::Label,
    pub(super) transfer_items: gtk::Label,
    pub(super) transfer_rate: gtk::Label,
    pub(super) archive_activity: gtk::Spinner,
    pub(super) indeterminate: Rc<Cell<bool>>,
    pub(super) pulse_source: Rc<RefCell<Option<glib::SourceId>>>,
    pub(super) last_transfer_render: Cell<Option<Instant>>,
}

struct TransferFooter {
    container: gtk::Box,
    items: gtk::Label,
    rate: gtk::Label,
}

impl FileProgressView {
    /// Builds the modal content; the layer is only added to the overlay when the
    /// operation is not confined to the dock.
    pub(super) fn build(host: ModalHost, heading: &ProgressHeading, dock_only: bool) -> Self {
        let ModalHost {
            overlay: window_overlay,
            blurred_root,
        } = host;
        let layout = modal_layout(
            &heading.icon,
            &crate::i18n::tr(&heading.title),
            &crate::i18n::tr(&heading.subtitle),
            &crate::i18n::tr("Cancel"),
        );
        layout.content.add_css_class("compact");
        layout.close.set_visible(false);
        layout.cancel.set_visible(false);
        let status = gtk::Label::new(Some(&crate::i18n::percent(0)));
        status.add_css_class("modal-progress-status");
        status.set_xalign(0.0);
        let summary = progress_summary("Transferred");
        let archive_activity = gtk::Spinner::new();
        archive_activity.set_visible(false);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status_row.append(&archive_activity);
        status_row.append(&status);
        layout.body.append(&status_row);

        summary.header.set_visible(false);
        layout.body.append(&summary.widget);

        let footer = build_transfer_footer();
        layout.body.append(&footer.container);

        let indeterminate = Rc::new(Cell::new(false));
        let pulse_source = Rc::new(RefCell::new(None));

        let layer = if dock_only {
            None
        } else {
            let layer = modal_layer(
                &layout.content,
                &window_overlay,
                blurred_root.clone(),
                Some(Rc::new(|| true)),
            );
            remember_modal_focus_for_listing(&layer, &window_overlay);
            window_overlay.add_overlay(&layer);
            Some(layer)
        };
        Self {
            layer: RefCell::new(layer),
            icon_name: heading.icon.clone(),
            cancel_label: presentation::cancel_accessible_label(&heading.title),
            compact: None,
            overlay: window_overlay,
            blurred_root,
            progress: summary.progress,
            status,
            title: layout.title,
            subtitle: layout.subtitle,
            cancel: layout.confirm,
            cancel_action: None,
            status_row,
            transfer_header: summary.header,
            transfer_footer: footer.container,
            transfer_percent: summary.percent,
            transfer_bytes: summary.amount,
            transfer_items: footer.items,
            transfer_rate: footer.rate,
            archive_activity,
            indeterminate,
            pulse_source,
            last_transfer_render: Cell::new(None),
        }
    }

    pub(super) fn ensure_indeterminate_pulse(&self) {
        if self.pulse_source.borrow().is_some() {
            return;
        }
        let weak_progress = self.progress.downgrade();
        let compact_progress = self
            .compact
            .as_ref()
            .map(|compact| compact.progress.downgrade());
        let indeterminate = self.indeterminate.clone();
        let pulse_source = self.pulse_source.clone();
        let source = glib::timeout_add_local(INDETERMINATE_PROGRESS_INTERVAL, move || {
            if !indeterminate.get() {
                pulse_source.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            let Some(progress) = weak_progress.upgrade() else {
                pulse_source.borrow_mut().take();
                return glib::ControlFlow::Break;
            };
            progress.pulse();
            if let Some(compact) = compact_progress.as_ref().and_then(glib::WeakRef::upgrade) {
                compact.pulse();
            }
            glib::ControlFlow::Continue
        });
        self.pulse_source.replace(Some(source));
    }
}

fn build_transfer_footer() -> TransferFooter {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    container.add_css_class("transfer-progress-footer");
    container.set_visible(false);
    let items = gtk::Label::new(None);
    items.set_xalign(0.0);
    items.set_halign(gtk::Align::Start);
    items.set_hexpand(true);
    items.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    // Equal minimum and maximum text widths keep live updates from resizing the modal.
    items.set_width_chars(TRANSFER_ITEM_TEXT_WIDTH_CHARS);
    items.set_max_width_chars(TRANSFER_ITEM_TEXT_WIDTH_CHARS);
    let rate = gtk::Label::new(None);
    rate.set_xalign(1.0);
    rate.set_ellipsize(gtk::pango::EllipsizeMode::End);
    rate.set_width_chars(TRANSFER_RATE_TEXT_WIDTH_CHARS);
    rate.set_max_width_chars(TRANSFER_RATE_TEXT_WIDTH_CHARS);
    container.append(&items);
    container.append(&rate);
    TransferFooter {
        container,
        items,
        rate,
    }
}
