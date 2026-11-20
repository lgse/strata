// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn media_preview_size(&self) -> MediaPreviewSize {
        let split = self.split.borrow();
        let width = split
            .as_ref()
            .filter(|split| split.width() > 0)
            .map(|split| {
                if self.animating.get() {
                    self.opening_width(split.width())
                } else {
                    split.width().saturating_sub(split.position())
                }
            })
            .filter(|width| *width > 0)
            .unwrap_or_else(|| {
                if self.content.width() > 0 {
                    self.content.width()
                } else {
                    DEFAULT_WIDTH
                }
            });
        let height = if self.content.height() > 0 {
            self.content.height()
        } else {
            split
                .as_ref()
                .map(|split| split.height())
                .filter(|height| *height > 0)
                .unwrap_or(DEFAULT_WIDTH)
        };
        MediaPreviewSize::for_viewport(
            width.min(media_layout::MAX_CONTENT_WIDTH),
            height,
            self.pane.scale_factor(),
        )
    }

    pub(super) fn retain_pending_playback(&self, entry: &FileEntry) {
        let keep = self
            .continue_playback
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.request.is_none() && pending.location == entry.location);
        if !keep {
            self.continue_playback.take();
        }
    }

    pub(super) fn load(self: &Rc<Self>, entry: FileEntry, pdf_page: i32) {
        self.load_with_password(entry, pdf_page, None);
    }

    pub(super) fn load_with_password(
        self: &Rc<Self>,
        entry: FileEntry,
        pdf_page: i32,
        archive_password: Option<SecretString>,
    ) {
        let render_document =
            super::super::preferences::PreferenceManager::shared().render_documents_by_default();
        self.document_view.set(if render_document {
            DocumentView::Rendered
        } else {
            DocumentView::Source
        });
        self.load_request(entry, pdf_page, render_document, archive_password);
    }

    pub(super) fn clear_raw_details(&self) {
        self.raw_metadata_load.borrow_mut().take();
        self.raw_details_scroll.set_visible(false);
        self.raw_details.reset();
    }

    pub(super) fn load_request(
        self: &Rc<Self>,
        entry: FileEntry,
        pdf_page: i32,
        render_document: bool,
        archive_password: Option<SecretString>,
    ) {
        if self
            .current
            .borrow()
            .as_ref()
            .is_none_or(|current| current.location != entry.location)
        {
            self.model_view.set(Default::default());
            self.model.borrow_mut().take();
        }
        self.raw_metadata_load.borrow_mut().take();
        self.released_from.set(None);
        self.load_raw_details(&entry);
        self.show_identity(&entry);
        self.load.borrow_mut().take();
        self.pdf_loads.borrow_mut().clear();

        let request_id = self.begin_request();
        self.retain_pending_playback(&entry);
        if let Some(pending) = self.continue_playback.borrow_mut().as_mut() {
            pending.request = Some(request_id);
        }
        if self.focus_archive_on_ready.replace(false) {
            self.focus_archive_request.set(Some(request_id));
        }
        self.show_loading(request_id);
        let weak = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.handle_event(request_id, event);
        });
        let load = self.provider.load(
            PreviewRequest {
                id: request_id,
                entry,
                text_byte_limit: TEXT_BYTE_LIMIT,
                render_document,
                pdf_page,
                media_size: self.media_preview_size(),
                model_palette: super::super::theme::ThemeManager::shared().active_model_palette(),
                model_view: self.model_view.get(),
                archive_password,
            },
            emit,
        );
        self.load.replace(Some(load));
    }

    fn load_raw_details(&self, entry: &FileEntry) {
        if super::super::raw_details::supports(entry) {
            let load = self
                .raw_details
                .load(entry.local_thumbnail_path().map(ToOwned::to_owned));
            self.raw_metadata_load.replace(Some(load));
            self.raw_details_scroll.set_visible(true);
        } else {
            self.clear_raw_details();
        }
    }

    /// Shows `entry` in the header and makes it the current file.
    fn show_identity(&self, entry: &FileEntry) {
        self.metadata.set_visible(true);
        self.icon.set_visible(true);
        self.open.set_sensitive(true);
        self.header_handle.set_cursor_from_name(Some("grab"));
        self.current.replace(Some(entry.clone()));
        crate::assets::set_primary_icon(&self.icon, super::super::browser::entry_icon(entry));
        self.title.set_text(&entry.display_name);
        crate::ui::accessibility::set_description(
            &self.title,
            Some(&entry.location.display_path()),
        );
        self.size.set_text(&metadata_size(entry));
        crate::ui::modified_date::set_modified_date(&self.modified, Some(entry), "—");
        if self.model.borrow().is_none() {
            self.content_type.set_text(file_extension(entry));
        }
    }

    fn begin_request(&self) -> PreviewRequestId {
        let request_id = PreviewRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        self.current_request.set(Some(request_id));
        request_id
    }

    pub(super) fn handle_event(self: &Rc<Self>, expected: PreviewRequestId, event: PreviewEvent) {
        let response = match &event {
            PreviewEvent::Progress { request_id, .. } => *request_id,
            PreviewEvent::Ready(preview) => preview.request_id,
            PreviewEvent::Failed { request_id, .. } => *request_id,
            PreviewEvent::NeedsPassword { request_id, .. } => *request_id,
        };
        if !accepts_preview_event(self.current_request.get(), expected, response) {
            return;
        }
        match event {
            PreviewEvent::Progress { stage, .. } => {
                if let Some(model) = self.model.borrow().as_ref() {
                    model.set_progress(stage);
                }
            }
            PreviewEvent::Ready(preview) if preview.request_id == expected => {
                self.cancel_loading();
                self.render(preview);
            }
            PreviewEvent::Failed {
                request_id,
                entry,
                message,
            } if request_id == expected => self.preview_failed(expected, entry, &message),
            PreviewEvent::NeedsPassword { request_id, entry } if request_id == expected => {
                self.continue_playback.take();
                self.load.borrow_mut().take();
                self.cancel_loading();
                self.title.set_text(&entry.display_name);
                self.render_archive_password_prompt(entry, None);
            }
            PreviewEvent::Ready(_)
            | PreviewEvent::Failed { .. }
            | PreviewEvent::NeedsPassword { .. } => {}
        }
    }

    fn preview_failed(
        self: &Rc<Self>,
        expected: PreviewRequestId,
        entry: FileEntry,
        message: &str,
    ) {
        self.continue_playback.take();
        self.load.borrow_mut().take();
        self.cancel_loading();
        self.current_request.set(Some(expected));
        self.title.set_text(&entry.display_name);
        // Failures carry English source messages: known ones have catalog
        // entries, and system error text passes through unchanged.
        let detail = crate::i18n::tr(message);
        if self.show_model_failure(&detail) {
            return;
        }
        let held = message == file_activity::HELD_FOR_OPERATION;
        if message == crate::services::INCORRECT_ARCHIVE_PASSWORD {
            self.render_archive_password_prompt(entry, Some(&detail));
        } else if held && !file_activity::is_releasing() {
            self.current_request.set(None);
            self.show_after_focus_change(entry, self.current_depth.get());
        } else {
            if held {
                self.released_from.set(Some(self.current_depth.get()));
            }
            self.current_request.set(None);
            self.show_failure(message, &detail);
        }
    }

    /// A model that already shows a picture keeps it under the error.
    fn show_model_failure(&self, detail: &str) -> bool {
        if let Some(model) = self
            .model
            .borrow()
            .as_ref()
            .filter(|model| model.picture.paintable().is_some())
        {
            self.model_view.set(model.view());
            model.show_error(detail);
            return true;
        }
        false
    }

    fn show_failure(&self, message: &str, detail: &str) {
        let title = crate::i18n::tr("Preview unavailable");
        let offers_open =
            message == crate::sandbox::PDF_PASSWORD_PROTECTED && self.allow_external_open;
        if offers_open {
            self.show_message_with_external_open(&title, detail);
        } else if !self.show_audio_error(&title, detail, None)
            && !self.show_video_error(&title, detail, None)
        {
            self.show_message(&title, detail);
        }
    }

    pub(super) fn show_loading(self: &Rc<Self>, request_id: PreviewRequestId) {
        if let Some(model) = self.model.borrow().as_ref() {
            self.cancel_loading();
            model.set_loading(self.model_view.get());
            return;
        }
        let family = self.current.borrow().as_ref().and_then(entry_family);
        self.reset_content(family);
        self.cancel_loading();
        if self.prepare_player() {
            return;
        }
        if self.show_model_loading() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(PREVIEW_SPINNER_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.loading_delay.borrow_mut().take();
            if state.current_request.get() != Some(request_id) {
                return;
            }
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("preview-spinner");
            spinner.set_halign(gtk::Align::Center);
            spinner.set_valign(gtk::Align::Center);
            spinner.start();
            spinner.set_vexpand(true);
            state.content.append(&spinner);
        });
        self.loading_delay.replace(Some(source));
    }

    /// A retained audio or video player shows the next file while it loads.
    fn prepare_player(self: &Rc<Self>) -> bool {
        if let Some(audio) = self.audio.borrow().as_ref() {
            let (folder, previous, next) = self.listing_position(MediaFamily::Audio);
            if let Some(entry) = self.current.borrow().as_ref() {
                audio.view.prepare(entry, folder, previous, next);
            }
            return true;
        }
        if let Some(video) = self.video.borrow().as_ref() {
            let (position, previous, next) = self.listing_position(MediaFamily::Video);
            if let Some(entry) = self.current.borrow().as_ref() {
                video.view.prepare(entry, position, previous, next);
            }
            return true;
        }
        false
    }

    pub(super) fn cancel_loading(&self) {
        if let Some(source) = self.loading_delay.borrow_mut().take() {
            source.remove();
        }
    }
}

pub(super) fn accepts_preview_event(
    current: Option<PreviewRequestId>,
    expected: PreviewRequestId,
    response: PreviewRequestId,
) -> bool {
    current == Some(expected) && response == expected
}
