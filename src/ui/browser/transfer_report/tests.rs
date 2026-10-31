// SPDX-License-Identifier: MIT

use std::{collections::HashMap, rc::Rc};

use gtk::prelude::*;

use super::*;
use crate::app::BrowserEvent;
use crate::services::{OperationEvent, OperationRequestId, PasteItem, TransferConflict};
use crate::test_support::operations::HeldOperations;
use crate::ui::browser::{BrowserView, PeekBehavior};

fn renamed(count: usize) -> TransferReport {
    TransferReport {
        renamed: (0..count)
            .map(|index| RenamedItem {
                requested: Location::local(format!("/copy/item {index}")),
                saved_as: Location::local(format!("/copy/item {index} (1)")),
                cause: RenameCause::Alias,
            })
            .collect(),
        ..TransferReport::default()
    }
}

#[test]
fn a_name_fixed_for_its_control_characters_shows_where_they_were() {
    let item = RenamedItem {
        requested: Location::local("/stick/line\none\u{1}\u{85}.txt"),
        saved_as: Location::local("/stick/line_one__.txt"),
        cause: RenameCause::Invalid,
    };

    assert_eq!(
        renamed_line(&item),
        rust_i18n::t!(
            "“%{name}” was saved as “%{new_name}”",
            name = "line\u{fffd}one\u{fffd}\u{fffd}.txt",
            new_name = "line_one__.txt"
        )
    );
}

#[test]
fn one_short_renamed_line_gets_a_notice_and_anything_more_a_dialog() {
    assert_eq!(presentation(&TransferReport::default()), None);
    let long = "a name long enough that the notice would cut its line short";
    let long = TransferReport {
        renamed: vec![RenamedItem {
            requested: Location::local(format!("/copy/{long}")),
            saved_as: Location::local(format!("/copy/{long} (1)")),
            cause: RenameCause::Alias,
        }],
        ..TransferReport::default()
    };
    for (report, notice, listed) in [
        (renamed(1), true, 1),
        (long, false, 1),
        (renamed(2), false, 2),
        (renamed(9), false, LISTED_ITEMS),
    ] {
        let count = report.renamed.len();
        let heading = crate::i18n::tr(if count == 1 {
            "The destination can\u{2019}t tell this name apart from another"
        } else {
            "The destination can\u{2019}t tell these names apart"
        });
        match presentation(&report) {
            Some(Presentation::Notice {
                heading: shown,
                text,
            }) => {
                assert!(notice, "{count}");
                assert_eq!(shown, heading);
                assert_eq!(text, renamed_line(&report.renamed[0]));
            }
            Some(Presentation::Information {
                message,
                summary,
                detail,
            }) => {
                assert!(!notice, "{count}");
                assert_eq!(summary, heading, "{count}");
                assert_eq!(message, crate::i18n::count("items_renamed", count));
                assert_eq!(detail.matches('•').count(), listed, "{count}");
                assert_eq!(
                    detail.ends_with(&crate::i18n::count("more_items", count - listed)),
                    count > listed,
                    "{count}"
                );
            }
            other => panic!("{count} renamed items are reported: {other:?}"),
        }
    }
}

#[test]
fn renamed_items_are_grouped_by_why_they_were_renamed() {
    let headings = [
        (
            RenameCause::Invalid,
            "The destination doesn\u{2019}t allow this name",
            "The destination doesn\u{2019}t allow these names",
        ),
        (
            RenameCause::Alias,
            "The destination can\u{2019}t tell this name apart from another",
            "The destination can\u{2019}t tell these names apart",
        ),
        (
            RenameCause::TooLong,
            "This name was too long for the destination",
            "These names were too long for the destination",
        ),
        (
            RenameCause::Taken,
            "This name was already taken",
            "These names were already taken",
        ),
    ];
    let causes = headings.map(|(cause, _, _)| cause);
    let mut report = renamed(causes.len() * 2);
    for (item, cause) in report.renamed.iter_mut().zip(causes.iter().cycle()) {
        item.cause = *cause;
    }

    let Some(Presentation::Information { detail, .. }) = presentation(&report) else {
        panic!("several causes need a dialog");
    };

    let sections = detail
        .split("\n\n")
        .filter(|line| !line.starts_with('•'))
        .collect::<Vec<_>>();
    assert_eq!(
        sections,
        headings.map(|(_, _, several)| crate::i18n::tr(several))
    );
    let distinct = sections.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(distinct.len(), causes.len(), "{sections:?}");
    for (cause, one, _) in headings {
        let only = TransferReport {
            renamed: report
                .renamed
                .iter()
                .filter(|item| item.cause == cause)
                .take(1)
                .cloned()
                .collect(),
            ..TransferReport::default()
        };
        let Some(Presentation::Notice { heading: shown, .. }) = presentation(&only) else {
            panic!("one renamed item gets a notice");
        };
        assert_eq!(shown, crate::i18n::tr(one), "{cause:?}");
    }
}

