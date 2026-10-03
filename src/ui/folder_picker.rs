// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::{Component, Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::glib;

use super::search_session::{SearchBatch, SearchInput, SearchScope, SearchSession};
pub(crate) use crate::services::RefusedFolders as Refused;

pub(crate) const NO_MATCHES: &str = "No matching folders";
pub(crate) const SEARCHING: &str = "Searching\u{2026}";
// Avoid crawling intermediate roots while a path is still being typed.
const TYPED_BASE_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PickerScope {
    pub(crate) base: PathBuf,
    pub(crate) query: String,
}

pub(crate) fn scope(
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
    let base = if stem == "~" {
        home.to_path_buf()
    } else if let Some(relative) = stem.strip_prefix("~/") {
        home.join(relative)
    } else if stem.starts_with('/') {
        PathBuf::from(stem)
    } else {
        current.ok_or("Type a full path here")?.join(stem)
    };
    Ok(Some(PickerScope {
        base: normalize(&base),
        query: query.trim().to_owned(),
    }))
}

pub(crate) fn typed_target(text: &str, current: Option<&Path>, home: &Path) -> Option<PathBuf> {
    split_typed_path(text.trim())?;
    let scope = scope(text, current, home).ok()??;
    Some(normalize(&scope.base.join(scope.query)))
}

/// The trailing slash makes the selected folder the next search root.
pub(crate) fn typed_path(folder: &Path, current: Option<&Path>, home: &Path) -> String {
    let below = |base: &Path, prefix: &str| {
        let relative = folder.strip_prefix(base).ok()?;
        Some(if relative.as_os_str().is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}{}/", relative.to_string_lossy())
        })
    };
    current
        .and_then(|current| below(current, "./"))
        .or_else(|| below(home, "~/"))
        .unwrap_or_else(|| {
            let absolute = folder.to_string_lossy();
            if absolute.ends_with('/') {
                absolute.into_owned()
            } else {
                format!("{absolute}/")
            }
        })
}

const LOCATION_SCHEMES: [&str; 10] = [
    "smb", "sftp", "ftp", "ftps", "dav", "davs", "trash", "network", "recent", "file",
];

// A colon alone is legal in a filename; don't treat every such name as a URI.
pub(crate) fn looks_like_uri(text: &str) -> bool {
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

fn is_scheme(text: &str) -> bool {
    text.starts_with(|character: char| character.is_ascii_alphabetic())
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
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

// Preserve logical paths rather than following symlinks.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}

fn wants_hidden(query: &str) -> bool {
    query.split_whitespace().any(|term| {
        term.trim_start_matches(['!', '^', '\'']).starts_with('.') || term.contains("/.")
    })
}

pub(crate) struct Request<'a> {
    pub(crate) text: &'a str,
    pub(crate) current: Option<&'a Path>,
    pub(crate) home: &'a Path,
    pub(crate) show_hidden: bool,
    /// Go accepts URI input without searching it; transfer prompts reject it.
    pub(crate) uris: bool,
    pub(crate) refused: Refused,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) hint: Option<&'static str>,
}

impl Shown {
    fn hint(hint: &'static str) -> Self {
        Self {
            paths: Vec::new(),
            hint: Some(hint),
        }
    }
}

type Deferred = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

#[derive(Clone, Default)]
pub(crate) struct FolderPicker {
    session: SearchSession,
    pending: Rc<Cell<bool>>,
    deferred: Deferred,
    delayed: Rc<RefCell<Option<glib::SourceId>>>,
}

impl FolderPicker {
    pub(crate) fn update(&self, request: Request<'_>, show: impl Fn(Shown) + 'static) {
        self.deferred.take();
        self.stop_delay();
        show(Shown::default());
        if request.uris && looks_like_uri(request.text.trim()) {
            return self.settle(&show, Shown::default());
        }
        let scope = match scope(request.text, request.current, request.home) {
            Ok(Some(scope)) => scope,
            Ok(None) => return self.settle(&show, Shown::default()),
            Err(reason) => return self.settle(&show, Shown::hint(reason)),
        };
        let refused = request.refused;
        let typed =
            (scope.query.is_empty() && !refused.refuses(&scope.base)).then(|| scope.base.clone());
        if let Some(typed) = &typed {
            show(Shown {
                paths: vec![typed.clone()],
                hint: None,
            });
        }
        let below_open_folder = request.current == Some(scope.base.as_path());
        let input = SearchInput {
            root: scope.base,
            show_hidden: request.show_hidden || wants_hidden(&scope.query),
            scope: SearchScope::Folders,
            refused,
        };
        let pending = self.pending.clone();
        let deferred = self.deferred.clone();
        pending.set(typed.is_none());
        let deliver: Rc<dyn Fn(SearchBatch)> = Rc::new(move |batch: SearchBatch| {
            let paths: Vec<_> = typed
                .iter()
                .cloned()
                .chain(batch.items.into_iter().map(|item| item.path))
                .collect();
            let hint = match (paths.is_empty(), batch.indexing) {
                (false, _) => None,
                (true, true) => Some(SEARCHING),
                (true, false) => Some(NO_MATCHES),
            };
            show(Shown { paths, hint });
            // Early Enter must not commit to a partial ranking.
            if !batch.indexing {
                pending.set(false);
                let action = deferred.take();
                if let Some(action) = action {
                    action();
                }
            }
        });
        if below_open_folder || self.session.searches(&input) {
            return self.session.update(input, &scope.query, false, deliver);
        }
        self.session.cancel();
        let session = self.session.clone();
        let delayed = self.delayed.clone();
        let query = scope.query;
        let source = glib::timeout_add_local_once(TYPED_BASE_DELAY, move || {
            delayed.take();
            session.update(input, &query, false, deliver);
        });
        self.delayed.replace(Some(source));
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.get()
    }

    /// Editing or cancelling invalidates the deferred submission.
    pub(crate) fn when_settled(&self, action: impl FnOnce() + 'static) {
        if self.pending.get() {
            self.deferred.replace(Some(Box::new(action)));
        } else {
            action();
        }
    }

    pub(crate) fn cancel(&self) {
        self.pending.set(false);
        self.deferred.take();
        self.stop_delay();
        self.session.cancel();
    }

    fn stop_delay(&self) {
        if let Some(source) = self.delayed.take() {
            source.remove();
        }
    }

    fn settle(&self, show: &impl Fn(Shown), shown: Shown) {
        self.cancel();
        show(shown);
    }
}

#[cfg(test)]
mod tests;
