// SPDX-License-Identifier: MIT

//! One view survives consecutive tracks to preserve artwork transitions and spectrum motion.

mod analysis;
mod artwork;
mod details;
mod layout;
mod palette;
mod scrubber;
mod spectrum;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    model::FileEntry, sandbox::metadata::AudioTags, services::SandboxedMedia,
    ui::media::DecodedMedia,
};

pub(in crate::ui) use palette::apply_theme;
pub(in crate::ui::preview) use scrubber::Scrubber;

pub(super) fn clock(microseconds: i64) -> String {
    let seconds = microseconds.max(0) / 1_000_000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[derive(Clone, Copy)]
pub(super) struct TrackPosition {
    /// One-based, unlike the listing cursor.
    pub(super) position: usize,
    pub(super) count: usize,
    pub(super) results: bool,
}

pub(super) fn track_caption(tags: &AudioTags, folder: Option<TrackPosition>) -> Option<String> {
    match (tags.track, tags.track_total, folder) {
        (Some(track), Some(total), _) => Some(format!("Track {track} of {total}")),
        (Some(track), None, _) => Some(format!("Track {track}")),
        (
            None,
            _,
            Some(TrackPosition {
                position,
                count,
                results,
            }),
        ) => Some(format!(
            "{position} of {count} in {}",
            if results { "results" } else { "folder" }
        )),
        (None, _, None) => None,
    }
}

pub(super) struct Track {
    pub(super) entry: FileEntry,
    pub(super) source: SandboxedMedia,
    pub(super) media: gtk::MediaStream,
    pub(super) folder: Option<TrackPosition>,
    pub(super) has_previous: bool,
    pub(super) has_next: bool,
}

pub(super) struct AudioView {
    root: gtk::Box,
    artwork: artwork::Artwork,
    spectrum: spectrum::Spectrum,
    scrubber: Scrubber,
    eyebrow: gtk::Label,
    title: gtk::Label,
    artist: gtk::Label,
    album: gtk::Label,
    elapsed: gtk::Label,
    total: gtk::Label,
    play_icon: gtk::Image,
    play: gtk::Button,
    error: gtk::Box,
    previous: gtk::Button,
    next: gtk::Button,
    media: RefCell<Option<gtk::MediaStream>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    hover: Cell<Option<i64>>,
    shown_seconds: Cell<(i64, i64)>,
    stem: RefCell<String>,
    folder: Cell<Option<TrackPosition>>,
    details: RefCell<Option<details::DetailsLoad>>,
    peaks: RefCell<Option<details::PeaksLoad>>,
}

fn label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn transport_button(icon: &str, name: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("preview-media-button");
    button.set_child(Some(&crate::assets::primary_icon(icon, 18)));
    button.set_tooltip_text(Some(tooltip));
    crate::ui::accessibility::set_label(&button, name);
    button
}

impl AudioView {
    /// `navigate(-1 | 1)` moves the listing to the previous or next audio file.
    pub(super) fn new(volume: &gtk::Widget, navigate: Rc<dyn Fn(i32)>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("preview-audio");
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_layout_manager(Some(glib::Object::new::<layout::NowPlayingLayout>()));

        let eyebrow = label("preview-audio-eyebrow");
        eyebrow.set_hexpand(true);
        let eyebrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        eyebrow_row.append(&eyebrow);
        eyebrow_row.append(volume);
        let title = label("preview-audio-title");
        let artist = label("preview-audio-artist");
        let album = label("preview-audio-album");
        album.set_hexpand(true);
        let byline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        byline.append(&artist);
        byline.append(&album);
        let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
        header.add_css_class("preview-audio-header");
        header.append(&eyebrow_row);
        header.append(&title);
        header.append(&byline);
        let error = gtk::Box::new(gtk::Orientation::Vertical, 4);
        error.set_visible(false);
        header.append(&error);

        let elapsed = label("preview-media-time");
        elapsed.add_css_class("preview-audio-time");
        let total = label("preview-media-time");
        total.add_css_class("preview-audio-time");
        total.set_xalign(1.0);
        let previous = transport_button(
            crate::assets::icons::SKIP_BACK,
            "Previous audio file",
            "Previous audio file (Ctrl+Alt+<)",
        );
        let next = transport_button(
            crate::assets::icons::SKIP_FORWARD,
            "Next audio file",
            "Next audio file (Ctrl+Alt+>)",
        );
        let play_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 22);
        let play = gtk::Button::new();
        play.add_css_class("preview-media-center");
        play.add_css_class("preview-audio-play");
        play.set_child(Some(&play_icon));
        play.set_tooltip_text(Some("Play/Pause (Ctrl+Alt+Space)"));
        crate::ui::accessibility::set_label(&play, "Play or pause");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.set_valign(gtk::Align::Center);
        buttons.append(&previous);
        buttons.append(&play);
        buttons.append(&next);
        let transport = gtk::CenterBox::new();
        transport.add_css_class("preview-audio-transport");
        transport.set_start_widget(Some(&elapsed));
        transport.set_center_widget(Some(&buttons));
        transport.set_end_widget(Some(&total));

        let artwork = artwork::Artwork::new();
        let spectrum = spectrum::Spectrum::new();
        let scrubber = Scrubber::new();
        root.append(&artwork);
        root.append(&header);
        root.append(&spectrum);
        root.append(&scrubber);
        root.append(&transport);

        let view = Rc::new(Self {
            root,
            artwork,
            spectrum,
            scrubber,
            eyebrow,
            title,
            artist,
            album,
            elapsed,
            total,
            play_icon,
            play: play.clone(),
            error,
            previous,
            next,
            media: RefCell::default(),
            handlers: RefCell::default(),
            hover: Cell::default(),
            shown_seconds: Cell::new((-1, -1)),
            stem: RefCell::default(),
            folder: Cell::default(),
            details: RefCell::default(),
            peaks: RefCell::default(),
        });

        let weak = Rc::downgrade(&view);
        play.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.toggle_playback();
            }
        });
        for surface in [
            view.artwork.upcast_ref::<gtk::Widget>(),
            view.spectrum.upcast_ref(),
        ] {
            let click = gtk::GestureClick::new();
            click.set_button(gtk::gdk::BUTTON_PRIMARY);
            let weak = Rc::downgrade(&view);
            click.connect_released(move |_, _, _, _| {
                if let Some(view) = weak.upgrade() {
                    view.toggle_playback();
                }
            });
            surface.add_controller(click);
        }
        for (button, step) in [(&view.previous, -1), (&view.next, 1)] {
            let navigate = navigate.clone();
            button.connect_clicked(move |_| navigate(step));
        }
        let weak = Rc::downgrade(&view);
        view.scrubber.connect_preview(move |time| {
            if let Some(view) = weak.upgrade() {
                view.hover.set(time);
                view.shown_seconds.set((-1, -1));
                view.sync_time();
            }
        });
        view
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub(super) fn detach(&self) {
        if let Some(media) = self.media.borrow_mut().take() {
            for handler in self.handlers.borrow_mut().drain(..) {
                media.disconnect(handler);
            }
        }
        self.details.borrow_mut().take();
        self.peaks.borrow_mut().take();
        self.scrubber.set_media(None);
        self.scrubber.clear_levels();
        self.spectrum.set_media(None);
        // Preserve the record's position between tracks.
        self.set_playing_icon(false);
    }

    pub(super) fn show(self: &Rc<Self>, track: Track) {
        self.detach();
        let Track {
            entry,
            source,
            media,
            folder,
            has_previous,
            has_next,
        } = track;
        self.prepare(&entry, folder, has_previous, has_next);
        self.play.set_sensitive(true);

        let weak = Rc::downgrade(self);
        let mut handlers = vec![
            media.connect_notify_local(Some("playing"), move |media, _| {
                if let Some(view) = weak.upgrade() {
                    view.sync_playing(media.is_playing());
                }
            }),
        ];
        for property in ["timestamp", "duration"] {
            let weak = Rc::downgrade(self);
            handlers.push(media.connect_notify_local(Some(property), move |_, _| {
                if let Some(view) = weak.upgrade() {
                    view.sync_time();
                }
            }));
        }
        self.handlers.replace(handlers);
        self.media.replace(Some(media.clone()));
        self.spectrum
            .set_media(media.downcast_ref::<DecodedMedia>());
        self.scrubber.set_media(Some(&media));
        // An unprepared replacement must not briefly tuck the record away.
        if media.is_prepared() || media.is_playing() {
            self.sync_playing(media.is_playing());
        }
        let weak = Rc::downgrade(self);
        self.handlers
            .borrow_mut()
            .push(media.connect_prepared_notify(move |media| {
                if media.is_prepared()
                    && let Some(view) = weak.upgrade()
                {
                    view.sync_playing(media.is_playing());
                }
            }));
        self.shown_seconds.set((-1, -1));
        self.sync_time();

        let key = details::TrackKey::of(&entry);
        if let Some(cached) = details::cached_details(&key) {
            self.show_tags(&cached.tags);
            self.artwork.set_cover(cached.cover.clone());
        } else {
            let weak = Rc::downgrade(self);
            let tags = move |tags: AudioTags| {
                if let Some(view) = weak.upgrade() {
                    view.show_tags(&tags);
                }
            };
            let weak = Rc::downgrade(self);
            let cover = move |cover| {
                if let Some(view) = weak.upgrade() {
                    view.artwork.set_cover(cover);
                }
            };
            self.details
                .replace(Some(details::load_details(&entry, &source, tags, cover)));
        }
        let weak = Rc::downgrade(self);
        self.peaks.replace(Some(details::load_peaks(
            &entry,
            &source,
            move |start, levels| {
                if let Some(view) = weak.upgrade() {
                    view.scrubber.add_levels(start, levels);
                }
            },
        )));
    }

    pub(super) fn prepare(
        &self,
        entry: &FileEntry,
        folder: Option<TrackPosition>,
        has_previous: bool,
        has_next: bool,
    ) {
        self.error.set_visible(false);
        self.play.set_sensitive(false);
        self.previous.set_sensitive(has_previous);
        self.next.set_sensitive(has_next);
        self.folder.set(folder);
        self.stem.replace(
            std::path::Path::new(&entry.display_name)
                .file_stem()
                .map_or_else(
                    || entry.display_name.clone(),
                    |stem| stem.to_string_lossy().into_owned(),
                ),
        );
        self.show_tags(&AudioTags::default());
    }

    pub(super) fn show_error(&self, title: &str, detail: &str, command: Option<&str>) {
        self.detach();
        self.sync_playing(false);
        self.play.set_sensitive(false);
        super::clear_box(&self.error);
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("preview-feedback-title");
        heading.set_wrap(true);
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("preview-feedback-detail");
        detail.set_wrap(true);
        self.error.append(&heading);
        self.error.append(&detail);
        if let Some(command) = command {
            self.error
                .append(&crate::ui::controls::copyable_command(command));
        }
        self.error.set_visible(true);
    }

    fn toggle_playback(&self) {
        let media = self.media.borrow().clone();
        if let Some(media) = media {
            media.set_playing(!media.is_playing());
        }
    }

    fn show_tags(&self, tags: &AudioTags) {
        let caption = track_caption(tags, self.folder.get());
        self.eyebrow.set_text(caption.as_deref().unwrap_or(""));
        self.title
            .set_text(tags.title.as_deref().unwrap_or(&self.stem.borrow()));
        self.artist.set_text(tags.artist.as_deref().unwrap_or(""));
        self.album.set_text(&match (&tags.artist, &tags.album) {
            (Some(_), Some(album)) => format!(" — {album}"),
            (None, Some(album)) => album.clone(),
            (_, None) => String::new(),
        });
    }

    fn set_playing_icon(&self, playing: bool) {
        crate::assets::set_primary_icon(
            &self.play_icon,
            if playing {
                crate::assets::icons::PAUSE
            } else {
                crate::assets::icons::PLAY
            },
        );
    }

    fn sync_playing(&self, playing: bool) {
        self.set_playing_icon(playing);
        self.artwork.set_playing(playing);
        self.spectrum.wake();
    }

    fn sync_time(&self) {
        let Some(media) = self.media.borrow().clone() else {
            return;
        };
        let duration = media.duration();
        let shown = self.hover.get().unwrap_or_else(|| media.timestamp());
        let seconds = (shown / 1_000_000, duration / 1_000_000);
        if self.shown_seconds.replace(seconds) == seconds {
            return;
        }
        self.elapsed.set_text(&clock(shown));
        if self.hover.get().is_some() {
            self.elapsed.add_css_class("preview-audio-time-target");
        } else {
            self.elapsed.remove_css_class("preview-audio-time-target");
        }
        self.total.set_text(&if duration > 0 {
            clock(duration)
        } else {
            "--:--".to_owned()
        });
    }
}

#[cfg(test)]
mod tests;