#[test]
fn items_left_out_or_left_behind_make_the_report_a_warning_or_follow_a_stop() {
    let skipped = |kind| SkippedItem {
        location: Location::local("/copy/run/socket"),
        kind,
    };
    let kept = |cause| KeptItem {
        location: Location::local("/source/run/data"),
        cause,
    };
    let in_trash = |cause| KeptItem {
        location: Location::local("/trash/files/run/data"),
        cause,
    };
    let denied = rustix::io::Errno::ACCESS.raw_os_error();
    let reason = crate::services::io_error_detail(&std::io::Error::from_raw_os_error(denied));
    let left_behind = [
        kept(KeptCause::Changed),
        kept(KeptCause::OriginalNotRemoved(reason.clone())),
        kept(KeptCause::Skipped(SpecialKind::Socket)),
        kept(KeptCause::CopyGone),
        kept(KeptCause::NameNotRestored {
            saved_as: Location::local("/source/run/data_1"),
            error: reason.clone(),
        }),
        kept(KeptCause::StillOpen(Some(Holder::Program(
            "Text Editor".to_owned(),
        )))),
        kept(KeptCause::StillOpen(Some(Holder::Strata))),
        kept(KeptCause::StillOpen(None)),
    ];
    let left_in_trash = [
        in_trash(KeptCause::Changed),
        in_trash(KeptCause::Undeletable(denied)),
        in_trash(KeptCause::ReplacementChanged),
    ];
    for (renamed_count, skipped_count, kept_count, in_trash_count) in [
        (0, 2, 0, 0),
        (1, 2, 0, 0),
        (3, 2, 0, 0),
        (0, 0, 8, 0),
        (1, 2, 8, 0),
        (0, 0, 0, 3),
    ] {
        let row = format!("{renamed_count}, {skipped_count}, {kept_count}, {in_trash_count}");
        let report = TransferReport {
            skipped: [skipped(SpecialKind::Socket), skipped(SpecialKind::Pipe)][..skipped_count]
                .to_vec(),
            kept: left_behind[..kept_count].to_vec(),
            kept_in_trash: left_in_trash[..in_trash_count].to_vec(),
            ..renamed(renamed_count)
        };
        let Some(Presentation::Warning { detail }) = presentation(&report) else {
            panic!("{row}");
        };
        let outcome = "Could not copy keys: Permission denied";
        assert_eq!(
            outcome_detail(outcome.to_owned(), &report),
            format!("{outcome}\n\n{detail}"),
            "{row}"
        );
        assert_eq!(
            detail.matches('•').count(),
            renamed_count + skipped_count + kept_count + in_trash_count,
            "{row}"
        );
        let heading = crate::i18n::count("items_kept", kept_count);
        assert_eq!(detail.contains(&heading), kept_count > 0, "{row}");
        let changed = rust_i18n::t!(
            "“%{name}” changed during the move and was left in “%{folder}”",
            name = "data",
            folder = "run"
        );
        assert_eq!(detail.contains(&*changed), kept_count > 0, "{row}");
        let gone = rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as its copy was no longer where the move put it",
            name = "data",
            folder = "run"
        );
        assert_eq!(detail.contains(&*gone), kept_count > 0, "{row}");
        let unrestored = rust_i18n::t!(
            "“%{new_name}” could not be renamed back to “%{name}”: %{error}",
            new_name = "data_1",
            name = "data",
            error = reason
        );
        assert_eq!(detail.contains(&*unrestored), kept_count > 0, "{row}");
        let original = rust_i18n::t!(
            "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
            name = "data",
            kept = "run",
            error = reason
        );
        assert_eq!(detail.contains(&*original), kept_count > 0, "{row}");
        let still_open = [
            rust_i18n::t!(
                "“%{name}” was left in “%{folder}”, as a file in it is still open in %{program}",
                name = "data",
                folder = "run",
                program = "Text Editor"
            ),
            rust_i18n::t!(
                "“%{name}” was left in “%{folder}”, as a file in it is still in use by Strata",
                name = "data",
                folder = "run"
            ),
            rust_i18n::t!(
                "“%{name}” was left in “%{folder}”, as a file in it is still open in another program",
                name = "data",
                folder = "run"
            ),
        ];
        for line in still_open {
            assert_eq!(detail.contains(&*line), kept_count > 0, "{row}: {line}");
        }
        let heading = crate::i18n::count("items_kept_in_trash", in_trash_count);
        assert_eq!(detail.contains(&heading), in_trash_count > 0, "{row}");
        let changed = rust_i18n::t!(
            "“%{name}” changed during the restore and was left in the Trash",
            name = "data"
        );
        assert_eq!(detail.contains(&*changed), in_trash_count > 0, "{row}");
        let replacement = rust_i18n::t!(
            "“%{name}” was left in the Trash, as what replaced it may have changed since",
            name = "data"
        );
        assert_eq!(detail.contains(&*replacement), in_trash_count > 0, "{row}");
        let undeletable = rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = "data",
            error = reason
        );
        assert_eq!(
            detail.matches(&*undeletable).count(),
            usize::from(in_trash_count > 0),
            "{row}"
        );
        assert_eq!(
            detail.contains(&reason),
            kept_count + in_trash_count > 0,
            "{row}"
        );
    }
}

