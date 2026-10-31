// SPDX-License-Identifier: MIT

//! Letter bookmarks are kept apart from sidebar pins and preferences. All
//! reads and read-modify-writes use the storage worker, so windows see each
//! other's changes without doing filesystem work on the GTK thread.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use gio::prelude::*;
use serde::{Deserialize, Serialize};

/// Unknown fields and unusable entries are ignored on reading, so a file a
/// newer version or a hand edit wrote still loads.
#[derive(Default, Deserialize, Serialize)]
struct Bookmarks {
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

fn state_path() -> PathBuf {
    gtk::glib::user_data_dir().join("strata/letter-bookmarks.json")
}

fn contents(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load(path: &Path) -> io::Result<Bookmarks> {
    contents(path)?.map_or_else(
        || Ok(Bookmarks::default()),
        |bytes| serde_json::from_slice(&bytes).map_err(io::Error::other),
    )
}

fn slot(key: &str) -> Option<char> {
    let mut letters = key.chars();
    let letter = letters.next().filter(char::is_ascii_lowercase)?;
    letters.next().is_none().then_some(letter)
}

fn read(path: &Path, letter: char) -> io::Result<Option<PathBuf>> {
    let bookmarks = load(path)?;
    bookmarks
        .entries
        .get(&letter.to_string())
        .map(|uri| local_directory(uri).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData)))
        .transpose()
}

fn local_directory(uri: &str) -> Option<PathBuf> {
    gio::File::for_uri(uri)
        .path()
        .filter(|path| path.is_absolute())
}

fn read_all(path: &Path) -> io::Result<Vec<(char, PathBuf)>> {
    Ok(load(path)?
        .entries
        .into_iter()
        .filter_map(|(key, uri)| Some((slot(&key)?, local_directory(&uri)?)))
        .collect())
}

fn write(path: &Path, letter: char, directory: &Path) -> io::Result<()> {
    if !letter.is_ascii_lowercase() || !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid directory mark",
        ));
    }
    // Contents that no longer parse are replaced rather than blocking every save.
    let mut bookmarks: Bookmarks = contents(path)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    bookmarks
        .entries
        .retain(|key, uri| slot(key).is_some() && local_directory(uri).is_some());
    bookmarks.entries.insert(
        letter.to_string(),
        gio::File::for_path(directory).uri().to_string(),
    );
    let contents = serde_json::to_vec_pretty(&bookmarks).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::storage::atomic_write(path, &contents)
}

pub(crate) async fn list() -> io::Result<Vec<(char, PathBuf)>> {
    crate::storage::writer::run(move || read_all(&state_path()))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn get(letter: char) -> io::Result<Option<PathBuf>> {
    crate::storage::writer::run(move || read(&state_path(), letter))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn set(letter: char, directory: PathBuf) -> io::Result<()> {
    crate::storage::writer::run(move || write(&state_path(), letter, &directory))
        .await
        .map_err(io::Error::other)?
}

#[cfg(test)]
mod tests;
