// SPDX-License-Identifier: MIT

//! One view survives consecutive videos, so stepping through a folder never
//! rebuilds the frame area or its controls. Like the audio view, the frame is
//! the hero with the header right under it, then the timeline and transport.

mod ambient;
mod badges;
mod details;
mod frame;
mod layout;
mod scrubber;
pub(super) mod storyboard;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, gsk, prelude::*};

use crate::{
    model::FileEntry,
    sandbox::metadata::Chapter,
    services::{MediaPreviewSize, SandboxedMedia},
    ui::media::DecodedMedia,
};

pub(in crate::ui::preview) use details::VideoDetails;

use super::{
    ListingPosition,
    audio::{clock, details::TrackKey},
};

pub(in crate::ui::preview) use scrubber::Timeline;

const BUBBLE_MARGIN: i32 = 10;
/// Padding plus border around the bubble's cell.
const BUBBLE_CHROME: i32 = 8;
/// A storyboard cell stands in at this strength while a seek decodes.
const SEEK_COVER_OPACITY: f64 = 0.9;
/// Autoplayed sound rises from silence over this long, from the first frame.
const EASE_IN_RAMP: Duration = Duration::from_secs(2);
const EASE_IN_STEP: Duration = Duration::from_millis(16);

/// Slow in, slow out, on a loudness-friendly curve: a smoothstep squared, so
/// the gain leaves zero and reaches one with no slope, and the ear hears an
/// even rise rather than a late jump.
pub(super) fn ease_in_gain(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let smooth = progress * progress * (3.0 - 2.0 * progress);
    smooth * smooth
}
const BADGE_FADE: Duration = Duration::from_millis(140);
const BADGE_STAGGER: Duration = Duration::from_millis(40);
const SKELETON_BADGE_WIDTHS: [i32; 3] = [44, 56, 38];

pub(super) struct Clip {
    pub(super) entry: FileEntry,
    pub(super) source: SandboxedMedia,
    pub(super) media: gtk::MediaStream,
    pub(super) position: Option<ListingPosition>,
    pub(super) has_previous: bool,
    pub(super) has_next: bool,
}

pub(super) struct VideoView {
    root: gtk::Box,
    layout: layout::PlayerLayout,
    picture: gtk::Picture,
    frame: gtk::Overlay,
    glow: ambient::Glow,
    band: Cell<i32>,
    placeholder: frame::Placeholder,
    eyebrow: gtk::Label,
    title: gtk::Label,
    badges: gtk::Box,
    error: gtk::Box,
    timeline: Timeline,
    bubble: gtk::Box,
    bubble_cell: gtk::Picture,
    bubble_time: gtk::Label,
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
    details: RefCell<Option<details::DetailsLoad>>,
    chapters: RefCell<Vec<Chapter>>,
    clip: RefCell<Option<(FileEntry, SandboxedMedia)>>,
    storyboard: RefCell<Option<Rc<storyboard::Storyboard>>>,
    storyboard_load: RefCell<Option<storyboard::StoryboardLoad>>,
    first_frame_seen: Cell<bool>,
    seek_covered: Cell<bool>,
    ease_in: Cell<EaseIn>,
    ease_timer: RefCell<Option<glib::SourceId>>,
}

