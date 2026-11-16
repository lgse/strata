// SPDX-License-Identifier: MIT

//! History (`z` / `Z`) and folder-picker (`g Space`, `M`, `C`, `; E`) candidates and submission
//! for 10xer's footer prompts (`src/ui/folder_picker.rs`, `src/ui/window/keyboard/prompts.rs`).

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use rune::Tx;
use strata_components::filter_bar::prompt::{Candidate, Candidates, CompletionQuery};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location, lexically_normalized},
};
use strata_fs::{
    adapters::gio_file_for_location,
    services::{NavigationHistory, RefusedFolders, sanitize_uri_credentials},
};

use super::{close_and_return, prompt_builder, spec::PromptKind};
use crate::{
    App, archive,
    filter::{
        self,
        session::{Input, Scope, Session},
    },
    open_failed, shell, transfer,
};

const NO_MATCHES: &str = "No matching folders";
const SEARCHING: &str = "Searching\u{2026}";
const TYPED_BASE_DELAY: Duration = Duration::from_millis(150);

const LOCATION_SCHEMES: [&str; 10] = [
    "smb", "sftp", "ftp", "ftps", "dav", "davs", "trash", "network", "recent", "file",
];

#[derive(Default)]
pub(crate) struct Completing {
    session: Session,
    delayed: Rc<RefCell<Option<glib::SourceId>>>,
    generation: u64,
}

impl Completing {
    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(source) = self.delayed.borrow_mut().take() {
            source.remove();
        }
        self.session.cancel();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PickerScope {
    base: PathBuf,
    query: String,
}

fn is_scheme(text: &str) -> bool {
    text.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

fn looks_like_uri(text: &str) -> bool {
    if text.starts_with("//") || text.starts_with('\\') {
        return true;
    }
    let first = text.split('/').next().unwrap_or_default();
    if let Some((scheme, _)) = first.split_once(':')
        && is_scheme(scheme)
        && (text[scheme.len() + 1..].starts_with("//")
            || LOCATION_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    first
        .split_once('@')
        .and_then(|(user, rest)| Some((user, rest.split_once(':')?.0)))
        .is_some_and(|(user, host)| {
            [user, host]
                .iter()
                .all(|part| !part.is_empty() && !part.contains(char::is_whitespace))
        })
}

fn split_typed_path(text: &str) -> Option<(&str, &str)> {
    if text == "~" || text == ".." {
        return Some((text, ""));
    }
    if !["/", "~/", "./", "../"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
    {
        return None;
    }
    let slash = text.rfind('/')?;
    match &text[slash + 1..] {
        "." | ".." => Some((text, "")),
        query => Some((&text[..=slash], query)),
    }
}

fn picker_scope(
    text: &str,
    current: Option<&Path>,
    home: &Path,
) -> Result<Option<PickerScope>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if looks_like_uri(text) {
        return Err("Only local folders can be chosen");
    }
    if text.starts_with('~') && text != "~" && !text.starts_with("~/") {
        return Err("Only ~ and ~/ are supported");
    }
    let Some((stem, query)) = split_typed_path(text) else {
        let base = current.ok_or("Type a full path here")?;
        return Ok(Some(PickerScope {
            base: base.to_path_buf(),
            query: text.to_owned(),
        }));
    };
    let base = resolve_stem(stem, current, home)?;
    Ok(Some(PickerScope {
        base: lexically_normalized(&base),
        query: query.trim().to_owned(),
    }))
}

fn resolve_stem(stem: &str, current: Option<&Path>, home: &Path) -> Result<PathBuf, &'static str> {
    if stem == "~" {
        Ok(home.to_path_buf())
    } else if let Some(relative) = stem.strip_prefix("~/") {
        Ok(home.join(relative.trim_start_matches('/')))
    } else if stem.starts_with('/') {
        Ok(PathBuf::from(stem))
    } else {
        Ok(current.ok_or("Type a full path here")?.join(stem))
    }
}

fn typed_target(text: &str, current: Option<&Path>, home: &Path) -> Option<PathBuf> {
    split_typed_path(text.trim())?;
    let scope = picker_scope(text, current, home).ok()??;
    Some(lexically_normalized(&scope.base.join(scope.query)))
}

fn typed_path(folder: &Path, current: Option<&Path>, home: &Path) -> String {
    let below = |base: &Path, prefix: &str| {
        let relative = folder.strip_prefix(base).ok()?;
        Some(if relative.as_os_str().is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}{}/", relative.to_string_lossy())
        })
    };
    current
        .and_then(|cur| below(cur, "./"))
        .or_else(|| below(home, "~/"))
        .unwrap_or_else(|| {
            let abs = folder.to_string_lossy();
            if abs.ends_with('/') {
                abs.into_owned()
            } else {
                format!("{abs}/")
            }
        })
}

fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

fn wants_hidden(query: &str) -> bool {
    query.split_whitespace().any(|term| {
        term.trim_start_matches(['!', '^', '\'']).starts_with('.') || term.contains("/.")
    })
}

fn active_folder(app: &App) -> Option<PathBuf> {
    app.browser
        .active_location()
        .and_then(|loc| loc.native_path().map(Path::to_path_buf))
}

fn to_candidates(paths: &[PathBuf], current: Option<&Path>, home: &Path) -> Vec<Candidate> {
    paths
        .iter()
        .map(|path| {
            let id = path.to_string_lossy().into_owned();
            let label = path
                .file_name()
                .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
                .into_owned();
            let hint = path
                .parent()
                .map_or_else(String::new, |parent| home_relative(parent, home));
            let insert = typed_path(path, current, home);
            Candidate::folder(id, label, hint, insert)
        })
        .collect()
}

pub(super) fn complete(app: &mut App, kind: PromptKind, query: CompletionQuery, tx: &mut Tx) {
    app.tenxer.prompt_state.completing.cancel();
    if kind.picks_history() {
        complete_history(app, kind, query, tx);
    } else if kind.picks_folder() {
        complete_folder(app, kind, query, tx);
    }
}

fn complete_history(app: &mut App, kind: PromptKind, query: CompletionQuery, tx: &mut Tx) {
    let current = active_folder(app);
    let history = NavigationHistory::shared();
    let items = if kind == PromptKind::Jump {
        history.search_excluding(&query.text, current.as_deref())
    } else {
        history.recent_excluding(&query.text, current.as_deref())
    };
    let paths: Vec<PathBuf> = items.into_iter().map(|item| item.path).collect();
    let home = glib::home_dir();
    let candidates = to_candidates(&paths, current.as_deref(), &home);
    let empty = candidates.is_empty();
    let polled = app.tenxer.prompt.deliver(Candidates {
        generation: query.generation,
        items: candidates,
        done: true,
    });
    let serial = app.tenxer.prompt.serial();
    let _ = app
        .tenxer
        .prompt
        .show_hint(serial, empty.then(|| tr(NO_MATCHES)));
    prompt_builder(app).route(app, polled, tx);
}

fn refused_destinations(kind: PromptKind, targets: &[FileEntry]) -> RefusedFolders {
    if !matches!(kind, PromptKind::MoveTo | PromptKind::CopyTo) {
        return RefusedFolders::default();
    }
    let trees = targets
        .iter()
        .filter(|entry| entry.is_directory())
        .filter_map(|entry| entry.location.native_path().map(Path::to_path_buf))
        .collect();
    let mut parents = targets.iter().map(|entry| {
        entry
            .location
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    });
    let first = parents.next().flatten();
    let folder = (kind == PromptKind::MoveTo && parents.all(|parent| parent == first))
        .then_some(first)
        .flatten();
    RefusedFolders { trees, folder }
}

fn deliver_empty(app: &mut App, generation: u64, hint: Option<&str>, tx: &mut Tx) {
    let polled = app.tenxer.prompt.deliver(Candidates {
        generation,
        items: Vec::new(),
        done: true,
    });
    let serial = app.tenxer.prompt.serial();
    let _ = app.tenxer.prompt.show_hint(serial, hint.map(tr));
    prompt_builder(app).route(app, polled, tx);
}

fn complete_folder(app: &mut App, kind: PromptKind, query: CompletionQuery, tx: &mut Tx) {
    let current = active_folder(app);
    let home = glib::home_dir();
    if kind == PromptKind::Go && looks_like_uri(query.text.trim()) {
        return deliver_empty(app, query.generation, None, tx);
    }
    let scope = match picker_scope(&query.text, current.as_deref(), &home) {
        Ok(Some(scope)) => scope,
        Ok(None) => return deliver_empty(app, query.generation, None, tx),
        Err(reason) => return deliver_empty(app, query.generation, Some(reason), tx),
    };
    let refused = refused_destinations(kind, &app.tenxer.prompt_state.destination_targets);
    let typed =
        (scope.query.is_empty() && !refused.refuses(&scope.base)).then(|| scope.base.clone());
    if let Some(typed_dir) = &typed {
        let items = to_candidates(std::slice::from_ref(typed_dir), current.as_deref(), &home);
        let polled = app.tenxer.prompt.deliver(Candidates {
            generation: query.generation,
            items,
            done: false,
        });
        prompt_builder(app).route(app, polled, tx);
    }
    start_folder_index(app, scope, refused, typed, query.generation);
}

