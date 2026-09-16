// SPDX-License-Identifier: MIT

use super::restore::{button, find_widget, view, wait_until, window};
use super::*;
use crate::{
    services::{CrossVolumeDropStrategy, DropCommit, TransferKind, VolumeRelation},
    ui::browser_modes::BrowserMode,
};
use std::{fs, path::Path};

fn dispatch_drop(view: &BrowserView, destination: &Path, source: &Path) -> DropCommit {
    let destination = Location::local(destination);
    let sources = vec![Location::local(source)];
    let prepared = crate::ui::browser::clipboard::prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    let commit = crate::ui::browser::clipboard::file_drop_commit(
        &prepared.target,
        &destination,
        &sources,
        &prepared.state,
    );
    view.state.commit_file_drop(destination, sources, commit);
    commit
}

#[test]
fn mixed_drop_transfers_valid_items_without_touching_noops() {
    crate::test_support::gtk_test(
        "ui::browser::tests::drops::mixed_drop_transfers_valid_items_without_touching_noops",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let dest = fixture.path().join("a");
            let from = fixture.path().join("b");
            fs::create_dir(&dest).expect("a");
            fs::create_dir(&from).expect("b");
            fs::write(dest.join("stay"), b"stay").expect("no-op");
            fs::write(from.join("move"), b"move").expect("transfer source");
            let view = view();
            let window = window(&view);
            view.state.commit_file_drop(
                Location::local(&dest),
                vec![
                    Location::local(dest.join("stay")),
                    Location::local(&dest),
                    Location::local(from.join("move")),
                ],
                DropCommit::Move,
            );
            wait_until(|| dest.join("move").exists() && !from.join("move").exists());
            assert_eq!(fs::read(dest.join("stay")).expect("unchanged"), b"stay");
            assert_eq!(fs::read(dest.join("move")).expect("moved"), b"move");
            assert!(button(&window.clone().upcast(), "Replace").is_none());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn saved_and_live_drop_strategy_dispatches_in_two_views_and_after_a_rebuild() {
    crate::test_support::gtk_test(
        "ui::browser::tests::drops::saved_and_live_drop_strategy_dispatches_in_two_views_and_after_a_rebuild",
        || {
            crate::ui::theme::ThemeManager::seed_saved_preferences_for_test();
            let manager = crate::ui::theme::ThemeManager::shared();
            let Some((home, stick)) = crate::test_support::distinct_device_dirs(
                "saved_and_live_drop_strategy_dispatches_in_two_views_and_after_a_rebuild",
            ) else {
                return;
            };
            let destinations = [stick.path().join("first"), stick.path().join("second")];
            for destination in &destinations {
                fs::create_dir(destination).expect("cross-device destination");
            }
            let views = [view(), view()];
            let windows = [window(&views[0]), window(&views[1])];

            for (index, view) in views.iter().enumerate() {
                let source = home.path().join(format!("saved-{index}"));
                fs::write(&source, b"saved move").expect("saved source");
                assert_eq!(
                    dispatch_drop(view, &destinations[index], &source),
                    DropCommit::Move
                );
                wait_until(|| {
                    destinations[index].join(format!("saved-{index}")).exists() && !source.exists()
                });
            }

            manager.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy);
            for (index, view) in views.iter().enumerate() {
                let source = home.path().join(format!("live-{index}"));
                fs::write(&source, b"live copy").expect("live source");
                assert_eq!(
                    dispatch_drop(view, &destinations[index], &source),
                    DropCommit::Copy
                );
                wait_until(|| destinations[index].join(format!("live-{index}")).exists());
                assert!(source.exists());
            }

            views[0].set_view_mode(BrowserMode::Icons);
            assert_eq!(views[0].view_mode(), BrowserMode::Icons);
            manager.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Move);
            let rebuilt_source = home.path().join("rebuilt");
            fs::write(&rebuilt_source, b"rebuilt move").expect("rebuilt source");
            assert_eq!(
                dispatch_drop(&views[0], &destinations[0], &rebuilt_source),
                DropCommit::Move
            );
            wait_until(|| destinations[0].join("rebuilt").exists() && !rebuilt_source.exists());

            for window in windows {
                window.destroy();
            }
            for view in views {
                view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn ask_without_a_window_host_reports_error_without_transferring() {
    crate::test_support::gtk_test(
        "ui::browser::tests::drops::ask_without_a_window_host_reports_error_without_transferring",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let dest = fixture.path().join("destination");
            fs::create_dir(&dest).expect("destination");
            let source = fixture.path().join("source");
            fs::write(&source, b"original").expect("source");
            let view = view();
            view.state.commit_file_drop(
                Location::local(&dest),
                vec![Location::local(&source)],
                DropCommit::Ask {
                    default: TransferKind::Copy,
                    volume: VolumeRelation::Unknown,
                },
            );
            assert!(
                find_widget(&view.widget(), &|label: &gtk::Label| label.text()
                    == "Unable to transfer")
                .is_some()
            );
            assert!(source.exists());
            assert!(!dest.join("source").exists());
            view.browser().clear_observer();
        },
    );
}
