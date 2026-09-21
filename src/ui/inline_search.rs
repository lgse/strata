// SPDX-License-Identifier: MIT

//! Recursive filtering for single-pane presentations.
//!
//! Miller Columns already swaps recursive results into its existing row collection. Icons and
//! List share `ResultCollection`: one stable model, selection/navigation/context/rename surface,
//! and presentation-specific factories. The browser and file chooser supply activation,
//! selection-mode, and selection-change policy through `SearchCollectionOptions`.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::{SearchEvent, SearchHandle, SearchItem, index_filter},
};

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

#[derive(Clone)]
struct CollectionBehavior {
    multiple_selection: Rc<Cell<bool>>,
    activate: Rc<dyn Fn(FileEntry)>,
    single_click: Rc<dyn Fn(FileEntry)>,
    focus_items: Rc<dyn Fn()>,
}

#[derive(Clone)]
struct CollectionInteractions {
    selection: gtk::MultiSelection,
    anchor: Rc<Cell<Option<u32>>>,
    items: Rc<RefCell<Vec<SearchItem>>>,
    gesture_selection: Rc<RefCell<Option<gtk::Bitset>>>,
    pointer_activation: Rc<Cell<Option<bool>>>,
    behavior: CollectionBehavior,
}

#[derive(Clone)]
enum ResultKind {
    Rows,
    Icons { thumbnail_size: Rc<Cell<i32>> },
}

struct BoundResult {
    item: glib::WeakRef<gtk::ListItem>,
    widget: glib::WeakRef<gtk::Widget>,
    rename_label: glib::WeakRef<gtk::Widget>,
}

struct ResultCollection {
    view: gtk::Widget,
    model: gio::ListStore,
    sorted: gtk::SortListModel,
    sorter: gtk::CustomSorter,
    positions: Rc<RefCell<HashMap<PathBuf, usize>>>,
    selection: gtk::MultiSelection,
    bound: Rc<RefCell<Vec<BoundResult>>>,
    anchor: Rc<Cell<Option<u32>>>,
    // GTK selects the pointer target before item gestures run; retain the intended
    // modified-click group so drag and context-menu focus can preserve it.
    gesture_selection: Rc<RefCell<Option<gtk::Bitset>>>,
    kind: ResultKind,
    root: PathBuf,
}

impl ResultCollection {
    fn item(&self, position: u32) -> Option<SearchItem> {
        let object = self
            .sorted
            .item(position)?
            .downcast::<glib::BoxedAnyObject>()
            .ok()?;
        Some(object.borrow::<SearchItem>().clone())
    }

