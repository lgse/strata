// SPDX-License-Identifier: MIT

use super::gain;

#[test]
fn the_curve_rises_evenly_and_lands_softly() {
    assert_eq!(gain(0.0), 0.0);
    assert_eq!(gain(1.0), 1.0);
    assert_eq!(gain(-0.5), 0.0);
    assert_eq!(gain(1.5), 1.0);
    assert!(gain(0.1) < 0.01, "the start is inaudible");
    assert!(gain(0.9) > 0.9, "the landing is nearly complete");
    let mut previous = 0.0;
    for step in 1..=100 {
        let value = gain(f64::from(step) / 100.0);
        assert!(value >= previous, "monotonic at step {step}");
        previous = value;
    }
    assert!(
        gain(0.5) - gain(0.4) > gain(0.1),
        "most of the rise happens in the middle"
    );
}
