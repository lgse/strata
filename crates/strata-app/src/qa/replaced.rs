// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use strata_components::contract::EntryId;
use strata_core::model::Location;
use strata_fs::app::{Browser, BrowserEvent};

use crate::{App, shell};

/// The replacements one tab's browser made: the last each entry was in, by the window's count.
struct Watched {
    browser: Weak<Browser>,
    last: HashMap<Location, u64>,
}

#[derive(Default)]
pub(super) struct Replacements {
    /// How many replacements the browsers have made.
    count: Cell<u64>,
    watched: RefCell<Vec<Watched>>,
}

impl Replacements {
    /// Counts the replacements each of `app`'s tabs' browsers makes from now on.
    pub(super) fn watch(app: &App) -> Rc<Self> {
        let replacements = Rc::new(Self::default());
        replacements.follow(app);
        replacements
    }

    /// Watches the browser of each tab opened since the last call too.
    pub(super) fn follow(self: &Rc<Self>, app: &App) {
        let mut watched = self.watched.borrow_mut();
        watched.retain(|watched| watched.browser.strong_count() > 0);
        for &key in shell::tabs::tabs(app) {
            let Some(browser) = shell::tabs::tab_browser(app, key) else {
                continue;
            };
            let known = watched
                .iter()
                .any(|watched| std::ptr::eq(watched.browser.as_ptr(), Rc::as_ptr(browser)));
            if known {
                continue;
            }
            let (mine, observed) = (Rc::downgrade(self), Rc::downgrade(browser));
            browser.observe(move |event| {
                if let BrowserEvent::EntriesReplaced { depth, .. } = event {
                    record(&mine, &observed, *depth);
                }
            });
            watched.push(Watched {
                browser: Rc::downgrade(browser),
                last: HashMap::new(),
            });
        }
    }

    /// The count as of now, which a remembered node keeps.
    pub(super) fn now(&self) -> u64 {
        self.count.get()
    }

    pub(super) fn replaced_since(&self, app: &App, id: EntryId, since: u64) -> bool {
        let watched = self.watched.borrow();
        let shown = watched
            .iter()
            .find(|watched| std::ptr::eq(watched.browser.as_ptr(), Rc::as_ptr(&app.browser)));
        shown.is_some_and(|shown| {
            shown
                .last
                .iter()
                .any(|(location, at)| *at > since && shell::known_id(app, location) == Some(id))
        })
    }
}

fn record(replacements: &Weak<Replacements>, browser: &Weak<Browser>, depth: usize) {
    let (Some(replacements), Some(browser)) = (replacements.upgrade(), browser.upgrade()) else {
        return;
    };
    let count = replacements.count.get() + 1;
    replacements.count.set(count);
    let mut watched = replacements.watched.borrow_mut();
    let Some(watched) = watched
        .iter_mut()
        .find(|watched| std::ptr::eq(watched.browser.as_ptr(), Rc::as_ptr(&browser)))
    else {
        return;
    };
    browser.with_column_entries(depth, |entries| {
        for entry in entries {
            watched.last.insert(entry.location.clone(), count);
        }
    });
}
