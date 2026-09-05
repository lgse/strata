// SPDX-License-Identifier: GPL-3.0-or-later

use crate::app::Browser;
use crate::model::Location;
use crate::ui::browser::entry::entry_matches;
use crate::ui::entry_list_model::EntryListModel;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

pub(crate) const FILTER_DEBOUNCE_DELAY: Duration = Duration::from_millis(200);

/// `scroll_to` before the view has a real height leaves ListView/GridView with a
/// one-row widget pool, so scrolling after a mode switch stays janky.
pub(crate) fn scroll_collection_when_allocated(view: &gtk::Widget, position: u32) {
    scroll_collection_when_allocated_with(view, position, gtk::ListScrollFlags::FOCUS);
}

pub(crate) fn focus_collection_item_when_allocated(view: &gtk::Widget, position: u32) {
    scroll_collection_when_allocated_with(
        view,
        position,
        gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
    );
}

fn scroll_collection_when_allocated_with(
    view: &gtk::Widget,
    position: u32,
    flags: gtk::ListScrollFlags,
) {
    if view.height() > 1 {
        apply_collection_scroll(view, position, flags);
        return;
    }
    // ponytail: a few frames is enough for the first layout. Upgrade: a real
    // allocate listener if GTK grows one that is safe to scroll from.
    let frames = Cell::new(0u8);
    view.add_tick_callback(move |view, _| {
        if view.height() > 1 {
            if flags.intersects(gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT)
                && !collection_view_holds_focus(view)
            {
                return glib::ControlFlow::Break;
            }
            apply_collection_scroll(view, position, flags);
            return glib::ControlFlow::Break;
        }
        let waited = frames.get().saturating_add(1);
        frames.set(waited);
        if waited >= 8 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn collection_view_holds_focus(view: &gtk::Widget) -> bool {
    let Some(focused) = view.root().and_then(|root| root.focus()) else {
        return false;
    };
    view.has_focus() || focused == *view || view.is_ancestor(&focused) || focused.is_ancestor(view)
}

fn apply_collection_scroll(view: &gtk::Widget, position: u32, flags: gtk::ListScrollFlags) {
    if let Ok(list) = view.clone().downcast::<gtk::ListView>() {
        if position < list.model().map_or(0, |model| model.n_items()) {
            list.scroll_to(position, flags, None);
        }
        return;
    }
    if let Ok(grid) = view.clone().downcast::<gtk::GridView>()
        && position < grid.model().map_or(0, |model| model.n_items())
    {
        grid.scroll_to(position, flags, None);
    }
}

pub(crate) fn detach_collection_view(view: &impl IsA<gtk::Widget>) {
    let view = view.as_ref();
    if let Ok(list) = view.clone().downcast::<gtk::ListView>() {
        list.set_factory(None::<&gtk::ListItemFactory>);
        list.set_model(None::<&gtk::SelectionModel>);
    } else if let Ok(grid) = view.clone().downcast::<gtk::GridView>() {
        grid.set_factory(None::<&gtk::ListItemFactory>);
        grid.set_model(None::<&gtk::SelectionModel>);
    }
}

pub(crate) fn focus_filter_entry(entry: &gtk::Entry, query: Option<&str>) {
    if let Some(query) = query {
        entry.set_text(query);
        // Regular grab_focus selects the seed again, so the next key would replace it.
        entry.grab_focus_without_selecting();
        entry.select_region(-1, -1);
    } else {
        entry.grab_focus();
    }
}

pub(crate) fn debounce_filter_entry(entry: &gtk::Entry, on_settled: impl Fn(String) + 'static) {
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let on_settled = Rc::new(on_settled);
    entry.connect_changed(move |entry| {
        cancel_source(&pending);
        let slot = pending.clone();
        let callback = on_settled.clone();
        let text = entry.text().to_string();
        *pending.borrow_mut() = Some(glib::timeout_add_local_once(
            FILTER_DEBOUNCE_DELAY,
            move || {
                slot.borrow_mut().take();
                callback(text);
            },
        ));
    });
}

pub(crate) fn filter_change_for(previous: &str, settled: &str) -> gtk::FilterChange {
    if settled.starts_with(previous) && settled.len() > previous.len() {
        gtk::FilterChange::MoreStrict
    } else if previous.starts_with(settled) && previous.len() > settled.len() {
        gtk::FilterChange::LessStrict
    } else {
        gtk::FilterChange::Different
    }
}

pub(crate) fn notify_filter_query(
    filter: &gtk::CustomFilter,
    query: &RefCell<String>,
    text: String,
) {
    let settled = text.to_lowercase();
    let previous = query.borrow().clone();
    if previous == settled {
        return;
    }
    let change = filter_change_for(&previous, &settled);
    *query.borrow_mut() = settled;
    filter.changed(change);
}

/// Restores a miller column to its directory listing after its recursive search filter is cleared.
///
/// The flag must drop before the model swap: GTK binds the visible rows synchronously inside
/// `set_model`, and the row factory reads it to decide whether to source entries from the
/// search results or the directory. Clearing it afterwards binds every visible row against the
/// already-emptied results, so they keep fallback icons and no hover size until an unrelated
/// rebuild.
pub(crate) fn deactivate_recursive_search(
    search_active: &Cell<bool>,
    search_results: &RefCell<Vec<crate::services::SearchItem>>,
    search_model: &gtk::StringList,
    filtered_model: &gtk::FilterListModel,
    directory_model: &EntryListModel,
) {
    search_active.set(false);
    search_results.borrow_mut().clear();
    search_model.splice(0, search_model.n_items(), &[]);
    if filtered_model.model().as_ref() != Some(directory_model.upcast_ref::<gio::ListModel>()) {
        filtered_model.set_model(Some(directory_model));
    }
}

pub(crate) fn apply_filter_query(
    model: &gtk::FilterListModel,
    filter: &gtk::CustomFilter,
    query: &RefCell<String>,
    settled: String,
) {
    let previous = query.borrow().clone();
    let change = filter_change_for(&previous, &settled);
    *query.borrow_mut() = settled;
    if query.borrow().is_empty() {
        model.set_filter(None::<&gtk::Filter>);
    } else if previous.is_empty() {
        model.set_filter(Some(filter));
    } else {
        filter.changed(change);
    }
}

#[derive(Default)]
pub(crate) struct PositionMap {
    query: String,
    generation: u64,
    forward: Vec<usize>,
    reverse: Vec<u32>,
}

pub(crate) const NO_FILTERED_POSITION: u32 = u32::MAX;

fn rebuild_position_map(
    source: &EntryListModel,
    query: &str,
    show_hidden: bool,
    generation: u64,
) -> PositionMap {
    let n_source = source.n_items() as usize;
    let mut forward = Vec::new();
    let mut reverse = vec![NO_FILTERED_POSITION; n_source];
    for (source_position, filtered_position) in reverse.iter_mut().enumerate() {
        let Some(text) = source.value(source_position as u32) else {
            continue;
        };
        if entry_matches(&text, show_hidden, query) {
            *filtered_position = forward.len() as u32;
            forward.push(source_position);
        }
    }
    PositionMap {
        query: query.to_owned(),
        generation,
        forward,
        reverse,
    }
}

#[derive(Clone)]
pub(crate) struct ViewMap {
    cache: Rc<RefCell<PositionMap>>,
    query: Rc<RefCell<String>>,
    show_hidden: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
    source: EntryListModel,
    filter: gtk::FilterListModel,
    placeholder: Option<gtk::StringList>,
}

impl ViewMap {
    pub(crate) fn new(
        query: Rc<RefCell<String>>,
        show_hidden: Rc<Cell<bool>>,
        generation: Rc<Cell<u64>>,
        source: EntryListModel,
        filter: gtk::FilterListModel,
        placeholder: Option<gtk::StringList>,
    ) -> Self {
        Self {
            cache: Rc::new(RefCell::new(PositionMap::default())),
            query,
            show_hidden,
            generation,
            source,
            filter,
            placeholder,
        }
    }

    fn placeholder_count(&self) -> u32 {
        self.placeholder
            .as_ref()
            .map_or(0, |placeholder| placeholder.n_items())
    }

    pub(crate) fn source_position(&self, visible_position: u32) -> Option<usize> {
        let filter_position = visible_position.checked_sub(self.placeholder_count())?;
        let query = self.query.borrow();
        if query.is_empty() && self.show_hidden.get() {
            if filter_position < self.source.n_items() && filter_position < self.filter.n_items() {
                return Some(filter_position as usize);
            }
            return None;
        }
        let mut cache = self.cache.borrow_mut();
        let generation = self.generation.get();
        if cache.query != *query || cache.generation != generation {
            *cache = rebuild_position_map(&self.source, &query, self.show_hidden.get(), generation);
        }
        cache.forward.get(filter_position as usize).copied()
    }

    pub(crate) fn view_position(&self, source_position: usize) -> Option<u32> {
        let query = self.query.borrow();
        if query.is_empty() && self.show_hidden.get() {
            let position = source_position as u64;
            if position < self.source.n_items() as u64 && position < self.filter.n_items() as u64 {
                return Some(position as u32 + self.placeholder_count());
            }
            return None;
        }
        let mut cache = self.cache.borrow_mut();
        let generation = self.generation.get();
        if cache.query != *query || cache.generation != generation {
            *cache = rebuild_position_map(&self.source, &query, self.show_hidden.get(), generation);
        }
        let position = *cache.reverse.get(source_position)?;
        (position != NO_FILTERED_POSITION).then_some(position + self.placeholder_count())
    }

    pub(crate) fn source_positions(&self, visible_positions: &[u32]) -> Vec<(u32, usize)> {
        visible_positions
            .iter()
            .filter_map(|position| {
                self.source_position(*position)
                    .map(|source| (*position, source))
            })
            .collect()
    }
}

pub(crate) fn recursive_search_activation_key(key: gtk::gdk::Key) -> bool {
    matches!(
        key,
        gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Right
    )
}

pub(crate) fn activate_recursive_search_result(
    browser: &Weak<Browser>,
    results: &RefCell<Vec<crate::services::SearchItem>>,
    position: u32,
) -> bool {
    let Some(item) = results.borrow().get(position as usize).cloned() else {
        return false;
    };
    let Some(browser) = browser.upgrade() else {
        return false;
    };
    if item.is_directory {
        browser.navigate(Location::local(item.path));
    } else if let Some(parent) = item.path.parent() {
        browser.navigate(Location::local(parent));
    } else {
        return false;
    }
    true
}

pub(crate) fn search_result_navigation_position(
    current: Option<u32>,
    count: u32,
    direction: i32,
) -> Option<u32> {
    if count == 0 {
        return None;
    }
    let Some(current) = current else {
        return Some(if direction < 0 { count - 1 } else { 0 });
    };
    Some((i64::from(current) + i64::from(direction)).clamp(0, i64::from(count - 1)) as u32)
}

pub(crate) enum SelectionPlan<'a> {
    All,
    Range { position: u32, count: u32 },
    Items(&'a [u32]),
}

pub(crate) fn plan_selection(n_items: u32, positions: &[u32]) -> SelectionPlan<'_> {
    let contiguous =
        !positions.is_empty() && positions.windows(2).all(|pair| pair[1] == pair[0] + 1);
    if contiguous && positions.len() as u32 == n_items && positions[0] == 0 {
        SelectionPlan::All
    } else if contiguous {
        SelectionPlan::Range {
            position: positions[0],
            count: positions.len() as u32,
        }
    } else {
        SelectionPlan::Items(positions)
    }
}

pub(crate) fn apply_selection_plan(
    selection: &gtk::MultiSelection,
    n_items: u32,
    positions: &[u32],
) {
    match plan_selection(n_items, positions) {
        SelectionPlan::All => {
            selection.select_all();
        }
        SelectionPlan::Range { position, count } => {
            selection.select_range(position, count, true);
        }
        SelectionPlan::Items(items) => {
            selection.unselect_all();
            for position in items {
                selection.select_item(*position, false);
            }
        }
    }
}

pub(super) fn bitset_positions(bitset: &gtk::Bitset) -> Vec<u32> {
    let Some((iterator, first)) = gtk::BitsetIter::init_first(bitset) else {
        return Vec::new();
    };
    std::iter::once(first).chain(iterator).collect()
}

pub(super) fn cancel_source(source: &RefCell<Option<glib::SourceId>>) {
    if let Some(source) = source.take() {
        source.remove();
    }
}

#[cfg(test)]
mod tests;
