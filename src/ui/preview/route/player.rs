// SPDX-License-Identifier: MIT

use super::super::{
    audio::palette::{follow_theme, palette},
    keyboard::{MediaCommand, MediaSeek},
};
use super::{distance, position};
use crate::services::route_preview::{RouteSample, profile_distance, profile_x};
use crate::ui::preferences::{PreferenceManager, UnitSystem};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const PLAYBACK_SECONDS: f64 = 30.0;

pub(super) struct Plot {
    pub frame: gtk::Overlay,
    pub picture: gtk::Picture,
    marker: gtk::DrawingArea,
    profile: bool,
}

impl Plot {
    pub fn new(png: Vec<u8>, label: &str, profile: bool) -> Result<Self, glib::Error> {
        let (picture, _) = super::picture(png, label)?;
        picture.set_content_fit(if profile {
            gtk::ContentFit::Fill
        } else {
            gtk::ContentFit::Contain
        });
        let frame = gtk::Overlay::new();
        frame.set_child(Some(&picture));
        frame.set_hexpand(true);
        frame.set_vexpand(!profile);
        let marker = gtk::DrawingArea::new();
        marker.set_can_target(false);
        frame.add_overlay(&marker);
        frame.set_cursor_from_name(Some("crosshair"));
        follow_theme(&marker);
        Ok(Self {
            frame,
            picture,
            marker,
            profile,
        })
    }

    fn transform(&self) -> (f64, f64, f64, f64) {
        let width = f64::from(self.marker.width());
        let height = f64::from(self.marker.height());
        if self.profile {
            return (0.0, 0.0, width / 800.0, height / 140.0);
        }
        let scale = (width / 800.0).min(height / 600.0);
        (
            (width - 800.0 * scale) / 2.0,
            (height - 600.0 * scale) / 2.0,
            scale,
            scale,
        )
    }
}

pub(in crate::ui::preview) struct RoutePlayer {
    pub(super) route: Plot,
    pub(super) profile: Option<Plot>,
    pub(in crate::ui::preview) play: gtk::ToggleButton,
    icon: gtk::Image,
    elapsed: gtk::Label,
    total: gtk::Label,
    units: Cell<UnitSystem>,
    samples: Vec<RouteSample>,
    distance_m: f64,
    pub(in crate::ui::preview) position: Cell<Option<RouteSample>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    last_frame: Cell<Option<i64>>,
    last_pointer: Cell<Option<(bool, f64, f64)>>,
}

