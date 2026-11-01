// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn build_media_view(
        self: &Rc<Self>,
        media: &gtk::MediaStream,
    ) -> (gtk::Overlay, gtk::Button) {
        let picture = gtk::Picture::for_paintable(media);
        picture.add_css_class("preview-media");
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_cursor_from_name(Some("grab"));
        install_preview_drag(&picture, self);
        let weak_state = Rc::downgrade(self);
        let weak_media = media.downgrade();
        picture.add_tick_callback(move |_, _| {
            let Some(state) = weak_state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(media) = weak_media.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state.animating.get()
                && let Some(media) = media.downcast_ref::<super::super::media::DecodedMedia>()
            {
                media.resize(state.media_preview_size());
            }
            glib::ControlFlow::Continue
        });

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.set_focusable(true);
        overlay.set_can_target(true);

        let center_play = gtk::Button::new();
        center_play.add_css_class("preview-media-center");
        center_play.set_halign(gtk::Align::Center);
        center_play.set_valign(gtk::Align::Center);
        center_play.set_visible(false);
        let center_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 48);
        center_play.set_child(Some(&center_icon));
        overlay.add_overlay(&center_play);

        let media_for_center = media.clone();
        center_play.connect_clicked(move |_| toggle_playing(&media_for_center));

        let media_for_click = media.clone();
        let overlay_for_focus = overlay.downgrade();
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(overlay) = overlay_for_focus.upgrade() {
                overlay.grab_focus();
            }
            toggle_playing(&media_for_click);
        });
        picture.add_controller(click);

        (overlay, center_play)
    }

    pub(super) fn append_media_controls(
        self: &Rc<Self>,
        media: &gtk::MediaStream,
        controls: MediaControls,
    ) {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("preview-media-bar");
        self.append_play_toggle(media, &bar, controls.center_play);

        if controls.is_gif {
            controls.section.append(&bar);
            return;
        }

        let time_label = gtk::Label::new(Some("0:00 / 0:00"));
        time_label.add_css_class("preview-media-time");
        time_label.set_xalign(0.5);
        let seek = seek_scale();

        let weak_media = media.downgrade();
        let volume = VolumeControls::new(controls.preferences, move || weak_media.upgrade());

        bar.append(&time_label);
        bar.append(&seek);
        bar.append(&volume.toggle);
        bar.append(&volume.slider);
        controls
            .preferences
            .bind_interface_scale(&bar, |widget, scale| {
                widget
                    .downcast_ref::<gtk::Box>()
                    .expect("media controls")
                    .set_orientation(if scale > 1.5 {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
            });
        controls.section.append(&bar);
        self.adopt_volume_controls(&volume);

        let seeking = Rc::new(Cell::new(false));
        self.follow_media_time(media, &time_label, &seek, &seeking);
        install_seek_input(media, &seek, seeking);
    }

    fn append_play_toggle(
        &self,
        media: &gtk::MediaStream,
        bar: &gtk::Box,
        center_play: &gtk::Button,
    ) {
        let play_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 18);
        let pause_icon = crate::assets::primary_icon(crate::assets::icons::PAUSE, 18);
        let play_button = gtk::Button::new();
        play_button.add_css_class("preview-media-button");
        let keys = keys::Keys::One(MEDIA_PLAY_PAUSE.shown());
        crate::ui::shortcut_reference::follow_tooltip(&play_button, "Play/Pause (%{keys})", keys);
        crate::ui::shortcut_reference::follow_label(&play_button, "Play/Pause (%{keys})", keys);
        play_button.set_child(Some(if media.is_playing() {
            &pause_icon
        } else {
            &play_icon
        }));
        let media_for_play = media.clone();
        play_button.connect_clicked(move |_| toggle_playing(&media_for_play));

        bar.append(&play_button);

        let center_for_notify = center_play.clone();
        let media_for_playing = media.clone();
        let handler = media.connect_notify_local(Some("playing"), move |_, _| {
            let playing = media_for_playing.is_playing();
            play_button.set_child(Some(if playing { &pause_icon } else { &play_icon }));
            center_for_notify.set_visible(!playing);
        });
        self.media_signals.borrow_mut().push(handler);
    }

    fn follow_media_time(
        &self,
        media: &gtk::MediaStream,
        time_label: &gtk::Label,
        seek: &gtk::Scale,
        seeking: &Rc<Cell<bool>>,
    ) {
        let update_time = {
            let media = media.clone();
            let time_label = time_label.clone();
            let seek = seek.clone();
            let seeking = seeking.clone();
            move || sync_media_time(&media, &time_label, &seek, &seeking)
        };
        update_time();
        for property in ["timestamp", "duration"] {
            let update = update_time.clone();
            let handler = media.connect_notify_local(Some(property), move |_, _| update());
            self.media_signals.borrow_mut().push(handler);
        }
    }

    pub(super) fn adopt_volume_controls(&self, volume: &VolumeControls) {
        self.media_volume_slider
            .replace(Some(volume.slider.clone()));
        self.media_toggle_mute
            .replace(Some(volume.toggle_mute.clone()));
    }
}

