// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsString;

use super::*;
use crate::model::{EntryKind, MetadataValue};

fn entry() -> FileEntry {
    FileEntry {
        location: Location::local("/home/project/child"),
        native_name: OsString::from("child"),
        display_name: "child".into(),
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn stale_requests_are_not_accepted() {
    let peek = PeekState::new(0, Location::local("/home/project"), RequestId(7));

    assert!(peek.accepts(RequestId(7)));
    assert!(!peek.accepts(RequestId(6)));
}

#[test]
fn completion_distinguishes_visible_entries_from_empty_results() {
    let mut populated = PeekState::new(0, Location::local("/home/project"), RequestId(1));
    populated.append(&[entry()]);
    populated.finish();
    assert_eq!(populated.load_state, LoadState::Ready);

    let mut empty = PeekState::new(0, Location::local("/home/empty"), RequestId(2));
    empty.finish();
    assert_eq!(empty.load_state, LoadState::Empty);
}
