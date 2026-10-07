// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    f64::consts::{FRAC_PI_2, TAU},
    rc::Rc,
};

use gtk::prelude::*;

use crate::ui::browser::format_file_size;

pub(super) struct DownloadProgress {
    pub(super) root: gtk::Box,
    name: gtk::Label,
    status: gtk::Label,
    indicator: gtk::Stack,
    ring: gtk::DrawingArea,
    spinner: gtk::Spinner,
    fraction: Rc<Cell<f64>>,
    cancel: gtk::Button,
}

impl DownloadProgress {
    pub(super) fn new(url: &str, on_cancel: Rc<dyn Fn()>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("chooser-download");
        root.append(&crate::assets::primary_icon(
            crate::assets::icons::DOWNLOADS,
            16,
        ));
        let name = gtk::Label::new(Some(
            &crate::services::remote_file_name(url).unwrap_or_else(|| crate::i18n::tr("Download")),
        ));
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name.set_max_width_chars(28);
        root.append(&name);

        let fraction = Rc::new(Cell::new(0.0));
        let ring = gtk::DrawingArea::builder()
            .content_width(20)
            .content_height(20)
            .accessible_role(gtk::AccessibleRole::ProgressBar)
            .build();
        ring.add_css_class("chooser-download-progress");
        crate::ui::accessibility::set_label(&ring, &crate::i18n::tr("Download progress"));
        ring.update_property(&[
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(100.0),
            gtk::accessible::Property::ValueNow(0.0),
        ]);
        let drawn_fraction = fraction.clone();
        ring.set_draw_func(move |area, context, width, height| {
            let color = area.color();
            let center_x = f64::from(width) / 2.0;
            let center_y = f64::from(height) / 2.0;
            let radius = (f64::from(width.min(height)) - 3.0).max(0.0) / 2.0;
            context.set_line_width(2.0);
            context.set_line_cap(gtk::cairo::LineCap::Round);
            let red = f64::from(color.red());
            let green = f64::from(color.green());
            let blue = f64::from(color.blue());
            let alpha = f64::from(color.alpha());
            context.set_source_rgba(red, green, blue, alpha * 0.2);
            context.arc(center_x, center_y, radius, 0.0, TAU);
            let _ = context.stroke();
            if drawn_fraction.get() > 0.0 {
                context.set_source_rgba(red, green, blue, alpha);
                context.arc(
                    center_x,
                    center_y,
                    radius,
                    -FRAC_PI_2,
                    -FRAC_PI_2 + TAU * drawn_fraction.get(),
                );
                let _ = context.stroke();
            }
        });
        let spinner = gtk::Spinner::new();
        spinner.add_css_class("chooser-download-progress");
        spinner.set_size_request(20, 20);
        spinner.start();
        let indicator = gtk::Stack::new();
        indicator.set_valign(gtk::Align::Center);
        indicator.add_named(&ring, Some("known"));
        indicator.add_named(&spinner, Some("unknown"));
        indicator.set_visible_child_name("unknown");
        root.append(&indicator);

        let status = gtk::Label::new(Some(&crate::i18n::tr("Connecting…")));
        status.add_css_class("chooser-download-status");
        root.append(&status);
        let cancel = gtk::Button::new();
        cancel.add_css_class("job-action");
        cancel.set_tooltip_text(Some(&crate::i18n::tr("Cancel download")));
        crate::ui::accessibility::set_label(&cancel, &crate::i18n::tr("Cancel download"));
        cancel.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            14,
        )));
        cancel.connect_clicked(move |_| on_cancel());
        root.append(&cancel);
        Self {
            root,
            name,
            status,
            indicator,
            ring,
            spinner,
            fraction,
            cancel,
        }
    }

    pub(super) fn set_activity(&self, text: &str) {
        self.spinner.start();
        self.indicator.set_visible_child_name("unknown");
        self.status.set_text(text);
        self.status.set_visible(true);
        self.cancel
            .set_tooltip_text(Some(&crate::i18n::tr("Cancel image processing")));
        crate::ui::accessibility::set_label(
            &self.cancel,
            &crate::i18n::tr("Cancel image processing"),
        );
    }

    pub(super) fn set_name(&self, name: &str) {
        self.name.set_text(name);
    }

    pub(super) fn update(&self, downloaded: u64, total: Option<u64>) {
        match total.filter(|total| *total > 0) {
            Some(total) => {
                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                self.fraction.set(fraction);
                self.ring.update_property(&[
                    gtk::accessible::Property::ValueNow(fraction * 100.0),
                    gtk::accessible::Property::ValueText(&format!(
                        "{}% downloaded",
                        (fraction * 100.0) as usize
                    )),
                ]);
                self.ring.queue_draw();
                self.spinner.stop();
                self.indicator.set_visible_child_name("known");
                self.status.set_visible(false);
            }
            None => {
                self.spinner.start();
                self.indicator.set_visible_child_name("unknown");
                self.status
                    .set_text(&format!("{} downloaded", format_file_size(downloaded)));
                self.status.set_visible(true);
            }
        }
    }
}
