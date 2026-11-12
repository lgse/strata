// SPDX-License-Identifier: MIT

use super::{Route, geometry};
use crate::services::{
    ModelPalette,
    route_preview::{
        PROFILE_HEIGHT, ROUTE_HEIGHT, ROUTE_WIDTH, RoutePreview, RouteSample, profile_x,
    },
};
use resvg::tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub(super) fn draw(route: &Route, palette: ModelPalette) -> Result<RoutePreview, String> {
    let trace = geometry::prepare(route);
    let mut paint = Paint::default();
    let accent = palette.accent;
    paint.set_color_rgba8((accent >> 16) as u8, (accent >> 8) as u8, accent as u8, 255);
    let stroke = Stroke {
        width: 3.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let png = draw_samples(&trace.samples, ROUTE_HEIGHT, &paint, &stroke, |sample| {
        Some(sample.position)
    })?;
    let profile = trace
        .elevation
        .map(|_| {
            draw_samples(
                &trace.samples,
                PROFILE_HEIGHT,
                &paint,
                &Stroke {
                    width: 2.0,
                    ..stroke
                },
                |sample| {
                    sample
                        .elevation_y
                        .map(|y| [profile_x(sample.distance_m, trace.distance_m) as f32, y])
                },
            )
        })
        .transpose()?;
    Ok(RoutePreview {
        png,
        profile,
        distance_m: trace.distance_m,
        duration_seconds: route.duration_seconds(),
        elevation: trace.elevation,
        points: trace.samples.len(),
        samples: trace.samples,
    })
}

fn draw_samples(
    samples: &[RouteSample],
    height: u32,
    paint: &Paint<'_>,
    stroke: &Stroke,
    point: impl Fn(&RouteSample) -> Option<[f32; 2]>,
) -> Result<Vec<u8>, String> {
    let mut image =
        Pixmap::new(ROUTE_WIDTH, height).ok_or("The sandboxed preview renderer failed")?;
    let mut segment = Vec::new();
    for sample in samples {
        let point = point(sample);
        if sample.starts_segment || point.is_none() {
            stroke_line(&mut image, &segment, paint, stroke);
            segment.clear();
        }
        if let Some(point) = point {
            segment.push(point);
        }
    }
    stroke_line(&mut image, &segment, paint, stroke);
    image.encode_png().map_err(|e| e.to_string())
}

fn stroke_line(image: &mut Pixmap, points: &[[f32; 2]], paint: &Paint<'_>, stroke: &Stroke) {
    let Some(&first) = points.first() else {
        return;
    };
    let mut path = PathBuilder::new();
    path.move_to(first[0], first[1]);
    let mut previous = first;
    for (index, &point) in points.iter().enumerate().skip(1) {
        if index + 1 == points.len()
            || (point[0] - previous[0]).hypot(point[1] - previous[1]) >= 0.6
        {
            path.line_to(point[0], point[1]);
            previous = point;
        }
    }
    if points.len() == 1 || previous == first {
        path.line_to(first[0] + 0.01, first[1]);
    }
    if let Some(path) = path.finish() {
        image.stroke_path(&path, paint, stroke, Transform::identity(), None);
    }
}
