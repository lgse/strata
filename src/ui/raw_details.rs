// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use gtk::{gio, glib, prelude::*};

use crate::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, raw_metadata::RawMetadata,
};

pub(super) struct MetadataLoad(pub(super) Cancellation);

impl Drop for MetadataLoad {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl MetadataLoad {
    pub(super) fn cancel(&self) {
        self.0.cancel();
    }
}

fn decimal(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn rows(metadata: &RawMetadata) -> [(&'static str, String); 7] {
    let available = |value: Option<String>| value.unwrap_or_else(|| "N/A".into());
    [
        (
            "DIMENSIONS",
            available(
                metadata
                    .dimensions
                    .map(|(width, height)| format!("{width} × {height} pixels")),
            ),
        ),
        ("CAMERA", available(metadata.camera.clone())),
        ("LENS", available(metadata.lens.clone())),
        (
            "FOCAL LENGTH",
            available(
                metadata
                    .focal_length
                    .map(|value| format!("{} mm", decimal(value))),
            ),
        ),
        (
            "SHUTTER SPEED",
            available(metadata.shutter_speed.map(|value| {
                if value < 1.0 {
                    format!("1/{} s", decimal(1.0 / value))
                } else {
                    format!("{} s", decimal(value))
                }
            })),
        ),
        ("ISO", available(metadata.iso.map(decimal))),
        (
            "GPS COORDINATES",
            available(
                metadata
                    .gps
                    .map(|(latitude, longitude)| format!("{latitude:.6}, {longitude:.6}")),
            ),
        ),
    ]
}

fn show(section: &gtk::Box, metadata: &RawMetadata) {
    while let Some(child) = section.first_child() {
        section.remove(&child);
    }
    let headings = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    for (name, value) in rows(metadata) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("properties-row");
        let heading = gtk::Label::new(Some(name));
        heading.add_css_class("properties-row-label");
        heading.set_xalign(0.0);
        headings.add_widget(&heading);
        let label = gtk::Label::new(Some(&value));
        label.add_css_class("properties-row-value");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_selectable(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(32);
        label.set_tooltip_text(Some(&value));
        label.update_property(&[gtk::accessible::Property::Description(name)]);
        row.append(&heading);
        row.append(&label);
        section.append(&row);
    }
    section.set_visible(true);
}

pub(super) fn load(section: &gtk::Box, path: Option<PathBuf>) -> MetadataLoad {
    let cancellation = Cancellation::default();
    let handle = MetadataLoad(cancellation.clone());
    show(section, &RawMetadata::default());
    let Some(path) = path else { return handle };
    let section = section.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let worker_cancellation = cancellation.clone();
        let result = gio::spawn_blocking(move || {
            sandbox::parse(
                &path,
                ParseOperation::RawMetadata,
                0,
                MediaPreviewBackend::Software,
                &worker_cancellation,
            )
            .and_then(|output| RawMetadata::from_json(&output.data))
        })
        .await;
        if cancellation.is_cancelled() {
            return;
        }
        if let Some(section) = section.upgrade() {
            let metadata = result.ok().and_then(Result::ok).unwrap_or_default();
            show(&section, &metadata);
        }
    });
    handle
}

#[cfg(test)]
mod tests;
