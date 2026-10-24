// SPDX-License-Identifier: MIT

use crate::services::route_preview::RouteSample;

fn interpolate(a: RouteSample, b: RouteSample, fraction: f64) -> RouteSample {
    let mix = |a: f32, b: f32| (f64::from(a) + f64::from(b - a) * fraction) as f32;
    RouteSample {
        position: [
            mix(a.position[0], b.position[0]),
            mix(a.position[1], b.position[1]),
        ],
        distance_m: a.distance_m + (b.distance_m - a.distance_m) * fraction,
        elevation_y: if fraction == 0.0 {
            a.elevation_y
        } else if fraction == 1.0 {
            b.elevation_y
        } else {
            a.elevation_y.zip(b.elevation_y).map(|(a, b)| mix(a, b))
        },
        starts_segment: false,
    }
}

pub(super) fn at_distance(samples: &[RouteSample], distance: f64) -> Option<RouteSample> {
    if distance <= 0.0 {
        return samples.first().copied();
    }
    let index = samples
        .partition_point(|p| p.distance_m <= distance)
        .saturating_sub(1);
    let a = *samples.get(index)?;
    let Some(&b) = samples.get(index + 1) else {
        return Some(a);
    };
    if b.starts_segment || b.distance_m <= a.distance_m {
        return Some(a);
    }
    Some(interpolate(
        a,
        b,
        ((distance - a.distance_m) / (b.distance_m - a.distance_m)).clamp(0.0, 1.0),
    ))
}

pub(super) fn nearest(samples: &[RouteSample], point: [f64; 2]) -> Option<RouteSample> {
    let mut closest = None;
    let mut best = f64::INFINITY;
    for (index, &a) in samples.iter().enumerate() {
        let candidate = match samples.get(index + 1).filter(|b| !b.starts_segment) {
            Some(&b) => {
                let dx = f64::from(b.position[0] - a.position[0]);
                let dy = f64::from(b.position[1] - a.position[1]);
                let length_squared = dx * dx + dy * dy;
                let fraction = if length_squared > 0.0 {
                    ((point[0] - f64::from(a.position[0])) * dx
                        + (point[1] - f64::from(a.position[1])) * dy)
                        / length_squared
                } else {
                    0.0
                };
                interpolate(a, b, fraction.clamp(0.0, 1.0))
            }
            None => a,
        };
        let distance = (point[0] - f64::from(candidate.position[0]))
            .hypot(point[1] - f64::from(candidate.position[1]));
        if distance < best {
            best = distance;
            closest = Some(candidate);
        }
    }
    closest
}

#[cfg(test)]
mod tests;