    fn position_at(&self, picked: &gtk::Widget) -> Option<u32> {
        self.bound
            .borrow_mut()
            .retain(|bound| bound.item.upgrade().is_some() && bound.widget.upgrade().is_some());
        self.bound.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            let widget = bound.widget.upgrade()?;
            (picked == &widget
                || picked.is_ancestor(&widget)
                || widget.parent().as_ref() == Some(picked))
            .then(|| item.position())
        })
    }

    fn current_position(&self) -> Option<u32> {
        self.view
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| self.position_at(&focus))
            .or_else(|| {
                (0..self.selection.n_items()).find(|position| self.selection.is_selected(*position))
            })
    }

    fn selected_positions(&self) -> Vec<usize> {
        (0..self.selection.n_items())
            .filter(|position| self.selection.is_selected(*position))
            .map(|position| position as usize)
            .collect()
    }

    fn bound_at(&self, position: u32) -> Option<(gtk::ListItem, gtk::Widget)> {
        self.bound.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position)
                .then(|| Some((item, bound.widget.upgrade()?)))
                .flatten()
        })
    }

    fn focus(&self, position: u32, exclusive: bool) -> bool {
        if position >= self.selection.n_items() {
            return false;
        }
        if exclusive {
            self.gesture_selection.borrow_mut().take();
        }
        self.selection.select_item(position, exclusive);
        self.gesture_selection
            .replace(Some(self.selection.selection().copy()));
        self.view.grab_focus();
        if let Some(list) = self.view.downcast_ref::<gtk::ListView>() {
            list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        } else if let Some(grid) = self.view.downcast_ref::<gtk::GridView>() {
            grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        }
        true
    }

    fn first_visual_row(&self, position: u32) -> bool {
        if self.view.is::<gtk::ListView>() {
            return position == 0;
        }
        let bounds_for = |target| {
            self.bound_at(target)
                .and_then(|(_, widget)| widget.compute_bounds(&self.view))
        };
        let (Some(first), Some(current)) = (bounds_for(0), bounds_for(position)) else {
            return false;
        };
        (first.y() - current.y()).abs() < 1.0
    }

    fn rename_widgets(&self, position: u32) -> Option<(gtk::Entry, gtk::Widget)> {
        let (_, widget) = self.bound_at(position)?;
        if let Some(card) = widget
            .downcast_ref::<gtk::Box>()
            .filter(|card| card.has_css_class("icons-card"))
        {
            let (_, label) = super::icons_cell::parts(card)?;
            let field = super::icons_cell::ensure_rename_field(card)?;
            return Some((field, label.upcast()));
        }
        let display = widget
            .first_child()?
            .next_sibling()?
            .downcast::<gtk::Box>()
            .ok()?;
        let field = display.next_sibling()?.downcast::<gtk::Entry>().ok()?;
        Some((field, display.upcast()))
    }

    fn rename_label(&self, position: u32) -> Option<gtk::Widget> {
        self.bound.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position)
                .then(|| bound.rename_label.upgrade())
                .flatten()
        })
    }

    fn update(&self, items: &[SearchItem], recursive: bool) {
        self.gesture_selection.borrow_mut().take();
        let selected_paths: Vec<_> = self
            .selected_positions()
            .into_iter()
            .filter_map(|position| self.item(position as u32).map(|item| item.path))
            .collect();
        let selected_slot = self.selected_positions().into_iter().next();
        let focused_path = self
            .view
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| self.position_at(&focus))
            .and_then(|position| self.item(position).map(|item| item.path));

        self.positions.replace(
            items
                .iter()
                .enumerate()
                .map(|(position, item)| (item.path.clone(), position))
                .collect(),
        );
        let wanted: HashSet<_> = items.iter().map(|item| item.path.clone()).collect();
        for position in (0..self.model.n_items()).rev() {
            let remove = self
                .model
                .item(position)
                .and_downcast::<glib::BoxedAnyObject>()
                .is_none_or(|object| !wanted.contains(&object.borrow::<SearchItem>().path));
            if remove {
                self.model.remove(position);
            }
        }
        let mut existing = HashMap::new();
        for position in 0..self.model.n_items() {
            if let Some(object) = self
                .model
                .item(position)
                .and_downcast::<glib::BoxedAnyObject>()
            {
                let path = object.borrow::<SearchItem>().path.clone();
                existing.insert(path, object);
            }
        }
        for item in items {
            let unchanged = existing.get(&item.path).is_some_and(|object| {
                let old = object.borrow::<SearchItem>();
                old.name == item.name && old.is_directory == item.is_directory
            });
            if !unchanged {
                if let Some(object) = existing.remove(&item.path)
                    && let Some(position) = self.model.find(&object)
                {
                    self.model.remove(position);
                }
                self.model.append(&glib::BoxedAnyObject::new(item.clone()));
            }
        }
        self.sorter.changed(gtk::SorterChange::Different);
        let selected = gtk::Bitset::new_empty();
        for position in 0..self.selection.n_items() {
            if self
                .item(position)
                .is_some_and(|item| selected_paths.contains(&item.path))
            {
                selected.add(position);
            }
        }
        if selected.is_empty()
            && let Some(position) = selected_slot.filter(|_| !items.is_empty())
        {
            selected.add(position.min(items.len() - 1) as u32);
        }
        self.selection.set_selection(
            &selected,
            &gtk::Bitset::new_range(0, self.selection.n_items()),
        );
        self.gesture_selection.replace(Some(selected));
        if let Some(position) = focused_path.as_ref().and_then(|path| {
            (0..self.selection.n_items())
                .find(|position| self.item(*position).is_some_and(|item| &item.path == path))
        }) {
            self.focus(position, false);
        }
        self.refresh_bindings(recursive);
    }

    fn refresh_bindings(&self, recursive: bool) {
        self.bound.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            if let Some(result) = self.item(item.position()) {
                bind_result_widget_with_root(
                    &self.kind, &widget, &item, &result, &self.root, recursive,
                );
            }
            true
        });
    }

    fn clear(&self) {
        super::thumbnail::cancel_thumbnails_in(&self.view);
        self.model.remove_all();
        self.positions.borrow_mut().clear();
        self.anchor.set(None);
        self.gesture_selection.borrow_mut().take();
    }

    fn refresh_cut_rows(&self) {
        self.bound.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            if let (Some(result), Some(row)) = (
                self.item(item.position()),
                widget.downcast_ref::<gtk::Box>(),
            ) {
                super::browser::set_cut_result_style(row, &Location::local(&result.path));
            }
            true
        });
    }

    fn set_max_columns(&self, max_columns: u32) {
        if let Some(grid) = self.view.downcast_ref::<gtk::GridView>() {
            grid.set_max_columns(max_columns);
        }
    }
}

