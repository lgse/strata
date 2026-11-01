// SPDX-License-Identifier: MIT

use super::*;

fn sample(
    x: f32,
    y: f32,
    distance_m: f64,
    elevation_y: Option<f32>,
    starts_segment: bool,
) -> RouteSample {
    RouteSample {
        position: [x, y],
        distance_m,
        elevation_y,
        starts_segment,
    }
}

#[test]
fn hovering_a_route_segment_and_seeking_distance_choose_the_same_elevation() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 100.0, 1000.0, Some(20.0), false),
    ];
    let hovered = nearest(&samples, [25.0, 25.0], None, 0.0).expect("nearest route position");
    assert_eq!(hovered.position, [25.0, 25.0]);
    assert_eq!(hovered.distance_m, 250.0);
    assert_eq!(hovered.elevation_y, Some(95.0));
    assert_eq!(at_distance(&samples, 250.0), Some(hovered));
    assert_eq!(
        at_distance(&samples, 5000.0)
            .expect("clamped route endpoint")
            .distance_m,
        1000.0
    );
}

#[test]
fn hovering_does_not_invent_a_connection_across_route_or_elevation_gaps() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 0.0, 100.0, None, false),
        sample(500.0, 0.0, 100.0, Some(20.0), true),
        sample(600.0, 0.0, 200.0, Some(40.0), false),
    ];
    let gap = nearest(&samples, [300.0, 0.0], None, 0.0).expect("nearest segment endpoint");
    assert!(gap.position == [100.0, 0.0] || gap.position == [500.0, 0.0]);
    assert_eq!(
        at_distance(&samples, 50.0)
            .expect("position with missing elevation")
            .elevation_y,
        None
    );
    assert_eq!(
        at_distance(&samples, 150.0)
            .expect("position within second segment")
            .position,
        [550.0, 0.0]
    );
    assert_eq!(
        at_distance(&samples, 150.0)
            .expect("elevation within second segment")
            .elevation_y,
        Some(30.0)
    );
}

#[test]
fn stationary_and_isolated_points_can_be_hovered_without_dividing_by_zero() {
    let samples = [
        sample(10.0, 20.0, 0.0, None, true),
        sample(10.0, 20.0, 0.0, None, false),
        sample(400.0, 100.0, 0.0, None, true),
    ];
    assert_eq!(
        nearest(&samples, [399.0, 100.0], None, 0.0)
            .expect("nearest isolated point")
            .position,
        [400.0, 100.0]
    );
    assert_eq!(at_distance(&samples, 0.0), Some(samples[0]));
    assert!(at_distance(&[], 0.0).is_none());
}

#[test]
fn sub_metre_profile_hover_selects_the_same_route_position_as_the_drawing() {
    use crate::services::route_preview::{profile_distance, profile_x};
    let samples = [
        sample(0.0, 0.0, 0.0, Some(120.0), true),
        sample(100.0, 100.0, 0.5, Some(20.0), false),
    ];
    let endpoint = profile_x(0.5, 0.5);
    let hovered =
        at_distance(&samples, profile_distance(endpoint, 0.5)).expect("profile endpoint position");
    assert_eq!(hovered.position, [100.0, 100.0]);
    assert_eq!(hovered.elevation_y, Some(20.0));
    assert_eq!(
        at_distance(&samples, profile_distance(endpoint / 2.0 + 1.0, 0.5))
            .expect("profile midpoint position")
            .distance_m,
        0.25
    );
}

#[test]
fn hover_stays_on_the_same_nearby_pass_without_snapping_to_recorded_points() {
    let samples = [
        sample(0.0, 0.0, 0.0, Some(10.0), true),
        sample(100.0, 0.0, 100.0, Some(20.0), false),
        sample(100.0, 1.0, 100.0, Some(30.0), true),
        sample(0.0, 1.0, 200.0, Some(40.0), false),
    ];
    let mut previous = 170.0;
    for x in [25.0, 24.5, 24.0] {
        let hovered =
            nearest(&samples, [x, 0.4], Some(previous), 2.0).expect("position on the return pass");
        assert_eq!(hovered.position, [x as f32, 1.0]);
        assert_eq!(hovered.distance_m, 200.0 - x);
        assert_eq!(at_distance(&samples, hovered.distance_m), Some(hovered));
        previous = hovered.distance_m;
    }
    let switched = nearest(&samples, [25.0, 0.0], Some(previous), 0.5)
        .expect("position on the clearly closer outward pass");
    assert_eq!(switched.distance_m, 25.0);
    assert_eq!(switched.position, [25.0, 0.0]);
}