fn start_folder_index(
    app: &mut App,
    scope: PickerScope,
    refused: RefusedFolders,
    typed: Option<PathBuf>,
    prompt_gen: u64,
) {
    let current = active_folder(app);
    let below_open = current.as_deref() == Some(scope.base.as_path());
    let show_hidden = app.browser.preferences().show_hidden || wants_hidden(&scope.query);
    let input = Input {
        scope: Scope::Folders {
            root: scope.base,
            refused,
        },
        show_hidden,
    };
    let limits = filter::limits(shell::preferences(app));
    let session = app.tenxer.prompt_state.completing.session.clone();
    let later = app.tenxer.later.clone();
    let run_gen = app.tenxer.prompt_state.completing.generation;
    let query = scope.query;
    let launch = move || {
        session.update(input, &query, limits, move |batch| {
            let typed = typed.clone();
            later.then(move |app, tx| {
                if app.tenxer.prompt_state.completing.generation != run_gen {
                    return false;
                }
                deliver_folder_batch(app, prompt_gen, typed, batch, tx);
                true
            });
        });
    };
    if below_open {
        launch();
    } else {
        let delayed = app.tenxer.prompt_state.completing.delayed.clone();
        let source = glib::timeout_add_local_once(TYPED_BASE_DELAY, move || {
            delayed.borrow_mut().take();
            launch();
        });
        app.tenxer
            .prompt_state
            .completing
            .delayed
            .replace(Some(source));
    }
}

fn deliver_folder_batch(
    app: &mut App,
    generation: u64,
    typed: Option<PathBuf>,
    batch: filter::session::Batch,
    tx: &mut Tx,
) {
    let paths: Vec<PathBuf> = typed
        .into_iter()
        .chain(batch.items.into_iter().map(|item| item.path))
        .collect();
    let hint = match (paths.is_empty(), batch.indexing) {
        (false, _) => None,
        (true, true) => Some(tr(SEARCHING)),
        (true, false) => Some(tr(NO_MATCHES)),
    };
    let current = active_folder(app);
    let home = glib::home_dir();
    let items = to_candidates(&paths, current.as_deref(), &home);
    let serial = app.tenxer.prompt.serial();
    let _ = app.tenxer.prompt.show_hint(serial, hint);
    let polled = app.tenxer.prompt.deliver(Candidates {
        generation,
        items,
        done: !batch.indexing,
    });
    prompt_builder(app).route(app, polled, tx);
}

pub(super) struct SubmitArgs {
    pub(super) kind: PromptKind,
    pub(super) text: String,
    pub(super) chosen: Option<String>,
    pub(super) stepped: bool,
    pub(super) settled: bool,
    pub(super) serial: u64,
}

pub(super) fn submit_history(app: &mut App, chosen: Option<String>, serial: u64, tx: &mut Tx) {
    let Some(id) = chosen else {
        let _ = app.tenxer.prompt.show_hint(serial, Some(tr(NO_MATCHES)));
        return;
    };
    close_and_return(app, tx);
    shell::publish(app);
    app.browser
        .navigate_with_selection(Location::local(PathBuf::from(id)), true);
    shell::settle(app, tx);
}

pub(super) fn submit_folder(app: &mut App, args: SubmitArgs, tx: &mut Tx) {
    if args.stepped {
        return finish_folder_submit(app, args, None, tx);
    }
    let current = active_folder(app);
    let home = glib::home_dir();
    let Some(target) = typed_target(&args.text, current.as_deref(), &home) else {
        return wait_or_finish(app, args, tx);
    };
    let later = app.tenxer.later.clone();
    let nav = app.browser.navigation_generation();
    glib::MainContext::default().spawn_local(async move {
        let probed = target.clone();
        let exists = gio::spawn_blocking(move || probed.exists())
            .await
            .unwrap_or(false);
        later.then(move |app, tx| {
            if app.tenxer.prompt.kind() != Some(args.kind)
                || app.tenxer.prompt.serial() != args.serial
                || app.browser.navigation_generation() != nav
            {
                return false;
            }
            if exists {
                finish_folder_submit(app, args, Some(target), tx);
            } else {
                wait_or_finish(app, args, tx);
            }
            true
        });
    });
}

