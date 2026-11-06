// SPDX-License-Identifier: MIT

use std::{ffi::OsString, fmt::Debug};

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::DirectoryChange,
};

mod columns;
mod fill_commands;
mod fill_tracking;
mod history;
mod loading;
mod metadata;
mod monitor;
mod paging;
mod ranges;
mod reload;
mod selection;
mod sorting;
mod visual;

const ABC: [&str; 3] = ["alpha", "bravo", "charlie"];

/// The active column and its keyboard cursor, as `active_focus` reports them.
type Focus = Option<(usize, Option<usize>)>;

fn location(path: &str) -> Location {
    Location::local(path)
}

fn entry(path: &str) -> FileEntry {
    named_entry(path, "child")
}

/// A visible folder with no loaded details.
fn named_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        thumbnail_path: None,
        location: location(path),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

fn hidden_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        is_hidden: true,
        ..named_entry(path, name)
    }
}

fn file_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        kind: EntryKind::File,
        ..named_entry(path, name)
    }
}

fn child(parent: &str, name: &str) -> FileEntry {
    named_entry(&format!("{parent}/{name}"), name)
}

fn children(parent: &str, names: &[&str]) -> Vec<FileEntry> {
    names.iter().map(|name| child(parent, name)).collect()
}

/// `/home` with alpha, a hidden bravo and charlie.
fn home_with_a_hidden_bravo() -> Vec<FileEntry> {
    vec![
        named_entry("/home/alpha", "alpha"),
        hidden_entry("/home/bravo", "bravo"),
        named_entry("/home/charlie", "charlie"),
    ]
}

fn list(state: &mut NavigationState, root: &str, entries: Vec<FileEntry>) {
    state.navigate(location(root), RequestId(1));
    state.apply_batch(RequestId(1), entries);
}

fn loaded(root: &str, entries: Vec<FileEntry>) -> NavigationState {
    let mut state = NavigationState::default();
    list(&mut state, root, entries);
    state
}

fn loaded_with_a_load_cursor(root: &str, entries: Vec<FileEntry>) -> NavigationState {
    let mut state = NavigationState::default();
    state.navigate(location(root), RequestId(1));
    state.select_first_on_load(0);
    state.apply_batch(RequestId(1), entries);
    state
}

fn listing_without_a_load_cursor() -> NavigationState {
    loaded("/fixture", children("/fixture", &ABC))
}

fn listing_with_the_first_entry_selected() -> NavigationState {
    loaded_with_a_load_cursor("/fixture", children("/fixture", &ABC))
}

/// Selects the first entry at `depth` and opens it, at `path`, as the next column.
fn open_first(state: &mut NavigationState, depth: usize, path: &str, request: u64) {
    assert!(state.select(depth, 0));
    assert!(state.descend(depth, location(path), RequestId(request)));
}

fn descending() -> ViewPreferences {
    ViewPreferences {
        sort_direction: SortDirection::Descending,
        ..ViewPreferences::default()
    }
}

fn names<'a>(entries: impl IntoIterator<Item = &'a FileEntry>) -> Vec<String> {
    entries
        .into_iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn column_names(state: &NavigationState, depth: usize) -> Vec<String> {
    names(&state.columns[depth].entries)
}

fn selected_names(state: &NavigationState) -> Vec<String> {
    names(&state.selected_entries())
}

fn command_names(state: &NavigationState, depth: usize) -> Vec<String> {
    names(&state.command_entries(depth))
}

/// The rows a full scan of the listing selects, bypassing the cached positions.
fn scanned_positions(state: &NavigationState, depth: usize) -> Vec<usize> {
    let column = &state.columns[depth];
    column.selected_locations.scan(&column.entries)
}

fn focused_position(state: &NavigationState) -> Option<usize> {
    state.focused_entry().map(|(_, position, _)| position)
}

fn focused_name(state: &NavigationState) -> Option<String> {
    state
        .focused_entry()
        .map(|(_, _, entry)| entry.display_name)
}

fn fill_and_focus(state: &NavigationState, depth: usize) -> (Vec<usize>, Focus) {
    (state.selected_positions(depth), state.active_focus())
}

fn fill_focus_and_anchor(state: &NavigationState) -> (Vec<usize>, Focus, Option<usize>) {
    (
        state.selected_positions(0),
        state.active_focus(),
        state.selection_anchor_position(0),
    )
}

/// The range a keyboard extension selects.
fn extend(state: &mut NavigationState, direction: i32) -> Option<Vec<usize>> {
    state.extend_selection(direction).map(|(_, _, range)| range)
}

/// Applies each input in turn and asserts the result of every step.
fn assert_steps<I: Copy + Debug, O: PartialEq + Debug>(
    state: &mut NavigationState,
    step: impl Fn(&mut NavigationState, I) -> O,
    cases: &[(I, O)],
) {
    for (index, (input, expected)) in cases.iter().enumerate() {
        assert_eq!(&step(state, *input), expected, "step {index}: {input:?}");
    }
}