#[test]
fn links_left_out_get_their_own_section_and_undo_says_what_it_leaves_out() {
    let skipped = |kind| SkippedItem {
        location: Location::local("/copy/env/bin/python"),
        kind,
    };
    for (count, special, links) in [
        (
            1,
            "This special file can\u{2019}t be copied and was left out",
            "This symbolic link can\u{2019}t be stored at the destination and was left out",
        ),
        (
            2,
            "These special files can\u{2019}t be copied and were left out",
            "These symbolic links can\u{2019}t be stored at the destination and were left out",
        ),
    ] {
        let report = TransferReport {
            skipped: [SpecialKind::Socket, SpecialKind::Symlink]
                .into_iter()
                .flat_map(|kind| std::iter::repeat_n(skipped(kind), count))
                .collect(),
            ..TransferReport::default()
        };
        let Some(Presentation::Warning { detail }) = presentation(&report) else {
            panic!("anything left out makes a warning");
        };
        let sections = detail
            .split("\n\n")
            .filter(|line| !line.starts_with('•'))
            .collect::<Vec<_>>();
        assert_eq!(
            sections,
            [crate::i18n::tr(special), crate::i18n::tr(links)],
            "{count}"
        );
        assert_eq!(detail.matches('•').count(), 2 * count, "{count}");
    }

    let kept = KeptItem {
        location: Location::local("/source/env/bin/python"),
        cause: KeptCause::Skipped(SpecialKind::Symlink),
    };
    for (in_trash, left_out, noted) in [
        (false, 0, false),
        (false, 1, true),
        (false, LISTED_ITEMS + 2, true),
        (true, 1, false),
    ] {
        let row = format!("in Trash: {in_trash}, kept out of undo: {left_out}");
        let note = crate::i18n::tr(if left_out == 1 {
            "Undo can\u{2019}t move this item back, as part of it stayed in its original folder."
        } else {
            "Undo can\u{2019}t move these items back, as part of each stayed in its original folder."
        });
        let mut report = TransferReport {
            kept_out_of_undo: (0..left_out)
                .map(|index| Location::local(format!("/source/project {index}")))
                .collect(),
            ..TransferReport::default()
        };
        if in_trash {
            report.kept_in_trash.push(kept.clone());
        } else {
            report.kept.push(kept.clone());
        }
        let Some(Presentation::Warning { detail }) = presentation(&report) else {
            panic!("{row}");
        };
        let (kept_lines, after) = detail.split_once(&note).unwrap_or((&detail, ""));
        assert_eq!(after.is_empty(), !noted, "{row}: {detail}");
        assert_eq!(kept_lines.matches('•').count(), 1, "{row}");
        let names = after
            .split("\n\n")
            .filter_map(|line| line.strip_prefix("• "))
            .collect::<Vec<_>>();
        let listed = (0..left_out.min(LISTED_ITEMS))
            .filter(|_| noted)
            .map(|index| crate::i18n::quoted(&format!("project {index}")))
            .collect::<Vec<_>>();
        assert_eq!(names, listed, "{row}");
        assert_eq!(
            after.ends_with(&crate::i18n::count("more_items", 2)),
            left_out > LISTED_ITEMS,
            "{row}"
        );
    }
}

