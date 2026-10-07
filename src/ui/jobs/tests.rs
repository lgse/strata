// SPDX-License-Identifier: MIT

use super::*;
#[test]
fn job_output_follows_new_lines_until_scrolled_up() {
    crate::test_support::gtk_test(
        "ui::jobs::tests::job_output_follows_new_lines_until_scrolled_up",
        || {
            let label = gtk::Label::new(Some(&"line\n".repeat(80)));
            let scroll = gtk::ScrolledWindow::builder()
                .child(&label)
                .max_content_height(160)
                .propagate_natural_height(true)
                .build();
            follow_log_output(&scroll);
            let window = gtk::Window::builder().child(&scroll).build();
            window.present();
            let adjustment = scroll.vadjustment();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while adjustment.upper() <= adjustment.page_size() {
                glib::MainContext::default().iteration(false);
                assert!(
                    std::time::Instant::now() < deadline,
                    "output did not become scrollable"
                );
            }
            assert!(at_log_bottom(&adjustment));

            let prior_upper = adjustment.upper();
            label.set_text(&"line\n".repeat(100));
            while adjustment.upper() <= prior_upper && std::time::Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
            }
            assert!(adjustment.upper() > prior_upper, "new output was laid out");
            assert!(at_log_bottom(&adjustment));

            adjustment.set_value(adjustment.lower());
            let prior_upper = adjustment.upper();
            label.set_text(&"line\n".repeat(120));
            while adjustment.upper() <= prior_upper && std::time::Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
            }
            assert!(
                adjustment.upper() > prior_upper,
                "later output was laid out"
            );
            assert!(!at_log_bottom(&adjustment), "scrolling up pauses following");

            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            let prior_upper = adjustment.upper();
            label.set_text(&"line\n".repeat(140));
            while adjustment.upper() <= prior_upper && std::time::Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
            }
            assert!(
                at_log_bottom(&adjustment),
                "returning to the bottom resumes following"
            );
            window.close();
        },
    );
}
