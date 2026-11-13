// SPDX-License-Identifier: MIT

//! The rows' thumbnails: a store viewer for each open column, and none left behind.

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