struct State {
    entry: glib::WeakRef<gtk::Entry>,
    stack: gtk::Stack,
    status: gtk::Label,
    collection: ResultCollection,
    items: Rc<RefCell<Vec<SearchItem>>>,
    handle: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
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
            .filter_map(|position| {
                self.items
                    .borrow()
                    .get(position)
                    .map(super::browser::search_result_entry)
            })
            .collect()
    }

    fn emit_selection_changed(&self) {
        let entries = self.selected_entries();
        for callback in self.selection_callbacks.borrow().iter() {
            callback(entries.clone());
        }
    }

    fn activate(&self, position: u32) -> bool {
        let Some(entry) = self
            .items
            .borrow()
            .get(position as usize)
            .map(super::browser::search_result_entry)
        else {
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
}

impl InlineSearch {
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

    pub fn install_context_menu(&self, view: &Rc<super::browser::ViewState>, depth: usize) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(state);
        let resolve = Rc::new(move |picked: &gtk::Widget| {
            let state = weak.upgrade()?;
            let position = state.collection.position_at(picked)?;
            let entry = state
                .items
                .borrow()
                .get(position as usize)
                .map(super::browser::search_result_entry)?;
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
        let trigger = super::browser::install_resolved_item_context_menu(
            view,
            &state.collection.view,
            resolve,
            depth,
        );
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

    pub(in crate::ui) fn begin_rename(
        &self,
        entry: &FileEntry,
        active: Rc<RefCell<Option<super::browser_modes::ActiveModeRename>>>,
        browser: std::rc::Weak<Browser>,
        view_state: std::rc::Weak<super::browser::ViewState>,
    ) -> bool {
        let Some(state) = self
            .state
            .as_ref()
            .filter(|state| state.stack.visible_child_name().as_deref() == Some("search"))
        else {
            return false;
        };
        let Some(path) = entry.location.native_path() else {
            return false;
        };
        let Some(position) = state
            .items
            .borrow()
            .iter()
            .position(|item| item.path == path)
        else {
            return false;
        };
        let position = position as u32;
        let Some((field, display)) = state.collection.rename_widgets(position) else {
            return false;
        };
        field.set_text(&entry.display_name);
        field.set_sensitive(true);
        field.remove_css_class("error");
        field.set_tooltip_text(None);
        display.set_visible(false);
        field.set_visible(true);
        super::browser_modes::install_mode_rename_handlers(
            &field,
            active.clone(),
            browser,
            view_state,
        );
        active.replace(Some(super::browser_modes::ActiveModeRename::new(
            entry.clone(),
            field.clone(),
            display,
            None,
        )));
        field.grab_focus();
        let weak_field = field.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(field) = weak_field.upgrade()
                && gtk::prelude::WidgetExt::is_visible(&field)
            {
                field.grab_focus();
            }
        });
        field.select_region(
            0,
            if entry.is_directory() {
                -1
            } else {
                super::browser::rename_stem_end(&entry.display_name)
            },
        );
        true
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
            .items
            .borrow()
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
            .items
            .borrow()
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

    pub fn refresh_source_filter(&self, browser: &Browser) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        let handle = state.handle.borrow();
        let Some(handle) = handle.as_ref() else {
            return;
        };
        let items = state
            .items
            .borrow()
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
        if state.handle.borrow().is_none() {
            return;
        }
        let pruned: Vec<_> = state
            .items
            .borrow()
            .iter()
            .filter(|item| search_path_present(&item.path))
            .cloned()
            .collect();
        if pruned.len() != state.items.borrow().len() {
            update_results(state, pruned, state.recursive.get());
        }
    }

    pub(in crate::ui) fn set_icons_max_columns(&self, max_columns: u32) {
        if let Some(state) = self.state.as_ref() {
            state.collection.set_max_columns(max_columns);
        }
    }

    pub fn show_directory_listing(&self) {
        if let Some(state) = self.state.as_ref() {
            show_directory_listing(state);
        }
    }
}

fn show_directory_listing(state: &State) {
    state.generation.set(state.generation.get().wrapping_add(1));
    state.handle.borrow_mut().take();
    state.items.borrow_mut().clear();
    state.collection.clear();
    state.stack.set_visible_child_name("files");
}

fn new_row() -> (gtk::Box, gtk::Widget) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("file-row");
    row.add_css_class("filter-result");
    let icon = super::thumbnail::ThumbnailSlot::new(17);
    row.append(&icon);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    name.add_css_class("alternate-rename-label");
    let origin = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    origin.add_css_class("file-search-path");
    labels.append(&name);
    labels.append(&origin);
    let field = gtk::Entry::new();
    field.add_css_class("inline-rename");
    super::accessibility::set_label(&field, "Rename");
    field.set_hexpand(true);
    field.set_width_chars(1);
    field.set_visible(false);
    row.append(&labels);
    row.append(&field);
    (row, name.upcast())
}

