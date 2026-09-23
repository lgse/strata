// SPDX-License-Identifier: MIT

//! Recursive filtering for single-pane presentations.
//!
//! Miller Columns already swaps recursive results into its existing row collection. Icons and
//! List share `ResultCollection`: one stable model, selection/navigation/context/rename surface,
//! and presentation-specific factories. The browser and file chooser supply activation,
//! selection-mode, and selection-change policy through `SearchCollectionOptions`.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::SearchItem,
};

mod collection;
mod presentation;
use collection::{
    CollectionBehavior, ResultCollection, ResultKind, build_collection, collection_entry,
};
#[cfg(test)]
use presentation::relative_result_path;

pub(super) const SEARCH_RESULTS_LABEL: &str = "Search results";
pub(super) type SearchSelectionChanged = Rc<dyn Fn(Vec<FileEntry>)>;
pub(super) type SearchSelectionHandlers = Rc<RefCell<Vec<SearchSelectionChanged>>>;

#[derive(Clone)]
pub(super) enum SearchPresentation {
    Rows,
    Icons {
        thumbnail_size: Rc<Cell<i32>>,
        max_columns: u32,
    },
}

/// Consumer-owned behavior for the shared filtered-results collection.
pub(super) struct SearchCollectionOptions {
    pub(super) presentation: SearchPresentation,
    pub(super) multiple_selection: Rc<Cell<bool>>,
    pub(super) activate: Rc<dyn Fn(FileEntry)>,
    pub(super) single_click: Rc<dyn Fn(FileEntry)>,
    pub(super) selection_changed: SearchSelectionChanged,
    pub(super) focus_items: Rc<dyn Fn()>,
}

enum Publication {
    Results(Vec<SearchItem>, bool),
    Directory,
}

struct State {
    entry: glib::WeakRef<gtk::Entry>,
    stack: gtk::Stack,
    status: gtk::Label,
    collection: ResultCollection,
    session: super::search_session::SearchSession,
    query_binding: RefCell<Option<super::browser::FilterQueryBinding>>,
    updating: Cell<bool>,
    publishing: Cell<bool>,
    selection_dirty: Cell<bool>,
    published_selection: RefCell<Vec<FileEntry>>,
    pending_update: RefCell<Option<Publication>>,
    detached: Cell<bool>,
    recursive: Cell<bool>,
    context_menu_trigger: RefCell<Option<super::browser::ContextMenuTrigger>>,
    activate: Rc<dyn Fn(FileEntry)>,
    selection_callbacks: RefCell<Vec<SearchSelectionChanged>>,
}

impl State {
    fn selected_entries(&self) -> Vec<FileEntry> {
        self.collection
            .selected_positions()
            .into_iter()
            .filter_map(|position| collection_entry(&self.collection.sorted, position as u32))
            .collect()
    }

    fn emit_selection_changed(&self) {
        self.selection_dirty.set(true);
        if self.updating.get() || self.publishing.replace(true) {
            return;
        }
        while self.selection_dirty.replace(false) {
            let entries = self.selected_entries();
            let previous = self.published_selection.replace(entries.clone());
            if previous
                .iter()
                .map(|entry| &entry.location)
                .eq(entries.iter().map(|entry| &entry.location))
            {
                continue;
            }
            let callbacks = self.selection_callbacks.borrow().clone();
            for callback in callbacks {
                if self.detached.get() {
                    break;
                }
                callback(entries.clone());
            }
        }
        self.publishing.set(false);
        if let Some(publication) = self.pending_update.take() {
            match publication {
                Publication::Results(items, recursive) => update_results(self, items, recursive),
                Publication::Directory => show_directory_listing(self),
            }
        }
    }

    fn activate(&self, position: u32) -> bool {
        let Some(entry) = collection_entry(&self.collection.sorted, position) else {
            return false;
        };
        (self.activate)(entry);
        true
    }
}

#[derive(Clone)]
pub(super) struct InlineSearch {
    pub widget: gtk::Widget,
    state: Option<Rc<State>>,
    marquee: Option<super::marquee::Marquee>,
}

impl InlineSearch {
    pub(super) fn active_marquee(&self) -> Option<super::marquee::Marquee> {
        let state = self.state.as_ref()?;
        (state.stack.visible_child_name().as_deref() == Some("search"))
            .then(|| self.marquee.clone())
            .flatten()
    }

