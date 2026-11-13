// SPDX-License-Identifier: MIT

//! The rows' thumbnails: a store viewer for each open column, and none left behind.

use strata_components::{contract::EntrySource, thumbnails::State};

use super::{Seen, click, fixture, folder_name, host_listed, hosted, open, press, rest, wait};

/// Opening a folder adds a column and its viewer; closing it drops the viewer again, however
/// often the two alternate.
#[test]
fn a_closed_column_leaves_no_thumbnail_viewer_behind() {
    hosted(
        "thumbs::a_closed_column_leaves_no_thumbnail_viewer_behind",
        |mut host| {
            let folder = fixture();
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            assert_eq!(host.app().columns.thumbs.viewers(), 1, "the first column");
            for round in 0..10 {
                click(&mut host, &root, "documents");
                wait(&mut host, "documents to open", |seen: &Seen| {
                    host_listed(seen, "documents")
                });
                rest(&mut host);
                assert_eq!(
                    host.app().columns.thumbs.viewers(),
                    2,
                    "round {round}, opened"
                );

                press(&mut host, "alt+Up");
                wait(&mut host, "the column to close", |seen: &Seen| {
                    seen.directory.as_deref() == Some(root.as_str())
                });
                rest(&mut host);
                assert_eq!(super::seen(&host).panes.len(), 1, "round {round}");
                assert_eq!(
                    host.app().columns.thumbs.viewers(),
                    1,
                    "round {round}, closed"
                );
            }
        },
    );
}

/// A sibling opened at the same depth is another folder: the column's viewer asks for its
/// rows' thumbnails, not the last folder's.
#[test]
fn a_sibling_opened_in_place_has_its_thumbnails_asked_for() {
    hosted(
        "thumbs::a_sibling_opened_in_place_has_its_thumbnails_asked_for",
        |mut host| {
            let folder = tempfile::tempdir().expect("folder");
            for (dir, file) in [("a", "one.png"), ("b", "two.png")] {
                std::fs::create_dir(folder.path().join(dir)).expect("folder");
                std::fs::write(folder.path().join(dir).join(file), file).expect("file");
            }
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            for (dir, file) in [("a", "one.png"), ("b", "two.png")] {
                click(&mut host, &root, dir);
                wait(&mut host, "the folder to open", |seen: &Seen| {
                    seen.pane(dir).is_some_and(|pane| pane.names() == [file])
                });
                rest(&mut host);
                let app = host.app();
                let column = &app.columns.listings.columns[1];
                let key = column.entry(0).thumbnail.expect("a thumbnail key");
                let state = crate::shell::thumbnails(app).state(key);
                assert_ne!(state, State::Empty, "{dir}'s {file} was never asked for");
            }
        },
    );
}
