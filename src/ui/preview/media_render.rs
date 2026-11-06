// SPDX-License-Identifier: MIT

use super::*;

/// The file behind a sandboxed media preview.
pub(super) struct SandboxedPreview {
    pub(super) entry: FileEntry,
    pub(super) content_type: String,
    /// Stepping from a playing file keeps the next one playing.
    pub(super) continue_playback: bool,
}

impl PreviewState {
    pub(super) fn render_sandboxed_media(
        self: &Rc<Self>,
        preview: SandboxedPreview,
        mut source: crate::services::SandboxedMedia,
    ) {
        let is_gif = preview.content_type == "image/gif";
        let family = media_family(&preview.content_type);
        let handed_off = self.handed_off.borrow().as_ref() == Some(&preview.entry.location);
        source.audio_only = family == Some(MediaFamily::Audio);
        let preferences = super::super::preferences::PreferenceManager::shared();
        let starts_at_zero = is_gif || crate::services::is_raw_video_stream(&preview.content_type);
        let start_percent = if starts_at_zero {
            0
        } else {
            preferences.preview_video_start()
        };
        let media = self.open_media_stream(source.clone(), start_percent);
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
        let track = MediaTrack {
            entry: preview.entry,
            source,
            media: media.clone(),
        };
        let center_play = match family {
            Some(MediaFamily::Audio) => {
                self.render_audio(track, &preferences);
                self.watch_for_video(&media);
                None
            }
            Some(MediaFamily::Video) => {
                self.render_video(track, &preferences);
                None
            }
            None | Some(MediaFamily::Route) => {
                Some(self.render_plain_media(&media, &preferences, is_gif))
            }
        };
        let autoplay = preferences.preview_autoplay();
        let continues = preview.continue_playback && family.is_some();
        if !handed_off && (autoplay || continues) {
            if autoplay && !preview.continue_playback {
                self.ease_in_player(family);
            }
            self.sizing.play_or_defer(&media);
        } else if let Some(center_play) = center_play {
            center_play.set_visible(true);
        }

        if let Some(error) = media.error() {
            self.show_media_error(&error);
        }
    }

    fn open_media_stream(
        self: &Rc<Self>,
        source: crate::services::SandboxedMedia,
        start_percent: u32,
    ) -> gtk::MediaStream {
        let decoded = super::super::media::DecodedMedia::new(source);
        decoded.set_start_percent(start_percent);
        let media = decoded.upcast::<gtk::MediaStream>();
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
        media
    }

    /// An audio guess that turns out to carry video moves to the video player.
    fn watch_for_video(self: &Rc<Self>, media: &gtk::MediaStream) {
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
    }

    fn render_plain_media(
        self: &Rc<Self>,
        media: &gtk::MediaStream,
        preferences: &Rc<super::super::preferences::PreferenceManager>,
        is_gif: bool,
    ) -> gtk::Button {
        let (overlay, center_play) = self.build_media_view(media);
        let section = media_layout::section(&overlay, media);
        self.content.append(&section);
        self.append_media_controls(
            media,
            MediaControls {
                preferences,
                section: &section,
                center_play: &center_play,
                is_gif,
            },
        );
        center_play
    }

    fn ease_in_player(&self, family: Option<MediaFamily>) {
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
}
