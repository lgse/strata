// SPDX-License-Identifier: MIT

use super::super::keyboard::{MediaCommand, MediaSeek};
use super::{
    distance,
    plot::{Plot, PlotKind},
    position,
};
use crate::services::route_preview::{RouteSample, profile_elevation};
use crate::ui::preferences::{PreferenceManager, UnitSystem};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const PLAYBACK_SECONDS: f64 = 15.0;

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
    elevation: Option<[f64; 2]>,
    pub(super) elevation_label: gtk::Label,
    pub(in crate::ui::preview) position: Cell<Option<RouteSample>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    last_frame: Cell<Option<i64>>,
    last_pointer: Cell<Option<(PlotKind, f64, f64)>>,
}

impl RoutePlayer {
    pub(super) fn new(
        route: Plot,
        profile: Option<Plot>,
        samples: Vec<RouteSample>,
        distance_m: f64,
        elevation: Option<[f64; 2]>,
    ) -> Rc<Self> {
        let icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 22);
        let play = gtk::ToggleButton::new();
        play.add_css_class("preview-media-center");
        play.add_css_class("preview-video-play");
        play.set_child(Some(&icon));
        play.set_sensitive(distance_m > 0.0);
        crate::ui::shortcut_reference::follow_tooltip(
            &play,
            "Play/Pause (%{keys})",
            crate::ui::keys::Keys::One(crate::ui::keys::bindings::MEDIA_PLAY_PAUSE.shown()),
        );
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
            elevation,
            elevation_label: super::super::components::label("preview-media-time"),
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
        player.bind_plot(PlotKind::Route);
        if player.profile.is_some() {
            player.bind_plot(PlotKind::Elevation);
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

    fn plot(&self, kind: PlotKind) -> Option<&Plot> {
        match kind {
            PlotKind::Route => Some(&self.route),
            PlotKind::Elevation => self.profile.as_ref(),
        }
    }

    fn bind_plot(self: &Rc<Self>, kind: PlotKind) {
        let Some(plot) = self.plot(kind) else {
            return;
        };
        let weak = Rc::downgrade(self);
        plot.marker.set_draw_func(move |_, context, _, _| {
            if let Some(player) = weak.upgrade()
                && let (Some(plot), Some(position)) = (player.plot(kind), player.position.get())
            {
                plot.draw_marker(context, position, player.distance_m);
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_enter(move |_, x, y| {
            if let Some(player) = weak.upgrade() {
                player.last_pointer.set(None);
                player.hover(kind, x, y);
            }
        });
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(player) = weak.upgrade() {
                player.hover(kind, x, y);
            }
        });
        plot.frame.add_controller(motion);
    }

    pub(in crate::ui::preview) fn hover(&self, kind: PlotKind, x: f64, y: f64) {
        // Redraws can re-emit motion without the pointer moving after an explicit play.
        if self.last_pointer.replace(Some((kind, x, y))) == Some((kind, x, y)) {
            return;
        }
        self.pause();
        let Some(plot) = self.plot(kind) else {
            return;
        };
        if let Some(position) = plot.pick(&self.samples, self.distance_m, self.position.get(), x, y)
        {
            self.position.set(Some(position));
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

    fn elevation_text(&self) -> Option<String> {
        let position = self.position.get()?;
        let height = profile_elevation(position.elevation_y?, self.elevation?);
        Some(
            match self.units.get() {
                UnitSystem::Metric => rust_i18n::t!(
                    "%{elevation} m",
                    elevation = crate::i18n::decimal(height, 0),
                ),
                UnitSystem::Imperial => rust_i18n::t!(
                    "%{elevation} ft",
                    elevation = crate::i18n::decimal(height / 0.3048, 0),
                ),
            }
            .into_owned(),
        )
    }

    fn sync(&self) {
        let elevation = self.elevation_text();
        self.elevation_label
            .set_text(elevation.as_deref().unwrap_or_default());
        self.elevation_label.set_visible(elevation.is_some());
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
}

impl Drop for RoutePlayer {
    fn drop(&mut self) {
        self.stop_tick();
    }
}
