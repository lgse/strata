// SPDX-License-Identifier: MIT

use super::super::*;

#[test]
fn progress_throttle_reports_the_first_update_and_limits_subsequent_bursts() {
    let throttle = SizeProgressThrottle::default();
    let started = Instant::now();
    assert!(throttle.should_update(started));
    for milliseconds in 1..150 {
        assert!(!throttle.should_update(started + Duration::from_millis(milliseconds)));
    }
    assert!(throttle.should_update(started + SIZE_PROGRESS_INTERVAL));
    assert!(!throttle.should_update(started + SIZE_PROGRESS_INTERVAL));
    assert!(!throttle.should_update(started + Duration::from_millis(299)));
    assert!(throttle.should_update(started + Duration::from_millis(300)));
    assert!(throttle.should_update(started + Duration::from_secs(2)));
    assert!(SizeProgressThrottle::default().should_update(started));
}

#[test]
fn size_value_and_spinner_bounds_are_stable_across_digits_and_units() {
    crate::test_support::gtk_test(
        "ui::browser::properties::tests::progress::size_value_and_spinner_bounds_are_stable_across_digits_and_units",
        || {
            crate::ui::window::load_styles();
            let _theme = crate::ui::theme::ThemeManager::shared();
            let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("properties-size-spinner");
            spinner.set_valign(gtk::Align::Center);
            let size = properties_size_row(&details, "0 B", &spinner);
            let measurement = details.measure(gtk::Orientation::Horizontal, -1);
            details.allocate(600, 40, -1, None);
            let size_bounds = size.compute_bounds(&details).expect("size bounds");
            let spinner_bounds = spinner.compute_bounds(&details).expect("spinner bounds");
            assert_eq!(spinner_bounds.x() + spinner_bounds.width(), 600.0);
            assert!(size_bounds.x() + size_bounds.width() <= spinner_bounds.x());
            for value in [
                "9 B",
                "99 B",
                "999 B",
                "1 kB",
                "111.1 MB",
                "888.8 MB",
                "1 GB",
                "≥ 999.9 GB",
                "≥ 18446744.1 TB",
                "Unavailable",
            ] {
                size.set_text(value);
                assert_eq!(
                    details.measure(gtk::Orientation::Horizontal, -1),
                    measurement,
                    "{value}"
                );
                details.allocate(600, 40, -1, None);
                assert_eq!(size.compute_bounds(&details), Some(size_bounds), "{value}");
                assert_eq!(
                    spinner.compute_bounds(&details),
                    Some(spinner_bounds),
                    "{value}"
                );
            }
            size.set_text("111.1 MB");
            let narrow_digits = size.layout().pixel_size();
            size.set_text("888.8 MB");
            assert_eq!(size.layout().pixel_size(), narrow_digits);
        },
    );
}
