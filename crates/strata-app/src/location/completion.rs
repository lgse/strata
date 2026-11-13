// SPDX-License-Identifier: MIT

//! Path suggestions for the location bar's entry: GTK's planner, ported, answering C6's
//! `LocationIntent::Complete` (`PathCompletion::refresh` and `plan_completions` in
//! `src/ui/browser/location/completion.rs`).
//!
//! - **What is suggested:** the folders under the folder the typed text names, matched by the
//!   leaf being typed, case-insensitively, at most 50, sorted by name; `~` offers `~/`, `~/`
//!   lists the home folder, a relative path lists from the shown folder. When one folder or
//!   none matches and the text names a folder itself, that folder's children are offered. Hidden
//!   folders show when the leaf starts with a dot or hidden files are shown.
//! - **Off Rune's thread:** a listing runs on `strata-fs`'s mount lanes (`child_folders`), and
//!   its answer comes back through the part's [`Later`]. A newer request or the bar's
//!   `CancelCompletion` aborts the one in flight, and C6 drops an answer whose generation is
//!   not the latest (`LocationBarState::deliver`).
//!
//! C6 holds the rest: the popup, its keys, the common prefix Tab completes, the deferred keys
//! while a listing is out, and dismissing on the window's deactivation.
//!
//! **Ported from GTK: watch merges from `main`.** The planner (`plan_completions`, `Folder`,
//! `Listing`, `compact_hint`, `directory_prefix`) is a copy of `src/ui/browser/location/completion.rs`
//! as of `65019ba16cb530a2bc7235cc665f367574d19239` (`plan_completions`'s `~/` and `/` arms split out as `plan_home` and
//! `plan_absolute`, unchanged), and its tests of `completion/tests.rs`. A fix there that
//! reaches `main` is ported here by hand.

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use glib::JoinHandle;
use rune::Tx;
use strata_components::location_bar::{Candidate, CompletionQuery, Completions};
use strata_fs::adapters::probes;
use strata_kit::icons::IconName;

use crate::{App, slots::Later};

const MAX_COMPLETION_CANDIDATES: usize = 50;
const MAX_SCANNED_ENTRIES: usize = 10_000;

/// One suggestion as GTK's planner makes it (`CompletionCandidate`).
#[derive(Clone, Debug, Eq, PartialEq)]
struct Suggestion {
    display_name: String,
    replacement: String,
    parent_hint: String,
    /// How many characters of `display_name` the typed leaf matched.
    match_len: usize,
}

impl Suggestion {
    /// The row C6 draws: the matched characters in the accent, the folder icon.
    fn candidate(self) -> Candidate {
        let matched = matched_bytes(&self.display_name, self.match_len);
        Candidate::new(self.display_name, self.replacement)
            .matched(matched)
            .hint(self.parent_hint)
            .icon(IconName::Folder)
    }
}

/// The bytes of the first `chars` characters of `text`, if any.
fn matched_bytes(text: &str, chars: usize) -> Option<Range<usize>> {
    if chars == 0 {
        return None;
    }
    let end = text
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(at, _)| at);
    Some(0..end)
}

/// The listing in flight, if any.
#[derive(Default)]
pub(super) struct Completing {
    task: Option<JoinHandle<()>>,
}

impl Completing {
    /// Stops the listing in flight: a newer request supersedes it, or the bar has stopped
    /// listing.
    pub(super) fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Answers the bar's request for suggestions: at once when nothing has to be listed, else
/// once the listing comes back through `later`.
pub(super) fn complete(app: &mut App, query: CompletionQuery, later: &Later<App>, tx: &mut Tx) {
    app.location.completing.cancel();
    let current = app
        .browser
        .active_location()
        .and_then(|location| location.native_path().map(Path::to_path_buf));
    let show_hidden = app.browser.preferences().show_hidden;
    let home = glib::home_dir();
    let generation = query.generation;
    match plan_completions(&query.text, current.as_deref(), &home, show_hidden) {
        Plan::Ready(suggestions) => answer(app, generation, suggestions, tx),
        Plan::List(listing) => {
            let later = later.clone();
            let task = glib::MainContext::default().spawn_local(async move {
                let suggestions = listing.candidates().await;
                later.then(move |app: &mut App, tx: &mut Tx| {
                    app.location.completing.task = None;
                    answer(app, generation, suggestions, tx);
                    true
                });
            });
            app.location.completing.task = Some(task);
        }
    }
}

fn answer(app: &mut App, generation: u64, suggestions: Vec<Suggestion>, tx: &mut Tx) {
    let candidates = suggestions.into_iter().map(Suggestion::candidate).collect();
    let polled = app
        .location
        .state
        .deliver(Completions::new(generation, candidates));
    super::intents::route(app, polled, tx);
}

enum Plan {
    Ready(Vec<Suggestion>),
    List(Listing),
}

struct Folder {
    path: PathBuf,
    prepend: String,
    parent_hint: String,
}

impl Folder {
    fn new(path: PathBuf, prepend: String, home: &Path) -> Self {
        let parent_hint = compact_hint(&path, home);
        Self {
            path,
            prepend,
            parent_hint,
        }
    }