fn row_parts(
    widget: &gtk::Widget,
) -> Option<(
    super::thumbnail::ThumbnailSlot,
    gtk::Box,
    gtk::Label,
    gtk::Label,
    gtk::Entry,
)> {
    let row = widget.downcast_ref::<gtk::Box>()?;
    let icon = row
        .first_child()?
        .downcast::<super::thumbnail::ThumbnailSlot>()
        .ok()?;
    let labels = icon.next_sibling()?.downcast::<gtk::Box>().ok()?;
    let name = labels.first_child()?.downcast::<gtk::Label>().ok()?;
    let origin = name.next_sibling()?.downcast::<gtk::Label>().ok()?;
    let field = labels.next_sibling()?.downcast::<gtk::Entry>().ok()?;
    Some((icon, labels, name, origin, field))
}

fn collection_entry(model: &impl IsA<gio::ListModel>, position: u32) -> Option<FileEntry> {
    let object = model
        .item(position)?
        .downcast::<glib::BoxedAnyObject>()
        .ok()?;
    Some(super::browser::search_result_entry(
        &object.borrow::<SearchItem>(),
    ))
}

fn pointer_selection(
    previous: Option<&gtk::Bitset>,
    position: u32,
    control: bool,
    shift: bool,
    anchor: Option<u32>,
    multiple: bool,
) -> gtk::Bitset {
    if !multiple {
        return gtk::Bitset::new_range(position, 1);
    }
    if shift {
        let anchor = anchor.unwrap_or(position);
        let start = anchor.min(position);
        return gtk::Bitset::new_range(start, anchor.max(position) - start + 1);
    }
    if control {
        let selected = previous.map_or_else(gtk::Bitset::new_empty, gtk::Bitset::copy);
        if selected.contains(position) {
            selected.remove(position);
        } else {
            selected.add(position);
        }
        return selected;
    }
    if let Some(selected) = previous.filter(|selected| {
        super::browser::should_preserve_drag_selection(selected.contains(position), selected.size())
    }) {
        return selected.copy();
    }
    gtk::Bitset::new_range(position, 1)
}

