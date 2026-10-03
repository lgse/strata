// SPDX-License-Identifier: MIT

//! The 10xer folder picker behind **g Space**, **M**, **C**, and **; E**. Typed terms
//! match the paths of folders below the open folder, fzf style, as **s** does.
//! Text that starts as a path (`/`, `~`, `./`, `../`) searches below the
//! folder it names instead; whatever follows its last `/` is the query.

use std::{
    cell::{Cell, RefCell},
    path::{Component, Path, PathBuf},
    rc::Rc,
};

use super::search_session::{SearchInput, SearchScope, SearchSession};

pub(crate) const NO_MATCHES: &str = "No matching folders";
pub(crate) const SEARCHING: &str = "Searching\u{2026}";

/// Where a picker searches, and for what. An empty query offers `base`
/// itself, then every folder below it, the most visited first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PickerScope {
    pub(crate) base: PathBuf,
    pub(crate) query: String,
}

/// Reads typed text as a search below `current`, or below the folder a typed
/// path names. Empty text searches nothing.
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

/// How **Tab** writes a picked folder back into the prompt: below the open
/// folder as `./`, below home as `~/`, otherwise absolute. The trailing `/`
/// makes the folder the search base, so it is offered alone and more typing
/// searches inside it.
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

/// A scheme (`sftp:`), `//host`, `\\host`, or `user@host:`. Such text is
/// never searched or probed.
pub(crate) fn looks_like_uri(text: &str) -> bool {
    text.starts_with("//")
        || text.starts_with('\\')
        || text
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
}

/// Splits text that starts as a path into the folder part, through its last
/// `/`, and the query after it.
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

/// Resolves `.` and `..` the way a shell's `cd` does, without following links.
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

/// Hidden folders are offered when the listing shows them or a term asks for
/// one by its leading dot.
fn wants_hidden(query: &str) -> bool {
    query.split_whitespace().any(|term| {
        term.trim_start_matches(['!', '^', '\'']).starts_with('.') || term.contains("/.")
    })
}

/// Folders a transfer would refuse, so they are never offered by a search.
#[derive(Clone, Debug, Default)]
pub(crate) struct Refused {
    /// Folders being moved or copied, which cannot hold themselves.
    pub(crate) trees: Vec<PathBuf>,
    /// The folder a move would leave its items in.
    pub(crate) folder: Option<PathBuf>,
}

impl Refused {
    fn refuses(&self, path: &Path) -> bool {
        self.folder.as_deref() == Some(path) || self.trees.iter().any(|tree| path.starts_with(tree))
    }
}

pub(crate) struct Request<'a> {
    pub(crate) text: &'a str,
    pub(crate) current: Option<&'a Path>,
    pub(crate) home: &'a Path,
    pub(crate) show_hidden: bool,
    /// Whether URIs are accepted as typed rather than refused; either way
    /// they list nothing.
    pub(crate) uris: bool,
    pub(crate) refused: Refused,
}

/// What the prompt should list, or why it lists nothing.
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
}

impl FolderPicker {
    /// Lists folders for `request`. Searches report to `show` as results
    /// arrive; everything else reports before this returns.
    pub(crate) fn update(&self, request: Request<'_>, show: impl Fn(Shown) + 'static) {
        self.deferred.take();
        if request.uris && looks_like_uri(request.text.trim()) {
            return self.settle(&show, Shown::default());
        }
        let scope = match scope(request.text, request.current, request.home) {
            Ok(Some(scope)) => scope,
            Ok(None) => return self.settle(&show, Shown::default()),
            Err(reason) => return self.settle(&show, Shown::hint(reason)),
        };
        // The typed folder leads, so Enter picks it however the rest ranks.
        let typed = scope.query.is_empty().then(|| scope.base.clone());
        if let Some(typed) = &typed {
            show(Shown {
                paths: vec![typed.clone()],
                hint: None,
            });
        }
        let input = SearchInput {
            root: scope.base,
            show_hidden: request.show_hidden || wants_hidden(&scope.query),
            scope: SearchScope::Folders,
        };
        let refused = request.refused;
        let pending = self.pending.clone();
        let deferred = self.deferred.clone();
        pending.set(typed.is_none());
        self.session.update(
            input,
            &scope.query,
            false,
            Rc::new(move |batch| {
                let paths: Vec<_> = typed
                    .iter()
                    .cloned()
                    .chain(
                        batch
                            .items
                            .into_iter()
                            .map(|item| item.path)
                            .filter(|path| !refused.refuses(path)),
                    )
                    .collect();
                let hint = match (paths.is_empty(), batch.indexing) {
                    (false, _) => None,
                    (true, true) => Some(SEARCHING),
                    (true, false) => Some(NO_MATCHES),
                };
                let settled = hint != Some(SEARCHING);
                show(Shown { paths, hint });
                if settled {
                    pending.set(false);
                    let action = deferred.take();
                    if let Some(action) = action {
                        action();
                    }
                }
            }),
        );
    }

    /// Whether the listed folders are from earlier text than the prompt's,
    /// or a search has found nothing yet but is still looking.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.get()
    }

    /// Runs `action` now, or once the search for the current text lists a
    /// folder or finishes. Editing the text or cancelling drops it.
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
        self.session.cancel();
    }

    fn settle(&self, show: &impl Fn(Shown), shown: Shown) {
        self.cancel();
        show(shown);
    }
}

#[cfg(test)]
mod tests;