#[test]
fn a_stop_lists_renamed_items_by_cause_under_its_outcome() {
    let outcome = "Could not copy keys: Permission denied";
    let mut report = renamed(LISTED_ITEMS + 2);
    report.renamed[0].cause = RenameCause::Taken;

    let detail = outcome_detail(outcome.to_owned(), &report);

    assert!(detail.starts_with(&format!("{outcome}\n\n")), "{detail}");
    for heading in [
        "The destination can\u{2019}t tell these names apart",
        "This name was already taken",
    ] {
        let heading = crate::i18n::tr(heading);
        assert_eq!(detail.matches(&heading).count(), 1, "{heading}");
    }
    assert!(detail.contains(&renamed_line(&report.renamed[0])));
    assert_eq!(detail.matches('•').count(), 1 + LISTED_ITEMS);
    assert!(detail.ends_with(&crate::i18n::count("more_items", 1)));
}

fn shows(overlay: &gtk::Overlay, class: &str) -> bool {
    shows_widget(overlay, |widget| widget.has_css_class(class))
}

fn shows_text(overlay: &gtk::Overlay, text: &str) -> bool {
    shows_widget(overlay, |widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == text)
    })
}

fn shows_widget(overlay: &gtk::Overlay, found: impl Fn(&gtk::Widget) -> bool) -> bool {
    let mut stack = vec![overlay.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if found(&widget) {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    false
}

#[test]
fn a_report_after_a_transfer_shows_its_notice_or_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::transfer_report::tests::a_report_after_a_transfer_shows_its_notice_or_dialog",
        || {
            let left_out = TransferReport {
                skipped: vec![SkippedItem {
                    location: Location::local("/copy/run/socket"),
                    kind: SpecialKind::Socket,
                }],
                ..TransferReport::default()
            };
            for (report, background, dialog) in [
                (renamed(1), false, false),
                (renamed(3), false, true),
                (renamed(1), true, false),
                (renamed(3), true, true),
                (left_out, false, true),
            ] {
                let row = format!("{report:?}, background: {background}");
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                let overlay = view.overlay();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                // A Send to notice already on screen must survive the report.
                let sent = "Copied to USB";
                crate::ui::notice::show_notice(&overlay, None, sent, NOTICE_DURATION);
                let event = BrowserEvent::ItemsReported { report };
                let event = if background {
                    BrowserEvent::BackgroundOperation {
                        request_id: OperationRequestId(1),
                        event: Box::new(event),
                    }
                } else {
                    event
                };

                view.state.handle(&event);

                let context = gtk::glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }
                let report_notice = shows_text(&overlay, &renamed_line(&renamed(1).renamed[0]));
                assert_eq!(report_notice, !dialog, "{row}");
                assert_eq!(shows(&overlay, "app-modal-layer"), dialog, "{row}");
                assert!(shows_text(&overlay, sent), "{row}");
                window.destroy();
            }
        },
    );
}

fn modal_layers(window: &gtk::Window) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![window.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("app-modal-layer") {
            found.push(widget);
            continue;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    found
}

fn dialogs(window: &gtk::Window) -> Vec<Vec<String>> {
    modal_layers(window)
        .iter()
        .filter(|layer| !crate::ui::modal::is_closing_layer(layer))
        .map(texts)
        .collect()
}

fn texts(root: &gtk::Widget) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            found.push(label.label().to_string());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    found
}

