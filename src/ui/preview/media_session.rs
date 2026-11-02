// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn media_command(self: &Rc<Self>, command: MediaCommand) -> bool {
        if let Some(route) = self.route.borrow().as_ref() {
            return route.command(command);
        }
        match command {
            MediaCommand::Previous => return self.step_media(-1, true),
            MediaCommand::Next => return self.step_media(1, true),
            _ => {}
        }
        self.end_media_ease_in();
        let has_volume = self.media_volume_slider.borrow().is_some();
        if has_volume && self.volume_command(command) {
            return true;
        }
        let Some(media) = self.media.borrow().clone() else {
            return false;
        };
        playback_command(&media, command)
    }

    fn end_media_ease_in(&self) {
        if let Some(video) = self.video.borrow().as_ref() {
            video.view.end_ease_in();
        }
        if let Some(audio) = self.audio.borrow().as_ref() {
            audio.view.end_ease_in();
        }
    }

    fn volume_command(&self, command: MediaCommand) -> bool {
        if let Some(delta) = command.volume_delta() {
            keyboard::step_preview_volume(delta);
            return true;
        }
        if command == MediaCommand::Mute
            && let Some(toggle) = self.media_toggle_mute.borrow().as_ref()
        {
            toggle();
            return true;
        }
        false
    }

    /// A volume toggle and slider for a player, tracking whichever stream is current.
    fn player_volume(
        self: &Rc<Self>,
        preferences: &Rc<super::super::preferences::PreferenceManager>,
        css_class: &str,
    ) -> (VolumeControls, gtk::Box) {
        let weak = Rc::downgrade(self);
        let volume = VolumeControls::new(preferences, move || {
            weak.upgrade()
                .and_then(|state| state.media.borrow().clone())
        });
        let volume_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        volume_box.add_css_class(css_class);
        volume_box.append(&volume.toggle);
        volume_box.append(&volume.slider);
        (volume, volume_box)
    }

    fn step_media_handler(self: &Rc<Self>) -> Rc<dyn Fn(i32)> {
        let weak = Rc::downgrade(self);
        Rc::new(move |step| {
            if let Some(state) = weak.upgrade() {
                state.step_media(step, false);
            }
        })
    }

    pub(super) fn render_audio(
        self: &Rc<Self>,
        track: MediaTrack,
        preferences: &Rc<super::super::preferences::PreferenceManager>,
    ) {
        let existing = self.audio.borrow_mut().take();
        let audio = existing.unwrap_or_else(|| {
            let (volume, volume_box) = self.player_volume(preferences, "preview-audio-volume");
            let view = audio::AudioView::new(volume_box.upcast_ref(), self.step_media_handler());
            self.content.append(view.widget());
            AudioPreview { view, volume }
        });
        self.adopt_volume_controls(&audio.volume);
        let (folder, has_previous, has_next) = self.listing_position(MediaFamily::Audio);
        audio.view.show(audio::Track {
            entry: track.entry,
            source: track.source,
            media: track.media,
            folder,
            has_previous,
            has_next,
        });
        self.audio.replace(Some(audio));
    }

    pub(super) fn render_video(
        self: &Rc<Self>,
        track: MediaTrack,
        preferences: &Rc<super::super::preferences::PreferenceManager>,
    ) {
        let existing = self.video.borrow_mut().take();
        let video = existing.unwrap_or_else(|| {
            let (volume, volume_box) = self.player_volume(preferences, "preview-video-volume");
            let navigate = self.step_media_handler();
            let weak = Rc::downgrade(self);
            let decode_size = Rc::new(move || {
                weak.upgrade()
                    .filter(|state| !state.animating.get())
                    .map(|state| state.media_preview_size())
            });
            let view = video::VideoView::new(volume_box.upcast_ref(), navigate, decode_size);
            install_preview_drag(view.picture(), self);
            self.content.append(view.widget());
            VideoPreview { view, volume }
        });
        self.adopt_volume_controls(&video.volume);
        let (position, has_previous, has_next) = self.listing_position(MediaFamily::Video);
        video.view.show(video::Clip {
            entry: track.entry,
            source: track.source,
            media: track.media,
            position,
            has_previous,
            has_next,
        });
        self.video.replace(Some(video));
    }

    pub(super) fn has_media_view(&self) -> bool {
        self.media.borrow().is_some()
            || self.audio.borrow().is_some()
            || self.video.borrow().is_some()
            || self.route.borrow().is_some()
    }

    pub(super) fn stepping_family(&self) -> Option<MediaFamily> {
        if self.audio.borrow().is_some() {
            Some(MediaFamily::Audio)
        } else if self.video.borrow().is_some() {
            Some(MediaFamily::Video)
        } else {
            None
        }
    }

    pub(super) fn listing_position(
        self: &Rc<Self>,
        family: MediaFamily,
    ) -> (Option<ListingPosition>, bool, bool) {
        let Some((files, index)) = self.family_listing(family, None) else {
            return (None, false, false);
        };
        let results = self
            .keyboard_view
            .borrow()
            .as_ref()
            .and_then(super::super::browser::WeakBrowserView::upgrade)
            .is_some_and(|view| view.results_replace_listing());
        (
            Some(ListingPosition {
                position: index + 1,
                count: files.len(),
                results,
            }),
            index > 0,
            index + 1 < files.len(),
        )
    }

    fn refresh_listing_position(self: &Rc<Self>) {
        if let Some(audio) = self.audio.borrow().as_ref() {
            let (folder, has_previous, has_next) = self.listing_position(MediaFamily::Audio);
            audio.view.set_listing(folder, has_previous, has_next);
        } else if let Some(video) = self.video.borrow().as_ref() {
            let (position, has_previous, has_next) = self.listing_position(MediaFamily::Video);
            video.view.set_listing(position, has_previous, has_next);
        }
    }

    /// The files of `named` that hold media, plus the shown `current` one; files not read
    /// yet stay out and are read, refreshing the shown player as they answer.
    fn media_sequence(
        self: &Rc<Self>,
        named: Vec<FileEntry>,
        family: MediaFamily,
        current: &crate::model::Location,
    ) -> Vec<FileEntry> {
        let checks = self.media_checks.borrow();
        checks.keep_only(&named);
        let shown = named
            .iter()
            .position(|entry| entry.location == *current)
            .unwrap_or_default();
        let mut unread = media_content::Unread::new(shown);
        let files = named
            .into_iter()
            .enumerate()
            .filter(|(index, entry)| {
                let media = checks.answer(entry, family);
                if media.is_none() {
                    unread.push(*index, entry);
                }
                media == Some(true) || entry.location == *current
            })
            .map(|(_, entry)| entry)
            .collect();
        drop(checks);
        let weak = Rc::downgrade(self);
        self.media_checks
            .borrow_mut()
            .read(current, unread, move || {
                weak.upgrade().is_some_and(|state| {
                    state.refresh_listing_position();
                    state.stepping_family().is_some()
                })
            });
        files
    }

    /// The shown file anchors the sequence; other files join once their bytes are known
    /// to be media, and the shown player's position updates when they are.
    pub(super) fn family_listing(
        self: &Rc<Self>,
        family: MediaFamily,
        around: Option<crate::model::Location>,
    ) -> Option<(Vec<FileEntry>, usize)> {
        let view = self
            .keyboard_view
            .borrow()
            .as_ref()
            .and_then(super::super::browser::WeakBrowserView::upgrade)?;
        let depth = self.current_depth.get()?;
        let current = self.current.borrow().as_ref()?.location.clone();
        let named =
            view.displayed_entries_matching(depth, |entry| entry_family(entry) == Some(family));
        let files = self.media_sequence(named, family, &current);
        let position = |location: &crate::model::Location| {
            files.iter().position(|entry| entry.location == *location)
        };
        let index = around
            .as_ref()
            .and_then(position)
            .or_else(|| position(&current))?;
        Some((files, index))
    }

    pub(super) fn step_media(self: &Rc<Self>, step: i32, keyboard: bool) -> bool {
        let Some(family) = self.stepping_family() else {
            return false;
        };
        // Repeated presses outrun the preview's debounce, so step from the cursor.
        let cursor = self.current_depth.get().and_then(|depth| {
            self.keyboard_view
                .borrow()
                .as_ref()
                .and_then(super::super::browser::WeakBrowserView::upgrade)?
                .displayed_cursor_entry(depth)
        });
        let Some((files, index)) = self.family_listing(family, cursor.map(|entry| entry.location))
        else {
            return false;
        };
        let Some(target) = index
            .checked_add_signed(step as isize)
            .and_then(|index| files.get(index))
        else {
            return false;
        };
        let (Some(view), Some(depth)) = (
            self.keyboard_view
                .borrow()
                .as_ref()
                .and_then(super::super::browser::WeakBrowserView::upgrade),
            self.current_depth.get(),
        ) else {
            return false;
        };
        let owned = self.content_owns_keys();
        let playing = self.media.borrow().as_ref().map_or_else(
            || self.continue_playback.borrow().is_some(),
            |media| media.is_playing(),
        );
        self.continue_playback
            .replace(playing.then(|| PlaybackContinuation {
                location: target.location.clone(),
                request: None,
            }));
        let moved = view.step_to(depth, &target.location, keyboard.then_some(step));
        if moved && view.results_replace_listing() {
            self.show_after_focus_change(target.clone(), Some(depth));
        } else if !moved {
            self.continue_playback.take();
        }
        self.keep_keys_in_content(owned);
        moved
    }

    pub(super) fn stop_media(&self) {
        let stream = self.media.borrow_mut().take();
        if let Some(stream) = stream {
            for handler in self.media_signals.borrow_mut().drain(..) {
                stream.disconnect(handler);
            }
            stream.set_playing(false);
            if let Some(media) = stream.downcast_ref::<super::super::media::DecodedMedia>() {
                media.close();
            }
        }
    }

    pub(super) fn show_media_error(&self, error: &glib::Error) {
        let message = error.message();
        let (title, detail, command) = media_error_feedback(message);
        self.continue_playback.take();
        if !self.show_audio_error(&title, &detail, command)
            && !self.show_video_error(&title, &detail, command)
        {
            self.show_message_with_icon(
                &title,
                &detail,
                Some(crate::assets::icons::TRIANGLE_ALERT),
                command,
            );
        }
    }

    pub(super) fn show_audio_error(
        &self,
        title: &str,
        detail: &str,
        command: Option<&str>,
    ) -> bool {
        let audio = self.audio.borrow();
        let Some(audio) = audio.as_ref() else {
            return false;
        };
        audio.view.show_error(title, detail, command);
        self.stop_media();
        true
    }

    pub(super) fn show_video_error(
        &self,
        title: &str,
        detail: &str,
        command: Option<&str>,
    ) -> bool {
        let video = self.video.borrow();
        let Some(video) = video.as_ref() else {
            return false;
        };
        video.view.show_error(title, detail, command);
        self.stop_media();
        true
    }

    pub(super) fn replace_audio_with_video(self: &Rc<Self>, media: &gtk::MediaStream) {
        let Some(audio) = self.audio.take() else {
            return;
        };
        // The stream keeps playing, so an armed ease-in must not leave it silent.
        audio.view.end_ease_in();
        audio.view.detach();
        clear_box(&self.content);
        let (Some(entry), Some(source)) = (
            self.current.borrow().clone(),
            media
                .downcast_ref::<super::super::media::DecodedMedia>()
                .and_then(super::super::media::DecodedMedia::source),
        ) else {
            return;
        };
        self.render_video(
            MediaTrack {
                entry,
                source,
                media: media.clone(),
            },
            &super::super::preferences::PreferenceManager::shared(),
        );
    }
}

