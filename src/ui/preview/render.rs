// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn render(self: &Rc<Self>, preview: Preview) {
        let continue_playback = self
            .continue_playback
            .take()
            .is_some_and(|pending| pending.request == Some(preview.request_id));
        self.content_type.set_text(&preview.content_type);
        let reuse_model = matches!(preview.content, PreviewContent::Model { mesh: Some(_), .. })
            && self.model.borrow().is_some();
        if !reuse_model {
            self.reset_content(
                matches!(preview.content, PreviewContent::SandboxedMedia { .. })
                    .then(|| media_family(&preview.content_type))
                    .flatten(),
            );
        }
        let Preview {
            request_id,
            entry,
            content_type,
            content,
        } = preview;
        self.zoom.set_enabled(matches!(
            content,
            PreviewContent::Text { .. }
                | PreviewContent::Document { .. }
                | PreviewContent::Rendered { .. }
                | PreviewContent::Rasterized { .. }
                | PreviewContent::Pdf { .. }
        ));
        match content {
            PreviewContent::Text { content, truncated } => self.render_text(PendingSourcePreview {
                entry,
                content_type,
                content,
                truncated,
            }),
            PreviewContent::Document {
                source,
                document,
                fallback_reason,
                warnings,
                truncated,
            } => {
                self.show_wrap_toggle();
                self.render_document_preview(
                    PendingSourcePreview {
                        entry,
                        content_type,
                        content: source,
                        truncated,
                    },
                    document,
                    fallback_reason,
                    warnings,
                );
            }
            PreviewContent::Rendered { document, warnings } => {
                let word_processor =
                    crate::sandbox::DocumentFormat::for_file(&content_type, &entry.native_name)
                        .is_some();
                self.render_rendered(document, warnings, word_processor);
            }
            PreviewContent::Route { preview } => self.render_route(preview),
            PreviewContent::Model { png, mesh } => self.render_model(png, mesh),
            PreviewContent::Rasterized { png } => self.render_rasterized(&entry, png),
            PreviewContent::SandboxedMedia { media } => self.render_sandboxed_media(
                SandboxedPreview {
                    entry,
                    content_type,
                    continue_playback,
                },
                media,
            ),
            PreviewContent::Image | PreviewContent::Media => {
                self.show_message(
                    &crate::i18n::tr("Preview unavailable"),
                    &crate::i18n::tr("The sandboxed renderer returned no preview"),
                );
            }
            PreviewContent::Pdf {
                png,
                page,
                pages,
                text_layer,
            } => {
                self.render_pdf_viewer(
                    entry,
                    pdf::PdfFirstPage {
                        png,
                        page,
                        text_layer,
                    },
                    pages,
                );
            }
            PreviewContent::Archive { tree } => {
                let focus = self.focus_archive_request.get() == Some(request_id);
                self.focus_archive_request.set(None);
                self.render_archive(tree, focus);
            }
            PreviewContent::Unsupported => {
                self.show_message(
                    &crate::i18n::tr("No visual preview"),
                    &crate::i18n::tr("Metadata is available for this file type."),
                );
            }
        }
        self.refresh_find();
        self.hand_keys_to_document();
    }

    fn show_wrap_toggle(&self) {
        self.wrap.set_visible(true);
        self.wrap
            .set_active(super::super::preferences::PreferenceManager::shared().preview_text_wrap());
    }

    fn render_text(&self, source: PendingSourcePreview) {
        self.wrap.set_visible(true);
        let (view, virtualized) = self.source_preview.show(
            &source.entry,
            &source.content_type,
            &source.content,
            source.truncated,
        );
        self.content.append(&view);
        self.text_view
            .replace(Some(self.source_preview.view.clone()));
        self.text_scroll
            .replace(self.source_preview.scroll.borrow().clone());
        self.wrap
            .set_active(super::super::preferences::PreferenceManager::shared().preview_text_wrap());
        if source.truncated && !virtualized {
            let notice =
                gtk::Label::new(Some(&crate::i18n::tr("Preview limited to the first 1 MB")));
            notice.add_css_class("preview-note");
            self.content.append(&notice);
        }
    }

    fn render_rendered(
        &self,
        document: DocumentLayout,
        warnings: Vec<String>,
        word_processor: bool,
    ) {
        let (view, state) = super::super::virtual_preview::rendered_document(
            document,
            warnings,
            word_processor,
            None,
        );
        self.rendered_text.replace(Some(state));
        self.content.append(&view);
    }

    fn render_rasterized(self: &Rc<Self>, entry: &FileEntry, png: Vec<u8>) {
        let bytes = glib::Bytes::from_owned(png);
        match gtk::gdk::Texture::from_bytes(&bytes) {
            Ok(texture) => {
                let picture = gtk::Picture::for_paintable(&texture);
                let label =
                    rasterized_label(entry).unwrap_or_else(|| crate::i18n::tr("Image preview"));
                super::super::accessibility::set_label(&picture, &label);
                picture.add_css_class("preview-image");
                picture.set_can_shrink(true);
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_hexpand(true);
                picture.set_vexpand(true);
                picture.set_cursor_from_name(Some("grab"));
                install_preview_drag(&picture, self);
                self.content.append(&self.zoom.image(&picture, &texture));
            }
            Err(error) => {
                self.show_message(&crate::i18n::tr("Preview unavailable"), &error.to_string())
            }
        }
    }

    pub(super) fn clear_password_entry(&self) {
        if let Some(entry) = self.password_entry.borrow_mut().take() {
            entry.set_text("");
        }
    }

    pub(super) fn clear_content(&self) {
        self.reset_content(None);
    }

    pub(super) fn reset_content(&self, keep: Option<MediaFamily>) {
        self.zoom.clear();
        if let Some(route) = self.route.borrow_mut().take() {
            route.pause();
        }
        self.model.borrow_mut().take();
        let owned = self.content_owns_keys();
        self.detach_find();
        self.rendered_text.borrow_mut().take();
        self.source_preview.cancel();
        self.source_preview.scroll.borrow_mut().take();
        self.source_preview.virtual_state.borrow_mut().take();
        self.document_view_button.set_visible(false);
        self.document_preview.borrow_mut().take();
        let audio = self.audio.borrow_mut().take();
        if let Some(audio) = audio.as_ref() {
            audio.view.detach();
        }
        let video = self.video.borrow_mut().take();
        if let Some(video) = video.as_ref() {
            video.view.detach();
        }
        self.stop_media();
        let retained = match keep {
            Some(MediaFamily::Audio) => audio.is_some(),
            Some(MediaFamily::Video) => video.is_some(),
            None | Some(MediaFamily::Route) => false,
        };
        if !retained {
            self.media_checks.borrow_mut().stop();
            self.media_toggle_mute.replace(None);
            self.media_volume_slider.replace(None);
        }
        self.wrap.set_visible(false);
        self.text_view.take();
        self.text_scroll.take();
        self.archive_browser.take();
        self.clear_password_entry();
        match (keep, audio, video) {
            (Some(MediaFamily::Audio), Some(audio), _) => {
                self.audio.replace(Some(audio));
            }
            (Some(MediaFamily::Video), _, Some(video)) => {
                self.video.replace(Some(video));
            }
            _ => clear_box(&self.content),
        }
        self.keep_keys_in_content(owned);
    }
}

fn rasterized_label(entry: &FileEntry) -> Option<String> {
    if crate::services::is_model(entry) {
        Some(crate::i18n::tr("Model preview"))
    } else if crate::sandbox::CoverFormat::for_name(&entry.native_name).is_some() {
        Some(crate::i18n::tr("Cover preview"))
    } else {
        None
    }
}