fn install_result_interactions(
    widget: &gtk::Widget,
    item: &gtk::ListItem,
    interactions: &CollectionInteractions,
) {
    let selection = &interactions.selection;
    let anchor = &interactions.anchor;
    let items = &interactions.items;
    let multiple_selection = &interactions.behavior.multiple_selection;
    let gesture_selection = &interactions.gesture_selection;
    let pointer_activation = &interactions.pointer_activation;
    let focus_items = &interactions.behavior.focus_items;
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let modified_press = Rc::new(Cell::new(false));
    let modified_for_press = modified_press.clone();
    let modified_for_cancel = modified_press.clone();
    let modified_for_release = modified_press.clone();
    let weak_item = item.downgrade();
    let selection_for_press = selection.clone();
    let anchor_for_press = anchor.clone();
    let multiple_for_press = multiple_selection.clone();
    let gesture_selection_for_press = gesture_selection.clone();
    let pointer_for_press = pointer_activation.clone();
    let focus_items_for_press = focus_items.clone();
    click.connect_pressed(move |gesture, _, _, _| {
        let Some(item) = weak_item.upgrade() else {
            return;
        };
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let modifiers = gesture.current_event_state();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        modified_for_press.set(control || shift);
        pointer_for_press.set(Some(!control && !shift));
        let selected = pointer_selection(
            gesture_selection_for_press.borrow().as_ref(),
            position,
            control,
            shift,
            anchor_for_press.get(),
            multiple_for_press.get(),
        );
        if !shift {
            anchor_for_press.set(Some(position));
        }
        gesture_selection_for_press.replace(Some(selected.clone()));
        selection_for_press.set_selection(
            &selected,
            &gtk::Bitset::new_range(0, selection_for_press.n_items()),
        );
        if let Some(widget) = gesture.widget().and_then(|widget| widget.parent()) {
            widget.grab_focus();
        }
        focus_items_for_press();
    });
    let pointer_for_cancel = pointer_activation.clone();
    click.connect_cancel(move |_, _| {
        modified_for_cancel.set(false);
        pointer_for_cancel.set(None);
    });
    let pointer_for_release = pointer_activation.clone();
    click.connect_released(move |gesture, _, _, _| {
        if modified_for_release.replace(false) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
        let pointer = pointer_for_release.clone();
        glib::idle_add_local_once(move || pointer.set(None));
    });

    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak_item = item.downgrade();
    let weak_widget = widget.downgrade();
    let selection_for_drag = selection.clone();
    let items_for_drag = items.clone();
    drag.connect_prepare(move |source, _, _| {
        let item = weak_item.upgrade()?;
        let widget = weak_widget.upgrade()?;
        let position = item.position();
        let items = items_for_drag.borrow();
        let positions: Vec<_> = if selection_for_drag.is_selected(position) {
            (0..selection_for_drag.n_items())
                .filter(|position| selection_for_drag.is_selected(*position))
                .collect()
        } else {
            vec![position]
        };
        let entries: Vec<_> = positions
            .into_iter()
            .filter_map(|position| items.get(position as usize))
            .map(super::browser::search_result_entry)
            .collect();
        source.set_actions(super::browser::drag_actions_for_modifiers(
            source.current_event_state(),
        ));
        source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&widget))), 0, 0);
        super::browser::file_drag_content(&entries)
    });
    widget.add_controller(drag.clone());
    widget.add_controller(click.clone());
    drag.group_with(&click);
}