/// Where the plain media bar goes and how much of it the stream needs.
pub(super) struct MediaControls<'a> {
    pub(super) preferences: &'a Rc<super::super::preferences::PreferenceManager>,
    pub(super) section: &'a gtk::Box,
    pub(super) center_play: &'a gtk::Button,
    pub(super) is_gif: bool,
}

fn toggle_playing(media: &gtk::MediaStream) {
    if media.is_playing() {
        media.pause();
    } else {
        media.play();
    }
}

fn seek_scale() -> gtk::Scale {
    let seek = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .draw_value(false)
        .build();
    seek.add_css_class("preview-media-seek");
    seek.set_range(0.0, 0.0);
    seek.set_increments(MEDIA_SEEK_STEP_US as f64, MEDIA_SEEK_PAGE_US as f64);
    seek.set_sensitive(false);
    seek
}

/// A drag seeks once on release; keys, paging and the wheel seek now,
/// and the knob follows the stream's position.
fn install_seek_input(media: &gtk::MediaStream, seek: &gtk::Scale, seeking: Rc<Cell<bool>>) {
    let drag = gtk::GestureDrag::new();
    let seeking_for_begin = seeking.clone();
    drag.connect_drag_begin(move |_, _, _| {
        seeking_for_begin.set(true);
    });
    let seeking_for_end = seeking.clone();
    let media_for_drag_end = media.clone();
    let seek_for_drag_end = seek.downgrade();
    drag.connect_drag_end(move |_, _, _| {
        seeking_for_end.set(false);
        if let Some(seek) = seek_for_drag_end.upgrade() {
            media_for_drag_end.seek(seek.value() as i64);
        }
    });
    seek.add_controller(drag);
    let media = media.clone();
    seek.connect_change_value(move |_, scroll, value| {
        let dragging = scroll == gtk::ScrollType::Jump && seeking.get();
        if dragging || !media.is_seekable() {
            return glib::Propagation::Proceed;
        }
        let target = (value as i64).clamp(0, media_last_frame(media.duration()));
        // Touchpad scrolling sends tiny jumps; one within the current frame
        // would only restart the decoder.
        if (target - media.timestamp()).abs() >= crate::media::timestamp(1) as i64 {
            media.seek(target);
        }
        glib::Propagation::Stop
    });
}

pub(super) struct VolumeControls {
    pub(super) toggle: gtk::Button,
    pub(super) slider: gtk::Scale,
    pub(super) toggle_mute: Rc<dyn Fn()>,
}

