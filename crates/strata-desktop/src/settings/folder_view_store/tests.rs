// SPDX-License-Identifier: MIT

use std::{
    future::Future,
    io,
    pin::pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use super::*;
use crate::settings::{
    folder_view_file::tests::{file, folder, isolated, saved, views, write_file},
    lock_saves,
};

const MODULE: &str = "settings::folder_view_store::tests";

fn own_process(name: &str, file: Option<&str>, run: impl FnOnce()) {
    isolated(&format!("{MODULE}::{name}"), file, run);
}

/// What a future has once the writer has settled, polled by hand: taking a write's result
/// needs no executor and no dispatch. (`writer::settle` itself iterates GLib's context.)
fn settled<T>(pending: impl Future<Output = T>) -> T {
    writer::settle();
    let mut pending = pin!(pending);
    match pending
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(ready) => ready,
        Poll::Pending => panic!("the writer settled and the future has no result"),
    }
}

fn written(store: &FolderViewStore, exit: bool) -> Written {
    let Ok(write) = store.begin_write(exit) else {
        panic!("saving is off");
    };
    settled(write.finished())
}

const DOWNLOADS: (&str, i32) = ("/home/me/Downloads", 96);
const PICTURES: (&str, i32) = ("/home/me/Pictures", 128);

fn both() -> String {
    format!(
        "{}{}",
        folder(DOWNLOADS.0, DOWNLOADS.1, 1),
        folder(PICTURES.0, PICTURES.1, 2)
    )
}

/// The change every test makes: `/home/me/Pictures` loses its icon size.
fn change(store: &mut FolderViewStore) {
    assert!(store.views_mut().prune_default_icons_size(PICTURES.1));
}

fn only_downloads() -> FolderViews {
    views(&folder(DOWNLOADS.0, DOWNLOADS.1, 1))
}

#[test]
fn a_change_is_saved_when_its_write_is_finished_and_not_before() {
    let before = both();
    own_process(
        "a_change_is_saved_when_its_write_is_finished_and_not_before",
        Some(&before),
        || {
            let mut store = FolderViewStore::load();
            assert!(store.load_failure().is_none() && !store.views().has_unsaved());
            change(&mut store);
            assert!(store.views().has_unsaved());

            let Written::Saved {
                merged: None,
                write,
            } = written(&store, false)
            else {
                panic!("the write was not written as asked");
            };
            assert!(saved().same_views(&only_downloads()));
            assert!(store.views().has_unsaved(), "until the write is finished");
            store.finish_write(write);
            assert!(!store.views().has_unsaved());
        },
    );
}

#[test]
fn a_change_made_while_a_write_is_out_stays_unsaved() {
    let before = both();
    own_process(
        "a_change_made_while_a_write_is_out_stays_unsaved",
        Some(&before),
        || {
            let mut store = FolderViewStore::load();
            change(&mut store);
            let Ok(write) = store.begin_write(false) else {
                panic!("saving is off");
            };
            assert!(store.views_mut().prune_default_icons_size(DOWNLOADS.1));
            let Written::Saved { write, .. } = settled(write.finished()) else {
                panic!("the write was not written");
            };
            store.finish_write(write);
            assert!(
                saved().same_views(&only_downloads()),
                "the write's own snapshot"
            );
            assert!(
                store.views().has_unsaved(),
                "the later change is still owed"
            );

            let Written::Saved { write, .. } = written(&store, false) else {
                panic!("the second write was not written");
            };
            store.finish_write(write);
            assert!(saved().is_empty() && !store.views().has_unsaved());
        },
    );
}

#[test]
fn a_file_unreadable_at_start_up_is_never_written_over() {
    let unreadable = "folder = [";
    own_process(
        "a_file_unreadable_at_start_up_is_never_written_over",
        Some(unreadable),
        || {
            let mut store = FolderViewStore::load();
            let kind = store.load_failure().map(io::Error::kind);
            assert_eq!(kind, Some(io::ErrorKind::InvalidData));
            assert!(!store.views_mut().clear(), "there is nothing to clear");
            let Err(UnreadableAtStartup { path, error }) = store.begin_write(true) else {
                panic!("a write was started over an unreadable file");
            };
            assert_eq!(path, folder_views_path());
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            writer::settle();
            assert_eq!(file().as_deref(), Some(unreadable));
        },
    );
}

#[test]
fn what_another_process_saved_is_adopted_under_this_ones_unsaved_changes() {
    let before = both();
    own_process(
        "what_another_process_saved_is_adopted_under_this_ones_unsaved_changes",
        Some(&before),
        || {
            let mut store = FolderViewStore::load();
            assert!(
                settled(FolderViewStore::reread()).is_none(),
                "nothing changed"
            );
            change(&mut store);

            // Another process saves a third folder and gives Pictures another size.
            let music = folder("/home/me/Music", 64, 3);
            let elsewhere = format!(
                "{}{}{music}",
                folder(DOWNLOADS.0, DOWNLOADS.1, 1),
                folder(PICTURES.0, 256, 4)
            );
            write_file(&elsewhere);
            let reread = settled(FolderViewStore::reread()).expect("another process's values");
            assert!(store.adopt(reread));
            let expected = views(&format!("{}{music}", folder(DOWNLOADS.0, DOWNLOADS.1, 1)));
            assert!(
                store.views().same_views(&expected),
                "this process's change wins"
            );
            assert!(store.views().has_unsaved());
            assert!(
                !store.adopt(views(&elsewhere)),
                "adopting the same again changes nothing"
            );
        },
    );
}

#[test]
fn a_write_brings_back_what_another_process_saved_meanwhile() {
    let before = both();
    own_process(
        "a_write_brings_back_what_another_process_saved_meanwhile",
        Some(&before),
        || {
            let mut store = FolderViewStore::load();
            change(&mut store);
            let music = folder("/home/me/Music", 64, 3);
            write_file(&format!("{before}{music}"));

            let Written::Saved {
                merged: Some(merged),
                write,
            } = written(&store, false)
            else {
                panic!("the other process's values did not come back");
            };
            assert!(store.adopt(merged));
            store.finish_write(write);
            let expected = views(&format!("{}{music}", folder(DOWNLOADS.0, DOWNLOADS.1, 1)));
            assert!(store.views().same_views(&expected) && saved().same_views(&expected));
            assert!(!store.views().has_unsaved());
        },
    );
}

#[test]
fn a_write_that_finds_another_process_saving_is_owed_still() {
    let before = both();
    own_process(
        "a_write_that_finds_another_process_saving_is_owed_still",
        Some(&before),
        || {
            let mut store = FolderViewStore::load();
            change(&mut store);
            let other_process =
                lock_saves(&folder_views_path(), Duration::ZERO).expect("the save lock");
            assert!(matches!(written(&store, false), Written::Busy));
            assert!(store.views().has_unsaved());
            assert_eq!(file().as_deref(), Some(before.as_str()));
            drop(other_process);

            let Written::Saved { write, .. } = written(&store, false) else {
                panic!("the retried write was not written");
            };
            store.finish_write(write);
            assert!(saved().same_views(&only_downloads()));
        },
    );
}
