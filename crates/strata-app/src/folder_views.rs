// SPDX-License-Identifier: MIT

//! The per-folder views this process keeps, `folder-views.toml`: the one
//! [`FolderViewStore`] of the process, and when it is written. The store's rules are its
//! module's (`strata_desktop::settings::folder_view_store`); which values it keeps is
//! `strata_desktop::settings::folder_view_policy`'s, shared with the GTK application. When a
//! write happens stays in each interface, by the owner's ruling (`docs/rune/shared-state.md`),
//! so this is the GTK manager's `schedule_folder_views_write`, `write_folder_views` and
//! `folder_views_saved` (`src/ui/preferences/folder_view_settings.rs`) on Rune's loop:
//!
//! - **Loaded at start-up,** once, with the window.
//! - **One timer.** A change asks for a write [`SAVE_DELAY`] later; a pending write keeps its
//!   time unless a shorter wait is asked for. It is stamped with the window's clock when it is
//!   asked (`Tx::clock`) and fired from the poll, so it follows Rune's clock, virtual in tests.
//! - **A write's result** comes back through the window's queue: `Busy` asks again
//!   [`SAVE_DELAY`] later; a write that reached the file adopts what another process had saved,
//!   publishes it and then counts its own changes as saved, in that order.
//! - **At exit** a store with unsaved changes writes once more, with `exit` set
//!   ([`flush_at_exit`], from the window's `on_close` and `on_quit`), before the settings
//!   writer is flushed.
//! - **What another process saves** is read again when the file changes, and once when the
//!   monitor is live, and adopted under this process's unsaved changes.
//!
//! The GTK notices (saving off for a file that could not be read, a write that failed, and the
//! notice cleared by the next write that succeeds) are only logged: this interface has no
//! notice yet. A view reads a folder's values with [`views`], and follows a change by
//! [`revision`].
//!
//! **The folders' sorts** (A10's): while `remember_folder_views` is on, the browser resolves a
//! folder's sort through the store ([`Kept::follow_sorts`], GTK's `set_folder_sorts` binding),
//! so a folder opens in its saved sort as its column is made. A folder only opened counts as
//! used, and a write is asked [`USE_SAVE_DELAY`] later. A sort chosen in a remembered folder is
//! kept for it ([`chosen`], `BrowserEvent::FolderSortChosen`), and when the folders' sorts
//! change, here or in another process, the browser's columns follow
//! (`resync_column_sorts`). The policy is `strata_desktop::settings::folder_sort_policy`.
//!
//! The store is shared with the browser's resolver, which the browser calls while it opens a
//! folder: a caller of [`views`] drops the borrow before it calls into the browser.

use std::{
    cell::{Cell, Ref, RefCell},
    rc::Rc,
    time::Duration,
};

use rune::Tx;
use strata_core::model::{FolderSort, LoadHandle, Location, SortOptions};
use strata_desktop::settings::{
    FOLDER_VIEWS_SAVE_DELAY as SAVE_DELAY, FOLDER_VIEWS_USE_SAVE_DELAY as USE_SAVE_DELAY,
    FolderViewStore, Preferences, UnreadableAtStartup, Written, folder_sort_policy,
    folder_views::{FolderViews, key_for_location},
    folder_views_path,
};
use strata_fs::{
    adapters::{Watched, watch},
    app::{Browser, FolderSortResolver},
};

use crate::{App, slots::Later};

/// The one timer a write waits on, on the window's clock (`Tx::clock`).
#[derive(Debug, Default)]
struct Timer {
    /// When the write is due, and the delay it was asked with.
    due: Option<(f64, Duration)>,
}

/// What a poll finds of the timer.
#[derive(Debug, PartialEq)]
enum Tick {
    Idle,
    Wait(Duration),
    Due,
}

impl Timer {
    /// Asks at `clock` for a write `delay` later: a pending write keeps its time unless this
    /// wait is shorter. When the timer took it, the time it is due, for the caller to wake at.
    fn ask(&mut self, clock: f64, delay: Duration) -> Option<f64> {
        if self.due.is_some_and(|(_, held)| held <= delay) {
            return None;
        }
        let due = clock + delay.as_secs_f64();
        self.due = Some((due, delay));
        Some(due)
    }

