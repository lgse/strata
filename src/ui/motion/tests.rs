// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn emphasized_deceleration_has_expected_endpoints() {
    assert!(emphasized_deceleration(0.0).abs() < 0.0001);
    assert!((emphasized_deceleration(1.0) - 1.0).abs() < 0.0001);
}

#[test]
fn emphasized_deceleration_is_monotonic_and_front_loaded() {
    let samples: Vec<_> = (0..=20)
        .map(|sample| emphasized_deceleration(f64::from(sample) / 20.0))
        .collect();

    assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(emphasized_deceleration(0.5) > 0.8);
}

#[test]
fn reduce_motion_holds_toolkit_animations_off_until_it_is_cleared() {
    crate::test_support::gtk_test(
        "ui::motion::tests::reduce_motion_holds_toolkit_animations_off_until_it_is_cleared",
        || {
            let settings = gtk::Settings::default().expect("GTK settings");
            set_reduce_motion(false);
            settings.reset_property(TOOLKIT_ANIMATIONS);
            let desktop = settings.is_gtk_enable_animations();

            set_reduce_motion(true);
            assert!(!settings.is_gtk_enable_animations());
            assert!(!animations_enabled());
            set_reduce_motion(false);
            assert_eq!(settings.is_gtk_enable_animations(), desktop);

            settings.set_gtk_enable_animations(!desktop);
            set_reduce_motion(false);
            assert_eq!(
                settings.is_gtk_enable_animations(),
                !desktop,
                "clearing must not reset a value reduce motion never overrode"
            );
            settings.reset_property(TOOLKIT_ANIMATIONS);
        },
    );
}
