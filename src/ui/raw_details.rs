// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::FileEntry,
    sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation, raw_metadata::RawMetadata},
};

pub(super) fn supports(entry: &FileEntry) -> bool {
    !entry.is_directory()
        && (sandbox::raw_metadata::is_raw(Path::new(&entry.native_name))
            || sandbox::raw_metadata::is_raw(Path::new(&entry.display_name)))
}

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
                let reciprocal = value.recip();
                if value < 1.0 && (reciprocal - reciprocal.round()).abs() < 1e-6 {
                    format!("1/{reciprocal:.0} s")
                } else {
                    let seconds = format!("{value:.9}");
                    format!("{} s", seconds.trim_end_matches('0').trim_end_matches('.'))
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

pub(super) struct RawDetails {
    pub(super) section: gtk::Box,
    values: [gtk::Label; 7],
}

impl RawDetails {
    pub(super) fn new() -> Self {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let headings = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let values = rows(&RawMetadata::default()).map(|(name, value)| {
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
            label
        });
        Self { section, values }
    }

    pub(super) fn reset(&self) {
        for (label, (_, value)) in self.values.iter().zip(rows(&RawMetadata::default())) {
            label.set_text(&value);
            label.set_tooltip_text(Some(&value));
        }
    }

    pub(super) fn load(&self, path: Option<PathBuf>) -> MetadataLoad {
        let cancellation = Cancellation::default();
        let handle = MetadataLoad(cancellation.clone());
        self.reset();
        let Some(path) = path else { return handle };
        let values = self.values.each_ref().map(|label| label.downgrade());
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
            let metadata = result.ok().and_then(Result::ok).unwrap_or_default();
            for (label, (_, value)) in values.iter().zip(rows(&metadata)) {
                if let Some(label) = label.upgrade() {
                    label.set_text(&value);
                    label.set_tooltip_text(Some(&value));
                }
            }
        });
        handle
    }
}

pub(super) fn load(section: &gtk::Box, path: Option<PathBuf>) -> MetadataLoad {
    let details = RawDetails::new();
    section.append(&details.section);
    section.set_visible(true);
    details.load(path)
}

#[cfg(test)]
mod tests;
