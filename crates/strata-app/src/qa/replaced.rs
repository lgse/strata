// SPDX-License-Identifier: MIT

//! Which rows the browser has replaced since a step remembered one. GTK replaces a column's
//! model whole when the browser replaces its entries (`EntriesReplaced`, which a reload such
//! as F5 ends in: `on_entries_replaced` in `src/ui/browser/events/listing.rs`), so each of its
//! rows is a new widget, while the events that insert, splice or fill rows keep them. Rune's
//! row keeps its node, since the collection keys a row by its entry's id and an id is the
//! entry's location. So the backend counts the replacements and tells a row of a column
//! replaced since it was remembered from the row it was, as GTK's `replaced` and recall do.
//!
//! Only `EntriesReplaced` is modelled. GTK also gives a row a new widget when the view is
//! rebuilt (`rebuild_icons`, `rebuild_list`), when a filter hides the row and shows it again,
//! and when the row is scrolled out and back, where Rune keeps the row. A run that relies on
//! one of those needs this extended first.
//!
//! Each tab has a browser of its own (`shell::tabs`): each is watched from the first step
//! after its tab opens, and a row is asked after in the tab shown.

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

/// The replacements every tab's browser makes, counted across the window, and asked of the
/// tab shown: a row is another row only when its own tab's column was replaced.
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
            let known = watched.iter().any(|watched| {
                std::ptr::eq(watched.browser.as_ptr(), Rc::as_ptr(browser))
            });
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

    /// Whether the entry the components call `id` was in a replacement of the shown tab's
    /// browser after `since`.
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