    async fn candidates(
        &self,
        leaf_prefix: &str,
        include_hidden: bool,
        match_len: usize,
    ) -> Vec<Suggestion> {
        let names =
            probes::child_folders(&self.path, leaf_prefix, include_hidden, MAX_SCANNED_ENTRIES)
                .await;
        let mut dirs: Vec<_> = names
            .into_iter()
            .map(|name| Suggestion {
                display_name: format!("{name}/"),
                replacement: format!("{}{name}/", self.prepend),
                parent_hint: self.parent_hint.clone(),
                match_len,
            })
            .collect();
        dirs.sort_by_cached_key(|suggestion| suggestion.display_name.to_lowercase());
        dirs.truncate(MAX_COMPLETION_CANDIDATES);
        dirs
    }
}

struct Listing {
    folder: Folder,
    leaf_prefix: String,
    show_hidden: bool,
    inside: Option<Folder>,
}

impl Listing {
    async fn candidates(self) -> Vec<Suggestion> {
        let include_hidden = self.leaf_prefix.starts_with('.') || self.show_hidden;
        let match_len = self.leaf_prefix.chars().count();
        let candidates = self
            .folder
            .candidates(&self.leaf_prefix, include_hidden, match_len)
            .await;
        if candidates.len() <= 1
            && let Some(inside) = &self.inside
        {
            let children = inside.candidates("", self.show_hidden, 0).await;
            if !children.is_empty() {
                return children;
            }
        }
        candidates
    }
}

fn compact_hint(path: &Path, home: &Path) -> String {
    if path == home {
        return "~".to_owned();
    }
    if let Ok(suffix) = path.strip_prefix(home) {
        return format!("~/{}", suffix.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

fn directory_prefix(path: &Path) -> String {
    let mut prefix = path.to_string_lossy().into_owned();
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    prefix
}

fn plan_completions(
    input: &str,
    current_dir: Option<&Path>,
    home: &Path,
    show_hidden: bool,
) -> Plan {
    let input = input.trim();
    let list = |folder: Folder, leaf_prefix: &str, inside: Option<Folder>| {
        Plan::List(Listing {
            folder,
            leaf_prefix: leaf_prefix.to_owned(),
            show_hidden,
            inside: inside.filter(|_| !input.ends_with('/')),
        })
    };
    if input.is_empty() {
        return match current_dir {
            Some(current) => list(
                Folder::new(current.to_path_buf(), String::new(), home),
                "",
                None,
            ),
            None => Plan::Ready(Vec::new()),
        };
    }

    if input == "~" {
        return Plan::Ready(vec![Suggestion {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            parent_hint: "home".to_owned(),
            match_len: 1,
        }]);
    }

    if let Some(relative) = input.strip_prefix("~/") {
        return plan_home(relative, home, list);
    }

    if input.starts_with('~') {
        return Plan::Ready(Vec::new());
    }

    if let Some(stripped) = input.strip_prefix('/') {
        return plan_absolute(input, stripped, home, list);
    }

    let Some(current) = current_dir else {
        return Plan::Ready(Vec::new());
    };
    let (base_dir, leaf_prefix) = if input.ends_with('/') {
        (current.join(input), "")
    } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
        (current.join(parent_part), leaf)
    } else {
        (current.to_path_buf(), input)
    };
    let input_path = current.join(input);
    let inside = Folder::new(input_path.clone(), directory_prefix(&input_path), home);
    list(
        Folder::new(base_dir.clone(), directory_prefix(&base_dir), home),
        leaf_prefix,
        Some(inside),
    )
}

/// `~/…`: what follows the home folder.
fn plan_home(
    relative: &str,
    home: &Path,
    list: impl Fn(Folder, &str, Option<Folder>) -> Plan,
) -> Plan {
    let (base_dir, leaf_prefix, prepend) = if relative.ends_with('/') {
        (
            home.join(relative.trim_start_matches('/')),
            "",
            format!("~/{relative}"),
        )
    } else if let Some((parent_rel, leaf)) = relative.rsplit_once('/') {
        (
            home.join(parent_rel.trim_start_matches('/')),
            leaf,
            format!("~/{parent_rel}/"),
        )
    } else {
        (home.to_path_buf(), relative, "~/".to_owned())
    };
    let rel_trimmed = relative.trim_start_matches('/');
    let inside = (!rel_trimmed.is_empty()).then(|| {
        Folder::new(
            home.join(rel_trimmed),
            format!("~/{}/", rel_trimmed.trim_end_matches('/')),
            home,
        )
    });
    list(Folder::new(base_dir, prepend, home), leaf_prefix, inside)
}

/// `/…`: an absolute path.
fn plan_absolute(
    input: &str,
    stripped: &str,
    home: &Path,
    list: impl Fn(Folder, &str, Option<Folder>) -> Plan,
) -> Plan {
    let (base_dir, leaf_prefix, prepend) = if input == "/" {
        (PathBuf::from("/"), "", "/".to_owned())
    } else if input.ends_with('/') {
        (PathBuf::from(input), "", input.to_owned())
    } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
        let base = if parent_part.is_empty() {
            PathBuf::from("/")
        } else {
            PathBuf::from(parent_part)
        };
        let prepend = if parent_part.is_empty() {
            "/".to_owned()
        } else {
            format!("{parent_part}/")
        };
        (base, leaf, prepend)
    } else {
        (PathBuf::from("/"), stripped, "/".to_owned())
    };
    let inside = (input != "/").then(|| {
        Folder::new(
            PathBuf::from(input),
            format!("{}/", input.trim_end_matches('/')),
            home,
        )
    });
    list(Folder::new(base_dir, prepend, home), leaf_prefix, inside)
}

#[cfg(test)]
mod tests;