    pub(in crate::ui) fn has_item_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.state.as_ref().is_some_and(|state| {
            focused.is_some_and(|focused| {
                focused == &state.collection.view || focused.is_ancestor(&state.collection.view)
            })
        })
    }

    pub fn is_item_target(&self, picked: &gtk::Widget) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.collection.position_at(picked).is_some())
    }

    pub fn install_context_menu(
        &self,
        install: impl FnOnce(
            &gtk::Widget,
            super::browser::ContextResolver,
        ) -> super::browser::ContextMenuTrigger,
    ) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(state);
        let resolve = Rc::new(move |picked: &gtk::Widget| {
            let state = weak.upgrade()?;
            let position = state.collection.position_at(picked)?;
            let entry = collection_entry(&state.collection.sorted, position)?;
            let preserved = state
                .collection
                .gesture_selection
                .borrow()
                .clone()
                .filter(|selected| selected.size() > 1 && selected.contains(position));
            if let Some(selected) = preserved {
                state.collection.selection.set_selection(
                    &selected,
                    &gtk::Bitset::new_range(0, state.collection.selection.n_items()),
                );
            } else if !state.collection.selection.is_selected(position) {
                state.collection.selection.select_item(position, true);
            }
            Some((None, entry))
        });
        let trigger = install(&state.collection.view, resolve);
        state.context_menu_trigger.replace(Some(trigger));
    }

    pub fn context_menu_target(&self) -> Option<super::browser::ContextMenuTarget> {
        let state = self.state.as_ref()?;
        let position = state.collection.current_position()?;
        let (_, widget) = state.collection.bound_at(position)?;
        let bounds = widget.compute_bounds(&state.collection.view)?;
        let x = match &state.collection.kind {
            ResultKind::Rows => bounds.center().x(),
            ResultKind::Icons { .. } => bounds.x() + bounds.width(),
        };
        Some((
            state.context_menu_trigger.borrow().as_ref()?.clone(),
            f64::from(x),
            f64::from(bounds.center().y()),
        ))
    }

    pub fn selected_entry(&self) -> Option<FileEntry> {
        self.selected_entries()?.into_iter().next()
    }

    pub fn selected_entries(&self) -> Option<Vec<FileEntry>> {
        let state = self.state.as_ref()?;
        if state.stack.visible_child_name().as_deref() != Some("search") {
            return None;
        }
        Some(state.selected_entries())
    }

    pub fn select_all(&self) -> bool {
        let Some(state) = self
            .state
            .as_ref()
            .filter(|state| state.stack.visible_child_name().as_deref() == Some("search"))
        else {
            return false;
        };
        state.collection.selection.select_all();
        true
    }

    pub(in crate::ui) fn edit_target(
        &self,
        entry: &FileEntry,
    ) -> Option<super::collection_edit::EditTarget> {
        let state = self
            .state
            .as_ref()
            .filter(|state| state.stack.visible_child_name().as_deref() == Some("search"))?;
        let path = entry.location.native_path()?;
        let position = state
            .collection
            .items()
            .iter()
            .position(|item| item.path == path)? as u32;
        Some(state.collection.edit_widgets(position)?.into())
    }

    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let Some(state) = self.state.as_ref() else {
            return Vec::new();
        };
        state
            .collection
            .items()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                let location = Location::local(&item.path);
                location == *old_location
                    || new_location.is_some_and(|new_location| location == *new_location)
            })
            .filter_map(|(position, _)| state.collection.rename_label(position as u32))
            .collect()
    }

    pub fn focus_result(&self, path: &Path) -> bool {
        let Some(state) = self.state.as_ref() else {
            return false;
        };
        if state.stack.visible_child_name().as_deref() != Some("search") {
            return false;
        }
        let Some(position) = state
            .collection
            .items()
            .iter()
            .position(|item| item.path == path)
        else {
            return false;
        };
        let position = position as u32;
        state
            .collection
            .focus(position, !state.collection.selection.is_selected(position))
    }

    pub fn refresh_cut_rows(&self) {
        if let Some(state) = self.state.as_ref() {
            state.collection.refresh_cut_rows();
        }
    }

    pub(super) fn set_show_hidden(&self, show_hidden: bool) {
        if let Some(state) = &self.state {
            state.session.set_show_hidden(show_hidden);
        }
    }

    pub fn refresh_source_filter(&self, browser: &Browser) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if !state.session.is_active() {
            return;
        }
        let handle = &state.session;
        let items = state
            .collection
            .items()
            .iter()
            .filter(|item| browser.allows_entry(&super::browser::search_result_entry(item)))
            .cloned()
            .collect();
        update_results(state, items, state.recursive.get());
        if let Some(entry) = state.entry.upgrade() {
            handle.query(entry.text().trim());
        }
    }

    pub fn prune_missing(&self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if !state.session.is_active() {
            return;
        }
        let pruned: Vec<_> = state
            .collection
            .items()
            .iter()
            .filter(|item| search_path_present(&item.path))
            .cloned()
            .collect();
        if pruned.len() != state.collection.sorted.n_items() as usize {
            update_results(state, pruned, state.recursive.get());
        }
    }

    pub(in crate::ui) fn set_icons_max_columns(&self, max_columns: u32) {
        if let Some(state) = self.state.as_ref() {
            state.collection.set_max_columns(max_columns);
        }
    }

    pub(super) fn detach(&self) {
        if let Some(state) = &self.state {
            state.detached.set(true);
            state.pending_update.take();
            state.query_binding.take();
            state.session.cancel();
            state.updating.set(true);
            state.collection.clear();
            super::browser::detach_collection_view(&state.collection.view);
            state.updating.set(false);
        }
    }

    pub fn show_directory_listing(&self) {
        if let Some(state) = self.state.as_ref() {
            show_directory_listing(state);
        }
    }
}

