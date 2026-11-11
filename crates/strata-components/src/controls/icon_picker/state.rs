//! The picker's state: its source, what it shows, the search field and the grid's collection.

use std::rc::Rc;

use rune::{TextEdit, Tx};

use super::data::{Choice, PickerSource};
use super::matching::{Hit, Query};
use crate::collection::element::CollectionState;
use crate::collection::keyboard::Keyboard;
use crate::collection::selection::Listing as Loaded;
use crate::contract::{Entry, EntryId, EntryKind, EntrySource};

/// What the grid shows: the recent choices, one category, or a search's results.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    /// The recently used choices, newest first.
    Recent,
    /// A category, by its index in the source.
    Category(usize),
    /// What the query matches, in every category, in the source's order.
    Results,
}

/// The items the grid shows, as the collection element reads them.
#[derive(Debug)]
pub(super) struct Shown {
    source: Rc<PickerSource>,
    /// Item indices, in the order shown.
    items: Vec<u32>,
    /// Each item's place in `items`, or `u32::MAX`: [`EntrySource::index_of`] in O(1).
    place: Vec<u32>,
    id_base: u64,
}

impl Shown {
    fn new(source: Rc<PickerSource>, id_base: u64) -> Self {
        let place = vec![u32::MAX; source.items().len()];
        Self {
            source,
            items: Vec::new(),
            place,
            id_base,
        }
    }

    fn set(&mut self, items: Vec<u32>) {
        for &i in &self.items {
            self.place[i as usize] = u32::MAX;
        }
        for (at, &i) in items.iter().enumerate() {
            self.place[i as usize] = at as u32;
        }
        self.items = items;
    }

    /// The item shown at `index`.
    pub(super) fn item(&self, index: usize) -> Option<usize> {
        self.items.get(index).map(|&i| i as usize)
    }

    /// The item an entry id stands for.
    pub(super) fn item_of(&self, id: EntryId) -> Option<usize> {
        let item = usize::try_from(id.get().checked_sub(self.id_base)?).ok()?;
        (item < self.place.len()).then_some(item)
    }
}

impl EntrySource for Shown {
    fn len(&self) -> usize {
        self.items.len()
    }

    fn id_at(&self, index: usize) -> EntryId {
        EntryId::new(self.id_base + u64::from(self.items[index]))
    }

    fn index_of(&self, id: EntryId) -> Option<usize> {
        let item = self.item_of(id)?;
        let at = self.place[item];
        (at != u32::MAX).then_some(at as usize)
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let item = self.items[index] as usize;
        Entry::new(
            self.id_at(index),
            &self.source.items()[item].name,
            EntryKind::File,
            self.source.cell_key(item),
        )
    }
}

/// An icon picker's state. The host keeps one per picker and builds an `IconPicker` over it
/// each frame. The source, the recent choices and the current choice come from the host as
/// data; nothing here does I/O.
///
/// Used by: hosts.
#[derive(Debug)]
pub struct IconPickerState {
    pub(super) shown: Shown,
    pub(super) coll: CollectionState,
    pub(super) field: TextEdit,
    query: Query,
    recent: Vec<u32>,
    current: Option<u32>,
    page: Page,
    /// The page shown before a search began: where clearing the query goes back to.
    browse: Page,
    version: u64,
    open: bool,
    dismissed: bool,
}

impl IconPickerState {
    /// A picker over `source`, showing its first category.
    pub fn new(source: Rc<PickerSource>) -> Self {
        let mut st = Self {
            shown: Shown::new(source, 0),
            coll: fresh_collection(),
            field: TextEdit::new(""),
            query: Query::default(),
            recent: Vec::new(),
            current: None,
            page: Page::Category(0),
            browse: Page::Category(0),
            version: 0,
            open: false,
            dismissed: false,
        };
        st.page = st.first_page();
        st.browse = st.page;
        st.refresh(None);
        st
    }