impl VolumeControls {
    /// `media` resolves the stream to update, which can change while the controls live.
    pub(super) fn new(
        preferences: &Rc<super::super::preferences::PreferenceManager>,
        media: impl Fn() -> Option<gtk::MediaStream> + 'static,
    ) -> Self {
        let toggle = gtk::Button::new();
        toggle.add_css_class("preview-media-button");
        let keys = keys::Keys::One(MEDIA_MUTE.shown());
        crate::ui::shortcut_reference::follow_tooltip(&toggle, "Mute/unmute (%{keys})", keys);
        crate::ui::shortcut_reference::follow_label(&toggle, "Mute/unmute (%{keys})", keys);
        let muted = preferences.preview_muted();
        let icon = crate::assets::primary_icon(volume_icon(muted), 16);
        toggle.set_child(Some(&icon));

        let slider = volume_slider(if muted {
            0.0
        } else {
            preferences.preview_volume()
        });

        let updating_slider = Rc::new(Cell::new(false));
        let updating = updating_slider.clone();
        let weak_icon = icon.downgrade();
        preferences.bind_preference(
            &slider,
            |preferences| (preferences.preview_volume(), preferences.preview_muted()),
            move |widget, (volume, muted)| {
                let Some(slider) = widget.downcast_ref::<gtk::Scale>() else {
                    return;
                };
                let previous = updating.replace(true);
                slider.set_value(if muted { 0.0 } else { volume });
                if let Some(media) = media() {
                    media.set_volume(volume);
                    media.set_muted(muted);
                }
                if let Some(icon) = weak_icon.upgrade() {
                    crate::assets::set_primary_icon(&icon, volume_icon(muted));
                }
                updating.set(previous);
            },
        );

        let toggle_mute: Rc<dyn Fn()> = Rc::new({
            let preferences = preferences.clone();
            move || {
                preferences.set_preview_muted(!preferences.preview_muted());
            }
        });
        let toggle_for_click = toggle_mute.clone();
        toggle.connect_clicked(move |_| {
            toggle_for_click();
        });

        let preferences = preferences.clone();
        slider.connect_value_changed(move |scale| {
            if !updating_slider.get() {
                preferences.set_preview_audio(scale.value(), scale.value() == 0.0);
            }
        });
        Self {
            toggle,
            slider,
            toggle_mute,
        }
    }
}

/// Seeking exactly to the duration would end the stream, so media keys and
/// sliders stop on the last frame.
pub(super) fn media_last_frame(duration: i64) -> i64 {
    let duration = duration.max(0) as u64;
    crate::media::timestamp(crate::media::seek_tick(duration, duration)) as i64
}

/// Repeated float steps would stop just short of zero and never mute, so
/// stepped volumes land on the step grid.
pub(super) fn snapped_volume(volume: f64) -> f64 {
    ((volume / MEDIA_VOLUME_STEP).round() * MEDIA_VOLUME_STEP).clamp(0.0, 1.0)
}

pub(super) fn sync_media_time(
    media: &impl IsA<gtk::MediaStream>,
    time_label: &gtk::Label,
    seek: &gtk::Scale,
    seeking: &Rc<Cell<bool>>,
) {
    let timestamp = media.timestamp();
    let duration = media.duration();
    let max = duration.max(timestamp) as f64;
    let adjustment = seek.adjustment();
    if adjustment.upper() != max {
        adjustment.set_upper(max);
        seek.set_sensitive(max > 0.0 && media.is_seekable());
    }
    if !seeking.get() {
        seek.set_value(timestamp as f64);
    }
    time_label.set_text(&format_media_time(timestamp, duration));
}

pub(super) fn format_media_time(timestamp_us: i64, duration_us: i64) -> String {
    format!("{}/{}", fmt_time(timestamp_us), fmt_time(duration_us))
}

pub(super) fn fmt_time(microseconds: i64) -> String {
    let total_seconds = microseconds.max(0) / 1_000_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn volume_icon(muted: bool) -> &'static str {
    if muted {
        crate::assets::icons::VOLUME_X
    } else {
        crate::assets::icons::VOLUME_2
    }
}

fn volume_slider(value: f64) -> gtk::Scale {
    let slider = gtk::Scale::builder()
        .orientation(gtk::Orientation::Horizontal)
        .draw_value(false)
        .width_request(72)
        .build();
    slider.add_css_class("preview-media-volume");
    slider.set_range(0.0, 1.0);
    slider.set_increments(MEDIA_VOLUME_STEP, MEDIA_VOLUME_PAGE);
    slider.set_value(value);
    // Keys and trough clicks step on the grid; a drag stays free.
    slider.connect_change_value(|scale, scroll, value| {
        if matches!(
            scroll,
            gtk::ScrollType::Jump | gtk::ScrollType::Start | gtk::ScrollType::End
        ) {
            return glib::Propagation::Proceed;
        }
        scale.set_value(snapped_volume(value));
        glib::Propagation::Stop
    });
    slider
}