    fn tick(&mut self, clock: f64) -> Tick {
        match self.due {
            None => Tick::Idle,
            Some((due, _)) if clock >= due => {
                self.due = None;
                Tick::Due
            }
            Some((due, _)) => Tick::Wait(Duration::from_secs_f64(due - clock)),
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The process's store, its timer, and what follows the file.
pub(crate) struct Kept {
    /// Shared with the browser's resolver of folders' sorts ([`Kept::follow_sorts`]).
    store: Rc<RefCell<FolderViewStore>>,
    timer: Timer,
    /// Moves whenever the values change, here or from another process.
    revision: u64,
    later: Later<App>,
    /// Whether the last write before the process ends has started: closing the window and
    /// quitting may both ask for it. Strata does not use Rune's `keep_running`; if it ever
    /// does, a closed window can open again, and this must be cleared on reopen, or dropped.
    exiting: bool,
    /// Follows what another Strata process, such as the portal chooser, saves.
    _monitor: LoadHandle,
    /// The browser's resolver counted a folder only opened as used: a write is owed.
    used: Rc<Cell<bool>>,
    /// Moves when the folders' sorts may have changed, and what the browser's columns last
    /// followed.
    sorts: (u64, u64),
}

impl Kept {
    /// Reads the file, once in a process, and follows it through the window's queue.
    pub(crate) fn load(later: Later<App>) -> Self {
        let monitor = follow_file(later.clone());
        Self {
            store: Rc::new(RefCell::new(FolderViewStore::load())),
            timer: Timer::default(),
            revision: 0,
            exiting: false,
            later,
            _monitor: monitor,
            used: Rc::default(),
            sorts: (0, 0),
        }
    }

    /// Gives `browser` the folders' sorts while `remember_folder_views` is on, as GTK's
    /// binding does (`set_folder_sorts`), before its first folder opens. The browser resolves
    /// a folder's sort as its column is made, from the store.
    pub(crate) fn follow_sorts(&self, browser: &Rc<Browser>, saved: &Preferences) {
        if saved.remember_folder_views {
            browser.set_folder_sorts(Some(self.resolver()));
        }
    }

    /// `resolve_folder_sort`: a remembered folder's own sort, or the default.
    fn resolver(&self) -> FolderSortResolver {
        let (store, used) = (self.store.clone(), self.used.clone());
        Rc::new(move |location: &Location, opened: bool| {
            // The browser holds it only while folders' views are remembered.
            let Some(key) = key_for_location(location) else {
                return FolderSort::Unremembered;
            };
            let mut store = store.borrow_mut();
            let (sort, touched) = folder_sort_policy::folder_sort(store.views_mut(), &key, opened);
            if touched {
                used.set(true);
            }
            sort
        })
    }

    /// Asks for a write `delay` from now, unless one is due sooner (`schedule_folder_views_write`).
    fn schedule(&mut self, delay: Duration, tx: &mut Tx) {
        if self.store.borrow().load_failure().is_some() {
            self.write(false);
            return;
        }
        if let Some(due) = self.timer.ask(tx.clock(), delay) {
            tx.wake_at(due);
        }
    }

    /// Starts a write of the values as they are (`write_folder_views`).
    fn write(&mut self, exit: bool) {
        self.timer.clear();
        let pending = match self.store.borrow_mut().begin_write(exit) {
            Ok(pending) => pending,
            Err(UnreadableAtStartup { path, error }) => {
                // The GTK window shows that saving is off; this interface has no notice yet.
                tracing::warn!(%error, path = %path.display(), "folder views can't be saved");
                return;
            }
        };
        let later = self.later.clone();
        glib::MainContext::default().spawn_local(async move {
            let written = pending.finished().await;
            later.then(move |app: &mut App, tx: &mut Tx| saved(app, written, tx));
        });
    }
}

/// The values, as a view reads a folder's. The borrow is dropped before anything calls into
/// the browser, whose resolver borrows the store.
pub(crate) fn views(app: &App) -> Ref<'_, FolderViews> {
    Ref::map(
        app.window.folder_views().store.borrow(),
        FolderViewStore::views,
    )
}

/// Moves whenever the values change: a view that shows a folder's value reads it again.
pub(crate) fn revision(app: &App) -> u64 {
    app.window.folder_views().revision
}

/// Changes the values; when `change` says they changed, a write is asked for.
pub(crate) fn change(app: &mut App, change: impl FnOnce(&mut FolderViews) -> bool, tx: &mut Tx) {
    let kept = app.window.folder_views_mut();
    let changed = change(kept.store.borrow_mut().views_mut());
    if changed {
        kept.revision += 1;
        kept.schedule(SAVE_DELAY, tx);
    }
}

/// A sort chosen in `location`, a remembered folder (`BrowserEvent::FolderSortChosen`): kept
/// for it as `folder_sort_policy` says (`set_folder_sort_options`).
pub(crate) fn chosen(app: &mut App, location: &Location, options: SortOptions, tx: &mut Tx) {
    let settings = crate::shell::preferences(app);
    let kept = folder_sort_policy::keep_folder_sort(settings, location, options);
    if let Some(kept) = kept {
        change(app, |views| kept.apply(views), tx);
        app.window.folder_views_mut().sorts.0 += 1;
    }
}

/// From the window's poll: starts the write when it is due.
pub(crate) fn poll(app: &mut App, clock: f64, tx: &mut Tx) {
    let kept = app.window.folder_views_mut();
    if kept.used.take() {
        kept.schedule(USE_SAVE_DELAY, tx);
    }
    match kept.timer.tick(clock) {
        Tick::Idle => {}
        Tick::Wait(left) => tx.wake_after(left),
        Tick::Due => kept.write(false),
    }
    // The folders' sorts changed: the open columns follow (`folder_sorts_revision`'s binding).
    let (changed, followed) = kept.sorts;
    if changed != followed {
        kept.sorts.1 = changed;
        app.browser.resync_column_sorts();
    }
}

/// The last write, before the process ends, when anything is unsaved (`flush_folder_views`).
/// Once: the window's `on_close` and `on_quit` both call it, since a quit runs no `on_close`
/// and the last window closing may run both.
pub(crate) fn flush_at_exit(app: &mut App) {
    let kept = app.window.folder_views_mut();
    if !kept.exiting && kept.store.borrow().views().has_unsaved() {
        kept.exiting = true;
        kept.write(true);
    }
}

/// How a write went (`folder_views_saved`).
fn saved(app: &mut App, written: Written, tx: &mut Tx) -> bool {
    let kept = app.window.folder_views_mut();
    match written {
        Written::Busy => {
            kept.schedule(SAVE_DELAY, tx);
            false
        }
        Written::Failed { path, error } => {
            tracing::warn!(%error, path = %path.display(), "unable to save the folder views");
            false
        }
        Written::Saved { merged, write } => {
            // GTK clears its notice here, then shows what it adopted, then settles the write.
            let adopted = merged.is_some_and(|merged| kept.adopt(merged));
            kept.store.borrow_mut().finish_write(write);
            adopted
        }
    }
}

impl Kept {
    /// Merges this process's unsaved changes over what another one saved.
    fn adopt(&mut self, saved: FolderViews) -> bool {
        let changed = self.store.borrow_mut().adopt(saved);
        if changed {
            self.revision += 1;
            // What another process saved may hold other sorts.
            self.sorts.0 += 1;
        }
        changed
    }
}

/// Reads the file again whenever it changes, and once when the monitor is live, which covers
/// a save made between the load and the monitor starting; adopts what another process saved
/// (`merge_folder_views_saved_elsewhere`, which GTK's monitor calls both ways).
fn follow_file(later: Later<App>) -> LoadHandle {
    let live = later.clone();
    watch(
        &Location::local(folder_views_path()),
        Watched::File,
        move |_, _| reread(&later),
        move || reread(&live),
    )
}

fn reread(later: &Later<App>) {
    let later = later.clone();
    glib::MainContext::default().spawn_local(async move {
        if let Some(saved) = FolderViewStore::reread().await {
            later.then(move |app: &mut App, _: &mut Tx| app.window.folder_views_mut().adopt(saved));
        }
    });
}

#[cfg(test)]
mod tests;
