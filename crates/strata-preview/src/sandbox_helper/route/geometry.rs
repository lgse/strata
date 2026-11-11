// SPDX-License-Identifier: MIT

use super::{Point, Route};
use crate::services::route_preview::{PROFILE_HEIGHT, ROUTE_HEIGHT, ROUTE_WIDTH, RouteSample};

pub(super) struct Trace {
    pub samples: Vec<RouteSample>,
    pub distance_m: f64,
    pub elevation: Option<[f64; 2]>,
}

pub(super) fn distance(a: Point, b: Point) -> f64 {
    let lat = ((b.lat - a.lat).to_radians() / 2.0).sin();
    let lon = ((b.lon - a.lon).to_radians() / 2.0).sin();
    let haversine = lat * lat + a.lat.to_radians().cos() * b.lat.to_radians().cos() * lon * lon;
    12_742_000.0 * haversine.clamp(0.0, 1.0).sqrt().asin()
}

pub(super) fn prepare(route: &Route) -> Trace {
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
            seam = longitudes[(i + 1) % longitudes.len()];
        }
    }
    let project = |p: &Point| {
        let x = (p.lon.rem_euclid(360.0) - seam)
            .rem_euclid(360.0)
            .to_radians();
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
    let width = f64::from(ROUTE_WIDTH);
    let height = f64::from(ROUTE_HEIGHT);
    let scale = ((width - 48.0) / (max[0] - min[0]).max(1e-8))
        .min((height - 48.0) / (max[1] - min[1]).max(1e-8));
    let xy = |point: &Point| {
        let p = project(point);
        [
            ((p[0] - (min[0] + max[0]) / 2.0) * scale + width / 2.0) as f32,
            ((p[1] - (min[1] + max[1]) / 2.0) * scale + height / 2.0) as f32,
        ]
    };
    let mut distance_m = 0.0;
    let mut samples = Vec::with_capacity(route.points);
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut has_profile = false;
    for segment in &route.segments {
        let mut previous: Option<Point> = None;
        for point in segment {
            if let Some(previous) = previous {
                let step = distance(previous, *point);
                distance_m += step;
                has_profile |=
                    step > 0.0 && previous.elevation.is_some() && point.elevation.is_some();
            }
            samples.push(RouteSample {
                position: xy(point),
                distance_m,
                elevation_y: None,
                starts_segment: previous.is_none(),
            });
            previous = Some(*point);
            if let Some(elevation) = point.elevation {
                low = low.min(elevation);
                high = high.max(elevation);
            }
        }
    }
    if has_profile {
        let height = f64::from(PROFILE_HEIGHT);
        for (sample, point) in samples.iter_mut().zip(route.segments.iter().flatten()) {
            sample.elevation_y = point.elevation.map(|e| {
                if high == low {
                    (height / 2.0) as f32
                } else {
                    (height - 4.0 - (e - low) / (high - low) * (height - 8.0)) as f32
                }
            });
        }
    }
    Trace {
        samples,
        distance_m,
        elevation: has_profile.then_some([low, high]),
    }
}
