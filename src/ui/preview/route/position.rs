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

pub(super) fn nearest(
    samples: &[RouteSample],
    point: [f64; 2],
    previous_m: Option<f64>,
    tolerance: f64,
) -> Option<RouteSample> {
    let (mut closest, best, _) = projections(samples, point).min_by(|a, b| a.1.total_cmp(&b.1))?;
    let Some(previous_m) = previous_m else {
        return Some(closest);
    };
    let mut consider = |candidate: RouteSample| {
        if (candidate.distance_m - previous_m).abs() < (closest.distance_m - previous_m).abs() {
            closest = candidate;
        }
    };
    let mut branch = None;
    let mut branch_distance = f64::INFINITY;
    // Keep nearby passes separate, but project freely along the selected pass.
    for (candidate, distance, join_distance) in projections(samples, point) {
        if join_distance > best + tolerance || distance > best + tolerance {
            if let Some(candidate) = branch.take() {
                consider(candidate);
            }
            branch_distance = f64::INFINITY;
        }
        if distance <= best + tolerance && distance < branch_distance {
            branch = Some(candidate);
            branch_distance = distance;
        }
    }
    if let Some(candidate) = branch {
        consider(candidate);
    }
    Some(closest)
}

fn projections(
    samples: &[RouteSample],
    point: [f64; 2],
) -> impl Iterator<Item = (RouteSample, f64, f64)> + '_ {
    samples.iter().enumerate().map(move |(index, &a)| {
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
        let join_distance = if a.starts_segment {
            f64::INFINITY
        } else {
            (point[0] - f64::from(a.position[0])).hypot(point[1] - f64::from(a.position[1]))
        };
        (candidate, distance, join_distance)
    })
}

#[cfg(test)]
mod tests;
