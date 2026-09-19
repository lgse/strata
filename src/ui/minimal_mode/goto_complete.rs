// SPDX-License-Identifier: MIT

//! Tab-complete folder names in the minimal `g` Space `go ›` prompt.

use std::path::{Path, PathBuf};

/// Cycling state for one prefix in one parent. Cleared when the typed parent
/// or stem no longer matches the last completion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GotoCycle {
    head: String,
    stem: String,
    names: Vec<String>,
    index: usize,
}

/// Next folder completion for `input`, or `None` to keep the typed text.
pub(crate) fn cycle_goto_path(
    input: &str,
    current: Option<&Path>,
    home: &Path,
    listing_folders: &[String],
    reverse: bool,
    cycle: &mut Option<GotoCycle>,
) -> Option<String> {
    let Some((head, stem, lookup, use_listing)) = parse_goto(input, current, home) else {
        *cycle = None;
        return None;
    };
    let continuing = cycle.as_ref().is_some_and(|cycle| {
        cycle.head == head && (stem == cycle.stem || cycle.names.iter().any(|name| name == &stem))
    });
    if !continuing {
        let names = matching_folders(&lookup, use_listing, listing_folders, &stem);
        if names.is_empty() {
            *cycle = None;
            return None;
        }
        let index = if reverse { names.len() - 1 } else { 0 };
        let completed = format!("{head}{}", names[index]);
        *cycle = Some(GotoCycle {
            head,
            stem,
            names,
            index,
        });
        return Some(completed);
    }
    let cycle = cycle.as_mut().expect("continuing cycle");
    let len = cycle.names.len();
    cycle.index = if reverse {
        (cycle.index + len - 1) % len
    } else {
        (cycle.index + 1) % len
    };
    Some(format!("{}{}", cycle.head, cycle.names[cycle.index]))
}

/// Resolve a go-prompt path so `submit_location` can accept a listing-relative
/// completion such as `documents`.
pub(crate) fn resolve_goto_input(input: &str, current: Option<&Path>) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed.contains("://")
    {
        return trimmed.to_owned();
    }
    match current {
        Some(base) => base.join(trimmed).to_string_lossy().into_owned(),
        None => trimmed.to_owned(),
    }
}

fn parse_goto(
    input: &str,
    current: Option<&Path>,
    home: &Path,
) -> Option<(String, String, PathBuf, bool)> {
    if input.contains("://") {
        return None;
    }
    if input.starts_with('~') && input != "~" && !input.starts_with("~/") {
        return None;
    }
    let (head, stem) = split_head_stem(input);
    let (lookup, use_listing) = lookup_dir(input, head, current, home)?;
    Some((head.to_owned(), stem.to_owned(), lookup, use_listing))
}

fn split_head_stem(input: &str) -> (&str, &str) {
    if input == "~" {
        return ("~/", "");
    }
    match input.rfind('/') {
        Some(index) => (&input[..=index], &input[index + 1..]),
        None => ("", input),
    }
}

fn lookup_dir(
    input: &str,
    head: &str,
    current: Option<&Path>,
    home: &Path,
) -> Option<(PathBuf, bool)> {
    if input == "~" || input.starts_with("~/") {
        let rest = head.strip_prefix("~/").unwrap_or("").trim_end_matches('/');
        let dir = if rest.is_empty() {
            home.to_path_buf()
        } else {
            home.join(rest)
        };
        return Some((dir, false));
    }
    if head.is_empty() {
        return Some((current.unwrap_or(Path::new("")).to_path_buf(), true));
    }
    let head_path = Path::new(head);
    if head_path.is_absolute() {
        return Some((head_path.to_path_buf(), false));
    }
    Some((current?.join(head), false))
}

fn matching_folders(
    lookup: &Path,
    use_listing: bool,
    listing_folders: &[String],
    stem: &str,
) -> Vec<String> {
    let mut names = if use_listing {
        listing_folders
            .iter()
            .filter(|name| folder_name_matches(name, stem))
            .cloned()
            .collect()
    } else {
        read_dir_folders(lookup, stem)
    };
    names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then(left.cmp(right))
    });
    names.dedup();
    names
}

fn read_dir_folders(dir: &Path, stem: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            folder_name_matches(&name, stem).then_some(name)
        })
        .collect()
}

fn folder_name_matches(name: &str, stem: &str) -> bool {
    if name == "." || name == ".." {
        return false;
    }
    if name.starts_with('.') && !stem.starts_with('.') {
        return false;
    }
    name.to_lowercase().starts_with(&stem.to_lowercase())
}

#[cfg(test)]
mod tests;
