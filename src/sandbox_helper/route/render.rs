// SPDX-License-Identifier: MIT

use super::{Point, Route};
use crate::services::{
    ModelPalette,
    route_preview::{RoutePreview, RouteSample, profile_x},
};
use resvg::tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub(super) fn distance(a: Point, b: Point) -> f64 {
    let lat = ((b.lat - a.lat).to_radians() / 2.0).sin();
    let lon = ((b.lon - a.lon).to_radians() / 2.0).sin();
    let haversine = lat * lat + a.lat.to_radians().cos() * b.lat.to_radians().cos() * lon * lon;
    12_742_000.0 * haversine.clamp(0.0, 1.0).sqrt().asin()
}

pub(super) fn draw(route: &Route, palette: ModelPalette) -> Result<RoutePreview, String> {
    // Put the longitude seam in the largest empty arc, keeping date-line routes together.
    let mut longitudes: Vec<_> = route
        .segments
        .iter()
        .flatten()
        .map(|p| p.lon.rem_euclid(360.0))
        .collect();
    longitudes.sort_by(f64::total_cmp);
    let mut seam = longitudes[0];
    let mut gap = 0.0;
    for (i, lon) in longitudes.iter().enumerate() {
        let next = longitudes
            .get(i + 1)
            .copied()
            .unwrap_or(longitudes[0] + 360.0);
        if next - lon > gap {
            gap = next - lon;
            seam = next.rem_euclid(360.0);
        }
    }
    let project = |p: &Point| {
        let x = (p.lon - seam).rem_euclid(360.0).to_radians();
        let latitude = p.lat.clamp(-85.05112878, 85.05112878).to_radians();
        let y = -(std::f64::consts::FRAC_PI_4 + latitude / 2.0).tan().ln();
        [x, y]
    };
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for point in route.segments.iter().flatten() {
        let xy = project(point);
        for axis in 0..2 {
            min[axis] = min[axis].min(xy[axis]);
            max[axis] = max[axis].max(xy[axis]);
        }
    }
    let width = 800;
    let height = 600;
    let scale = (752.0 / (max[0] - min[0]).max(1e-8)).min(552.0 / (max[1] - min[1]).max(1e-8));
    let xy = |point: &Point| {
        let p = project(point);
        [
            ((p[0] - (min[0] + max[0]) / 2.0) * scale + 400.0) as f32,
            ((p[1] - (min[1] + max[1]) / 2.0) * scale + 300.0) as f32,
        ]
    };
    let mut image = Pixmap::new(width, height).ok_or("The sandboxed preview renderer failed")?;
    let mut paint = Paint::default();
    let accent = palette.accent;
    paint.set_color_rgba8((accent >> 16) as u8, (accent >> 8) as u8, accent as u8, 255);
    let stroke = Stroke {
        width: 3.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let mut distance_m = 0.0;
    let mut samples = Vec::with_capacity(route.points);
    let mut profile_segments = Vec::<Vec<[f64; 2]>>::new();
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for segment in &route.segments {
        let screen: Vec<_> = segment.iter().map(xy).collect();
        stroke_line(&mut image, &screen, &paint, &stroke);
        let mut previous = None;
        let mut profile = Vec::new();
        for (index, point) in segment.iter().enumerate() {
            if let Some(previous) = previous {
                distance_m += distance(previous, *point);
            }
            previous = Some(*point);
            samples.push(RouteSample {
                position: xy(point),
                distance_m,
                elevation_y: None,
                starts_segment: index == 0,
            });
            if let Some(elevation) = point.elevation {
                low = low.min(elevation);
                high = high.max(elevation);
                profile.push([distance_m, elevation]);
            } else if !profile.is_empty() {
                profile_segments.push(std::mem::take(&mut profile));
            }
        }
        if !profile.is_empty() {
            profile_segments.push(profile);
        }
    }
    let has_profile = profile_segments
        .iter()
        .any(|s| s.len() >= 2 && s.last().is_some_and(|last| last[0] > s[0][0]));
    if has_profile {
        for (sample, point) in samples.iter_mut().zip(route.segments.iter().flatten()) {
            sample.elevation_y = point.elevation.map(|e| {
                if high == low {
                    70.0
                } else {
                    (136.0 - (e - low) / (high - low) * 132.0) as f32
                }
            });
        }
    }
    let profile = if has_profile {
        let mut graph = Pixmap::new(800, 140).ok_or("The sandboxed preview renderer failed")?;
        for segment in profile_segments {
            let points: Vec<_> = segment
                .iter()
                .map(|[d, e]| {
                    [
                        profile_x(*d, distance_m) as f32,
                        if high == low {
                            70.0
                        } else {
                            (136.0 - (e - low) / (high - low) * 132.0) as f32
                        },
                    ]
                })
                .collect();
            stroke_line(
                &mut graph,
                &points,
                &paint,
                &Stroke {
                    width: 2.0,
                    ..stroke.clone()
                },
            );
        }
        Some(graph.encode_png().map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok(RoutePreview {
        png: image.encode_png().map_err(|e| e.to_string())?,
        profile,
        distance_m,
        elevation: has_profile.then_some([low, high]),
        points: route.points,
        samples,
    })
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