/// Where autoplay's silent start is in its ease-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EaseIn {
    Off,
    /// Silent until the first frame arrives and the ramp starts.
    Armed,
    Ramping,
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
        let layout = layout::PlayerLayout::new();
        root.set_layout_manager(Some(layout.clone()));

        let picture = gtk::Picture::new();
        picture.add_css_class("preview-media");
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_cursor_from_name(Some("grab"));
        let glow = ambient::Glow::new();
        let frame = gtk::Overlay::new();
        // The glow is the base layer; the picture sits over it, inset by the band.
        frame.set_child(Some(&glow));
        frame.add_overlay(&picture);
        frame.set_focusable(true);
        frame.set_can_target(true);
        crate::ui::accessibility::set_label(&frame, "Video frame");
        let placeholder = frame::Placeholder::new();
        frame.add_overlay(&placeholder);
        let bubble_cell = gtk::Picture::new();
        bubble_cell.add_css_class("preview-video-bubble-cell");
        bubble_cell.set_content_fit(gtk::ContentFit::Fill);
        bubble_cell.set_can_shrink(false);
        let bubble_time = gtk::Label::new(None);
        bubble_time.add_css_class("preview-video-bubble-time");
        let bubble = gtk::Box::new(gtk::Orientation::Vertical, 2);
        bubble.add_css_class("preview-video-bubble");
        bubble.set_can_target(false);
        bubble.set_halign(gtk::Align::Start);
        bubble.set_valign(gtk::Align::End);
        bubble.set_margin_bottom(BUBBLE_MARGIN);
        bubble.set_visible(false);
        bubble.append(&bubble_cell);
        bubble.append(&bubble_time);
        frame.add_overlay(&bubble);

        let eyebrow = label("preview-video-eyebrow");
        eyebrow.set_hexpand(true);
        let eyebrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        eyebrow_row.append(&eyebrow);
        eyebrow_row.append(volume);
        let title = label("preview-video-title");
        let badges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        badges.add_css_class("preview-video-badges");
        let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
        header.add_css_class("preview-video-header");
        header.append(&eyebrow_row);
        header.append(&title);
        header.append(&badges);
        let error = gtk::Box::new(gtk::Orientation::Vertical, 4);
        error.set_visible(false);
        header.append(&error);

        let timeline = Timeline::new();

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
        root.append(&timeline);
        root.append(&transport);

        let view = Rc::new(Self {
            root,
            layout,
            picture,
            frame,
            glow,
            band: Cell::new(0),
            placeholder,
            eyebrow,
            title,
            badges,
            error,
            timeline,
            bubble,
            bubble_cell,
            bubble_time,
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
            details: RefCell::default(),
            chapters: RefCell::default(),
            clip: RefCell::default(),
            storyboard: RefCell::default(),
            storyboard_load: RefCell::default(),
            first_frame_seen: Cell::default(),
            seek_covered: Cell::default(),
            ease_in: Cell::new(EaseIn::Off),
            ease_timer: RefCell::default(),
        });

        let weak = Rc::downgrade(&view);
        play.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.toggle_playback();
            }
        });
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
        view.timeline.connect_preview(move |time| {
            if let Some(view) = weak.upgrade() {
                view.hover.set(time);
                view.shown_seconds.set((-1, -1));
                view.sync_time();
                view.refresh_bubble();
            }
        });
        let preferences = crate::ui::preferences::PreferenceManager::shared();
        let weak = Rc::downgrade(&view);
        preferences.bind_preference(
            &view.root,
            crate::ui::preferences::PreferenceManager::element_glow,
            move |_, enabled| {
                if let Some(view) = weak.upgrade() {
                    view.set_band(if enabled { ambient::BAND } else { 0 });
                }
            },
        );
        // Touching the volume or mute is a choice about sound: bring it in at once.
        let weak = Rc::downgrade(&view);
        preferences.bind_preference(
            &view.root,
            |preferences| (preferences.preview_volume(), preferences.preview_muted()),
            move |_, _| {
                if let Some(view) = weak.upgrade() {
                    view.end_ease_in();
                }
            },
        );
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

    #[cfg(test)]
    pub(super) fn glow(&self) -> &ambient::Glow {
        &self.glow
    }

    /// Reserves `band` pixels around the picture for the glow.
    fn set_band(&self, band: i32) {
        if self.band.replace(band) == band {
            return;
        }
        self.layout.set_margin(band);
        for widget in [
            self.picture.upcast_ref::<gtk::Widget>(),
            self.placeholder.upcast_ref(),
        ] {
            widget.set_margin_start(band);
            widget.set_margin_end(band);
            widget.set_margin_top(band);
            widget.set_margin_bottom(band);
        }
        self.bubble.set_margin_bottom(BUBBLE_MARGIN + band);
        if band == 0 {
            self.glow.clear();
        }
    }

    fn software_rendered(&self) -> bool {
        self.root
            .native()
            .and_then(|native| native.renderer())
            .is_some_and(|renderer| renderer.is::<gsk::CairoRenderer>())
    }

    /// Feeds the glow from the latest frame, or keeps it dark when disallowed.
    fn light(&self, media: &DecodedMedia) {
        let allowed = self.band.get() > 0
            && ambient::allowed(
                crate::ui::preferences::PreferenceManager::shared().element_glow(),
                crate::ui::motion::animations_enabled(),
                self.software_rendered(),
            );
        if !allowed {
            if self.glow.is_lit() {
                self.glow.clear();
            }
            return;
        }
        if let Some(grid) = media.edge_grid() {
            self.glow.update(&grid);
        }
    }

    pub(super) fn detach(&self) {
        if let Some(media) = self.media.borrow_mut().take() {
            for handler in self.handlers.borrow_mut().drain(..) {
                media.disconnect(handler);
            }
        }
        self.details.borrow_mut().take();
        self.stop_ease_in(EaseIn::Off);
        self.glow.clear();
        self.storyboard_load.borrow_mut().take();
        self.storyboard.borrow_mut().take();
        self.clip.borrow_mut().take();
        self.first_frame_seen.set(false);
        self.seek_covered.set(false);
        self.picture.set_opacity(1.0);
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
        self.layout.set_paintable(None);
        self.placeholder.reveal();
        self.bubble.set_visible(false);
        self.timeline.set_media(None);
        self.sync_playing(false);
    }

    pub(super) fn show(self: &Rc<Self>, clip: Clip) {
        self.detach();
        let Clip {
            entry,
            source,
            media,
            position,
            has_previous,
            has_next,
        } = clip;
        self.prepare(&entry, position, has_previous, has_next);
        self.play.set_sensitive(true);
        let key = TrackKey::of(&entry);
        self.storyboard.replace(storyboard::cached_storyboard(&key));
        self.clip.replace(Some((entry.clone(), source.clone())));
        if let Some(cached) = details::cached_details(&key) {
            self.show_badges(Some(cached));
        } else {
            let weak = Rc::downgrade(self);
            self.details.replace(Some(details::load_details(
                &entry,
                &source,
                move |details| {
                    if let Some(view) = weak.upgrade() {
                        view.show_badges(details);
                    }
                },
            )));
        }
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
            // retires the placeholder and starts the storyboard.
            let weak = Rc::downgrade(self);
            handlers.push(decoded.connect_invalidate_contents(move |media| {
                if media.has_frame()
                    && let Some(view) = weak.upgrade()
                {
                    view.frame_arrived();
                    view.light(media);
                }
            }));
            let weak = Rc::downgrade(self);
            handlers.push(decoded.connect_seeking_notify(move |media| {
                if let Some(view) = weak.upgrade() {
                    if media.is_seeking() {
                        view.end_ease_in();
                        view.cover_seek(media.timestamp().max(0) as u64);
                    } else {
                        view.uncover_seek();
                    }
                }
            }));
            self.sync_frame_size(decoded);
        }
        self.handlers.replace(handlers);
        self.media.replace(Some(media.clone()));
        self.timeline.set_media(Some(&media));
        if media
            .downcast_ref::<DecodedMedia>()
            .is_some_and(DecodedMedia::has_frame)
        {
            self.frame_arrived();
        }
        self.sync_playing(media.is_playing());
        self.shown_seconds.set((-1, -1));
        self.sync_time();
    }

    fn sync_frame_size(&self, media: &DecodedMedia) {
        let aspect = self.video_aspect(media);
        self.placeholder.set_aspect(aspect);
        if aspect.is_some() {
            self.layout.set_aspect(aspect);
        }
    }

    fn video_aspect(&self, media: &DecodedMedia) -> Option<f64> {
        media
            .video_size()
            .map(|(width, height)| f64::from(width) / f64::from(height))
    }

    fn frame_arrived(self: &Rc<Self>) {
        if self.first_frame_seen.replace(true) {
            return;
        }
        self.placeholder.conceal();
        self.start_storyboard();
        self.ramp_audio();
    }

    /// Autoplay starts silent; the sound fades in from the first frame.
    pub(super) fn start_silently(&self) {
        let Some(media) = self.media.borrow().clone() else {
            return;
        };
        if media.is_muted() {
            return;
        }
        if let Some(decoded) = media.downcast_ref::<DecodedMedia>() {
            decoded.set_fade(0.0);
            self.ease_in.set(EaseIn::Armed);
        }
    }

    /// Any deliberate playback input brings the sound in immediately.
    pub(super) fn end_ease_in(&self) {
        if self.ease_in.get() == EaseIn::Off {
            return;
        }
        self.stop_ease_in(EaseIn::Off);
        self.set_fade(1.0);
    }

    fn stop_ease_in(&self, state: EaseIn) {
        self.ease_in.set(state);
        if let Some(timer) = self.ease_timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn set_fade(&self, fade: f64) {
        if let Some(decoded) = self
            .media
            .borrow()
            .as_ref()
            .and_then(|media| media.downcast_ref::<DecodedMedia>())
        {
            decoded.set_fade(fade);
        }
    }

    fn ramp_audio(self: &Rc<Self>) {
        if self.ease_in.get() != EaseIn::Armed {
            return;
        }
        self.ease_in.set(EaseIn::Ramping);
        let started = std::time::Instant::now();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(EASE_IN_STEP, move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if view.ease_in.get() != EaseIn::Ramping {
                return glib::ControlFlow::Break;
            }
            let progress = started.elapsed().as_secs_f64() / EASE_IN_RAMP.as_secs_f64();
            if progress >= 1.0 {
                view.ease_timer.borrow_mut().take();
                view.ease_in.set(EaseIn::Off);
                view.set_fade(1.0);
                return glib::ControlFlow::Break;
            }
            view.set_fade(ease_in_gain(progress));
            glib::ControlFlow::Continue
        });
        self.ease_timer.replace(Some(timer));
    }

    #[cfg(test)]
    pub(super) fn is_easing_in(&self) -> bool {
        self.ease_in.get() != EaseIn::Off
    }

    /// Runs once the first frame is on screen, so it never delays playback.
    fn start_storyboard(self: &Rc<Self>) {
        let Some((entry, source)) = self.clip.borrow().clone() else {
            return;
        };
        let duration = self
            .media
            .borrow()
            .as_ref()
            .map_or(0, |media| media.duration().max(0)) as u64;
        if duration < crate::media::storyboard::MIN_DURATION_US
            || self
                .storyboard
                .borrow()
                .as_ref()
                .is_some_and(|board| board.is_complete())
        {
            return;
        }
        let weak = Rc::downgrade(self);
        let load = storyboard::load_storyboard(&entry, &source, move |board| {
            if let Some(view) = weak.upgrade() {
                view.storyboard.replace(Some(board));
                view.refresh_bubble();
            }
        });
        self.storyboard_load.replace(Some(load));
    }

    /// The nearest decoded cell replaces the stale frame while the seek decodes.
    fn cover_seek(&self, target_us: u64) {
        let cell = self
            .storyboard
            .borrow()
            .as_ref()
            .and_then(|board| board.nearest(target_us));
        let Some(cell) = cell else {
            return;
        };
        let aspect = self.media.borrow().as_ref().and_then(|media| {
            media
                .downcast_ref::<DecodedMedia>()
                .and_then(|media| self.video_aspect(media))
        });
        self.placeholder
            .set_poster_with_opacity(Some(cell), SEEK_COVER_OPACITY);
        self.placeholder.set_aspect(aspect);
        self.placeholder.reveal();
        self.picture.set_opacity(0.0);
        self.seek_covered.set(true);
    }

    fn uncover_seek(&self) {
        if self.seek_covered.replace(false) {
            self.picture.set_opacity(1.0);
            self.placeholder.conceal();
        }
    }

    /// The bubble follows the pointer along the frame's width.
    fn refresh_bubble(&self) {
        let Some(time) = self.hover.get().filter(|time| *time >= 0) else {
            self.bubble.set_visible(false);
            return;
        };
        let board = self.storyboard.borrow().clone();
        let cell = board.as_ref().and_then(|board| board.nearest(time as u64));
        let (width, height) = board.map_or_else(
            || {
                let aspect = self
                    .media
                    .borrow()
                    .as_ref()
                    .and_then(|media| {
                        media
                            .downcast_ref::<DecodedMedia>()
                            .and_then(|media| self.video_aspect(media))
                    })
                    .unwrap_or(16.0 / 9.0);
                let edge = storyboard::CELL_EDGE as f64;
                if aspect >= 1.0 {
                    (edge as i32, (edge / aspect).round() as i32)
                } else {
                    ((edge * aspect).round() as i32, edge as i32)
                }
            },
            |board| (board.sheet.width as i32, board.sheet.height as i32),
        );
        self.bubble_cell.set_size_request(width, height);
        self.bubble_cell.set_paintable(cell.as_ref());
        self.bubble_time
            .set_text(&match self.chapter_title_at(time) {
                Some(title) => format!("{} · {title}", clock(time)),
                None => clock(time),
            });
        let band = self.band.get();
        let picture_width = self.picture.width();
        let bubble_width = width + BUBBLE_CHROME;
        let fraction = self.timeline.pointer_fraction().unwrap_or(0.0);
        let x = (fraction * f64::from(picture_width)) as i32 - bubble_width / 2;
        self.bubble
            .set_margin_start(band + x.clamp(0, (picture_width - bubble_width).max(0)));
        self.bubble.set_visible(true);
    }

    #[cfg(test)]
    pub(super) fn set_storyboard_for_test(&self, board: Rc<storyboard::Storyboard>) {
        self.storyboard.replace(Some(board));
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
        let poster = crate::ui::thumbnail::cached_thumbnail(entry);
        // The frame keeps its place from the first paint: the poster's aspect
        // stands in until the probe answers.
        self.layout.set_aspect(
            poster
                .as_ref()
                .map(|poster| poster.intrinsic_aspect_ratio()),
        );
        self.placeholder.set_poster(poster);
        self.placeholder.set_aspect(None);
        self.placeholder.reveal();
        self.chapters.borrow_mut().clear();
        self.timeline.set_chapters(Vec::new());
        self.show_badge_skeleton();
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

    /// Empty pills hold the row's height until the probe answers.
    fn show_badge_skeleton(&self) {
        super::clear_box(&self.badges);
        for width in SKELETON_BADGE_WIDTHS {
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            pill.add_css_class("preview-video-badge");
            pill.add_css_class("preview-video-badge-skeleton");
            pill.set_size_request(width, -1);
            self.badges.append(&pill);
        }
    }

    /// Badges enter with a short stagger once; a failed probe leaves the row empty.
    fn show_badges(&self, details: Option<Rc<VideoDetails>>) {
        super::clear_box(&self.badges);
        let Some(details) = details else {
            return;
        };
        self.show_chapters(&details);
        let animated = crate::ui::motion::animations_enabled() && self.badges.is_mapped();
        for (index, badge) in badges::badges(&details.metadata, details.sidecar_captions)
            .into_iter()
            .enumerate()
        {
            let label = gtk::Label::new(Some(&badge.label));
            label.add_css_class("preview-video-badge");
            label.add_css_class(badge.kind.css_class());
            let revealer = gtk::Revealer::builder()
                .child(&label)
                .transition_type(gtk::RevealerTransitionType::Crossfade)
                .transition_duration(if animated {
                    BADGE_FADE.as_millis() as u32
                } else {
                    0
                })
                .build();
            self.badges.append(&revealer);
            if animated {
                let weak = revealer.downgrade();
                glib::timeout_add_local_once(BADGE_STAGGER * index as u32, move || {
                    if let Some(revealer) = weak.upgrade() {
                        revealer.set_reveal_child(true);
                    }
                });
            } else {
                revealer.set_reveal_child(true);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn badge_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        let mut child = self.badges.first_child();
        while let Some(widget) = child {
            if let Some(label) = widget
                .downcast_ref::<gtk::Revealer>()
                .and_then(gtk::Revealer::child)
                .and_downcast::<gtk::Label>()
            {
                labels.push(label.text().to_string());
            }
            child = widget.next_sibling();
        }
        labels
    }

    /// Chapter starts become ticks on the timeline and titles in the bubble.
    fn show_chapters(&self, details: &VideoDetails) {
        let duration = details.metadata.duration.filter(|seconds| *seconds > 0.0);
        self.chapters.replace(details.metadata.chapters.clone());
        self.timeline
            .set_chapters(duration.map_or_else(Vec::new, |duration| {
                details
                    .metadata
                    .chapters
                    .iter()
                    .map(|chapter| (chapter.start / duration).clamp(0.0, 1.0))
                    .filter(|fraction| *fraction > 0.0)
                    .collect()
            }));
    }

    fn chapter_title_at(&self, time_us: i64) -> Option<String> {
        let seconds = time_us as f64 / 1_000_000.0;
        self.chapters
            .borrow()
            .iter()
            .find(|chapter| seconds >= chapter.start && seconds < chapter.end)
            .and_then(|chapter| chapter.title.clone())
    }

    #[cfg(test)]
    pub(super) fn show_details_for_test(&self, details: Option<Rc<VideoDetails>>) {
        self.show_badges(details);
    }

    #[cfg(test)]
    pub(super) fn chapter_ticks(&self) -> Vec<f64> {
        self.timeline.chapters()
    }

    #[cfg(test)]
    pub(super) fn badges_row(&self) -> &gtk::Box {
        &self.badges
    }

    fn toggle_playback(&self) {
        self.end_ease_in();
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
