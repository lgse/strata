// SPDX-License-Identifier: MIT

//! Directory marks are navigation state, not sidebar pins or preferences.
//! All reads and read-modify-writes use the storage worker, so windows see each
//! other's changes without doing filesystem work on the GTK thread.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use gio::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Bookmarks {
    entries: BTreeMap<char, String>,
}

fn state_path() -> PathBuf {
    gtk::glib::user_data_dir().join("strata/letter-bookmarks.json")
}

fn load(path: &Path) -> io::Result<Bookmarks> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Bookmarks::default()),
        Err(error) => Err(error),
    }
}

fn read(path: &Path, letter: char) -> io::Result<Option<PathBuf>> {
    let bookmarks = load(path)?;
    Ok(bookmarks.entries.get(&letter).and_then(|uri| {
        gio::File::for_uri(uri)
            .path()
            .filter(|path| path.is_absolute())
    }))
}

fn read_all(path: &Path) -> io::Result<Vec<(char, PathBuf)>> {
    Ok(load(path)?
        .entries
        .into_iter()
        .filter_map(|(letter, uri)| {
            if !letter.is_ascii_lowercase() {
                return None;
            }
            gio::File::for_uri(&uri)
                .path()
                .filter(|path| path.is_absolute())
                .map(|path| (letter, path))
        })
        .collect())
}

fn write(path: &Path, letter: char, directory: &Path) -> io::Result<()> {
    if !letter.is_ascii_lowercase() || !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid directory mark",
        ));
    }
    let mut bookmarks = load(path)?;
    bookmarks
        .entries
        .insert(letter, gio::File::for_path(directory).uri().to_string());
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
