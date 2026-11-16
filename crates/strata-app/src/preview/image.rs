// SPDX-License-Identifier: MIT

//! C14 [`ImageView`] adapter for still images, SVGs, and sandboxed covers (`PreviewContent::Rasterized`).

use rune::{Cx, El, Pixels};
use strata_components::viewers::{
    image::ImageView,
    viewport::{Mode, ViewportState},
};
use strata_core::{i18n::tr, model::FileEntry};

use crate::App;

/// Key of the still-image viewer inside the preview drawer.
pub(crate) const IMAGE_KEY: &str = "preview-image";

/// GPU texture ceiling (`Viewport` module docs): larger renders are downsampled for upload while
/// keeping `size_px` and `ViewportState::set_content` at the original pixel dimensions.
const MAX_GPU_EDGE: u32 = 8192;

pub(super) struct ImageViewer {
    pub(super) viewport: ViewportState,
    pixels: Pixels,
    name: String,
    size_px: [u32; 2],
}

impl ImageViewer {
    pub(super) fn from_png(entry: &FileEntry, png: &[u8]) -> Result<Self, String> {
        let decoded = image::load_from_memory(png).map_err(|err| err.to_string())?;
        let orig_w = decoded.width().max(1);
        let orig_h = decoded.height().max(1);
        let uploaded = if orig_w > MAX_GPU_EDGE || orig_h > MAX_GPU_EDGE {
            decoded.resize(
                MAX_GPU_EDGE,
                MAX_GPU_EDGE,
                image::imageops::FilterType::Triangle,
            )
        } else {
            decoded
        };
        let rgba = uploaded.to_rgba8();
        let (tex_w, tex_h) = (rgba.width(), rgba.height());
        let pixels = Pixels::new();
        pixels.set(tex_w, tex_h, rgba.into_raw());

        let mut viewport = ViewportState::new(Mode::Fit);
        viewport.set_content(Some([orig_w as f32, orig_h as f32]));

        Ok(Self {
            viewport,
            pixels,
            name: image_label(entry),
            size_px: [orig_w, orig_h],
        })
    }

    pub(super) fn reset(&mut self) {
        let _ = self.viewport.reset();
    }
}

fn image_label(entry: &FileEntry) -> String {
    if strata_fs::services::is_model(entry) {
        tr("Model preview")
    } else if strata_preview::sandbox::CoverFormat::for_name(&entry.native_name).is_some() {
        tr("Cover preview")
    } else {
        entry.display_name.clone()
    }
}

pub(super) fn build_view(
    viewer: &ImageViewer,
    app: &App,
    cx: &mut Cx<App>,
    access: fn(&mut App) -> &mut ViewportState,
) -> El {
    ImageView::new(&viewer.viewport, access)
        .key(IMAGE_KEY)
        .pixels(&viewer.pixels)
        .name(viewer.name.clone())
        .size_px(viewer.size_px)
        .drag_source(crate::dnd::binder(app), crate::dnd::preview_drag_source())
        .build(&app.look, cx)
}