fn wait_or_finish(app: &mut App, args: SubmitArgs, tx: &mut Tx) {
    if !args.settled && app.tenxer.prompt.wait_until_settled(args.serial) {
        let _ = app.tenxer.prompt.show_hint(args.serial, Some(tr(SEARCHING)));
        return;
    }
    finish_folder_submit(app, args, None, tx);
}

fn finish_folder_submit(app: &mut App, args: SubmitArgs, typed: Option<PathBuf>, tx: &mut Tx) {
    if args.kind == PromptKind::Go {
        submit_go(app, &args.text, args.chosen, typed.is_some(), tx);
        return;
    }
    if args.text.trim().is_empty() || app.tenxer.prompt_state.destination_targets.is_empty() {
        close_and_return(app, tx);
        return;
    }
    let destination = typed.or_else(|| args.chosen.map(PathBuf::from));
    let Some(destination) = destination else {
        let current = active_folder(app);
        let reason = picker_scope(&args.text, current.as_deref(), &glib::home_dir())
            .err()
            .unwrap_or(NO_MATCHES);
        let _ = app.tenxer.prompt.show_hint(args.serial, Some(tr(reason)));
        return;
    };
    send_to_destination(app, args.kind, args.serial, destination);
}

fn submit_go(
    app: &mut App,
    text: &str,
    chosen: Option<String>,
    typed_exists: bool,
    tx: &mut Tx,
) {
    let picked = (!typed_exists).then_some(chosen).flatten();
    close_and_return(app, tx);
    if text.trim().is_empty() {
        return;
    }
    shell::publish(app);
    if let Some(folder) = picked {
        app.browser
            .navigate_with_selection(Location::local(PathBuf::from(folder)), true);
        shell::settle(app, tx);
        return;
    }
    let input = if text.contains("://") {
        sanitize_uri_credentials(text).map(|(sanitized, _)| sanitized)
    } else {
        Ok(text.to_owned())
    };
    let result = input.and_then(|cleaned| app.browser.navigate_input_located(&cleaned, None));
    shell::settle(app, tx);
    if let Err(error) = result {
        open_failed::typed_refused(app, &error);
    }
}

fn send_to_destination(app: &mut App, kind: PromptKind, serial: u64, destination: PathBuf) {
    let mut targets = app.tenxer.prompt_state.destination_targets.clone();
    if targets.is_empty() {
        return;
    }
    let nav = app.browser.navigation_generation();
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let check = gio::spawn_blocking(move || match std::fs::metadata(&destination) {
            Ok(meta) if meta.is_dir() => Ok(destination),
            Ok(_) => Err("Not a folder"),
            Err(_) => Err("No such folder"),
        })
        .await;
        later.then(move |app, tx| {
            if app.tenxer.prompt.kind() != Some(kind)
                || app.tenxer.prompt.serial() != serial
                || app.browser.navigation_generation() != nav
            {
                return false;
            }
            let outcome = match check {
                Ok(Ok(dest)) if kind == PromptKind::ExtractTo => {
                    archive::extract_entry_to(app, targets.remove(0), Location::local(dest), tx);
                    Ok(())
                }
                Ok(Ok(dest)) => validate_and_transfer(app, targets, dest, kind == PromptKind::MoveTo),
                Ok(Err(reason)) => Err(reason),
                Err(_) => Err("Unable to check folder"),
            };
            match outcome {
                Ok(()) => close_and_return(app, tx),
                Err(reason) => {
                    let _ = app.tenxer.prompt.show_hint(serial, Some(tr(reason)));
                }
            }
            shell::settle(app, tx);
            true
        });
    });
}

fn validate_and_transfer(
    app: &mut App,
    entries: Vec<FileEntry>,
    destination: PathBuf,
    moving: bool,
) -> Result<(), &'static str> {
    let dest_loc = Location::local(&destination);
    let dest_file = gio_file_for_location(&dest_loc);
    let inside_self = entries.iter().any(|entry| {
        let source = gio_file_for_location(&entry.location);
        entry.is_directory() && (dest_file.equal(&source) || dest_file.has_prefix(&source))
    });
    if inside_self {
        return Err("Can\u{2019}t put a folder inside itself");
    }
    let same_parent = moving
        && entries.iter().all(|entry| {
            gio_file_for_location(&entry.location)
                .parent()
                .is_some_and(|parent| parent.equal(&dest_file))
        });
    if same_parent {
        return Err("Already in this folder");
    }
    let sources = entries.into_iter().map(|entry| entry.location).collect();
    transfer::start_in_place_transfer(app, dest_loc, sources, moving);
    Ok(())
}