impl RoutePlayer {
    pub(super) fn new(
        route: Plot,
        profile: Option<Plot>,
        samples: Vec<RouteSample>,
        distance_m: f64,
    ) -> Rc<Self> {
        let icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 22);
        let play = gtk::ToggleButton::new();
        play.add_css_class("preview-media-center");
        play.add_css_class("preview-video-play");
        play.set_child(Some(&icon));
        play.set_sensitive(distance_m > 0.0);
        play.set_tooltip_text(Some(&crate::ui::shortcut_reference::with_keys(
            "Play/Pause (%{keys})",
            crate::ui::keys::Keys::One(crate::ui::keys::bindings::MEDIA_PLAY_PAUSE),
        )));
        crate::ui::accessibility::set_label(&play, &crate::i18n::tr("Play or pause"));
        let elapsed = super::super::components::label("preview-media-time");
        let position = Cell::new(samples.first().copied());
        let player = Rc::new(Self {
            route,
            profile,
            play,
            icon,
            elapsed,
            total: super::super::components::label("preview-media-time"),
            units: Cell::new(UnitSystem::Metric),
            samples,
            distance_m,
            position,
            tick: RefCell::default(),
            last_frame: Cell::default(),
            last_pointer: Cell::default(),
        });
        let weak = Rc::downgrade(&player);
        player.play.connect_toggled(move |play| {
            if let Some(player) = weak.upgrade() {
                if play.is_active() {
                    player.start();
                } else {
                    player.stop_tick();
                }
                crate::assets::set_primary_icon(
                    &player.icon,
                    if play.is_active() {
                        crate::assets::icons::PAUSE
                    } else {
                        crate::assets::icons::PLAY
                    },
                );
            }
        });
        let weak = Rc::downgrade(&player);
        player.route.frame.connect_unmap(move |_| {
            if let Some(player) = weak.upgrade() {
                player.pause();
            }
        });
        player.bind_plot(false);
        if player.profile.is_some() {
            player.bind_plot(true);
        }
        let weak = Rc::downgrade(&player);
        PreferenceManager::shared().bind_preference(
            &player.route.frame,
            PreferenceManager::unit_system,
            move |_, units| {
                if let Some(player) = weak.upgrade() {
                    player.units.set(units);
                    player.total.set_text(&distance(player.distance_m, units));
                    player.sync();
                }
            },
        );
        player
    }

    pub(super) fn controls(&self) -> gtk::CenterBox {
        let controls = gtk::CenterBox::new();
        controls.set_start_widget(Some(&self.elapsed));
        controls.set_center_widget(Some(&self.play));
        controls.set_end_widget(Some(&self.total));
        controls
    }

    fn plot(&self, profile: bool) -> Option<&Plot> {
        if profile {
            self.profile.as_ref()
        } else {
            Some(&self.route)
        }
    }

    fn bind_plot(self: &Rc<Self>, profile: bool) {
        let Some(plot) = self.plot(profile) else {
            return;
        };
        let weak = Rc::downgrade(self);
        plot.marker.set_draw_func(move |_, context, _, _| {
            if let Some(player) = weak.upgrade() {
                player.draw(context, profile);
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_enter(move |_, x, y| {
            if let Some(player) = weak.upgrade() {
                player.last_pointer.set(None);
                player.hover(profile, x, y);
            }
        });
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(player) = weak.upgrade() {
                player.hover(profile, x, y);
            }
        });
        plot.frame.add_controller(motion);
    }

    pub(in crate::ui::preview) fn hover(&self, profile: bool, x: f64, y: f64) {
        // Redraws can re-emit motion without the pointer moving after an explicit play.
        if self.last_pointer.replace(Some((profile, x, y))) == Some((profile, x, y)) {
            return;
        }
        self.pause();
        let Some(plot) = self.plot(profile) else {
            return;
        };
        let (left, top, sx, sy) = plot.transform();
        if sx <= 0.0 || sy <= 0.0 {
            return;
        }
        let x = (x - left) / sx;
        let y = (y - top) / sy;
        if profile {
            self.position.set(position::at_distance(
                &self.samples,
                profile_distance(x, self.distance_m),
            ));
        } else if (0.0..=800.0).contains(&x) && (0.0..=600.0).contains(&y) {
            self.position.set(position::nearest(&self.samples, [x, y]));
        }
        self.sync();
    }

    pub(in crate::ui::preview) fn command(&self, command: MediaCommand) -> bool {
        match command {
            MediaCommand::PlayPause if self.distance_m > 0.0 => {
                self.play.set_active(!self.play.is_active())
            }
            MediaCommand::Seek(seek) if self.distance_m > 0.0 => {
                self.pause();
                let distance = match seek {
                    MediaSeek::Start => 0.0,
                    MediaSeek::End => self.distance_m,
                    MediaSeek::By(delta) => {
                        self.position.get().map_or(0.0, |p| p.distance_m)
                            + delta as f64 / 1_000_000.0 / PLAYBACK_SECONDS * self.distance_m
                    }
                };
                self.position.set(position::at_distance(
                    &self.samples,
                    distance.clamp(0.0, self.distance_m),
                ));
                self.sync();
            }
            _ => return false,
        }
        true
    }

    fn start(self: &Rc<Self>) {
        if self
            .position
            .get()
            .is_some_and(|p| p.distance_m >= self.distance_m)
        {
            self.position.set(self.samples.first().copied());
            self.sync();
        }
        self.last_frame.set(None);
        let weak = Rc::downgrade(self);
        let id = self.route.frame.add_tick_callback(move |_, clock| {
            let Some(player) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            if let Some(last) = player.last_frame.replace(Some(now)) {
                let distance = player.position.get().map_or(0.0, |p| p.distance_m)
                    + (now - last) as f64 / 1_000_000.0 / PLAYBACK_SECONDS * player.distance_m;
                player.position.set(position::at_distance(
                    &player.samples,
                    distance.min(player.distance_m),
                ));
                player.sync();
                if distance >= player.distance_m {
                    player.tick.borrow_mut().take();
                    player.play.set_active(false);
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
        self.tick.replace(Some(id));
    }

    pub(in crate::ui::preview) fn pause(&self) {
        self.play.set_active(false);
        self.stop_tick();
    }

    fn stop_tick(&self) {
        if let Some(id) = self.tick.borrow_mut().take() {
            id.remove();
        }
        self.last_frame.set(None);
    }

    fn sync(&self) {
        let distance = distance(
            self.position.get().map_or(0.0, |p| p.distance_m),
            self.units.get(),
        );
        self.elapsed.set_text(&distance);
        for plot in std::iter::once(&self.route).chain(self.profile.iter()) {
            crate::ui::accessibility::set_description(&plot.picture, Some(&distance));
            plot.marker.queue_draw();
        }
    }

    fn draw(&self, context: &gtk::cairo::Context, profile: bool) {
        let (Some(plot), Some(position)) = (self.plot(profile), self.position.get()) else {
            return;
        };
        let (left, top, sx, sy) = plot.transform();
        let colors = palette();
        let color = |c: gdk::RGBA| {
            context.set_source_rgba(
                c.red().into(),
                c.green().into(),
                c.blue().into(),
                c.alpha().into(),
            )
        };
        let (x, y) = if profile {
            let x = profile_x(position.distance_m, self.distance_m) * sx;
            color(colors.dim);
            context.set_line_width(1.0);
            context.move_to(x, 0.0);
            context.line_to(x, f64::from(plot.marker.height()));
            let _ = context.stroke();
            let Some(y) = position.elevation_y else {
                return;
            };
            (x, f64::from(y) * sy)
        } else {
            (
                left + f64::from(position.position[0]) * sx,
                top + f64::from(position.position[1]) * sy,
            )
        };
        color(colors.accent_bright);
        context.arc(x, y, 4.5, 0.0, std::f64::consts::TAU);
        let _ = context.fill_preserve();
        color(colors.background);
        context.set_line_width(2.0);
        let _ = context.stroke();
    }
}

impl Drop for RoutePlayer {
    fn drop(&mut self) {
        self.stop_tick();
    }
}