fn build_collection(
    presentation: SearchPresentation,
    items: Rc<RefCell<Vec<SearchItem>>>,
    recursive: Rc<Cell<bool>>,
    root: PathBuf,
    behavior: CollectionBehavior,
) -> (ResultCollection, gtk::ScrolledWindow, gtk::Overlay) {
    let multiple_selection = behavior.multiple_selection.clone();
    let activate = behavior.activate.clone();
    let single_click = behavior.single_click.clone();
    let (kind, max_columns) = match presentation {
        SearchPresentation::Rows => (ResultKind::Rows, None),
        SearchPresentation::Icons {
            thumbnail_size,
            max_columns,
        } => (ResultKind::Icons { thumbnail_size }, Some(max_columns)),
    };
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    let positions = Rc::new(RefCell::new(HashMap::<PathBuf, usize>::new()));
    let positions_for_sort = positions.clone();
    let sorter = gtk::CustomSorter::new(move |left, right| {
        let position = |object: &glib::Object| {
            object
                .downcast_ref::<glib::BoxedAnyObject>()
                .and_then(|object| {
                    positions_for_sort
                        .borrow()
                        .get(&object.borrow::<SearchItem>().path)
                        .copied()
                })
                .unwrap_or(usize::MAX)
        };
        position(left).cmp(&position(right)).into()
    });
    let sorted = gtk::SortListModel::new(Some(model.clone()), Some(sorter.clone()));
    let selection = gtk::MultiSelection::new(Some(sorted.clone()));
    let bound: Rc<RefCell<Vec<BoundResult>>> = Rc::new(RefCell::new(Vec::new()));
    let anchor = Rc::new(Cell::new(None));
    let gesture_selection = Rc::new(RefCell::new(None::<gtk::Bitset>));
    let pointer_activation = Rc::new(Cell::new(None::<bool>));
    let syncing_selection = Rc::new(Cell::new(false));
    let syncing_for_selection = syncing_selection.clone();
    let multiple_for_selection = multiple_selection.clone();
    let gesture_selection_for_change = gesture_selection.clone();
    selection.connect_selection_changed(move |selection, position, count| {
        if syncing_for_selection.replace(true) {
            return;
        }
        let selected = selection.selection();
        if multiple_for_selection.get() {
            let restored = (selected.size() == 1)
                .then(|| (0..selection.n_items()).find(|position| selected.contains(*position)))
                .flatten()
                .and_then(|position| {
                    gesture_selection_for_change
                        .borrow()
                        .clone()
                        .filter(|preserved| preserved.size() > 1 && preserved.contains(position))
                });
            if let Some(preserved) = restored {
                selection
                    .set_selection(&preserved, &gtk::Bitset::new_range(0, selection.n_items()));
            } else if selected.size() > 1 {
                gesture_selection_for_change.replace(Some(selected.copy()));
            }
        } else if selected.size() > 1 {
            let end = position.saturating_add(count);
            let focused = (position..end)
                .rev()
                .find(|position| selection.is_selected(*position))
                .or_else(|| {
                    (0..selection.n_items())
                        .rev()
                        .find(|position| selection.is_selected(*position))
                });
            if let Some(focused) = focused {
                selection.select_item(focused, true);
            }
        }
        syncing_for_selection.set(false);
    });
    let factory = gtk::SignalListItemFactory::new();
    let kind_for_setup = kind.clone();
    let bound_for_setup = bound.clone();
    let interactions_for_setup = CollectionInteractions {
        selection: selection.clone(),
        anchor: anchor.clone(),
        items: items.clone(),
        gesture_selection: gesture_selection.clone(),
        pointer_activation: pointer_activation.clone(),
        behavior: behavior.clone(),
    };
    factory.connect_setup(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (widget, rename_label) = match &kind_for_setup {
            ResultKind::Rows => new_row(),
            ResultKind::Icons { thumbnail_size } => {
                let card = super::icons_cell::new_card(thumbnail_size.get());
                let Some((_, label)) = super::icons_cell::parts(&card) else {
                    return;
                };
                (card, label.upcast())
            }
        };
        widget.add_css_class("filter-result");
        install_result_interactions(widget.upcast_ref(), item, &interactions_for_setup);
        item.set_child(Some(&widget));
        let interaction = widget.parent().unwrap_or_else(|| widget.clone().upcast());
        if widget.has_css_class("icons-card") {
            interaction.set_halign(gtk::Align::Center);
            interaction.set_valign(gtk::Align::Start);
        }
        let item_ref = glib::WeakRef::new();
        item_ref.set(Some(item));
        let widget_ref = glib::WeakRef::new();
        widget_ref.set(Some(widget.upcast_ref()));
        let label_ref = glib::WeakRef::new();
        label_ref.set(Some(&rename_label));
        bound_for_setup.borrow_mut().push(BoundResult {
            item: item_ref,
            widget: widget_ref,
            rename_label: label_ref,
        });
    });
    let kind_for_bind = kind.clone();
    let recursive_for_bind = recursive.clone();
    let root_for_bind = root.clone();
    factory.connect_bind(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(widget) = item.child() else {
            return;
        };
        let Some(result) = item
            .item()
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|object| object.borrow::<SearchItem>().clone())
        else {
            return;
        };
        bind_result_widget_with_root(
            &kind_for_bind,
            &widget,
            item,
            &result,
            &root_for_bind,
            recursive_for_bind.get(),
        );
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));

    let view: gtk::Widget = match max_columns {
        Some(max_columns) => {
            let grid = gtk::GridView::new(Some(selection.clone()), Some(factory));
            grid.add_css_class("file-icons");
            grid.set_min_columns(1);
            grid.set_max_columns(max_columns);
            grid.set_enable_rubberband(false);
            grid.set_single_click_activate(true);
            grid.set_vexpand(false);
            grid.upcast()
        }
        None => {
            let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
            list.add_css_class("file-list");
            list.set_enable_rubberband(false);
            list.set_single_click_activate(true);
            list.set_vexpand(true);
            list.upcast()
        }
    };
    view.add_css_class("search-results");
    super::accessibility::set_label(&view, SEARCH_RESULTS_LABEL);
    if let ResultKind::Icons { thumbnail_size } = &kind {
        let last_size = Cell::new(thumbnail_size.get());
        let thumbnail_size = thumbnail_size.clone();
        let bound = bound.clone();
        view.add_tick_callback(move |_, _| {
            let size = thumbnail_size.get();
            if last_size.replace(size) != size {
                bound.borrow_mut().retain(|bound| {
                    let Some(widget) = bound.widget.upgrade() else {
                        return false;
                    };
                    if let Some(card) = widget.downcast_ref::<gtk::Box>() {
                        super::icons_cell::set_slot(card, size);
                    }
                    bound.item.upgrade().is_some()
                });
            }
            glib::ControlFlow::Continue
        });
    }
    let sorted_for_activate = sorted.clone();
    let selection_for_activate = selection.clone();
    let dispatch_activate: Rc<dyn Fn(u32)> = Rc::new(move |position| {
        let Some(entry) = collection_entry(&sorted_for_activate, position) else {
            return;
        };
        match pointer_activation.take() {
            Some(true) if selection_for_activate.selection().size() <= 1 => single_click(entry),
            Some(_) => {}
            None => activate(entry),
        }
    });
    if let Some(list) = view.downcast_ref::<gtk::ListView>() {
        let dispatch = dispatch_activate.clone();
        list.connect_activate(move |_, position| dispatch(position));
    } else if let Some(grid) = view.downcast_ref::<gtk::GridView>() {
        grid.connect_activate(move |_, position| dispatch_activate(position));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    (
        ResultCollection {
            view,
            model,
            sorted,
            sorter,
            positions,
            selection,
            bound,
            anchor,
            gesture_selection,
            kind,
            root,
        },
        scroll,
        overlay,
    )
}