/// A sandboxed stream and the file it plays, for the audio and video players.
pub(super) struct MediaTrack {
    pub(super) entry: FileEntry,
    pub(super) source: crate::services::SandboxedMedia,
    pub(super) media: gtk::MediaStream,
}

fn playback_command(media: &gtk::MediaStream, command: MediaCommand) -> bool {
    match command {
        MediaCommand::PlayPause => {
            if media.is_playing() {
                media.pause();
            } else {
                media.play();
            }
            true
        }
        MediaCommand::Seek(seek) if media.is_seekable() => {
            media.seek(seek.target(media.timestamp(), media.duration()));
            true
        }
        _ => false,
    }
}

pub(super) struct PlaybackContinuation {
    pub(super) location: crate::model::Location,
    pub(super) request: Option<PreviewRequestId>,
}

pub(super) struct AudioPreview {
    pub(super) view: Rc<audio::AudioView>,
    pub(super) volume: VolumeControls,
}

pub(super) struct VideoPreview {
    pub(super) view: Rc<video::VideoView>,
    pub(super) volume: VolumeControls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct ListingPosition {
    /// One-based, unlike the listing cursor.
    pub(in crate::ui) position: usize,
    pub(in crate::ui) count: usize,
    pub(in crate::ui) results: bool,
}

impl ListingPosition {
    pub(in crate::ui) fn caption(self) -> String {
        if self.results {
            rust_i18n::t!(
                "%{position} of %{count} in results",
                position = crate::i18n::integer(self.position as u64),
                count = crate::i18n::integer(self.count as u64)
            )
        } else {
            rust_i18n::t!(
                "%{position} of %{count} in folder",
                position = crate::i18n::integer(self.position as u64),
                count = crate::i18n::integer(self.count as u64)
            )
        }
        .into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MediaFamily {
    Audio,
    Video,
    Route,
}

pub(super) fn media_family(content_type: &str) -> Option<MediaFamily> {
    if is_audio_type(content_type) {
        Some(MediaFamily::Audio)
    } else if content_type.starts_with("video/") {
        Some(MediaFamily::Video)
    } else {
        None
    }
}

/// Cached per extension: stepping scans whole folders on the main thread. An entry whose
/// metadata probe found no media drops out of the sequence whatever its name says.
pub(super) fn entry_family(entry: &FileEntry) -> Option<MediaFamily> {
    thread_local! {
        static FAMILIES: RefCell<HashMap<String, Option<MediaFamily>>> =
            RefCell::new(HashMap::new());
    }
    let extension = Path::new(&entry.native_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|_| !entry.is_directory())
        .map(str::to_ascii_lowercase)?;
    if crate::services::route_preview::RouteFormat::for_name(&entry.native_name).is_some() {
        return Some(MediaFamily::Route);
    }
    let family = FAMILIES.with_borrow_mut(|known| {
        *known.entry(extension).or_insert_with_key(|extension| {
            media_family(
                &crate::services::guess_content_type(Path::new(&format!("a.{extension}"))).0,
            )
        })
    });
    // Elementary streams can have valid video dimensions without a duration.
    family.filter(|family| {
        entry.duration_seconds != MetadataValue::Unavailable
            || (*family == MediaFamily::Video
                && matches!(entry.image_dimensions, MetadataValue::Known(_)))
    })
}

pub(super) fn is_audio_type(content_type: &str) -> bool {
    content_type.starts_with("audio/")
        && !matches!(
            content_type,
            "audio/x-mpegurl"
                | "audio/mpegurl"
                | "audio/x-scpls"
                | "audio/midi"
                | "audio/x-midi"
                | "audio/sp-midi"
        )
}

pub(super) fn media_error_feedback(message: &str) -> (String, String, Option<&'static str>) {
    let normalized = message.to_ascii_lowercase();
    if [
        "gstreamer",
        "plug-in",
        "plugin",
        "missing decoder",
        "no decoder",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return (
            crate::i18n::tr("Additional media support required"),
            crate::i18n::tr(
                "On Arch or Omarchy, install the required GStreamer plugins, then restart Strata.",
            ),
            Some(MEDIA_PLUGIN_INSTALL_COMMAND),
        );
    }
    (
        crate::i18n::tr("Preview unavailable"),
        rust_i18n::t!(
            "Unable to play this media preview: %{message}",
            message = crate::i18n::tr(message)
        )
        .into_owned(),
        None,
    )
}
