// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rune::Tx;
use strata_platform::clipboard::{
    Change, Clipboard, Content, CopyId, CopyState, Error, Files, Limits, ReadError, Selection,
    TestRoute,
    files::{FileUri, file_path},
};

/// How a row is marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mark {
    #[default]
    None,
    Copied,
    Cut,
}

/// What a paste read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Paste {
    Files {
        paths: Vec<PathBuf>,
        moving: bool,
        foreign: Vec<String>,
    },
    /// An image and no files: PNG bytes.
    Image(Vec<u8>),
    /// Nothing to paste.
    Nothing(ReadError),
}

/// What the clipboard should do after a cut's paste ([`Board::moved`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum After<I> {
    /// Nothing: no cut of ours was consumed.
    Keep,
    /// Empty the clipboard if copy `I` still holds it.
    Clear(I),
    /// Copy the rest again, as a cut, if copy `I` still holds the clipboard.
    Recut(I, Vec<PathBuf>),
}

#[derive(Clone, Debug)]
struct Held<I> {
    copy: I,
    paths: Vec<PathBuf>,
    cut: bool,
}

#[derive(Clone, Debug)]
pub struct Board<I> {
    held: Option<Held<I>>,
    marks: HashMap<PathBuf, Mark>,
    badge: bool,
    /// Bumped by every change, so a file read that answers after a newer change is dropped.
    read: u64,
}

impl<I> Default for Board<I> {
    fn default() -> Self {
        Board {
            held: None,
            marks: HashMap::new(),
            badge: false,
            read: 0,
        }
    }
}

impl<I: Copy + PartialEq> Board<I> {
    pub fn mark(&self, path: &Path) -> Mark {
        self.marks.get(path).copied().unwrap_or_default()
    }

    pub fn badge(&self) -> bool {
        self.badge
    }

    /// Our copy, while it is ours.
    pub fn held(&self) -> Option<I> {
        self.held.as_ref().map(|h| h.copy)
    }

    /// Copy `copy` of `paths` was made (cut or not): it replaces our earlier one.
    pub fn copied(&mut self, copy: I, paths: Vec<PathBuf>, cut: bool) {
        let mark = if cut { Mark::Cut } else { Mark::Copied };
        self.marks = paths.iter().map(|p| (p.clone(), mark)).collect();
        self.held = Some(Held { copy, paths, cut });
        self.badge = true;
        self.read += 1;
    }

    pub fn lost(&mut self, copy: I) {
        if self.held() == Some(copy) {
            self.forget();
        }
    }

    pub fn offered(&mut self, may_hold_files: bool) -> Option<u64> {
        self.forget();
        may_hold_files.then_some(self.read)
    }

    /// Nobody holds the clipboard.
    pub fn emptied(&mut self) {
        self.forget();
    }

    /// Read `read` answered: whether the clipboard held any file.
    pub fn files_read(&mut self, read: u64, any: bool) {
        if read == self.read {
            self.badge = any;
        }
    }

    /// Whether `paths` are exactly what we cut (in any order).
    pub fn is_our_cut(&self, paths: &[PathBuf]) -> bool {
        let Some(h) = self.held.as_ref().filter(|h| h.cut) else {
            return false;
        };
        let ours: HashSet<&PathBuf> = h.paths.iter().collect();
        let theirs: HashSet<&PathBuf> = paths.iter().collect();
        !paths.is_empty() && h.paths.len() == paths.len() && ours == theirs
    }

    /// A paste moved `moved`: what we cut among them leaves the clipboard.
    pub fn moved(&mut self, moved: &[PathBuf]) -> After<I> {
        let Some(h) = self.held.as_mut().filter(|h| h.cut) else {
            return After::Keep;
        };
        let before = h.paths.len();
        h.paths.retain(|p| !moved.contains(p));
        if h.paths.len() == before {
            return After::Keep;
        }
        for p in moved {
            self.marks.remove(p);
        }
        if h.paths.is_empty() {
            let copy = h.copy;
            self.forget();
            After::Clear(copy)
        } else {
            After::Recut(h.copy, h.paths.clone())
        }
    }

    /// Let go of our copy (Strata's unyank): the copy to empty the clipboard of, if any.
    pub fn release(&mut self) -> Option<I> {
        let held = self.held();
        self.forget();
        held
    }

    fn forget(&mut self) {
        self.held = None;
        self.marks.clear();
        self.badge = false;
        self.read += 1;
    }
}

type At<S> = fn(&mut S) -> &mut FileClipboard<S>;

