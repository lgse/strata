// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use gtk::{gio, glib, prelude::*};

use crate::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, metadata::MediaMetadata,
};

pub(super) struct MetadataLoad(Cancellation);

impl Drop for MetadataLoad {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl MetadataLoad {
    pub(super) fn cancel(&self) {
        self.0.cancel();
    }
}

fn append_row(parent: &gtk::Box, name: &str, value: &str) -> gtk::Label {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let heading = gtk::Label::new(Some(name));
    heading.set_xalign(0.0);
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_selectable(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(48);
    label.set_tooltip_text(Some(value));
    row.add_css_class("properties-row");
    heading.add_css_class("properties-row-label");
    label.add_css_class("properties-row-value");
    row.append(&heading);
    row.append(&label);
    parent.append(&row);
    heading
}

fn rows(metadata: &MediaMetadata) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some((width, height)) = metadata.dimensions {
        rows.push(("RESOLUTION", format!("{width} × {height} pixels")));
    }
    if let Some(duration) = metadata.duration {
        let seconds = duration.round() as u64;
        rows.push((
            "DURATION",
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            ),
        ));
    }
    if let Some(bitrate) = metadata.bitrate {
        rows.push((
            "BITRATE",
            if bitrate >= 1_000_000.0 {
                format!("{:.2} Mb/s", bitrate / 1_000_000.0)
            } else {
                format!("{:.0} kb/s", bitrate / 1000.0)
            },
        ));
    }
    if let Some(codec) = &metadata.video_codec {
        rows.push(("VIDEO CODEC", codec.clone()));
    }
    if let Some(rate) = metadata.frame_rate {
        rows.push(("FRAME RATE", format!("{rate:.2} fps")));
    }
    if let Some(codec) = &metadata.audio_codec {
        rows.push(("AUDIO CODEC", codec.clone()));
    }
    if let Some(rate) = metadata.sample_rate {
        rows.push(("SAMPLE RATE", format!("{:.1} kHz", rate / 1000.0)));
    }
    if let Some(channels) = metadata.channels {
        rows.push((
            "CHANNELS",
            match channels {
                1 => "1 (Mono)".into(),
                2 => "2 (Stereo)".into(),
                _ => channels.to_string(),
            },
        ));
    }
    rows
}

pub(super) fn load(section: &gtk::Box, path: PathBuf) -> MetadataLoad {
    let cancellation = Cancellation::default();
    let handle = MetadataLoad(cancellation.clone());
    while let Some(child) = section.first_child() {
        section.remove(&child);
    }
    section.set_visible(false);
    let section = section.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let file = gio::File::for_path(&path);
        let Ok(info) = file
            .query_info_future(
                "standard::content-type,standard::type",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
        else {
            return;
        };
        if cancellation.is_cancelled() || info.file_type() != gio::FileType::Regular {
            return;
        }
        let Some(mime) = info
            .content_type()
            .and_then(|content_type| gio::content_type_get_mime_type(&content_type))
        else {
            return;
        };
        let image = mime.starts_with("image/");
        if !image && !mime.starts_with("audio/") && !mime.starts_with("video/") {
            return;
        }
        if let Some(section) = section.upgrade() {
            append_row(&section, "MEDIA", "Loading…");
            section.set_visible(true);
        } else {
            return;
        }
        let worker_cancellation = cancellation.clone();
        let result = gio::spawn_blocking(move || {
            sandbox::parse(
                &path,
                ParseOperation::MediaMetadata,
                0,
                MediaPreviewBackend::Software,
                &worker_cancellation,
            )
            .and_then(|output| MediaMetadata::from_json(&output.data, image))
        })
        .await;
        if cancellation.is_cancelled() {
            return;
        }
        let Some(section) = section.upgrade() else {
            return;
        };
        while let Some(child) = section.first_child() {
            section.remove(&child);
        }
        let rows = result
            .ok()
            .and_then(Result::ok)
            .map(|metadata| rows(&metadata))
            .unwrap_or_default();
        if rows.is_empty() {
            append_row(&section, "MEDIA", "Unavailable");
            return;
        }
        let headings = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        for (name, value) in rows {
            headings.add_widget(&append_row(&section, name, &value));
        }
    });
    handle
}

#[cfg(test)]
mod tests;