fn show_directory_listing(state: &State) {
    state.session.cancel();
    if state.updating.get() || state.publishing.get() {
        state.pending_update.replace(Some(Publication::Directory));
        return;
    }
    state.updating.set(true);
    state.collection.clear();
    state.stack.set_visible_child_name("files");
    state.updating.set(false);
    state.emit_selection_changed();
}

fn install_marquee(
    state: &Rc<State>,
    scroll: &gtk::ScrolledWindow,
    overlay: &gtk::Overlay,
    allow_drag: Rc<Cell<bool>>,
) -> super::marquee::Marquee {
    let weak = Rc::downgrade(state);
    let targets = Rc::new(RefCell::new(vec![super::marquee::MarqueeTarget {
        selection: state.collection.selection.clone(),
        visit_items: Rc::new(move |visit| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.collection.bound.borrow_mut().retain(|bound| {
                let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade())
                else {
                    return false;
                };
                visit(item.position(), &widget);
                true
            });
        }),
    }]));
    let weak = Rc::downgrade(state);
    super::marquee::install(super::marquee::MarqueeSetup {
        view: state.collection.view.clone(),
        surface: scroll.clone().upcast(),
        scroll: scroll.clone(),
        overlay: overlay.clone(),
        targets: targets.clone(),
        is_item: super::marquee::item_bounds_predicate(targets),
        clear_selection: Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                state.collection.selection.unselect_all();
            }
        }),
        allow_drag,
    })
}