fn pump_until(done: impl Fn() -> bool) {
    let context = gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if done() {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Started {
    Move,
    Copy,
    Restore,
    MergeUndo,
    Extract,
}

fn start(view: &BrowserView, operations: &HeldOperations, started: Started) -> OperationRequestId {
    let browser = view.browser();
    match started {
        Started::Move | Started::Copy => browser.transfer(
            Location::local("/destination"),
            vec![PasteItem {
                source: Location::local("/source/run"),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            }],
            started == Started::Move,
            false,
        ),
        Started::Restore => browser.restore(vec![crate::services::RestoreTrashItem {
            entry: crate::test_support::operations::entry(Location::local("/trash/files/run")),
            destination: "/source/run".into(),
        }]),
        Started::MergeUndo => {
            let copy = start(view, operations, Started::Copy);
            operations.emit(
                copy,
                OperationEvent::Merged {
                    request_id: copy,
                    source: Location::local("/source/run"),
                    created: vec![Location::local("/destination/run/app.conf")],
                    overwritten: Vec::new(),
                    placed: HashMap::new(),
                },
            );
            operations.emit(
                copy,
                OperationEvent::Pasted {
                    request_id: copy,
                    locations: Vec::new(),
                },
            );
            let (generation, created, overwritten) = browser
                .pending_undo_merge()
                .expect("the merged copy's undo");
            assert!(browser.undo_merge(generation, created, overwritten));
        }
        Started::Extract => {
            browser.extract(
                crate::test_support::operations::entry(Location::local("/source/run.tar")),
                Location::local("/destination"),
                false,
                None,
            );
            let id = browser.last_started_operation().expect("the extraction");
            operations.emit(
                id,
                OperationEvent::ArchiveStarted {
                    request_id: id,
                    total: 0,
                },
            );
        }
    }
    browser.last_started_operation().expect("an operation")
}

#[test]
fn an_outcome_shows_one_dialog_with_the_report_of_its_operation() {
    crate::test_support::gtk_test(
        "ui::browser::transfer_report::tests::an_outcome_shows_one_dialog_with_the_report_of_its_operation",
        || {
            super::super::progress::set_file_progress_delay_for_test(Duration::ZERO);
            let items = TransferReport {
                skipped: vec![SkippedItem {
                    location: Location::local("/destination/run/preview.sock"),
                    kind: SpecialKind::Socket,
                }],
                kept: vec![KeptItem {
                    location: Location::local("/source/run/app.sock"),
                    cause: KeptCause::Skipped(SpecialKind::Socket),
                }],
                kept_in_trash: vec![KeptItem {
                    location: Location::local("/trash/files/run/app.conf"),
                    cause: KeptCause::Changed,
                }],
                ..renamed(1)
            };
            let lines = [
                renamed_line(&items.renamed[0]),
                skipped_line(&items.skipped[0]),
                kept_line(&items.kept[0], false),
                kept_line(&items.kept_in_trash[0], true),
            ];
            let failure = "Could not copy keys: Permission denied";
            let errors = "Could not restore keys: Permission denied";
            let cancelled = crate::services::CancelledOperation {
                completed: vec![Location::local("/source/run")],
                not_attempted: vec![Location::local("/source/keys")],
                ..Default::default()
            };
            let summary = super::super::progress::cancelled_operation_summary(1, 0, 1);
            type Terminal = Box<dyn Fn(OperationRequestId) -> OperationEvent>;
            let cancellation = cancelled.clone();
            let extraction_endings: [(Terminal, &str, Option<&str>); 3] = [
                (
                    Box::new(|request_id| OperationEvent::Failed {
                        request_id,
                        message: failure.into(),
                        password_failure: None,
                    }),
                    "Unable to complete operation",
                    Some(failure),
                ),
                (
                    Box::new(move |request_id| OperationEvent::Cancelled {
                        request_id,
                        result: cancellation.clone(),
                    }),
                    "Operation cancelled",
                    Some(&summary),
                ),
                (
                    Box::new(|request_id| OperationEvent::Extracted {
                        request_id,
                        first_name: None,
                        created: Vec::new(),
                    }),
                    "Completed with errors",
                    None,
                ),
            ];
            let transfer_endings: [(Terminal, &str, Option<&str>); 3] = [
                (
                    Box::new(|request_id| OperationEvent::TransferFailed {
                        request_id,
                        completed_locations: Vec::new(),
                        message: failure.into(),
                    }),
                    "Unable to complete operation",
                    Some(failure),
                ),
                (
                    Box::new(move |request_id| OperationEvent::Cancelled {
                        request_id,
                        result: cancelled.clone(),
                    }),
                    "Operation cancelled",
                    Some(&summary),
                ),
                (
                    Box::new(|request_id| OperationEvent::Pasted {
                        request_id,
                        locations: Vec::new(),
                    }),
                    "Completed with errors",
                    None,
                ),
            ];
            let restored_with_errors: Terminal =
                Box::new(|request_id| OperationEvent::RestoreCompletedWithErrors {
                    request_id,
                    restored_locations: Vec::new(),
                    restored: Vec::new(),
                    message: errors.into(),
                });
            let completed_with_errors: Terminal =
                Box::new(|request_id| OperationEvent::CompletedWithErrors {
                    request_id,
                    deleted_locations: Vec::new(),
                    retryable_locations: Vec::new(),
                    has_non_retryable_failures: true,
                    message: errors.into(),
                });
            // A move, a restore, a merge undo and an extraction run in the
            // foreground and a copy in the background.
            let mut rows = Vec::new();
            for started in [Started::Move, Started::Copy] {
                for (terminal, title, outcome) in &transfer_endings {
                    rows.push((started, terminal, *title, *outcome));
                }
            }
            for (terminal, title, outcome) in &extraction_endings {
                rows.push((Started::Extract, terminal, *title, *outcome));
            }
            rows.extend([
                (
                    Started::Restore,
                    &restored_with_errors,
                    "Completed with errors",
                    Some(errors),
                ),
                (
                    Started::MergeUndo,
                    &completed_with_errors,
                    "Completed with errors",
                    Some(errors),
                ),
                (
                    Started::Copy,
                    &completed_with_errors,
                    "Completed with errors",
                    Some(errors),
                ),
            ]);
            for (started, terminal, title, outcome) in rows {
                for report in [items.clone(), TransferReport::default()] {
                    let row = format!(
                        "{started:?}, {title}, {outcome:?}, items: {}",
                        !report.is_empty()
                    );
                    let view = BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        PeekBehavior::default(),
                    );
                    let operations = Rc::new(HeldOperations::default());
                    view.browser().set_operation_provider(operations.clone());
                    let window = gtk::Window::builder().child(&view.overlay()).build();
                    window.present();
                    let id = start(&view, &operations, started);
                    let docked = view
                        .state
                        .background_file_progress
                        .borrow()
                        .get(&id)
                        .map(|background| background.progress.clone());
                    assert_eq!(docked.is_some(), started == Started::Copy, "{row}");
                    let progress = docked.unwrap_or_else(|| view.state.file_progress());
                    pump_until(|| progress.file_progress_view.borrow().is_some());
                    if !report.is_empty() {
                        operations.emit(
                            id,
                            OperationEvent::ItemsReported {
                                request_id: id,
                                report: report.clone(),
                            },
                        );
                    }

                    operations.emit(id, terminal(id));

                    pump_until(|| {
                        progress.file_progress_view.borrow().is_none()
                            && !modal_layers(&window)
                                .iter()
                                .any(crate::ui::modal::is_closing_layer)
                    });
                    let expected = outcome.is_some() || !report.is_empty();
                    let dialogs = dialogs(&window);
                    assert_eq!(dialogs.len(), usize::from(expected), "{row}: {dialogs:?}");
                    if let [texts] = dialogs.as_slice() {
                        let title = crate::i18n::tr(title);
                        assert!(texts.contains(&title), "{row}: {texts:?}");
                        let detail = match outcome {
                            Some(outcome) => texts
                                .iter()
                                .find(|text| text.starts_with(outcome))
                                .expect("the outcome"),
                            None => texts
                                .iter()
                                .find(|text| text.contains(lines[0].as_str()))
                                .expect("the report"),
                        };
                        if report.is_empty() {
                            assert_eq!(Some(detail.as_str()), outcome, "{row}");
                        }
                        for line in &lines {
                            assert_eq!(
                                detail.contains(line.as_str()),
                                !report.is_empty(),
                                "{row}: {detail}"
                            );
                        }
                    }
                    view.browser().clear_observer();
                    view.dispose_file_progress();
                    window.destroy();
                }
            }
        },
    );
}