fn bind_result_widget_with_root(
    kind: &ResultKind,
    widget: &gtk::Widget,
    item: &gtk::ListItem,
    result: &SearchItem,
    root: &Path,
    recursive: bool,
) {
    let entry = super::browser::search_result_entry(result);
    super::accessibility::describe_entry(item, &result.name, Some(&entry));
    match kind {
        ResultKind::Rows => {
            let Some((icon, labels, name, origin, field)) = row_parts(widget) else {
                return;
            };
            labels.set_visible(true);
            field.set_visible(false);
            name.set_text(&result.name);
            origin.set_text(&relative_result_path(root, &result.path));
            origin.set_visible(recursive);
            widget.set_tooltip_text(Some(&relative_result_path(root, &result.path)));
            if result.is_directory {
                super::thumbnail::show_customized_icon(
                    &icon,
                    &result.path,
                    crate::assets::icons::FOLDER,
                    17,
                );
            } else {
                super::thumbnail::set_thumbnail_or_icon_for_path(
                    &icon,
                    &result.path,
                    crate::assets::icons::DOCUMENTS,
                    17,
                    17,
                );
            }
        }
        ResultKind::Icons { thumbnail_size } => {
            let Some(card) = widget.downcast_ref::<gtk::Box>() else {
                return;
            };
            let Some((icon, label)) = super::icons_cell::parts(card) else {
                return;
            };
            super::icons_cell::set_slot(card, thumbnail_size.get());
            label.set_text(Some(&result.name));
            label.set_tooltip_text(Some(&result.name));
            label.set_visible(true);
            if let Some(field) = super::icons_cell::rename_field(card) {
                field.set_visible(false);
            }
            if let Some(details) = super::icons_cell::details_label(card) {
                details.add_css_class("file-search-path");
                details.set_text(&relative_result_path(root, &result.path));
                details.set_visible(recursive);
            }
            if result.is_directory {
                super::thumbnail::show_customized_icon(
                    &icon,
                    &result.path,
                    crate::assets::icons::FOLDER,
                    thumbnail_size.get(),
                );
            } else {
                super::thumbnail::set_thumbnail_or_icon_for_path(
                    &icon,
                    &result.path,
                    crate::assets::icons::DOCUMENTS,
                    thumbnail_size.get(),
                    thumbnail_size.get(),
                );
            }
        }
    }
    if let Some(row) = widget.downcast_ref::<gtk::Box>() {
        super::browser::set_cut_result_style(row, &Location::local(&result.path));
    }
}

