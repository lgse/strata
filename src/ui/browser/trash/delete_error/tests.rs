// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use gtk::prelude::*;

use super::*;
use crate::{
    model::Location,
    ui::browser::trash::tests::{button, view, wait_until, window},
};

#[test]
fn show_in_folder_names_one_of_several_blockers_and_selects_it() {
    crate::test_support::gtk_test(
        "ui::browser::trash::delete_error::tests::show_in_folder_names_one_of_several_blockers_and_selects_it",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let root = fixture.path().join("to-delete");
            let nested = root.join("nested");
            std::fs::create_dir_all(&nested).expect("folder");
            std::fs::write(nested.join("blocked.txt"), b"keep").expect("blocker");
            let view = view();
            let window = window(&view);
            for (one_of_several, label) in [
                (false, "Show in Folder"),
                (true, "Show “blocked.txt” in Folder"),
            ] {
                view.browser().navigate(Location::local(fixture.path()));
                view.state.show_partial_operation_failure(
                    "to-delete/nested/blocked.txt: Permission denied",
                    Some(DeleteBlocker {
                        root: root.clone(),
                        child: PathBuf::from("nested/blocked.txt"),
                        one_of_several,
                    }),
                    DeleteErrorRetry {
                        entries: Vec::new(),
                        require_choice: false,
                        run: ViewState::show_trash_unavailable_confirmation,
                    },
                );
                wait_until(|| button(window.upcast_ref(), "Close").is_some());
                button(window.upcast_ref(), label)
                    .unwrap_or_else(|| panic!("{label}"))
                    .emit_clicked();
                wait_until(|| {
                    view.browser().focused_item().is_some_and(|(_, _, entry)| {
                        entry.location == Location::local(nested.join("blocked.txt"))
                    })
                });
                assert_eq!(
                    view.browser().active_location(),
                    Some(Location::local(&nested))
                );
            }
            window.close();
        },
    );
}
