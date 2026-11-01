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
        match preview.content {
            PreviewContent::Text { content, truncated } => {
                self.wrap.set_visible(true);
                let (view, virtualized) = self.source_preview.show(
                    &preview.entry,
                    &preview.content_type,
                    &content,
                    truncated,
                );
                self.content.append(&view);
                self.text_view
                    .replace(Some(self.source_preview.view.clone()));
                self.text_scroll
                    .replace(self.source_preview.scroll.borrow().clone());
                self.wrap.set_active(
                    super::super::preferences::PreferenceManager::shared().preview_text_wrap(),
                );
                if truncated && !virtualized {
                    let notice = gtk::Label::new(Some(&crate::i18n::tr(
                        "Preview limited to the first 1 MB",
                    )));
                    notice.add_css_class("preview-note");
                    self.content.append(&notice);
                }
            }
            PreviewContent::Document {
                source,
                document,
                fallback_reason,
                warnings,
                truncated,
            } => {
                self.wrap.set_visible(true);
                self.wrap.set_active(
                    super::super::preferences::PreferenceManager::shared().preview_text_wrap(),
                );
                self.render_document_preview(
                    PendingSourcePreview {
                        entry: preview.entry,
                        content_type: preview.content_type,
                        content: source,
                        truncated,
                    },
                    document,
                    fallback_reason,
                    warnings,
                );
            }
            PreviewContent::Rendered { document, warnings } => {
                let word_processor = crate::sandbox::DocumentFormat::for_file(
                    &preview.content_type,
                    &preview.entry.native_name,
                )
                .is_some();
                let (view, state) = super::super::virtual_preview::rendered_document(
                    document,
                    warnings,
                    word_processor,
                    None,
                );
                self.rendered_text.replace(Some(state));
                self.content.append(&view);
            }
            PreviewContent::Route { preview } => self.render_route(preview),
            PreviewContent::Model { png, mesh } => self.render_model(png, mesh),
            PreviewContent::Rasterized { png } => {
                let model = crate::services::is_model(&preview.entry);
                let cover =
                    crate::sandbox::CoverFormat::for_name(&preview.entry.native_name).is_some();
                let bytes = glib::Bytes::from_owned(png);
                match gtk::gdk::Texture::from_bytes(&bytes) {
                    Ok(texture) => {
                        let picture = gtk::Picture::for_paintable(&texture);
                        if model {
                            super::super::accessibility::set_label(
                                &picture,
                                &crate::i18n::tr("Model preview"),
                            );
                        } else if cover {
                            super::super::accessibility::set_label(
                                &picture,
                                &crate::i18n::tr("Cover preview"),
                            );
                        }
                        picture.add_css_class("preview-image");
                        picture.set_can_shrink(true);
                        picture.set_content_fit(gtk::ContentFit::Contain);
                        picture.set_hexpand(true);
                        picture.set_vexpand(true);
                        picture.set_cursor_from_name(Some("grab"));
                        install_preview_drag(&picture, self);
                        self.content
                            .append(&media_layout::section(&picture, &texture));
                    }
                    Err(error) => self
                        .show_message(&crate::i18n::tr("Preview unavailable"), &error.to_string()),
                }
            }
            PreviewContent::SandboxedMedia { media: mut source } => {
                let is_gif = preview.content_type == "image/gif";
                let family = media_family(&preview.content_type);
                let handed_off = self.handed_off.borrow().as_ref() == Some(&preview.entry.location);
                source.audio_only = family == Some(MediaFamily::Audio);
                let media = super::super::media::DecodedMedia::new(source.clone())
                    .upcast::<gtk::MediaStream>();
                self.media.replace(Some(media.clone()));
                let weak = Rc::downgrade(self);
                media.connect_playing_notify(move |media| {
                    if media.is_playing()
                        && let Some(state) = weak.upgrade()
                    {
                        state.handed_off.take();
                    }
                });
                let weak = Rc::downgrade(self);
                media.connect_error_notify(move |media| {
                    let Some(error) = media.error() else {
                        return;
                    };
                    if let Some(state) = weak.upgrade() {
                        state.show_media_error(&error);
                    }
                });

                let preferences = super::super::preferences::PreferenceManager::shared();
                if is_gif {
                    media.set_loop(true);
                } else {
                    let muted = preferences.preview_muted();
                    media.set_volume(if muted {
                        0.0
                    } else {
                        preferences.preview_volume()
                    });
                    media.set_muted(muted);
                }
                let center_play = match family {
                    Some(MediaFamily::Audio) => {
                        self.render_audio(preview.entry, source, &media, &preferences);
                        let weak = Rc::downgrade(self);
                        let handler = media.connect_prepared_notify(move |media| {
                            if media.is_prepared()
                                && media.has_video()
                                && let Some(state) = weak.upgrade()
                            {
                                state.replace_audio_with_video(media);
                            }
                        });
                        self.media_signals.borrow_mut().push(handler);
                        None
                    }
                    Some(MediaFamily::Video) => {
                        self.render_video(preview.entry, source, &media, &preferences);
                        None
                    }
                    None | Some(MediaFamily::Route) => {
                        let (overlay, center_play) = self.build_media_view(&media);
                        let section = media_layout::section(&overlay, &media);
                        self.content.append(&section);
                        self.append_media_controls(
                            &media,
                            &preferences,
                            &section,
                            &center_play,
                            is_gif,
                        );
                        Some(center_play)
                    }
                };
                if !handed_off
                    && (preferences.preview_autoplay() || (continue_playback && family.is_some()))
                {
                    if preferences.preview_autoplay() && !continue_playback {
                        match family {
                            Some(MediaFamily::Video) => {
                                if let Some(video) = self.video.borrow().as_ref() {
                                    video.view.start_silently();
                                }
                            }
                            Some(MediaFamily::Audio) => {
                                if let Some(audio) = self.audio.borrow().as_ref() {
                                    audio.view.start_silently();
                                }
                            }
                            None | Some(MediaFamily::Route) => {}
                        }
                    }
                    self.sizing.play_or_defer(&media);
                } else if let Some(center_play) = center_play {
                    center_play.set_visible(true);
                }

                if let Some(error) = media.error() {
                    self.show_media_error(&error);
                }
            }
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
                self.render_pdf_viewer(preview.entry, png, page, pages, text_layer);
            }
            PreviewContent::Archive { tree } => {
                let focus = self.focus_archive_request.get() == Some(preview.request_id);
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

    pub(super) fn clear_password_entry(&self) {
        if let Some(entry) = self.password_entry.borrow_mut().take() {
            entry.set_text("");
        }
    }

    pub(super) fn clear_content(&self) {
        self.reset_content(None);
    }

    pub(super) fn reset_content(&self, keep: Option<MediaFamily>) {
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
