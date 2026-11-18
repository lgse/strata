// SPDX-License-Identifier: MIT

//! The previews that are a picture or a player, from the provider's answer: a still image, a
//! model, a route, sandboxed media and a PDF page each go to their viewer.

use rune::Tx;
use strata_core::{i18n::tr, model::FileEntry};
use strata_fs::services::{Preview, PreviewContent};

use super::{image::ImageViewer, media, model, pdf, route, viewer::Viewer};
use crate::App;

/// A preview that is a picture or a player: an image, a model, a route, media or a PDF page.
pub(super) fn viewer(app: &mut App, loaded: Preview, tx: &mut Tx) -> Viewer {
    let Preview {
        entry,
        content_type,
        content,
        ..
    } = loaded;
    match content {
        PreviewContent::Rasterized { png } => rasterized(&entry, &png),
        PreviewContent::Model { png, mesh } => model::ready(app, entry, png, mesh),
        PreviewContent::Route { preview } => route::ready(app, entry, preview),
        PreviewContent::SandboxedMedia { media } => {
            let arrived = media::Arrived {
                entry,
                content_type,
                media,
            };
            media::ready(app, arrived, tx)
        }
        PreviewContent::Pdf {
            png,
            page,
            pages,
            text_layer,
        } => pdf::viewer(entry, (&png, page, pages), text_layer),
        PreviewContent::Image | PreviewContent::Media => {
            unavailable(Some("The sandboxed renderer returned no preview"))
        }
        PreviewContent::Unsupported => Viewer::Message {
            title: tr("No visual preview"),
            detail: Some(tr("Metadata is available for this file type.")),
            can_open: false,
        },
        PreviewContent::Archive { .. }
        | PreviewContent::Text { .. }
        | PreviewContent::Document { .. }
        | PreviewContent::Rendered { .. } => unavailable(None),
    }
}

pub(super) fn unavailable(reason: Option<&str>) -> Viewer {
    Viewer::Message {
        title: tr("Preview unavailable"),
        detail: reason.map(tr),
        can_open: false,
    }
}

fn rasterized(entry: &FileEntry, png: &[u8]) -> Viewer {
    match ImageViewer::from_png(entry, png) {
        Ok(img) => Viewer::Image(Box::new(img)),
        Err(detail) => Viewer::Message {
            title: tr("Preview unavailable"),
            detail: Some(detail),
            can_open: false,
        },
    }
}