fn install_marquee(state: &Rc<State>, scroll: &gtk::ScrolledWindow, overlay: &gtk::Overlay) {
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
        allow_drag: Rc::new(Cell::new(true)),
    });
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
    let items = Rc::new(RefCell::new(Vec::new()));
    let recursive = Rc::new(Cell::new(false));
    let (collection, scroll, overlay) = build_collection(
        presentation,
        items.clone(),
        recursive.clone(),
        root.clone(),
        CollectionBehavior {
            multiple_selection,
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
        items: items.clone(),
        handle: RefCell::new(None),
        generation: Cell::new(0),
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
    install_marquee(&state, &scroll, &overlay);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&state);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if state.handle.borrow().is_none()
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
    };
    super::browser::bind_filter_query(entry, move |text, is_recursive, restart| {
        state.recursive.set(is_recursive);
        recursive.set(is_recursive);
        if restart {
            state.generation.set(state.generation.get().wrapping_add(1));
            state.handle.borrow_mut().take();
        }
        let query = text.trim();
        if query.is_empty() {
            show_directory_listing(&state);
            return;
        }
        state.stack.set_visible_child_name("search");
        if state.items.borrow().is_empty() {
            state.status.set_text("Searching…");
            state.status.set_visible(true);
        }
        if let Some(handle) = state.handle.borrow().as_ref() {
            handle.query(query);
            return;
        }
        let generation = state.generation.get();
        let show_hidden = weak_browser
            .upgrade()
            .is_some_and(|browser| browser.preferences().show_hidden);
        let (handle, receiver) = index_filter(root.clone(), show_hidden, is_recursive);
        handle.query(query);
        state.handle.replace(Some(handle));
        let weak = Rc::downgrade(&state);
        let browser = weak_browser.clone();
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(entry) = state.entry.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.generation.get() != generation || state.handle.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            let mut latest = None;
            for event in receiver.try_iter().take(8) {
                latest = Some(event);
            }
            if let Some(SearchEvent::Results {
                query: returned,
                items,
                indexing,
                coverage,
                has_more,
            }) = latest
                && !returned.is_empty()
                && returned == entry.text().trim()
            {
                let Some(browser) = browser.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let items = eligible_results(
                    &browser,
                    state.handle.borrow().as_ref().expect("active search"),
                    &returned,
                    items,
                    has_more,
                );
                state
                    .status
                    .set_visible(items.is_empty() || coverage.is_partial());
                state.status.set_text(&if coverage.is_partial() {
                    coverage.message()
                } else if indexing {
                    "Searching…".to_owned()
                } else {
                    "No matching files".to_owned()
                });
                update_results(&state, items, state.recursive.get());
            }
            glib::ControlFlow::Continue
        });
    });
    search
}

fn update_results(state: &State, items: Vec<SearchItem>, recursive: bool) {
    state.items.replace(items.clone());
    state.collection.update(&items, recursive);
}

pub(super) fn eligible_results(
    browser: &Browser,
    handle: &crate::services::SearchHandle,
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

fn relative_result_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;
