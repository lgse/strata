// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use gio::{glib, prelude::*};

use crate::{adapters::gio_file_for_location, model::Location};

type BookmarkCallback = dyn Fn(&Result<(), glib::Error>) -> bool;
pub(crate) type BookmarkWatch = Rc<BookmarkCallback>;

thread_local! {
    static WATCHERS: RefCell<Vec<Weak<BookmarkCallback>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn watch_changes(
    callback: impl Fn(&Result<(), glib::Error>) -> bool + 'static,
) -> BookmarkWatch {
    let callback: BookmarkWatch = Rc::new(callback);
    WATCHERS.with_borrow_mut(|watchers| {
        watchers.retain(|watcher| watcher.strong_count() > 0);
        watchers.push(Rc::downgrade(&callback));
    });
    callback
}

pub(crate) fn pinned_places_path() -> std::path::PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

pub(super) fn deletion_completed(locations: &[Location]) {
    if locations.is_empty() {
        return;
    }
    let result = remove_deleted_pins(&gio::File::for_path(pinned_places_path()), locations);
    if let Err(error) = &result {
        tracing::warn!(%error, "unable to remove deleted folder pins");
    }
    let watchers = WATCHERS.with_borrow(|watchers| watchers.clone());
    for watcher in watchers {
        if let Some(callback) = watcher.upgrade() {
            let handled = callback(&result);
            if result.is_err() && handled {
                break;
            }
        }
    }
}

pub(super) fn remove_deleted_pins(
    file: &gio::File,
    locations: &[Location],
) -> Result<(), glib::Error> {
    remove_deleted_pins_with(file, locations, |contents, etag| {
        file.replace_contents(
            contents,
            etag,
            false,
            gio::FileCreateFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map(|_| ())
    })
}

fn remove_deleted_pins_with(
    file: &gio::File,
    locations: &[Location],
    mut replace: impl FnMut(&[u8], Option<&str>) -> Result<(), glib::Error>,
) -> Result<(), glib::Error> {
    let deleted: Vec<_> = locations.iter().map(gio_file_for_location).collect();
    let mut retries = 0;
    loop {
        let (contents, etag) = match file.load_contents(gio::Cancellable::NONE) {
            Ok(loaded) => loaded,
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => return Ok(()),
            Err(error) => return Err(error),
        };
        let retained = retain_unrelated_bookmarks(&contents, &deleted);
        if retained == contents.as_ref() {
            return Ok(());
        }
        match replace(&retained, etag.as_deref()) {
            Ok(_) => return Ok(()),
            Err(error) if retries < 2 && error.matches(gio::IOErrorEnum::WrongEtag) => retries += 1,
            Err(error) => return Err(error),
        }
    }
}

fn retain_unrelated_bookmarks(contents: &[u8], deleted: &[gio::File]) -> Vec<u8> {
    contents
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| {
            let uri = line
                .split(|byte| matches!(byte, b' ' | b'\r' | b'\n'))
                .next()
                .unwrap_or_default();
            let Ok(uri) = std::str::from_utf8(uri) else {
                return true;
            };
            let pin = gio::File::for_uri(uri);
            !deleted
                .iter()
                .any(|root| pin.equal(root) || pin.has_prefix(root))
        })
        .flatten()
        .copied()
        .collect()
}

#[cfg(test)]
mod tests;
