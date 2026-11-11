// SPDX-License-Identifier: MIT

//! The window's history keys, as `keyboard_navigation/alt-up-and-history` drives them.

use super::{
    Seen, assert_browser_agrees, click, fixture, folder_name, host_listed, hosted, open, press,
    rest, wait,
};

#[test]
fn alt_up_and_the_history_keys_move_between_folders_and_leave_the_child_column_closed() {
    hosted(
        "history::alt_up_and_the_history_keys_move_between_folders_and_leave_the_child_column_closed",
        |mut host| {
            let folder = fixture();
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            click(&mut host, &root, "documents");
            let in_documents = |seen: &Seen| {
                host_listed(seen, "documents") && seen.directory.as_deref() == Some("documents")
            };
            wait(&mut host, "documents to open", in_documents);

            // Alt+Up leaves the folder; Alt+Left goes back into it; Alt+Right leaves it again.
            for (key, leaves) in [("alt+Up", true), ("alt+Left", false), ("alt+Right", true)] {
                press(&mut host, key);

                if !leaves {
                    wait(&mut host, key, in_documents);
                    assert_browser_agrees(&host, key);
                    continue;
                }
                let left = wait(&mut host, key, |seen| {
                    seen.directory.as_deref() == Some(root.as_str())
                        && seen.focused.as_deref() == Some("documents")
                });
                assert_eq!(left.panes[0].selected(), ["documents"], "{key}");
                rest(&mut host);
                let rested = super::seen(&host);
                assert_eq!(rested.pane_names(), [root.as_str()], "{key}");
                assert_browser_agrees(&host, key);
            }
        },
    );
}