/// The file clipboard for app state `S`, which keeps it at `at`.
pub struct FileClipboard<S: 'static> {
    clip: Clipboard<S>,
    at: At<S>,
    board: Board<CopyId>,
}

impl<S: 'static> FileClipboard<S> {
    pub fn new(at: At<S>) -> Self {
        let clip = Clipboard::<S>::new().on_change(move |s, change, _| {
            if selection_of(change) != Selection::Clipboard {
                return;
            }
            let me = at(s);
            match change {
                Change::Holding { .. } => {}
                Change::Lost { copy, .. } => me.board.lost(*copy),
                Change::Emptied { .. } => me.board.emptied(),
                Change::Offered { mimes, .. } => me.heard_offer(mimes),
            }
        });
        FileClipboard {
            clip,
            at,
            board: Board::default(),
        }
    }

    /// Another app's copy: read its files, if it may hold some, for the badge.
    fn heard_offer(&mut self, mimes: &[String]) {
        let Some(read) = self.board.offered(Files::offered(mimes)) else {
            return;
        };
        let at = self.at;
        // Err only when not attached, where nothing is offered either: no badge then.
        let _ = self.clip.read_files(
            Selection::Clipboard,
            Limits::default(),
            move |s, files, _| {
                let any = files.is_ok_and(|f| !f.uris.is_empty());
                at(s).board.files_read(read, any);
            },
        );
    }

    pub fn test_route(mut self, route: TestRoute) -> Self {
        self.clip = self.clip.test_route(route);
        self
    }

    pub fn handle(&self) -> Clipboard<S> {
        self.clip.clone()
    }

    pub fn mark(&self, path: &Path) -> Mark {
        self.board.mark(path)
    }

    pub fn badge(&self) -> bool {
        self.board.badge()
    }

    pub fn copy(&mut self, paths: &[PathBuf], cut: bool) -> Result<CopyId, Error> {
        if paths.is_empty() {
            return Err(Error::Empty);
        }
        let files = Files::from_paths(paths, cut);
        let copy = self
            .clip
            .copy(Selection::Clipboard, Content::new().files(&files))?;
        self.board.copied(copy, paths.to_vec(), cut);
        Ok(copy)
    }

    pub fn paste(&self, f: impl Fn(&mut S, Paste, &mut Tx) + 'static) -> Result<(), Error> {
        let f = Rc::new(f);
        let (at, clip) = (self.at, self.clip.clone());
        self.clip.read_files(
            Selection::Clipboard,
            Limits::default(),
            move |s, files, tx| match files {
                Ok(files) => {
                    let paths = files.paths();
                    let foreign = files
                        .uris
                        .iter()
                        .filter(|u| !matches!(file_path(u), Ok(FileUri::Local(_))))
                        .cloned()
                        .collect();
                    let moving = files.cut || at(s).board.is_our_cut(&paths);
                    f(
                        s,
                        Paste::Files {
                            paths,
                            moving,
                            foreign,
                        },
                        tx,
                    );
                }
                Err(ReadError::NothingOffered) => {
                    let image = f.clone();
                    let asked = clip.read(
                        Selection::Clipboard,
                        &["image/png"],
                        Limits::default(),
                        move |s, png, tx| match png {
                            Ok(d) => image(s, Paste::Image(d.bytes), tx),
                            Err(e) => image(s, Paste::Nothing(e), tx),
                        },
                    );
                    if let Err(e) = asked {
                        f(s, Paste::Nothing(ReadError::Platform(e.to_string())), tx);
                    }
                }
                Err(e) => f(s, Paste::Nothing(e), tx),
            },
        )
    }

    pub fn moved(&mut self, moved: &[PathBuf]) -> Result<bool, Error> {
        match self.board.moved(moved) {
            After::Keep => Ok(false),
            After::Clear(copy) => self.clip.clear(copy),
            After::Recut(copy, rest) => match self.clip.state(copy) {
                Some(CopyState::Holding | CopyState::Pending) => {
                    self.copy(&rest, true).map(|_| true)
                }
                _ => {
                    self.board.lost(copy);
                    Ok(false)
                }
            },
        }
    }

    pub fn release(&mut self) -> Result<bool, Error> {
        match self.board.release() {
            Some(copy) => self.clip.clear(copy),
            None => Ok(false),
        }
    }
}

fn selection_of(change: &Change) -> Selection {
    match change {
        Change::Offered { selection, .. }
        | Change::Emptied { selection }
        | Change::Holding { selection, .. }
        | Change::Lost { selection, .. } => *selection,
    }
}

#[cfg(test)]
mod tests;