    /// The same state with its cells' entry ids from `base` on. A cell's id is `base` plus its
    /// item's index in the source, and its key (`row-<id>`) lives in one Rune key namespace
    /// with every other picker's, so pickers shown at once need ranges `base..base + len`
    /// (`len` the source's item count, the largest source for one given `set_source`) that
    /// don't overlap: `0` and `10_000` for two pickers of up to 10,000 items, say.
    pub fn id_base(mut self, base: u64) -> Self {
        debug_assert!(
            base.checked_add(self.shown.place.len() as u64).is_some(),
            "id_base {base}: the ids overflow"
        );
        self.shown.id_base = base;
        self.refresh(None);
        self
    }

    /// The source.
    pub fn source(&self) -> &Rc<PickerSource> {
        &self.shown.source
    }

    /// Shows `source` instead, from its first page, with the query cleared. Recent and current
    /// choices it doesn't offer are dropped.
    pub fn set_source(&mut self, source: Rc<PickerSource>) {
        let recent: Vec<Choice> = self.recent_choices().cloned().collect();
        let current = self.current().cloned();
        let base = self.shown.id_base;
        self.shown = Shown::new(source, base);
        self.field.replace_text("");
        self.query = Query::default();
        self.set_recent(&recent);
        self.set_current(current.as_ref());
        self.page = self.first_page();
        self.browse = self.page;
        self.refresh(None);
    }

    /// The recently used choices, newest first: the host's list (see `remember`). Those the
    /// source doesn't offer are left out.
    pub fn set_recent(&mut self, recent: &[Choice]) {
        let source = &self.shown.source;
        let mut items: Vec<u32> = Vec::with_capacity(recent.len());
        for c in recent {
            if let Some(i) = source.find(c).map(|i| i as u32)
                && !items.contains(&i)
            {
                items.push(i);
            }
        }
        if items == self.recent {
            return;
        }
        self.recent = items;
        let showing = self.page == Page::Recent;
        if self.recent.is_empty() {
            // During a search `page` is Results and `browse` may still be Recent.
            let first = self.first_page();
            if self.page == Page::Recent {
                self.page = first;
            }
            if self.browse == Page::Recent {
                self.browse = first;
            }
        }
        if showing {
            self.refresh(None);
        }
    }

    /// The recent choices the picker shows, newest first.
    pub fn recent_choices(&self) -> impl Iterator<Item = &Choice> {
        self.recent
            .iter()
            .map(|&i| &self.shown.source.items()[i as usize].choice)
    }

    /// The host's current choice (the item's saved icon), marked in the grid; `None` for none,
    /// or one the source doesn't offer.
    ///
    /// The grid's cursor moves onto it when the page shows it (not a search's results).
    pub fn set_current(&mut self, choice: Option<&Choice>) {
        let current = choice
            .and_then(|c| self.shown.source.find(c))
            .map(|i| i as u32);
        if current == self.current {
            return;
        }
        self.current = current;
        if let Some(index) = self.current_index() {
            self.coll.selection.select_only(&self.shown, index);
        }
    }

    /// Where the current choice shows, unless the page is a search's results.
    fn current_index(&self) -> Option<usize> {
        let c = self.current.filter(|_| self.page != Page::Results)?;
        self.shown
            .index_of(EntryId::new(self.shown.id_base + u64::from(c)))
    }

    /// The current choice, if the source offers it.
    pub fn current(&self) -> Option<&Choice> {
        self.current
            .map(|i| &self.shown.source.items()[i as usize].choice)
    }

    pub(super) fn current_item(&self) -> Option<usize> {
        self.current.map(|i| i as usize)
    }

    /// What the grid shows.
    pub fn page(&self) -> Page {
        self.page
    }

    /// The tabs: the recent choices when there are any, then every category with items.
    pub fn tabs(&self) -> Vec<Page> {
        let mut tabs = Vec::new();
        if !self.recent.is_empty() {
            tabs.push(Page::Recent);
        }
        tabs.extend(
            (0..self.shown.source.categories().len())
                .filter(|&c| !self.shown.source.members(c).is_empty())
                .map(Page::Category),
        );
        tabs
    }

    fn first_page(&self) -> Page {
        self.tabs().first().copied().unwrap_or(Page::Category(0))
    }

    /// Shows `page` (a tab), clearing the query. Without a `Tx` (before the picker shows) the
    /// grid starts afresh; with one, it scrolls to the top.
    pub fn select_page(&mut self, page: Page) {
        self.select_page_in(page, None);
    }

