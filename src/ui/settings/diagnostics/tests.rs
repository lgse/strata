// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use super::*;
use crate::services::package_manager::PackageManager::{Apt, Dnf, Pacman, Zypper};

const BUBBLEWRAP: diagnostics::Packages = &[
    (Pacman, "bubblewrap"),
    (Apt, "bubblewrap"),
    (Dnf, "bubblewrap"),
    (Zypper, "bubblewrap"),
];

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        widgets.extend(descendants(&current));
    }
    widgets
}

fn labels(widget: &gtk::Widget) -> Vec<gtk::Label> {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .collect()
}

fn button(page: &gtk::Widget, name: &str) -> gtk::Button {
    descendants(page)
        .into_iter()
        .find(|widget| widget.widget_name() == name)
        .and_downcast()
        .expect("diagnostics button")
}

fn rows(page: &gtk::Widget) -> Vec<gtk::Widget> {
    descendants(page)
        .into_iter()
        .filter(|widget| widget.has_css_class("diagnostics-row"))
        .collect()
}

/// The message, visible detail, and whether a fix shows for the row titled `id`.
fn row_text(page: &gtk::Widget, id: CheckId) -> (String, Option<String>, bool) {
    let title = crate::i18n::tr(title(id));
    let row = rows(page)
        .into_iter()
        .find(|row| labels(row).iter().any(|label| label.text() == title))
        .unwrap_or_else(|| panic!("row for {id:?}"));
    let row_labels = labels(&row);
    let message = row_labels
        .iter()
        .find(|label| label.has_css_class("diagnostics-message"))
        .expect("message")
        .text()
        .to_string();
    let detail = row_labels
        .iter()
        .find(|label| label.has_css_class("diagnostics-detail") && label.is_visible())
        .map(|label| label.text().to_string());
    let fix = descendants(&row)
        .iter()
        .any(|widget| widget.has_css_class("diagnostics-fix") && widget.is_visible());
    (message, detail, fix)
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn run_to_completion(page: &gtk::Widget) {
    let run = button(page, "diagnostics-run");
    run.emit_clicked();
    assert!(!run.is_sensitive(), "a run in progress cannot be restarted");
    wait_until("diagnostics to finish", || run.is_sensitive());
}

#[test]
fn a_run_fills_every_row_and_marks_checks_that_never_reported() {
    crate::test_support::gtk_test(
        "ui::settings::diagnostics::tests::a_run_fills_every_row_and_marks_checks_that_never_reported",
        || {
            let first_run = Cell::new(true);
            let page = diagnostics_page_with(Rc::new(move |sender: Sender<Check>| {
                if !first_run.replace(false) {
                    return;
                }
                let sandbox = Check {
                    id: CheckId::Sandbox,
                    status: Status::Problem,
                    finding: Finding::BubblewrapMissing,
                    packages: BUBBLEWRAP,
                    detail: Some("bwrap: not found".to_owned()),
                };
                let gvfs = Check {
                    id: CheckId::Gvfs,
                    status: Status::Passed,
                    finding: Finding::GvfsRunning,
                    packages: BUBBLEWRAP,
                    detail: None,
                };
                // Workers report from another thread after the page starts polling.
                std::thread::spawn(move || {
                    for check in [sandbox, gvfs] {
                        std::thread::sleep(Duration::from_millis(20));
                        sender.send(check).expect("page is polling");
                    }
                });
            }));
            let copy = button(&page, "diagnostics-copy");
            assert!(!copy.is_sensitive(), "nothing to copy before a run");
            assert!(rows(&page).is_empty());

            run_to_completion(&page);
            let version = crate::build_info::displayed_version();
            assert!(
                descendants(&page)
                    .iter()
                    .filter(|widget| widget.has_css_class("about-detail-row"))
                    .flat_map(labels)
                    .any(|label| label.text() == version),
                "the system box shows the version"
            );
            let total = GROUPS.iter().map(|(_, ids)| ids.len()).sum::<usize>();
            assert_eq!(rows(&page).len(), total);
            let (message, detail, fix) = row_text(&page, CheckId::Sandbox);
            assert_eq!(message, message_for(Finding::BubblewrapMissing));
            assert_eq!(detail.as_deref(), Some("bwrap: not found"));
            assert!(fix, "a failed check offers its package");
            let (message, _, fix) = row_text(&page, CheckId::Gvfs);
            assert_eq!(message, crate::i18n::tr("GVfs is running."));
            assert!(!fix, "passed checks offer no install hint");
            assert_eq!(
                row_text(&page, CheckId::Workers).0,
                crate::i18n::tr("This check couldn't run.")
            );
            let pending = crate::i18n::tr("Checking…");
            for id in [
                CheckId::SyntaxHighlighting,
                CheckId::Terminal,
                CheckId::CustomActions,
            ] {
                assert_ne!(row_text(&page, id).0, pending, "{id:?}");
            }
            assert!(
                rows(&page)
                    .iter()
                    .flat_map(labels)
                    .all(|label| label.text() != pending)
            );
            assert!(copy.is_sensitive());

            copy.emit_clicked();
            let report = glib::MainContext::default()
                .block_on(copy.clipboard().read_text_future())
                .expect("copied report")
                .expect("report text")
                .to_string();
            for expected in [
                "Strata diagnostics",
                &format!("Version: {}", crate::build_info::displayed_version()),
                "PREVIEW SANDBOX",
                "[Problem] Preview sandbox: Bubblewrap (bwrap) isn't installed",
                "    bwrap: not found",
                "[Passed] GVfs: GVfs is running.",
                "[Warning] Sandboxed workers: This check couldn't run.",
            ] {
                assert!(report.contains(expected), "{expected:?} in\n{report}");
            }
            assert_eq!(copy.label().as_deref(), Some("Copied"));

            run_to_completion(&page);
            assert_eq!(
                rows(&page).len(),
                total,
                "a rerun replaces the previous rows"
            );
            assert_eq!(
                row_text(&page, CheckId::Sandbox).0,
                crate::i18n::tr("This check couldn't run.")
            );
        },
    );
}

fn message_for(finding: Finding) -> String {
    message(&Check {
        id: CheckId::Sandbox,
        status: Status::Problem,
        finding,
        packages: &[],
        detail: None,
    })
}
