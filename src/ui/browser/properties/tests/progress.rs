// SPDX-License-Identifier: MIT

use super::super::*;

#[test]
fn closing_properties_stops_live_measurement() {
    crate::test_support::gtk_test(
        "ui::browser::properties::tests::progress::closing_properties_stops_live_measurement",
        || {
            let root = tempfile::tempdir().expect("fixture");
            for index in 0..1000 {
                std::fs::write(root.path().join(index.to_string()), b"abc").expect("file");
            }
            for destroy_window in [false, true] {
                let view = crate::ui::browser::BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    crate::ui::browser::PeekBehavior::default(),
                );
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&view.widget()));
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                view.state
                    .show_folder_properties(&Location::local(root.path()));
                let size = super::size_label(overlay.upcast_ref()).expect("SIZE");
                let counts = super::row_label(overlay.upcast_ref(), "CONTAINS").expect("CONTAINS");
                let spinner = size
                    .next_sibling()
                    .and_downcast::<gtk::Spinner>()
                    .expect("spinner");
                let context = glib::MainContext::default();
                let deadline = Instant::now() + Duration::from_secs(5);
                while counts.text() == "0 files, 0 folders" {
                    assert!(Instant::now() < deadline, "measurement did not start");
                    context.iteration(false);
                }
                assert!(spinner.is_spinning());
                assert_ne!(counts.text(), "1000 files, 0 folders");
                let before_size = size.text();
                let before_counts = counts.text();
                if destroy_window {
                    window.destroy();
                } else {
                    let layer = overlay
                        .last_child()
                        .and_downcast::<gtk::Box>()
                        .expect("modal");
                    dismiss_modal_layer(&layer, &overlay, None);
                }
                context.block_on(glib::timeout_future(Duration::from_millis(300)));
                assert_eq!(size.text(), before_size);
                assert_eq!(counts.text(), before_counts);
                assert!(
                    spinner.is_spinning(),
                    "the cancelled task must not reach completion"
                );
                window.destroy();
                view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn measurement_warnings_distinguish_and_combine_incomplete_reasons() {
    use crate::adapters::directory_summary::MeasurementIssues;

    for (issues, reasons) in [
        (MeasurementIssues::default(), Vec::new()),
        (
            MeasurementIssues {
                unreadable: true,
                ..Default::default()
            },
            vec!["Some folders or entries couldn't be read."],
        ),
        (
            MeasurementIssues {
                timed_out: true,
                ..Default::default()
            },
            vec!["The five-minute calculation limit was reached."],
        ),
        (
            MeasurementIssues {
                depth_limited: true,
                ..Default::default()
            },
            vec!["Some folders exceeded the 64-level nesting limit."],
        ),
        (
            MeasurementIssues {
                unreadable: true,
                timed_out: true,
                depth_limited: true,
            },
            vec![
                "Some folders or entries couldn't be read.",
                "The five-minute calculation limit was reached.",
                "Some folders exceeded the 64-level nesting limit.",
            ],
        ),
    ] {
        let summary = DirectorySummary {
            issues,
            ..Default::default()
        };
        let warning = measurement_warning_text(&summary);
        if reasons.is_empty() {
            assert!(warning.is_none());
            assert!(!directory_counts_label(&summary).contains('≥'));
        } else {
            assert_eq!(
                warning,
                Some(format!("Totals are incomplete.\n{}", reasons.join("\n")))
            );
            assert_eq!(directory_counts_label(&summary), "≥ 0 files, ≥ 0 folders");
        }
    }
}

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