/// Keeps the view's normal presentation intact when the recursive query is dismissed.
pub(super) fn wrap(
    content: &impl IsA<gtk::Widget>,
    entry: &gtk::Entry,
    root: Option<PathBuf>,
    browser: &Rc<Browser>,
    options: SearchCollectionOptions,
) -> InlineSearch {
    let Some(root) = root else {
        return InlineSearch {
            widget: content.clone().upcast(),
            state: None,
            marquee: None,
        };
    };
    let SearchCollectionOptions {
        presentation,
        multiple_selection,
        activate,
        single_click,
        selection_changed,
        focus_items,
    } = options;
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(content, Some("files"));
    let results = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let status = gtk::Label::new(None);
    status.add_css_class("status-message");
    results.append(&status);
    let recursive = Rc::new(Cell::new(false));
    let (collection, scroll, overlay) = build_collection(
        presentation,
        recursive.clone(),
        root.clone(),
        CollectionBehavior {
            multiple_selection: multiple_selection.clone(),
            activate: activate.clone(),
            single_click,
            focus_items,
        },
    );
    results.append(&overlay);
    stack.add_named(&results, Some("search"));
    let state = Rc::new(State {
        entry: entry.downgrade(),
        stack: stack.clone(),
        status,
        collection,
        session: super::search_session::SearchSession::default(),
        query_binding: RefCell::new(None),
        updating: Cell::new(false),
        publishing: Cell::new(false),
        selection_dirty: Cell::new(false),
        published_selection: RefCell::new(Vec::new()),
        pending_update: RefCell::new(None),
        detached: Cell::new(false),
        recursive: Cell::new(false),
        context_menu_trigger: RefCell::new(None),
        activate,
        selection_callbacks: RefCell::new(vec![selection_changed]),
    });
    let weak_state = Rc::downgrade(&state);
    state
        .collection
        .selection
        .connect_selection_changed(move |_, _, _| {
            if let Some(state) = weak_state.upgrade() {
                state.emit_selection_changed();
            }
        });
    let marquee = install_marquee(&state, &scroll, &overlay, multiple_selection);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !state.session.is_active()
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        let current = state.collection.current_position();
        if key == gtk::gdk::Key::Up {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Down {
            if let Some(window) = state.stack.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(true);
            }
            state.collection.focus(current.unwrap_or(0), true);
            return glib::Propagation::Stop;
        }
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            && state.activate(current.unwrap_or(0))
        {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    entry.add_controller(keys);

    let return_to_filter = gtk::EventControllerKey::new();
    return_to_filter.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    return_to_filter.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key != gtk::gdk::Key::Up
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
            || !state
                .collection
                .current_position()
                .is_some_and(|position| state.collection.first_visual_row(position))
            || state
                .stack
                .root()
                .and_then(|root| root.focus())
                .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
                .is_some()
        {
            return glib::Propagation::Proceed;
        }
        state
            .entry
            .upgrade()
            .filter(|entry| entry.grab_focus_without_selecting())
            .map_or(glib::Propagation::Proceed, |_| glib::Propagation::Stop)
    });
    state.collection.view.add_controller(return_to_filter);

    let weak_browser = Rc::downgrade(browser);
    let search = InlineSearch {
        widget: stack.clone().upcast(),
        state: Some(state.clone()),
        marquee: Some(marquee),
    };
    let query_state = state.clone();
    let binding = super::browser::bind_filter_query(
        entry,
        &state.session,
        move |text, is_recursive, restart| {
            let state = &query_state;
            state.recursive.set(is_recursive);
            recursive.set(is_recursive);
            let query = text.trim();
            if query.is_empty() {
                show_directory_listing(state);
                return;
            }
            state.stack.set_visible_child_name("search");
            if state.collection.sorted.n_items() == 0 {
                state.status.set_text("Searching…");
                state.status.set_visible(true);
            }
            let show_hidden = weak_browser
                .upgrade()
                .is_some_and(|browser| browser.preferences().show_hidden);
            let weak = Rc::downgrade(state);
            let browser = weak_browser.clone();
            state.session.update(
                super::search_session::SearchInput {
                    root: root.clone(),
                    show_hidden,
                    recursive: is_recursive,
                },
                query,
                restart,
                Rc::new(move |batch| {
                    let (Some(state), Some(browser)) = (weak.upgrade(), browser.upgrade()) else {
                        return;
                    };
                    let items = eligible_results(
                        &browser,
                        &state.session,
                        &batch.query,
                        batch.items,
                        batch.has_more,
                    );
                    state
                        .status
                        .set_visible(items.is_empty() || batch.coverage.is_partial());
                    state.status.set_text(&if batch.coverage.is_partial() {
                        batch.coverage.message()
                    } else if batch.indexing {
                        "Searching…".to_owned()
                    } else {
                        "No matching files".to_owned()
                    });
                    update_results(&state, items, state.recursive.get());
                }),
            );
        },
    );
    state.query_binding.replace(Some(binding));
    search
}

fn update_results(state: &State, items: Vec<SearchItem>, recursive: bool) {
    if state.detached.get() {
        return;
    }
    if state.updating.get() || state.publishing.get() {
        state
            .pending_update
            .replace(Some(Publication::Results(items, recursive)));
        return;
    }
    state.updating.set(true);
    state.collection.update(&items, recursive);
    state.updating.set(false);
    state.emit_selection_changed();
}

pub(super) fn eligible_results(
    browser: &Browser,
    handle: &super::search_session::SearchSession,
    query: &str,
    items: Vec<SearchItem>,
    has_more: bool,
) -> Vec<SearchItem> {
    let candidates = items.len();
    let items = items
        .into_iter()
        .filter(|item| {
            browser.allows_entry(&super::browser::search_result_entry(item))
                && search_path_present(&item.path)
        })
        .take(crate::services::SEARCH_RESULT_LIMIT)
        .collect::<Vec<_>>();
    // Type/folder predicates run on GTK's thread, after ranking but before the display cap.
    // Continue through lower-ranked candidates rather than starving eligible matches.
    if has_more && items.len() < crate::services::SEARCH_RESULT_LIMIT {
        handle.query_candidates(query, candidates.saturating_mul(2));
    }
    items
}

pub(super) fn search_path_present(path: &Path) -> bool {
    // Preserve dangling symlinks and uncertain paths; only confirmed absence removes a hit.
    path.symlink_metadata().map_or_else(
        |error| error.kind() != std::io::ErrorKind::NotFound,
        |_| true,
    )
}

#[cfg(test)]
mod tests;
