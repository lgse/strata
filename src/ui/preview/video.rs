// SPDX-License-Identifier: MIT

//! One view survives consecutive videos, so stepping through a folder never
//! rebuilds the frame area or its controls.

mod frame;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{model::FileEntry, services::MediaPreviewSize, ui::media::DecodedMedia};

use super::{
    ListingPosition,
    audio::{Scrubber, clock},
    media_layout::MediaLayout,
};

const FRAME_MARGIN: i32 = 12;

pub(super) struct Clip {
    pub(super) entry: FileEntry,
    pub(super) media: gtk::MediaStream,
    pub(super) position: Option<ListingPosition>,
    pub(super) has_previous: bool,
    pub(super) has_next: bool,
}

pub(super) struct VideoView {
    root: gtk::Box,
    layout: MediaLayout,
    picture: gtk::Picture,
    frame: gtk::Overlay,
    placeholder: frame::Placeholder,
    center_play: gtk::Button,
    eyebrow: gtk::Label,
    title: gtk::Label,
    error: gtk::Box,
    scrubber: Scrubber,
    elapsed: gtk::Label,
    total: gtk::Label,
    play_icon: gtk::Image,
    play: gtk::Button,
    previous: gtk::Button,
    next: gtk::Button,
    media: RefCell<Option<gtk::MediaStream>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    hover: Cell<Option<i64>>,
    shown_seconds: Cell<(i64, i64)>,
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

impl VideoView {
    /// `navigate(-1 | 1)` moves the listing to the previous or next video.
    /// `decode_size` is the frame size to decode at, or `None` while the pane
    /// is still animating open.
    pub(super) fn new(
        volume: &gtk::Widget,
        navigate: Rc<dyn Fn(i32)>,
        decode_size: Rc<dyn Fn() -> Option<MediaPreviewSize>>,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("preview-video");
        root.set_hexpand(true);
        root.set_vexpand(true);
        let layout = MediaLayout::new();
        root.set_layout_manager(Some(layout.clone()));

        let picture = gtk::Picture::new();
        picture.add_css_class("preview-media");
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_cursor_from_name(Some("grab"));
        let frame = gtk::Overlay::new();
        frame.set_child(Some(&picture));
        frame.set_focusable(true);
        frame.set_can_target(true);
        frame.set_margin_start(FRAME_MARGIN);
        frame.set_margin_end(FRAME_MARGIN);
        frame.set_margin_top(FRAME_MARGIN);
        frame.set_margin_bottom(FRAME_MARGIN);
        crate::ui::accessibility::set_label(&frame, "Video frame");
        let placeholder = frame::Placeholder::new();
        frame.add_overlay(&placeholder);
        let center_play = gtk::Button::new();
        center_play.add_css_class("preview-media-center");
        center_play.set_halign(gtk::Align::Center);
        center_play.set_valign(gtk::Align::Center);
        center_play.set_visible(false);
        center_play.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::PLAY,
            48,
        )));
        crate::ui::accessibility::set_label(&center_play, "Play");
        frame.add_overlay(&center_play);

        let eyebrow = label("preview-video-eyebrow");
        eyebrow.set_hexpand(true);
        let eyebrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        eyebrow_row.append(&eyebrow);
        eyebrow_row.append(volume);
        let title = label("preview-video-title");
        let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
        header.add_css_class("preview-video-header");
        header.append(&eyebrow_row);
        header.append(&title);
        let error = gtk::Box::new(gtk::Orientation::Vertical, 4);
        error.set_visible(false);
        header.append(&error);

        let scrubber = Scrubber::new();
        scrubber.set_margin_start(FRAME_MARGIN);
        scrubber.set_margin_end(FRAME_MARGIN);

        let elapsed = label("preview-media-time");
        elapsed.add_css_class("preview-video-time");
        let total = label("preview-media-time");
        total.add_css_class("preview-video-time");
        total.set_xalign(1.0);
        let previous = transport_button(
            crate::assets::icons::SKIP_BACK,
            "Previous video",
            "Previous video (Ctrl+Alt+<)",
        );
        let next = transport_button(
            crate::assets::icons::SKIP_FORWARD,
            "Next video",
            "Next video (Ctrl+Alt+>)",
        );
        let play_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 22);
        let play = gtk::Button::new();
        play.add_css_class("preview-media-center");
        play.add_css_class("preview-video-play");
        play.set_child(Some(&play_icon));
        play.set_tooltip_text(Some("Play/Pause (Ctrl+Alt+Space)"));
        crate::ui::accessibility::set_label(&play, "Play or pause");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.set_valign(gtk::Align::Center);
        buttons.append(&previous);
        buttons.append(&play);
        buttons.append(&next);
        let transport = gtk::CenterBox::new();
        transport.add_css_class("preview-video-transport");
        transport.set_start_widget(Some(&elapsed));
        transport.set_center_widget(Some(&buttons));
        transport.set_end_widget(Some(&total));

        root.append(&frame);
        root.append(&header);
        root.append(&scrubber);
        root.append(&transport);

        let view = Rc::new(Self {
            root,
            layout,
            picture,
            frame,
            placeholder,
            center_play: center_play.clone(),
            eyebrow,
            title,
            error,
            scrubber,
            elapsed,
            total,
            play_icon,
            play: play.clone(),
            previous,
            next,
            media: RefCell::default(),
            handlers: RefCell::default(),
            hover: Cell::default(),
            shown_seconds: Cell::new((-1, -1)),
        });

        for button in [&play, &center_play] {
            let weak = Rc::downgrade(&view);
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.toggle_playback();
                }
            });
        }
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        let weak = Rc::downgrade(&view);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(view) = weak.upgrade() {
                view.frame.grab_focus();
                view.toggle_playback();
            }
        });
        view.picture.add_controller(click);
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
        // The decode rectangle follows the pane; `resize` ignores unchanged sizes.
        let weak = Rc::downgrade(&view);
        view.picture.add_tick_callback(move |_, _| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some(size) = decode_size()
                && let Some(media) = view.media.borrow().as_ref()
                && let Some(media) = media.downcast_ref::<DecodedMedia>()
            {
                media.resize(size);
            }
            glib::ControlFlow::Continue
        });
        view
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The frame picture, which the drawer makes a drag source for the file.
    pub(super) fn picture(&self) -> &gtk::Picture {
        &self.picture
    }

    #[cfg(test)]
    pub(super) fn placeholder(&self) -> &frame::Placeholder {
        &self.placeholder
    }

    pub(super) fn detach(&self) {
        if let Some(media) = self.media.borrow_mut().take() {
            for handler in self.handlers.borrow_mut().drain(..) {
                media.disconnect(handler);
            }
        }
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
        self.layout.set_paintable(None);
        self.placeholder.reveal();
        self.scrubber.set_media(None);
        self.scrubber.clear_levels();
        self.sync_playing(false);
        self.center_play.set_visible(false);
    }

    pub(super) fn show(self: &Rc<Self>, clip: Clip) {
        self.detach();
        let Clip {
            entry,
            media,
            position,
            has_previous,
            has_next,
        } = clip;
        self.prepare(&entry, position, has_previous, has_next);
        self.play.set_sensitive(true);
        self.picture.set_paintable(Some(&media));
        self.layout.set_paintable(Some(media.upcast_ref()));

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
        if let Some(decoded) = media.downcast_ref::<DecodedMedia>() {
            let weak = Rc::downgrade(self);
            handlers.push(decoded.connect_prepared_notify(move |media| {
                if let Some(view) = weak.upgrade() {
                    view.sync_frame_size(media);
                }
            }));
            // Every presented frame invalidates the paintable; the first one
            // retires the placeholder.
            let weak = Rc::downgrade(self);
            handlers.push(decoded.connect_invalidate_contents(move |media| {
                if media.has_frame()
                    && let Some(view) = weak.upgrade()
                {
                    view.placeholder.conceal();
                }
            }));
            self.sync_frame_size(decoded);
            if decoded.has_frame() {
                self.placeholder.conceal();
            }
        }
        self.handlers.replace(handlers);
        self.media.replace(Some(media.clone()));
        self.scrubber.set_media(Some(&media));
        self.sync_playing(media.is_playing());
        self.shown_seconds.set((-1, -1));
        self.sync_time();
    }

    fn sync_frame_size(&self, media: &DecodedMedia) {
        self.placeholder.set_aspect(
            media
                .video_size()
                .map(|(width, height)| f64::from(width) / f64::from(height)),
        );
    }

    pub(super) fn prepare(
        &self,
        entry: &FileEntry,
        position: Option<ListingPosition>,
        has_previous: bool,
        has_next: bool,
    ) {
        self.error.set_visible(false);
        self.play.set_sensitive(false);
        self.previous.set_sensitive(has_previous);
        self.next.set_sensitive(has_next);
        self.placeholder
            .set_poster(crate::ui::thumbnail::cached_thumbnail(entry));
        self.placeholder.set_aspect(None);
        self.placeholder.reveal();
        self.eyebrow
            .set_text(&position.map(ListingPosition::caption).unwrap_or_default());
        self.title.set_text(
            &std::path::Path::new(&entry.display_name)
                .file_stem()
                .map_or_else(
                    || entry.display_name.clone(),
                    |stem| stem.to_string_lossy().into_owned(),
                ),
        );
    }

    pub(super) fn show_error(&self, title: &str, detail: &str, command: Option<&str>) {
        self.detach();
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

    fn sync_playing(&self, playing: bool) {
        crate::assets::set_primary_icon(
            &self.play_icon,
            if playing {
                crate::assets::icons::PAUSE
            } else {
                crate::assets::icons::PLAY
            },
        );
        self.center_play
            .set_visible(!playing && self.media.borrow().is_some());
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
            self.elapsed.add_css_class("preview-video-time-target");
        } else {
            self.elapsed.remove_css_class("preview-video-time-target");
        }
        self.total.set_text(&if duration > 0 {
            clock(duration)
        } else {
            "--:--".to_owned()
        });
    }
}
