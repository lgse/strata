// SPDX-License-Identifier: MIT

use super::super::audio::palette::{follow_theme, palette};
use super::position;
use crate::services::route_preview::{
    PROFILE_HEIGHT, ROUTE_HEIGHT, ROUTE_WIDTH, RouteSample, profile_distance, profile_x,
};
use gtk::{gdk, glib, prelude::*};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::preview) enum PlotKind {
    Route,
    Elevation,
}

pub(super) struct Plot {
    pub frame: gtk::Overlay,
    pub picture: gtk::Picture,
    pub marker: gtk::DrawingArea,
    kind: PlotKind,
}

impl Plot {
    pub fn new(png: Vec<u8>, kind: PlotKind) -> Result<Self, glib::Error> {
        let texture = gdk::Texture::from_bytes(&glib::Bytes::from_owned(png))?;
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        let label = match kind {
            PlotKind::Route => "Route preview",
            PlotKind::Elevation => "Elevation profile",
        };
        crate::ui::accessibility::set_label(&picture, &crate::i18n::tr(label));
        picture.set_content_fit(if kind == PlotKind::Elevation {
            gtk::ContentFit::Fill
        } else {
            gtk::ContentFit::Contain
        });
        let frame = gtk::Overlay::new();
        frame.set_child(Some(&picture));
        frame.set_hexpand(true);
        frame.set_vexpand(kind == PlotKind::Route);
        let marker = gtk::DrawingArea::new();
        marker.set_can_target(false);
        frame.add_overlay(&marker);
        frame.set_cursor_from_name(Some("crosshair"));
        follow_theme(&marker);
        Ok(Self {
            frame,
            picture,
            marker,
            kind,
        })
    }

    fn transform(&self) -> (f64, f64, f64, f64) {
        let width = f64::from(self.marker.width());
        let height = f64::from(self.marker.height());
        if self.kind == PlotKind::Elevation {
            return (
                0.0,
                0.0,
                width / f64::from(ROUTE_WIDTH),
                height / f64::from(PROFILE_HEIGHT),
            );
        }
        let scale = (width / f64::from(ROUTE_WIDTH)).min(height / f64::from(ROUTE_HEIGHT));
        (
            (width - f64::from(ROUTE_WIDTH) * scale) / 2.0,
            (height - f64::from(ROUTE_HEIGHT) * scale) / 2.0,
            scale,
            scale,
        )
    }

    pub fn pick(
        &self,
        samples: &[RouteSample],
        total_m: f64,
        previous: Option<RouteSample>,
        x: f64,
        y: f64,
    ) -> Option<RouteSample> {
        let (left, top, sx, sy) = self.transform();
        if sx <= 0.0 || sy <= 0.0 {
            return None;
        }
        let x = (x - left) / sx;
        let y = (y - top) / sy;
        match self.kind {
            PlotKind::Elevation => position::at_distance(samples, profile_distance(x, total_m)),
            PlotKind::Route
                if (0.0..=f64::from(ROUTE_WIDTH)).contains(&x)
                    && (0.0..=f64::from(ROUTE_HEIGHT)).contains(&y) =>
            {
                position::nearest(samples, [x, y], previous.map(|p| p.distance_m), 2.0 / sx)
            }
            PlotKind::Route => None,
        }
    }
    pub fn draw_marker(&self, context: &gtk::cairo::Context, position: RouteSample, total_m: f64) {
        let (left, top, sx, sy) = self.transform();
        let colors = palette();
        let color = |c: gdk::RGBA| {
            context.set_source_rgba(
                c.red().into(),
                c.green().into(),
                c.blue().into(),
                c.alpha().into(),
            )
        };
        let (x, y) = if self.kind == PlotKind::Elevation {
            let x = profile_x(position.distance_m, total_m) * sx;
            color(colors.dim);
            context.set_line_width(1.0);
            context.move_to(x, 0.0);
            context.line_to(x, f64::from(self.marker.height()));
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
