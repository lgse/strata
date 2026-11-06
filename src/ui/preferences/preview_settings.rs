// SPDX-License-Identifier: MIT

//! Previews and thumbnails: documents, video backends, playback, and thumbnail rendering.

use super::*;

impl PreferenceManager {
    pub fn render_documents_by_default(&self) -> bool {
        self.preferences.borrow().render_documents_by_default
    }

    pub fn set_render_documents_by_default(&self, enabled: bool) {
        self.preferences.borrow_mut().render_documents_by_default = enabled;
        self.save_preferences();
    }

    pub fn hardware_accelerated_video_previews(&self) -> bool {
        configured_hardware_acceleration(
            &self.preferences.borrow(),
            crate::sandbox::polaris_gpu_available(),
        )
    }

    pub fn set_hardware_accelerated_video_previews(&self, enabled: bool) {
        self.preferences
            .borrow_mut()
            .hardware_accelerated_video_previews = Some(enabled);
        self.save_preferences();
    }

    pub fn video_preview_backend(&self) -> MediaPreviewBackend {
        configured_video_preview_backend(&self.preferences.borrow())
    }

    pub fn set_video_preview_backend(&self, backend: MediaPreviewBackend) {
        let backend = match backend {
            MediaPreviewBackend::Automatic => "automatic",
            MediaPreviewBackend::VaApi => "vaapi",
            MediaPreviewBackend::Vulkan => "vulkan",
            MediaPreviewBackend::Software => return,
        };
        self.preferences.borrow_mut().video_preview_backend = backend.to_owned();
        self.save_preferences();
    }

    pub(crate) fn media_preview_backend(&self) -> MediaPreviewBackend {
        if !self.hardware_accelerated_video_previews() {
            MediaPreviewBackend::Software
        } else {
            self.video_preview_backend()
        }
    }

    pub fn preview_muted(&self) -> bool {
        self.preferences.borrow().preview_muted
    }

    pub fn set_preview_muted(&self, muted: bool) {
        self.preferences.borrow_mut().preview_muted = muted;
        self.save_preferences();
    }

    pub fn preview_volume(&self) -> f64 {
        self.preferences.borrow().preview_volume
    }

    pub fn set_preview_volume(&self, volume: f64) {
        self.preferences.borrow_mut().preview_volume = normalized_volume(volume);
        self.save_preferences();
    }

    pub(crate) fn unit_system(&self) -> UnitSystem {
        self.preferences.borrow().unit_system
    }

    pub(crate) fn set_unit_system(&self, units: UnitSystem) {
        self.preferences.borrow_mut().unit_system = units;
        self.save_preferences();
    }

    pub fn preview_text_wrap(&self) -> bool {
        self.preferences.borrow().preview_text_wrap
    }

    pub fn set_preview_text_wrap(&self, wrapped: bool) {
        self.preferences.borrow_mut().preview_text_wrap = wrapped;
        self.save_preferences();
    }

    pub fn preview_autoplay(&self) -> bool {
        self.preferences.borrow().preview_autoplay
    }

    pub fn set_preview_autoplay(&self, autoplay: bool) {
        self.preferences.borrow_mut().preview_autoplay = autoplay;
        self.save_preferences();
    }

    /// Percent of the duration a first-time video preview starts at.
    pub fn preview_video_start(&self) -> u32 {
        self.preferences.borrow().preview_video_start
    }

    pub fn set_preview_video_start(&self, percent: u32) {
        self.preferences.borrow_mut().preview_video_start = normalized_video_start(percent);
        self.save_preferences();
    }

    pub fn set_preview_audio(&self, volume: f64, muted: bool) {
        self.preferences.borrow_mut().preview_muted = muted;
        if volume > 0.0 {
            self.set_preview_volume(volume);
        } else {
            self.save_preferences();
        }
    }

    pub fn show_thumbnails(&self) -> bool {
        self.preferences.borrow().show_thumbnails
    }

    pub fn set_show_thumbnails(&self, enabled: bool) {
        self.preferences.borrow_mut().show_thumbnails = enabled;
        self.save_preferences();
    }

    pub fn thumbnail_workers(&self) -> usize {
        self.preferences.borrow().thumbnail_workers
    }

    pub fn stl_thumbnails(&self) -> bool {
        self.preferences.borrow().stl_thumbnails
    }

    pub fn set_stl_thumbnails(&self, enabled: bool) {
        self.preferences.borrow_mut().stl_thumbnails = enabled;
        self.save_preferences();
    }

    pub fn set_thumbnail_workers(&self, workers: usize) {
        self.preferences.borrow_mut().thumbnail_workers =
            workers.clamp(1, crate::sandbox::browser::MAX_WORKERS);
        self.save_preferences();
    }

    pub fn icons_thumbnail_size(&self) -> i32 {
        self.preferences.borrow().icons_thumbnail_size
    }

    pub fn set_icons_thumbnail_size(&self, size: i32) {
        self.preferences.borrow_mut().icons_thumbnail_size =
            size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        self.save_preferences();
    }
}
