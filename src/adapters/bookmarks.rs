// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs, io,
    io::Read,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use futures_channel::oneshot;
use gio::{glib, prelude::*};

use crate::{
    adapters::{gio_file_for_location, location_for_file},
    model::Location,
    services::sanitize_uri_credentials,
    storage::{FileStamp, stamp_of, writer},
    util::ui_thread,
};

pub(crate) type Pins = Vec<(Location, String)>;
type BookmarkCallback = dyn Fn(Result<&Pins, &io::Error>) -> bool;
pub(crate) type BookmarkWatch = Rc<BookmarkCallback>;

const RETRIES: usize = 2;

thread_local! {
    static WATCHERS: RefCell<Vec<Weak<BookmarkCallback>>> = const { RefCell::new(Vec::new()) };
    static CURRENT: RefCell<Option<Pins>> = const { RefCell::new(None) };
    static QUEUED: Cell<u64> = const { Cell::new(0) };
    static ADOPTED: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn watch_changes(
    callback: impl Fn(Result<&Pins, &io::Error>) -> bool + 'static,
) -> BookmarkWatch {
    let callback: BookmarkWatch = Rc::new(callback);
    WATCHERS.with_borrow_mut(|watchers| {
        watchers.retain(|watcher| watcher.strong_count() > 0);
        watchers.push(Rc::downgrade(&callback));
    });
    callback
}

pub(crate) fn pinned_places_path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

pub(crate) fn current() -> Pins {
    if let Some(pins) = CURRENT.with_borrow(Clone::clone) {
        refresh();
        return pins;
    }
    let pins = {
        let _startup = ui_thread::allow(ui_thread::Exception::Startup);
        read_pins(&pinned_places_path()).unwrap_or_default()
    };
    CURRENT.replace(Some(pins.clone()));
    pins
}

pub(crate) async fn update(
    edit: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
) -> io::Result<bool> {
    let path = pinned_places_path();
    let (order, applied) = queue(move || apply(&path, &edit, replace));
    let (pins, changed) = applied
        .await
        .unwrap_or_else(|_| Err(io::Error::from_raw_os_error(libc::ECANCELED)))?;
    adopt(order, pins);
    Ok(changed)
}

fn queue<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> (u64, oneshot::Receiver<T>) {
    let order = QUEUED.get() + 1;
    QUEUED.set(order);
    (order, writer::run(job))
}

fn adopt(order: u64, pins: Pins) {
    if ADOPTED.get() > order {
        return;
    }
    ADOPTED.set(order);
    CURRENT.replace(Some(pins.clone()));
    notify(Ok(&pins));
}

fn forget_deleted(locations: Vec<Location>) {
    if locations.is_empty() {
        return;
    }
    let path = pinned_places_path();
    let edit = forgetting(locations);
    let (order, applied) = queue(move || {
        apply(&path, &edit, replace).inspect_err(|error| {
            tracing::warn!(%error, "unable to remove deleted folder pins");
        })
    });
    let shown = CURRENT.with_borrow(Option::is_some)
        || WATCHERS
            .with_borrow(|watchers| watchers.iter().any(|watcher| watcher.strong_count() > 0));
    if !shown {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        match applied
            .await
            .unwrap_or_else(|_| Err(io::Error::from_raw_os_error(libc::ECANCELED)))
        {
            Ok((pins, _)) => adopt(order, pins),
            Err(error) => notify(Err(&error)),
        }
    });
}

fn forgetting(locations: Vec<Location>) -> impl Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static {
    move |contents| {
        let deleted: Vec<_> = locations.iter().map(gio_file_for_location).collect();
        let retained = retain_unrelated_bookmarks(contents, &deleted);
        (retained != contents).then_some(retained)
    }
}

#[derive(Default)]
pub(crate) struct DeletedPins(Vec<Location>);

impl DeletedPins {
    pub(crate) fn push(&mut self, location: Location) {
        self.0.push(location);
    }
}

impl Drop for DeletedPins {
    fn drop(&mut self) {
        forget_deleted(std::mem::take(&mut self.0));
    }
}

fn refresh() {
    glib::MainContext::default().spawn_local(async {
        if let Err(error) = update(|_| None).await {
            tracing::debug!(%error, "unable to re-read pinned folders");
        }
    });
}

fn notify(result: Result<&Pins, &io::Error>) {
    let watchers = WATCHERS.with_borrow(|watchers| watchers.clone());
    for watcher in watchers {
        if let Some(callback) = watcher.upgrade()
            && callback(result)
            && result.is_err()
        {
            break;
        }
    }
}

pub(crate) fn read_pins(path: &Path) -> io::Result<Pins> {
    read_stamped(path).map(|(contents, _)| parse_pinned_places(&contents))
}

fn read_stamped(path: &Path) -> io::Result<(Vec<u8>, Option<FileStamp>)> {
    ui_thread::assert_off();
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), None)),
        Err(error) => return Err(error),
    };
    let stamp = stamp_of(&file.metadata()?);
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    Ok((contents, Some(stamp)))
}

fn replace(path: &Path, contents: &[u8], read: Option<FileStamp>) -> io::Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::storage::atomic_write_config_unless_changed(path, contents, read)
}

fn apply(
    path: &Path,
    edit: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
    mut replace: impl FnMut(&Path, &[u8], Option<FileStamp>) -> io::Result<bool>,
) -> io::Result<(Pins, bool)> {
    let mut retries = 0;
    loop {
        let (contents, stamp) = read_stamped(path)?;
        let Some(edited) = edit(&contents) else {
            return Ok((parse_pinned_places(&contents), false));
        };
        if replace(path, &edited, stamp)? {
            return Ok((parse_pinned_places(&edited), true));
        }
        if retries == RETRIES {
            return Err(io::ErrorKind::ResourceBusy.into());
        }
        retries += 1;
    }
}

/// GTK bookmarks may contain non-UTF-8 labels.
pub(crate) fn parse_pinned_places(contents: &[u8]) -> Pins {
    let mut places = Vec::new();
    for line in contents.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let (uri, label) = match line.iter().position(|byte| *byte == b' ') {
            Some(space) => (&line[..space], Some(&line[space + 1..])),
            None => (line, None),
        };
        if uri.is_empty() {
            continue;
        }
        let Ok(uri) = std::str::from_utf8(uri) else {
            continue;
        };
        let file = gio::File::for_uri(uri);
        let Some(location) = location_for_file(&file) else {
            continue;
        };
        if places
            .iter()
            .any(|(existing, _): &(Location, String)| existing == &location)
        {
            continue;
        }
        let name = label
            .filter(|label| !label.is_empty())
            .map(|label| String::from_utf8_lossy(label).into_owned())
            .unwrap_or_else(|| location.display_name());
        places.push((location, name));
    }
    places
}

pub(crate) fn serialize_pinned_places(places: &[(Location, String)]) -> String {
    let mut contents = String::new();
    for (location, name) in places {
        let uri = location
            .native_path()
            .map(gio::File::for_path)
            .map(|file| file.uri().to_string())
            .or_else(|| location.uri_value().map(str::to_owned));
        let Some(uri) = uri.and_then(|uri| sanitize_uri_credentials(&uri).ok().map(|(uri, _)| uri))
        else {
            continue;
        };
        let label = name.replace(['\n', '\r'], " ");
        contents.push_str(&format!("{uri} {label}\n"));
    }
    contents
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
