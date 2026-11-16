// SPDX-License-Identifier: MIT

use std::time::Instant;

use gtk::{glib, prelude::*};

use super::{EntryTooltip, REST_DELAY, name_text};
use crate::test_support::gtk_test;

#[test]
fn pointer_must_rest_before_the_tooltip_shows() {
    let tooltip = EntryTooltip::default();
    let start = Instant::now();
    assert!(!tooltip.is_rested(start + REST_DELAY));

    assert!(tooltip.pointer_moved(10.0, 10.0, start));
    assert!(!tooltip.is_rested(start + REST_DELAY / 2));
    assert!(tooltip.is_rested(start + REST_DELAY));

    assert!(!tooltip.pointer_moved(11.0, 10.0, start + REST_DELAY));
    assert!(tooltip.is_rested(start + REST_DELAY));

    let moved = start + REST_DELAY;
    assert!(tooltip.pointer_moved(30.0, 10.0, moved));
    assert!(!tooltip.is_rested(moved + REST_DELAY / 2));
    assert!(tooltip.is_rested(moved + REST_DELAY));

    tooltip.reset();
    assert!(!tooltip.is_rested(moved + REST_DELAY * 2));
}

#[test]
fn installed_tooltip_answers_queries_only_after_resting() {
    gtk_test(
        "ui::entry_tooltip::tests::installed_tooltip_answers_queries_only_after_resting",
        || {
            let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let state = EntryTooltip::install(&widget, |_, _, _| Some("name".to_owned()));
            let tooltip = glib::Object::new::<gtk::Tooltip>();
            let query = |keyboard: bool| {
                widget.emit_by_name::<bool>("query-tooltip", &[&10i32, &10i32, &keyboard, &tooltip])
            };

            assert!(!query(false));
            state.pointer_moved(10.0, 10.0, Instant::now() - REST_DELAY);
            assert!(query(false));
            assert!(!query(true));
            state.reset();
            assert!(!query(false));
        },
    );
}

#[test]
fn name_text_shows_truncated_names_and_dates() {
    for (truncated, date, expected) in [
        (true, Some("12 Nov 2026"), Some("a.txt\n12 Nov 2026")),
        (false, Some("12 Nov 2026"), Some("a.txt\n12 Nov 2026")),
        (true, None, Some("a.txt")),
        (true, Some(""), Some("a.txt")),
        (false, None, None),
        (false, Some(""), None),
    ] {
        assert_eq!(
            name_text("a.txt", truncated, date).as_deref(),
            expected,
            "truncated={truncated} date={date:?}"
        );
    }
}
