// SPDX-License-Identifier: MIT

use super::restore::{button, find_widget, view, wait_until, window};
use super::*;
use crate::services::{DropCommit, TransferKind, VolumeRelation};
use std::fs;

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
