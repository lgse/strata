// SPDX-License-Identifier: MIT

mod player;
pub(in crate::ui::preview) mod plot;
mod position;

pub(super) use player::RoutePlayer;

use super::{ListingPosition, MediaFamily, PreviewState, components, player_layout};
use crate::services::route_preview::{RouteFormat, RoutePreview};
use crate::ui::preferences::{PreferenceManager, UnitSystem};
use gtk::{glib, prelude::*};
use plot::{Plot, PlotKind};
use std::rc::Rc;

fn distance(metres: f64, units: UnitSystem) -> String {
    match units {
        UnitSystem::Metric => rust_i18n::t!(
            "%{distance} km",
            distance = crate::i18n::decimal(metres / 1000.0, 1)
        ),
        UnitSystem::Imperial => rust_i18n::t!(
            "%{distance} mi",
            distance = crate::i18n::decimal(metres / 1609.344, 1)
        ),
    }
    .into_owned()
}

fn elevation_range([low, high]: [f64; 2], units: UnitSystem) -> String {
    match units {
        UnitSystem::Metric => rust_i18n::t!(
            "%{minimum}–%{maximum} m",
            minimum = crate::i18n::decimal(low, 0),
            maximum = crate::i18n::decimal(high, 0)
        ),
        UnitSystem::Imperial => rust_i18n::t!(
            "%{minimum}–%{maximum} ft",
            minimum = crate::i18n::decimal(low / 0.3048, 0),
            maximum = crate::i18n::decimal(high / 0.3048, 0)
        ),
    }
    .into_owned()
}

impl PreviewState {
    pub(super) fn render_route(self: &Rc<Self>, preview: RoutePreview) {
        if let Err(error) = self.append_route(preview) {
            self.show_message(&crate::i18n::tr("Preview unavailable"), &error.to_string());
        }
    }

    fn append_route(self: &Rc<Self>, preview: RoutePreview) -> Result<(), glib::Error> {
        let route = Plot::new(preview.png, PlotKind::Route)?;
        super::install_preview_drag(&route.frame, self);
        let profile_plot = preview
            .profile
            .map(|png| Plot::new(png, PlotKind::Elevation))
            .transpose()?;
        let player = RoutePlayer::new(
            route,
            profile_plot,
            preview.samples,
            preview.distance_m,
            preview.elevation,
        );
        let header = components::DetailsHeader::new(None);
        header.eyebrow.set_text(
            &self
                .listing_position(MediaFamily::Route)
                .0
                .map(ListingPosition::caption)
                .unwrap_or_default(),
        );
        let badges = components::badge_row();
        if let Some(entry) = self.current.borrow().as_ref() {
            header.title.set_text(&entry.display_name);
            if let Some(format) = RouteFormat::for_name(&entry.native_name) {
                badges.append(&components::badge(format.label()));
            }
        }
        let manager = PreferenceManager::shared();
        let distance_badge = components::badge("");
        manager.bind_preference(
            &distance_badge,
            PreferenceManager::unit_system,
            move |widget, units| {
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    label.set_text(&distance(preview.distance_m, units));
                }
            },
        );
        badges.append(&distance_badge);
        if let Some(seconds) = preview.duration_seconds {
            badges.append(&components::badge(&super::audio::clock(
                seconds as i64 * 1_000_000,
            )));
        }
        header.root.append(&badges);
        let profile =
            if let (Some(plot), Some([low, high])) = (player.profile.as_ref(), preview.elevation) {
                let section = gtk::Box::new(gtk::Orientation::Vertical, components::GAP);
                section.set_margin_bottom(components::GAP - components::CONTROLS_GAP);
                let labels = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let label = components::label("preview-media-time");
                label.set_text(&crate::i18n::tr("Elevation"));
                let range = components::label("preview-media-time");
                range.set_hexpand(true);
                range.set_xalign(1.0);
                manager.bind_preference(
                    &range,
                    PreferenceManager::unit_system,
                    move |widget, units| {
                        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                            label.set_text(&elevation_range([low, high], units));
                        }
                    },
                );
                labels.append(&label);
                labels.append(&player.elevation_label);
                labels.append(&range);
                section.append(&labels);
                plot.frame.set_size_request(-1, 45);
                section.append(&plot.frame);
                Some(section)
            } else {
                None
            };
        let controls = player.controls();
        let (root, layout) = player_layout::section(player_layout::Areas {
            content: player.route.frame.upcast_ref(),
            details: header.root.upcast_ref(),
            badges: None,
            timeline: profile.as_ref().map(|p| p.upcast_ref()),
            controls: controls.upcast_ref(),
        });
        layout.set_paintable(player.route.picture.paintable().as_ref());
        self.content.append(&root);
        self.route.replace(Some(player));
        Ok(())
    }
}
