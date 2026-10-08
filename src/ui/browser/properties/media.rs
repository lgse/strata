// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use gtk::{gio, glib, prelude::*};

use crate::sandbox::{
    self, Cancellation, MediaPreviewBackend, ParseOperation, metadata::MediaMetadata,
};

pub(super) use crate::ui::raw_details::MetadataLoad;

fn append_row(parent: &gtk::Box, name: &str, value: &str) -> gtk::Label {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let heading = gtk::Label::new(Some(&crate::i18n::tr(name)));
    heading.set_xalign(0.0);
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_selectable(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(48);
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
        rows.push((
            "RESOLUTION",
            rust_i18n::t!(
                "%{width} × %{height} pixels",
                width = width,
                height = height
            )
            .into_owned(),
        ));
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
                rust_i18n::t!(
                    "%{rate} Mb/s",
                    rate = crate::i18n::decimal(bitrate / 1_000_000.0, 2)
                )
                .into_owned()
            } else {
                rust_i18n::t!(
                    "%{rate} kb/s",
                    rate = crate::i18n::decimal(bitrate / 1000.0, 0)
                )
                .into_owned()
            },
        ));
    }
    if let Some(codec) = &metadata.video_codec {
        rows.push(("VIDEO CODEC", codec.clone()));
    }
    if let Some(hdr) = metadata.hdr_format() {
        rows.push(("HDR", hdr.into()));
    }
    if let Some(rate) = metadata.frame_rate {
        rows.push((
            "FRAME RATE",
            rust_i18n::t!("%{rate} fps", rate = crate::i18n::decimal(rate, 2)).into_owned(),
        ));
    }
    if let Some(codec) = &metadata.audio_codec {
        rows.push(("AUDIO CODEC", codec.clone()));
    }
    if let Some(rate) = metadata.sample_rate {
        rows.push((
            "SAMPLE RATE",
            rust_i18n::t!("%{rate} kHz", rate = crate::i18n::decimal(rate / 1000.0, 1))
                .into_owned(),
        ));
    }
    if let Some(channels) = metadata.channels {
        rows.push((
            "CHANNELS",
            match channels {
                1 => crate::i18n::tr("1 (Mono)"),
                2 => crate::i18n::tr("2 (Stereo)"),
                _ => channels.to_string(),
            },
        ));
    }
    if !metadata.subtitle_tracks.is_empty() {
        let languages: Vec<&str> = metadata
            .subtitle_tracks
            .iter()
            .filter_map(|track| track.language.as_deref())
            .collect();
        rows.push((
            "SUBTITLES",
            if languages.is_empty() {
                metadata.subtitle_tracks.len().to_string()
            } else {
                format!(
                    "{} ({})",
                    metadata.subtitle_tracks.len(),
                    languages.join(", ")
                )
            },
        ));
    }
    if !metadata.chapters.is_empty() {
        rows.push(("CHAPTERS", metadata.chapters.len().to_string()));
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
            append_row(&section, "MEDIA", &crate::i18n::tr("Loading…"));
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
            append_row(&section, "MEDIA", &crate::i18n::tr("Unavailable"));
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
