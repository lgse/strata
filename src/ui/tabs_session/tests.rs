// SPDX-License-Identifier: MIT

use std::fs;

use super::*;
use crate::model::Location;

fn save_to(path: &Path, tabs: &[Location], active: usize) -> io::Result<()> {
    fs::write(path, session_contents(tabs, active)?)
}

fn session_file() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("session fixture");
    let path = directory.path().join("tabs.toml");
    (directory, path)
}

fn existing_dir(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir_all(&path).expect("session directory fixture");
    path
}

#[test]
fn round_trip_preserves_order_and_active_tab() {
    let (directory, path) = session_file();
    let first = existing_dir(directory.path(), "first");
    let second = existing_dir(directory.path(), "second");
    let tabs = vec![Location::local(&first), Location::local(&second)];

    save_to(&path, &tabs, 1).expect("save session");

    let restored = load_from(&path).expect("restored session");
    assert_eq!(restored.tabs, tabs);
    assert_eq!(restored.active, 1);
}

#[test]
fn save_excludes_credentials_and_transient_locations() {
    let (_directory, path) = session_file();
    let tabs = [
        Location::uri("smb://user:secret@server/share"),
        Location::uri("trash:///deleted"),
        Location::uri("smb://server/share"),
    ];
    save_to(&path, &tabs, 2).expect("save session");
    let contents = fs::read_to_string(&path).expect("saved session");
    assert!(!contents.contains("secret"));
    assert!(!contents.contains("deleted"));
    let restored = load_from(&path).expect("restored session");
    assert_eq!(restored.tabs, vec![tabs[2].clone()]);
    assert_eq!(restored.active, 0);
}

#[test]
fn relative_directories_are_dropped_and_missing_ones_are_left_to_the_window() {
    let (directory, path) = session_file();
    let kept = existing_dir(directory.path(), "kept");
    let gone = directory.path().join("gone");
    save_to(
        &path,
        &[
            Location::local(&gone),
            Location::local(&kept),
            Location::local("relative/path"),
        ],
        0,
    )
    .expect("save session");

    let restored = load_from(&path).expect("restored session");
    assert_eq!(
        restored.tabs,
        vec![Location::local(&gone), Location::local(&kept)]
    );
    assert_eq!(restored.active, 0);
}

#[test]
fn uri_entries_keep_roots_and_reject_secrets_and_transients() {
    let (_directory, path) = session_file();
    let stored = StoredSession {
        version: SESSION_VERSION,
        active: 3,
        tabs: vec![
            StoredTab::from(KIND_URI, "trash:///"),
            StoredTab::from(KIND_URI, "recent:///"),
            StoredTab::from(KIND_URI, "smb://fileserver/share"),
            StoredTab::from(KIND_URI, "smb://user@fileserver/share"),
            StoredTab::from(KIND_URI, "smb://user:secret@fileserver/share"),
            StoredTab::from(KIND_URI, "trash:///some-deleted-file"),
            StoredTab::from(KIND_URI, "recent:///some-recent-file"),
            StoredTab::from(KIND_URI, "gphoto2://camera"),
            StoredTab::from(KIND_URI, "admin:///run/media/user/drive/backups"),
            StoredTab::from(KIND_URI, "https://example.com/share"),
            StoredTab::from("bookmark", "whatever"),
        ],
    };
    fs::write(&path, toml::to_string(&stored).expect("session fixture"))
        .expect("write session fixture");

    let restored = load_from(&path).expect("restored session");
    assert_eq!(
        restored.tabs,
        vec![
            Location::uri("trash:///"),
            Location::uri("recent:///"),
            Location::uri("smb://fileserver/share"),
            Location::uri("smb://user@fileserver/share"),
        ]
    );
    assert_eq!(restored.active, 3);
}

#[test]
fn invalid_tab_before_active_preserves_the_selected_location() {
    let (directory, path) = session_file();
    let first = existing_dir(directory.path(), "first");
    let selected = existing_dir(directory.path(), "selected");
    save_to(
        &path,
        &[Location::local(&first), Location::local(&selected)],
        1,
    )
    .expect("save session");
    let mut stored: StoredSession =
        toml::from_str(&fs::read_to_string(&path).expect("saved fixture"))
            .expect("parse saved fixture");
    stored
        .tabs
        .insert(0, StoredTab::from(KIND_PATH, "definitely/not/here"));
    stored.active = 2;
    fs::write(&path, toml::to_string(&stored).expect("serialize fixture")).expect("write fixture");
    let restored = load_from(&path).expect("restored session");
    assert_eq!(restored.tabs[restored.active], Location::local(selected));
}

#[test]
fn corrupt_missing_and_versioned_out_sessions_restore_nothing() {
    let (_directory, path) = session_file();
    assert!(load_from(&path.join("absent.toml")).is_none());

    fs::write(&path, "active = [not a number]").expect("corrupt fixture");
    assert!(load_from(&path).is_none());

    let stored = StoredSession {
        version: SESSION_VERSION + 1,
        active: 0,
        tabs: vec![StoredTab::from(KIND_URI, "trash:///")],
    };
    fs::write(&path, toml::to_string(&stored).expect("session fixture"))
        .expect("write session fixture");
    assert!(load_from(&path).is_none());
}

#[test]
fn empty_and_fully_invalid_sessions_restore_nothing() {
    let (_directory, path) = session_file();
    let stored = StoredSession {
        version: SESSION_VERSION,
        active: 0,
        tabs: vec![StoredTab::from(KIND_PATH, "definitely/not/here")],
    };
    fs::write(&path, toml::to_string(&stored).expect("session fixture"))
        .expect("write session fixture");
    assert!(load_from(&path).is_none());

    let stored = StoredSession {
        version: SESSION_VERSION,
        active: 0,
        tabs: Vec::new(),
    };
    fs::write(&path, toml::to_string(&stored).expect("session fixture"))
        .expect("write session fixture");
    assert!(load_from(&path).is_none());
}

#[test]
fn active_tab_clamps_to_the_last_restorable_tab() {
    let (directory, path) = session_file();
    let first = existing_dir(directory.path(), "first");
    let second = existing_dir(directory.path(), "second");
    save_to(
        &path,
        &[Location::local(&first), Location::local(&second)],
        99,
    )
    .expect("save session");

    let restored = load_from(&path).expect("restored session");
    assert_eq!(restored.active, 1);
}