    pub(super) fn select_page_in(&mut self, page: Page, at: Option<(&str, &mut Tx)>) {
        if page == Page::Results {
            return;
        }
        self.field.replace_text("");
        self.query = Query::default();
        self.page = page;
        self.browse = page;
        self.refresh(at);
    }

    /// The search field's text, for C21's `FieldMenu`.
    pub fn edit(&self) -> &TextEdit {
        &self.field
    }

    /// The search field's text, for C21's `FieldMenu`; call `IconPicker::edited` after a
    /// change.
    pub fn edit_mut(&mut self) -> &mut TextEdit {
        &mut self.field
    }

    /// The search text.
    pub fn query(&self) -> &str {
        &self.field.text
    }

    /// Sets the search text, as typing it would.
    pub fn set_query(&mut self, text: &str) {
        self.field.replace_text(text);
        self.query_changed(None);
    }

    pub(super) fn query_changed(&mut self, at: Option<(&str, &mut Tx)>) {
        let query = Query::new(&self.field.text);
        if query == self.query {
            return;
        }
        self.query = query;
        self.page = if self.query.is_empty() {
            self.browse
        } else {
            Page::Results
        };
        self.refresh(at);
    }

    /// What the query matches in item `item`.
    pub(super) fn hit(&self, item: usize) -> Option<Hit> {
        self.query.hit(self.shown.source.words(item))
    }

    /// How many results a search found, or `None` while not searching.
    pub fn results(&self) -> Option<usize> {
        (self.page == Page::Results).then(|| self.shown.len())
    }

    /// The item under the grid's cursor.
    pub fn cursor_item(&self) -> Option<usize> {
        let index = self.coll.selection.cursor_index(&self.shown)?;
        self.shown.item(index)
    }

    /// The choice under the grid's cursor.
    pub fn cursor_choice(&self) -> Option<&Choice> {
        self.cursor_item()
            .map(|i| &self.shown.source.items()[i].choice)
    }

    /// Changes whenever what the grid shows changes: the collection's listing version.
    pub(super) fn version(&self) -> u64 {
        self.version
    }

    /// Opens the popover: on the recent choices when there are any, else the first category,
    /// with the query cleared (GTK's chooser opens on its recent section).
    pub fn open(&mut self) {
        self.open = true;
        self.dismissed = false;
        self.select_page(self.first_page());
    }

    /// Closes the popover.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Whether the popover is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A press outside closed the popover (Rune's modal dismissal, which has no `Tx`): the
    /// picker's `poll` reports it.
    pub(super) fn dismiss(&mut self) {
        if self.open {
            self.open = false;
            self.dismissed = true;
        }
    }

    pub(super) fn take_dismissed(&mut self) -> bool {
        std::mem::take(&mut self.dismissed)
    }

    /// Lists what the page shows and puts the cursor on its first item, or on the current
    /// choice when the page shows it (not in results). With `at`, the grid scrolls to the top
    /// on that `Tx` (its scope is the collection's key); without, it starts afresh.
    fn refresh(&mut self, at: Option<(&str, &mut Tx)>) {
        let source = self.shown.source.clone();
        let items: Vec<u32> = match self.page {
            Page::Recent => self.recent.clone(),
            Page::Category(c) => source.members(c).to_vec(),
            Page::Results => (0..source.items().len() as u32)
                .filter(|&i| self.query.hit(source.words(i as usize)).is_some())
                .collect(),
        };
        self.shown.set(items);
        self.version += 1;
        match at {
            Some((scope, tx)) => {
                self.coll
                    .enter(scope, &self.shown, None, Loaded::Complete, tx);
            }
            None => self.coll = fresh_collection(),
        }
        if !self.shown.is_empty() {
            let index = self.current_index().unwrap_or(0);
            self.coll.selection.select_only(&self.shown, index);
        }
    }
}

/// A collection with the picker's controllers: C1c's keyboard and C1b's pointer intent, read
/// as plain presses.
fn fresh_collection() -> CollectionState {
    let mut coll = CollectionState::new();
    coll.controllers.keyboard = Some(Box::<Keyboard>::default());
    coll.controllers.pointer = Some(Box::<super::grid::OneChoice>::default());
    coll
}
