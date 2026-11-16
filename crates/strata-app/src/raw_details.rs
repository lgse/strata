// SPDX-License-Identifier: MIT

//! Shared RAW camera metadata detection, row formatting, sandboxed loading, and section rendering
//! (`src/ui/raw_details.rs`), used by both the preview drawer (`crate::preview`) and single-file
//! Properties (`crate::properties`).

use std::path::{Path, PathBuf};

use rune::{El, Tx, col, row, text};
use strata_components::dialog::keep_words_whole;
use strata_core::model::FileEntry;
use strata_kit::look::Look;
use strata_preview::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, raw_metadata::RawMetadata,
};

use crate::{App, slots::Later};

pub(crate) type RawRows = [(&'static str, String); 7];

pub(crate) fn supports(entry: &FileEntry) -> bool {
    !entry.is_directory()
        && (sandbox::raw_metadata::is_raw(Path::new(&entry.native_name))
            || sandbox::raw_metadata::is_raw(Path::new(&entry.display_name)))
}

pub(crate) fn default_rows() -> RawRows {
    rows(&RawMetadata::default())
}

pub(crate) fn rows(metadata: &RawMetadata) -> RawRows {
    [
        (
            "DIMENSIONS",
            available(dimensions_text(metadata.dimensions)),
        ),
        ("CAMERA", available(metadata.camera.clone())),
        ("LENS", available(metadata.lens.clone())),
        (
            "FOCAL LENGTH",
            available(
                metadata
                    .focal_length
                    .map(|mm| format!("{} mm", decimal(mm))),
            ),
        ),
        (
            "SHUTTER SPEED",
            available(metadata.shutter_speed.map(shutter_speed_text)),
        ),
        ("ISO", available(metadata.iso.map(decimal))),
        ("GPS COORDINATES", available(metadata.gps.map(gps_text))),
    ]
}

fn available(value: Option<String>) -> String {
    value.unwrap_or_else(|| "N/A".to_owned())
}

fn dimensions_text(dimensions: Option<(u32, u32)>) -> Option<String> {
    dimensions.map(|(width, height)| format!("{width} × {height} pixels"))
}

fn decimal(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn shutter_speed_text(value: f64) -> String {
    let reciprocal = value.recip();
    if value < 1.0 && (reciprocal - reciprocal.round()).abs() < 1e-6 {
        return format!("1/{reciprocal:.0} s");
    }
    let seconds = format!("{value:.9}");
    format!("{} s", seconds.trim_end_matches('0').trim_end_matches('.'))
}

fn gps_text((latitude, longitude): (f64, f64)) -> String {
    format!("{latitude:.6}, {longitude:.6}")
}

pub(crate) struct RawLoad {
    cancel: Cancellation,
    task: Option<glib::JoinHandle<()>>,
}

impl RawLoad {
    pub(crate) fn cancel(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

impl Drop for RawLoad {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn spawn(
    path: Option<PathBuf>,
    later: Later<App>,
    apply: impl FnOnce(&mut App, RawMetadata) -> bool + 'static,
) -> RawLoad {
    let cancel = Cancellation::default();
    let Some(path) = path else {
        return RawLoad { cancel, task: None };
    };
    let task_cancel = cancel.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let worker_cancel = task_cancel.clone();
        let result = gio::spawn_blocking(move || {
            sandbox::parse(
                &path,
                ParseOperation::RawMetadata,
                0,
                MediaPreviewBackend::Software,
                &worker_cancel,
            )
            .and_then(|output| RawMetadata::from_json(&output.data))
        })
        .await;
        if task_cancel.is_cancelled() {
            return;
        }
        let metadata = result.ok().and_then(Result::ok).unwrap_or_default();
        later.then(move |app: &mut App, _: &mut Tx| apply(app, metadata));
    });
    RawLoad {
        cancel,
        task: Some(task),
    }
}

pub(crate) fn section_el(look: &Look, key: &str, values: &RawRows, label_width: f32) -> El {
    let kit = &look.kit;
    let mut section = col().key(key.to_owned()).w_full().gap(look.space.xs);
    for (name, value) in values {
        let line = row()
            .w_full()
            .gap(look.space.md)
            .center_y()
            .child(
                text((*name).to_owned())
                    .label((*name).to_owned())
                    .font(look.ty.heading)
                    .color(kit.c.text_muted)
                    .w(label_width),
            )
            .child(
                text(keep_words_whole(value))
                    .label(value.clone())
                    .description((*name).to_owned())
                    .font(kit.ty.ui)
                    .color(kit.c.text_base)
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
            );
        section = section.child(line);
    }
    section
}
