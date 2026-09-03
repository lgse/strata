// SPDX-License-Identifier: GPL-3.0-or-later

//! Alternate browser presentations.
//!
//! This module is deliberately isolated from the Miller-column implementation. It consumes the
//! same application events and emits the same navigation/selection intents, so adding another
//! presentation does not require scattering mode checks throughout the main browser view.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::{Browser, BrowserColumnSnapshot, BrowserEvent},
    model::{FileEntry, Location, MetadataValue, SortDirection, SortKey},
};

const EXPLORER_COLUMN_WIDTHS: [i32; 4] = [600, 90, 120, 150];
const DEFAULT_GRID_THUMBNAIL_SIZE: i32 = 64;
const MIN_GRID_THUMBNAIL_SIZE: i32 = 64;
const MAX_GRID_THUMBNAIL_SIZE: i32 = 256;

#[derive(Clone)]
struct ExplorerColumnLayout {
    widths: Rc<Vec<Cell<i32>>>,
    cells: Rc<Vec<RefCell<Vec<glib::WeakRef<gtk::Widget>>>>>,
}

impl ExplorerColumnLayout {
    fn new() -> Self {
        Self {
            widths: Rc::new(EXPLORER_COLUMN_WIDTHS.into_iter().map(Cell::new).collect()),
            cells: Rc::new((0..4).map(|_| RefCell::new(Vec::new())).collect()),
        }
    }
}

type TransferHandler = Rc<dyn Fn(Location, Vec<Location>, bool)>;
type TransferHandlerSlot = Rc<RefCell<Option<TransferHandler>>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserMode {
    #[default]
    Columns,
    Grid,
    Explorer,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BrowserDensity {
    #[default]
    Compact,
    Airy,
}

struct ActiveModeRename {
    field: gtk::Entry,
    label: gtk::Label,
}

struct ActiveModeNewEntry {
    is_directory: bool,
    field: gtk::Entry,
    placeholder: Option<gtk::StringList>,
    stack: Option<gtk::Stack>,
    source_model: Option<gtk::StringList>,
    view: gtk::Widget,
}

struct BoundModeItem {
    item: glib::WeakRef<gtk::ListItem>,
    widget: glib::WeakRef<gtk::Widget>,
}

#[derive(Clone)]
struct Pane {
    depth: usize,
    shell: gtk::Box,
    model: gtk::StringList,
    selection: gtk::MultiSelection,
    filtered_model: Option<gio::ListModel>,
    filter_model: Option<gtk::FilterListModel>,
    syncing_selection: Rc<Cell<bool>>,
    stack: gtk::Stack,
    status: gtk::Label,
    spinner: gtk::Spinner,
    truncated_hint: gtk::Image,
    view: gtk::Widget,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    filter_entry: Option<gtk::Entry>,
    filter_button: Option<gtk::ToggleButton>,
    empty_trash_button: Option<gtk::Button>,
    new_entry_placeholder: Option<gtk::StringList>,
    new_entry_is_directory: Option<Rc<Cell<bool>>>,
}

pub struct ModeViews {
    stack: gtk::Stack,
    grid_root: gtk::Box,
    explorer_root: gtk::Box,
    grid_panes: Vec<Pane>,
    explorer_pane: Option<Pane>,
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    context_state: RefCell<Option<Weak<super::browser::ViewState>>>,
    active_rename: Rc<RefCell<Option<ActiveModeRename>>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    mode: BrowserMode,
    density: BrowserDensity,
    grid_thumbnail_size: Rc<Cell<i32>>,
}

impl ModeViews {
    pub fn new(columns: &gtk::ScrolledWindow, browser: Rc<Browser>) -> Self {
        let grid_root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grid_root.add_css_class("mode-grid-columns");
        grid_root.set_halign(gtk::Align::Fill);
        grid_root.set_hexpand(true);
        grid_root.set_vexpand(true);
        let grid_scroll = gtk::ScrolledWindow::builder()
            .child(&grid_root)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();

        let explorer_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        explorer_root.add_css_class("mode-explorer");
        explorer_root.set_hexpand(true);
        explorer_root.set_vexpand(true);
        // The explorer pane header belongs to the viewport, while its user-resizable table
        // columns scroll independently below it.
        let explorer_scroll = gtk::ScrolledWindow::builder()
            .child(&explorer_root)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(120)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(columns, Some("columns"));
        stack.add_named(&grid_scroll, Some("grid"));
        stack.add_named(&explorer_scroll, Some("explorer"));
        stack.set_visible_child_name("columns");

        Self {
            stack,
            grid_root,
            explorer_root,
            grid_panes: Vec::new(),
            explorer_pane: None,
            browser,
            single_click_previews: Rc::new(Cell::new(true)),
            transfer_handler: Rc::new(RefCell::new(None)),
            cut_locations: Rc::new(RefCell::new(HashSet::new())),
            context_state: RefCell::new(None),
            active_rename: Rc::new(RefCell::new(None)),
            active_new_entry: Rc::new(RefCell::new(None)),
            mode: BrowserMode::Columns,
            density: BrowserDensity::Compact,
            grid_thumbnail_size: Rc::new(Cell::new(DEFAULT_GRID_THUMBNAIL_SIZE)),
        }
    }

    pub fn widget(&self) -> gtk::Stack {
        self.stack.clone()
    }

    pub fn mode(&self) -> BrowserMode {
        self.mode
    }

    pub fn selected_positions(&self) -> Option<(usize, Vec<usize>)> {
        let pane = match self.mode {
            BrowserMode::Columns => return None,
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
        }?;
        let positions = bitset_positions(&pane.selection.selection())
            .into_iter()
            .filter_map(|position| {
                source_position_for_view(&pane.model, pane.filtered_model.as_ref(), position as u32)
            })
            .collect();
        Some((pane.depth, positions))
    }

    pub fn rename_is_active(&self) -> bool {
        self.active_rename.borrow().is_some()
    }

    pub fn new_entry_is_active(&self) -> bool {
        self.active_new_entry.borrow().is_some()
    }

    pub fn cancel_new_entry(&self) -> bool {
        let Some(active) = self.active_new_entry.take() else {
            return false;
        };
        active.field.set_text("");
        active.field.remove_css_class("error");
        active.field.set_tooltip_text(None);
        finish_mode_new_entry(&active);
        true
    }

    pub fn begin_new_entry(&self, depth: usize, is_directory: bool) -> bool {
        self.cancel_new_entry();
        self.cancel_rename();
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Grid => self.grid_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        let Some(placeholder) = pane.new_entry_placeholder.as_ref() else {
            return false;
        };
        let Some(entry_kind) = pane.new_entry_is_directory.as_ref() else {
            return false;
        };
        entry_kind.set(is_directory);
        placeholder.splice(0, placeholder.n_items(), &[""]);
        pane.stack.set_visible_child_name("content");
        let bound_items = pane.bound_items.clone();
        let active = self.active_new_entry.clone();
        let placeholder = placeholder.clone();
        let stack = pane.stack.clone();
        let source_model = pane.model.clone();
        let view = pane.view.clone();
        view.add_css_class("creating-entry");
        if let Ok(grid) = view.clone().downcast::<gtk::GridView>() {
            grid.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
        } else if let Ok(list) = view.clone().downcast::<gtk::ListView>() {
            list.scroll_to(0, gtk::ListScrollFlags::FOCUS, None);
        }
        glib::idle_add_local_once(move || {
            let field = bound_items.borrow().iter().find_map(|bound| {
                let item = bound.item.upgrade()?;
                if item.position() != 0 {
                    return None;
                }
                let widget = bound.widget.upgrade()?;
                descendant_with_class(&widget, "inline-rename")?
                    .downcast::<gtk::Entry>()
                    .ok()
            });
            let Some(field) = field else {
                placeholder.splice(0, placeholder.n_items(), &[]);
                view.remove_css_class("creating-entry");
                return;
            };
            field.set_text("");
            active.replace(Some(ActiveModeNewEntry {
                is_directory,
                field: field.clone(),
                placeholder: Some(placeholder),
                stack: Some(stack),
                source_model: Some(source_model),
                view,
            }));
            field.grab_focus();
        });
        true
    }

    pub fn cancel_rename(&self) -> bool {
        let Some(rename) = self.active_rename.take() else {
            return false;
        };
        rename.label.set_visible(true);
        rename.field.set_visible(false);
        rename.field.set_sensitive(true);
        true
    }

    pub fn begin_rename(&self, depth: usize, source_position: usize, entry: &FileEntry) -> bool {
        self.cancel_rename();
        let pane = match self.mode {
            BrowserMode::Columns => return false,
            BrowserMode::Grid => self.grid_panes.iter().find(|pane| pane.depth == depth),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth),
        };
        let Some(pane) = pane else {
            return false;
        };
        let Some(position) =
            view_position_for_source(&pane.model, pane.filtered_model.as_ref(), source_position)
        else {
            return false;
        };
        let widget = pane.bound_items.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position).then(|| bound.widget.upgrade())?
        });
        let Some(widget) = widget else {
            return false;
        };
        let Some(label) =
            descendant_with_class(&widget, "alternate-rename-label").and_downcast::<gtk::Label>()
        else {
            return false;
        };
        let Some(field) =
            descendant_with_class(&widget, "inline-rename").and_downcast::<gtk::Entry>()
        else {
            return false;
        };
        field.set_text(&entry.display_name);
        field.set_visible(true);
        label.set_visible(false);
        let browser = Rc::downgrade(&self.browser);
        let renamed_entry = entry.clone();
        let active = self.active_rename.clone();
        field.connect_activate(move |field| {
            let name = field.text().to_string();
            if name == renamed_entry.display_name {
                if let Some(rename) = active.take() {
                    rename.label.set_visible(true);
                    rename.field.set_visible(false);
                }
            } else if let Some(browser) = browser.upgrade() {
                field.set_sensitive(false);
                browser.rename(renamed_entry.clone(), name);
            }
        });
        field.grab_focus();
        field.select_region(0, super::browser::rename_stem_end(&entry.display_name));
        self.active_rename
            .replace(Some(ActiveModeRename { field, label }));
        true
    }

    pub fn filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| widget_has_focus(entry, focused.as_ref()))
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .any(|pane| widget_has_focus(&pane.view, focused.as_ref()))
    }

    pub fn empty_filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| entry.text().is_empty() && widget_has_focus(entry, focused.as_ref()))
    }

    pub fn show_filter(&self) -> bool {
        let pane = match self.mode {
            BrowserMode::Columns => None,
            BrowserMode::Grid => self.grid_panes.first(),
            BrowserMode::Explorer => self.explorer_pane.as_ref(),
        };
        let Some(pane) = pane else {
            return false;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return false;
        };
        button.set_active(true);
        entry.grab_focus();
        true
    }

    pub fn dismiss_focused_filter(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        let Some(pane) = self
            .grid_panes
            .iter()
            .chain(self.explorer_pane.iter())
            .find(|pane| {
                pane.filter_entry
                    .as_ref()
                    .is_some_and(|entry| widget_has_focus(entry, focused.as_ref()))
            })
        else {
            return false;
        };
        if let Some(button) = pane.filter_button.as_ref() {
            button.set_active(false);
        }
        pane.view.grab_focus();
        true
    }

    pub fn set_mode(&mut self, mode: BrowserMode) {
        self.cancel_new_entry();
        self.cancel_rename();
        self.mode = mode;
        self.stack.set_visible_child_name(match mode {
            BrowserMode::Columns => "columns",
            BrowserMode::Grid => "grid",
            BrowserMode::Explorer => "explorer",
        });
        if mode == BrowserMode::Grid {
            self.rebuild_grid();
        } else if mode == BrowserMode::Explorer {
            self.rebuild_explorer();
        }
        if let Some(depth) = self.browser.active_depth() {
            self.focus_visible_pane(depth);
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.single_click_previews.set(enabled);
    }

    pub fn set_transfer_handler(&self, handler: TransferHandler) {
        self.transfer_handler.replace(Some(handler));
    }

    pub fn set_context_state(&self, state: Weak<super::browser::ViewState>) {
        self.context_state.replace(Some(state));
    }

    pub fn set_cut_locations(&self, locations: &[Location]) {
        self.cut_locations
            .replace(locations.iter().cloned().collect());
        for pane in self.grid_panes.iter().chain(self.explorer_pane.iter()) {
            refresh_cut_pane(pane, &self.browser, locations);
        }
    }

    pub fn set_density(&mut self, density: BrowserDensity) {
        self.density = density;
        for pane in &self.grid_panes {
            configure_grid_density(pane, density);
        }
        for root in [&self.grid_root, &self.explorer_root] {
            root.remove_css_class("density-compact");
            root.remove_css_class("density-airy");
            root.add_css_class(match density {
                BrowserDensity::Compact => "density-compact",
                BrowserDensity::Airy => "density-airy",
            });
        }
    }

    pub fn handle(&mut self, event: &BrowserEvent) {
        if matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnAdded { .. }
        ) {
            self.cancel_new_entry();
        }
        match event {
            BrowserEvent::Reset => {
                clear_box(&self.grid_root);
                self.grid_panes.clear();
                clear_box(&self.explorer_root);
                self.explorer_pane = None;
            }
            BrowserEvent::ColumnsTruncated { len } => {
                while self.grid_panes.len() > *len {
                    if let Some(pane) = self.grid_panes.pop() {
                        self.grid_root.remove(&pane.shell);
                    }
                }
                self.rebuild_grid();
                self.rebuild_explorer();
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                clear_box(&self.grid_root);
                self.grid_panes.clear();
                let pane = build_grid_pane(
                    self.browser.clone(),
                    self.single_click_previews.clone(),
                    self.transfer_handler.clone(),
                    self.cut_locations.clone(),
                    GridOptions {
                        peek_state: self.context_state.borrow().clone(),
                        thumbnail_size: self.grid_thumbnail_size.clone(),
                        active_new_entry: self.active_new_entry.clone(),
                    },
                    *depth,
                    &location.display_name(),
                );
                configure_grid_density(&pane, self.density);
                self.install_context_menu(&pane);
                self.grid_root.append(&pane.shell);
                self.grid_panes.push(pane);

                clear_box(&self.explorer_root);
                let pane = build_explorer_pane(
                    self.browser.clone(),
                    self.single_click_previews.clone(),
                    self.transfer_handler.clone(),
                    self.cut_locations.clone(),
                    self.active_new_entry.clone(),
                    *depth,
                    &location.display_name(),
                );
                self.install_context_menu(&pane);
                self.explorer_root.append(&pane.shell);
                self.explorer_pane = Some(pane);
            }
            BrowserEvent::EntriesInserted { depth, insertions } => {
                for pane in self.panes_at(*depth) {
                    for insertion in insertions {
                        let values: Vec<_> = insertion
                            .entries
                            .iter()
                            .map(|entry| entry.display_name.as_str())
                            .collect();
                        pane.model.splice(insertion.position as u32, 0, &values);
                    }
                    if !pane.spinner.is_spinning() {
                        show_count(pane);
                    }
                }
            }
            BrowserEvent::EntriesReplaced { depth, entries } => {
                for pane in self.panes_at(*depth) {
                    replace_entries(pane, entries);
                }
            }
            BrowserEvent::SortingStarted { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.spinner.set_tooltip_text(Some("Sorting…"));
                    pane.spinner.set_visible(true);
                    pane.spinner.start();
                }
            }
            BrowserEvent::SortingFinished { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.spinner.stop();
                    pane.spinner.set_visible(false);
                    pane.spinner.set_tooltip_text(None);
                }
            }
            BrowserEvent::EntriesSpliced { depth, splices, .. } => {
                for pane in self.panes_at(*depth) {
                    for splice in splices {
                        let values: Vec<_> = splice
                            .entries
                            .iter()
                            .map(|entry| entry.display_name.as_str())
                            .collect();
                        pane.model
                            .splice(splice.position as u32, splice.removed as u32, &values);
                    }
                    show_count(pane);
                }
            }
            BrowserEvent::ColumnReloaded { depth } => {
                for pane in self.panes_at(*depth) {
                    pane.syncing_selection.set(true);
                    pane.selection.set_model(None::<&gio::ListModel>);
                    if let Some(filtered) = pane.filter_model.as_ref() {
                        filtered.set_model(None::<&gio::ListModel>);
                    }
                    pane.model.splice(0, pane.model.n_items(), &[]);
                    pane.truncated_hint.set_visible(false);
                    pane.spinner.set_visible(true);
                    pane.spinner.start();
                    pane.stack.set_visible_child_name("loading");
                }
            }
            BrowserEvent::LoadFinished { depth, truncated } => {
                for pane in self.panes_at(*depth) {
                    reconnect_pane_model(pane);
                    pane.spinner.stop();
                    pane.spinner.set_visible(false);
                    pane.truncated_hint.set_visible(*truncated);
                    show_count(pane);
                }
            }
            BrowserEvent::LoadFailed { depth, message } => {
                for pane in self.panes_at(*depth) {
                    reconnect_pane_model(pane);
                    pane.spinner.stop();
                    pane.status
                        .set_label(&format!("Unable to read this directory\n{message}"));
                    pane.status.add_css_class("error");
                    pane.stack.set_visible_child_name("status");
                }
            }
            BrowserEvent::SelectionSetChanged {
                depth,
                positions,
                take_focus,
                ..
            } => {
                for pane in self.panes_at(*depth) {
                    set_selections(pane, positions);
                }
                if *take_focus {
                    self.focus_visible_pane(*depth);
                }
            }
            BrowserEvent::FocusChanged { depth, position } => {
                for pane in self.panes_at(*depth) {
                    set_selections(pane, &position.iter().copied().collect::<Vec<_>>());
                }
                self.focus_visible_pane(*depth);
            }
            BrowserEvent::RenameCompleted => {
                self.cancel_rename();
            }
            BrowserEvent::RenameFailed { message } => {
                if let Some(rename) = self.active_rename.borrow().as_ref() {
                    rename.field.set_sensitive(true);
                    rename.field.add_css_class("error");
                    rename.field.set_tooltip_text(Some(message));
                    rename.field.grab_focus();
                }
            }
            _ => {}
        }
    }

    fn focus_visible_pane(&self, depth: usize) {
        match self.mode {
            BrowserMode::Columns => {}
            BrowserMode::Grid => {
                if let Some(pane) = self.grid_panes.iter().find(|pane| pane.depth == depth) {
                    pane.view.grab_focus();
                }
            }
            BrowserMode::Explorer => {
                if let Some(pane) = self
                    .explorer_pane
                    .as_ref()
                    .filter(|pane| pane.depth == depth)
                {
                    pane.view.grab_focus();
                }
            }
        }
    }

    fn panes_at(&self, depth: usize) -> Vec<&Pane> {
        match self.mode {
            BrowserMode::Columns => Vec::new(),
            BrowserMode::Grid => self
                .grid_panes
                .iter()
                .find(|pane| pane.depth == depth)
                .into_iter()
                .collect(),
            BrowserMode::Explorer => self
                .explorer_pane
                .as_ref()
                .filter(|pane| pane.depth == depth)
                .into_iter()
                .collect(),
        }
    }

    fn install_context_menu(&self, pane: &Pane) {
        let Some(state) = self.context_state.borrow().as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let items = pane.bound_items.clone();
        let pick_position = Rc::new(move |picked: &gtk::Widget| {
            let mut candidate = Some(picked.clone());
            while let Some(widget) = candidate {
                let position = items.borrow().iter().find_map(|bound| {
                    let bound_widget = bound.widget.upgrade()?;
                    let item = bound.item.upgrade()?;
                    (bound_widget == widget).then_some(item.position())
                });
                if position.is_some() {
                    return position;
                }
                candidate = widget.parent();
            }
            None
        });
        let source = pane.model.clone();
        let filtered = pane.filtered_model.clone();
        let source_position =
            Rc::new(move |position| source_position_for_view(&source, filtered.as_ref(), position));
        if let Some(location) = self.browser.location_at(pane.depth) {
            let item_position = pick_position.clone();
            super::browser::install_folder_context_menu(
                &state,
                pane.stack.upcast_ref(),
                &pane.selection,
                Rc::new(move |picked| item_position(picked).is_some()),
                pane.depth,
                location,
            );
        }
        super::browser::install_item_context_menu(
            &state,
            &pane.view,
            &pane.selection,
            pick_position,
            source_position,
            pane.depth,
        );
    }

    fn rebuild_grid(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        clear_box(&self.grid_root);
        self.grid_panes.clear();
        let pane = build_grid_pane(
            self.browser.clone(),
            self.single_click_previews.clone(),
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            GridOptions {
                peek_state: self.context_state.borrow().clone(),
                thumbnail_size: self.grid_thumbnail_size.clone(),
                active_new_entry: self.active_new_entry.clone(),
            },
            depth,
            &snapshot.location.display_name(),
        );
        configure_grid_density(&pane, self.density);
        apply_snapshot(&pane, &snapshot);
        self.install_context_menu(&pane);
        self.grid_root.append(&pane.shell);
        self.grid_panes.push(pane);
    }

    fn rebuild_explorer(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        clear_box(&self.explorer_root);
        let pane = build_explorer_pane(
            self.browser.clone(),
            self.single_click_previews.clone(),
            self.transfer_handler.clone(),
            self.cut_locations.clone(),
            self.active_new_entry.clone(),
            depth,
            &snapshot.location.display_name(),
        );
        apply_snapshot(&pane, &snapshot);
        self.install_context_menu(&pane);
        self.explorer_root.append(&pane.shell);
        self.explorer_pane = Some(pane);
    }
}

fn widget_has_focus(widget: &impl IsA<gtk::Widget>, focused: Option<&gtk::Widget>) -> bool {
    widget.has_focus()
        || focused.is_some_and(|focused| {
            focused == widget.as_ref() || focused.is_ancestor(widget.as_ref())
        })
}

#[derive(Clone)]
struct GridOptions {
    peek_state: Option<Weak<super::browser::ViewState>>,
    thumbnail_size: Rc<Cell<i32>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
}

fn submit_mode_new_entry(
    active: &RefCell<Option<ActiveModeNewEntry>>,
    browser: &Weak<Browser>,
    location: &Option<Location>,
    field: &gtk::Entry,
) {
    if !active
        .borrow()
        .as_ref()
        .is_some_and(|active| active.field == *field)
    {
        return;
    }
    let name = field.text().to_string();
    if !super::browser::update_basename_validation(field) {
        field.grab_focus();
        return;
    }
    let Some(active) = active.take() else {
        return;
    };
    finish_mode_new_entry(&active);
    if let (Some(browser), Some(location)) = (browser.upgrade(), location.clone()) {
        if active.is_directory {
            browser.create_directory(location, name);
        } else {
            browser.create_file(location, name);
        }
    }
}

fn finish_mode_new_entry(active: &ActiveModeNewEntry) {
    active.field.set_text("");
    active.field.remove_css_class("error");
    active.field.set_tooltip_text(None);
    active.view.remove_css_class("creating-entry");
    if let Some(placeholder) = active.placeholder.as_ref() {
        placeholder.splice(0, placeholder.n_items(), &[]);
    }
    if active
        .source_model
        .as_ref()
        .is_some_and(|model| model.n_items() == 0)
        && let Some(stack) = active.stack.as_ref()
    {
        stack.set_visible_child_name("status");
    }
}

struct GridControls {
    leading: gtk::Box,
    actions: gtk::Box,
    filter_entry: gtk::Entry,
    filter_revealer: gtk::Revealer,
    filter_button: gtk::ToggleButton,
    thumbnail_scale: gtk::Scale,
    thumbnail_value: gtk::Label,
    empty_trash_button: Option<gtk::Button>,
}

fn filter_controls(tooltip: &str) -> (gtk::Entry, gtk::Revealer, gtk::ToggleButton) {
    let entry = gtk::Entry::builder()
        .placeholder_text("Filter items…")
        .has_frame(false)
        .hexpand(true)
        .build();
    entry.add_css_class("column-filter-entry");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.add_css_class("column-filter");
    row.append(&crate::assets::primary_icon(
        crate::assets::icons::FUNNEL,
        16,
    ));
    row.append(&entry);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&row)
        .build();
    let button = gtk::ToggleButton::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::FUNNEL,
        16,
    )));
    button.add_css_class("column-header-action");
    let shown_filter = revealer.clone();
    let focused_filter = entry.clone();
    button.connect_toggled(move |button| {
        shown_filter.set_reveal_child(button.is_active());
        if button.is_active() {
            focused_filter.grab_focus();
        } else {
            focused_filter.set_text("");
        }
    });
    (entry, revealer, button)
}

fn grid_controls(browser: &Rc<Browser>, depth: usize, thumbnail_size: i32) -> GridControls {
    let leading = explorer_navigation(browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("grid-header-actions");

    let thumbnail_popover = gtk::Popover::new();
    thumbnail_popover.set_has_arrow(false);
    thumbnail_popover.add_css_class("grid-thumbnail-popover");
    let thumbnail_content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let thumbnail_heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let thumbnail_title = gtk::Label::new(Some("Thumbnail size"));
    thumbnail_title.add_css_class("grid-thumbnail-title");
    thumbnail_title.set_xalign(0.0);
    thumbnail_title.set_hexpand(true);
    let thumbnail_value = gtk::Label::new(Some(&format!("{thumbnail_size} px")));
    thumbnail_value.add_css_class("grid-thumbnail-value");
    thumbnail_heading.append(&thumbnail_title);
    thumbnail_heading.append(&thumbnail_value);
    let thumbnail_scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(MIN_GRID_THUMBNAIL_SIZE),
        f64::from(MAX_GRID_THUMBNAIL_SIZE),
        16.0,
    );
    thumbnail_scale.add_css_class("grid-thumbnail-scale");
    thumbnail_scale.set_draw_value(false);
    thumbnail_scale.set_value(f64::from(thumbnail_size));
    thumbnail_scale.set_size_request(220, -1);
    let thumbnail_extremes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    thumbnail_extremes.add_css_class("grid-thumbnail-extremes");
    let small = gtk::Label::new(Some("Small"));
    small.set_xalign(0.0);
    small.set_hexpand(true);
    let large = gtk::Label::new(Some("Large"));
    large.set_xalign(1.0);
    thumbnail_extremes.append(&small);
    thumbnail_extremes.append(&large);
    thumbnail_content.append(&thumbnail_heading);
    thumbnail_content.append(&thumbnail_scale);
    thumbnail_content.append(&thumbnail_extremes);
    thumbnail_popover.set_child(Some(&thumbnail_content));
    let thumbnail_menu = gtk::MenuButton::builder()
        .tooltip_text("Thumbnail size")
        .popover(&thumbnail_popover)
        .build();
    thumbnail_menu.add_css_class("column-header-action");
    thumbnail_menu.add_css_class("grid-thumbnail-menu");
    thumbnail_menu.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PICTURES,
        16,
    )));
    let empty_trash = super::browser::empty_trash_button(browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    actions.append(&thumbnail_menu);
    actions.append(&super::browser::column_sort_direction_toggle(
        browser, depth,
    ));
    actions.append(&super::browser::column_sort_menu(browser, depth));

    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter grid (Ctrl+F)");
    actions.append(&filter_button);
    GridControls {
        leading,
        actions,
        filter_entry,
        filter_revealer,
        filter_button,
        thumbnail_scale,
        thumbnail_value,
        empty_trash_button: is_trash.then_some(empty_trash),
    }
}

fn build_grid_pane(
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    options: GridOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let controls = grid_controls(&browser, depth, options.thumbnail_size.get());
    let thumbnail_size = options.thumbnail_size;
    let active_new_entry = options.active_new_entry;
    let (pane, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "grid-pane",
        Some(controls.leading.clone().upcast()),
        Some(controls.actions.clone().upcast()),
    );
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&stack, destination, transfer_handler.clone());
    }
    content.append(&controls.filter_revealer);
    let filter_query = Rc::new(RefCell::new(String::new()));
    let query = filter_query.clone();
    let filter = gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let query = query.borrow();
        query.is_empty() || item.string().to_lowercase().contains(query.as_str())
    });
    let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
    let new_entry_placeholder = gtk::StringList::new(&[]);
    let new_entry_is_directory = Rc::new(Cell::new(true));
    let flattened_models = gio::ListStore::new::<gio::ListModel>();
    flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
    flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
    let view_model = gtk::FlattenListModel::new(Some(flattened_models));
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));
    controls.filter_entry.connect_changed(move |entry| {
        *filter_query.borrow_mut() = entry.text().to_lowercase();
        filter.changed(gtk::FilterChange::Different);
    });
    let factory = gtk::SignalListItemFactory::new();
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let bound_items_for_setup = bound_items.clone();
    let selection_for_setup = selection.clone();
    let selection_anchor = Rc::new(Cell::new(None::<u32>));
    let browser_for_setup = Rc::downgrade(&browser);
    let previews_for_setup = single_click_previews.clone();
    let source_for_setup = model.clone();
    let filtered_for_setup = view_model.clone().upcast::<gio::ListModel>();
    let transfers_for_setup = transfer_handler.clone();
    let peek_for_setup = options.peek_state;
    let active_for_setup = active_new_entry.clone();
    let folder_location = browser.location_at(depth);
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("grid-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        let centered = gtk::CenterBox::new();
        centered.set_orientation(gtk::Orientation::Vertical);
        centered.set_vexpand(true);
        let item_content = gtk::Box::new(gtk::Orientation::Vertical, 3);
        item_content.set_halign(gtk::Align::Center);
        item_content.set_valign(gtk::Align::Center);
        let icon = gtk::Image::new();
        icon.set_pixel_size(26);
        icon.add_css_class("grid-card-icon");
        let label = gtk::Label::new(None);
        label.add_css_class("grid-card-label");
        label.add_css_class("alternate-rename-label");
        label.set_justify(gtk::Justification::Center);
        label.set_width_chars(12);
        label.set_max_width_chars(16);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_width_chars(12);
        field.set_visible(false);
        field.connect_changed(|field| {
            super::browser::update_basename_validation(field);
        });
        let active_for_submit = active_for_setup.clone();
        let browser_for_submit = browser_for_setup.clone();
        let location_for_submit = folder_location.clone();
        field.connect_activate(move |field| {
            submit_mode_new_entry(
                &active_for_submit,
                &browser_for_submit,
                &location_for_submit,
                field,
            );
        });
        let focus = gtk::EventControllerFocus::new();
        let active_for_leave = active_for_setup.clone();
        let browser_for_leave = browser_for_setup.clone();
        let location_for_leave = folder_location.clone();
        let field_for_leave = field.clone();
        focus.connect_leave(move |_| {
            submit_mode_new_entry(
                &active_for_leave,
                &browser_for_leave,
                &location_for_leave,
                &field_for_leave,
            );
        });
        field.add_controller(focus);
        item_content.append(&icon);
        item_content.append(&label);
        item_content.append(&field);
        centered.set_center_widget(Some(&item_content));
        card.append(&centered);
        install_preview_click(
            &card,
            item,
            browser_for_setup.clone(),
            previews_for_setup.clone(),
            depth,
            Some((source_for_setup.clone(), filtered_for_setup.clone())),
        );
        install_modified_selection_click(
            &card,
            item,
            selection_for_setup.clone(),
            selection_anchor.clone(),
        );
        install_grid_peek(
            &card,
            item,
            peek_for_setup.clone(),
            browser_for_setup.clone(),
            source_for_setup.clone(),
            filtered_for_setup.clone(),
            depth,
        );
        install_explorer_drag_drop(
            &card,
            item,
            browser_for_setup.clone(),
            transfers_for_setup.clone(),
            depth,
            Some((source_for_setup.clone(), filtered_for_setup.clone())),
        );
        item.set_child(Some(&card));
        register_bound_mode_item(&bound_items_for_setup, item, &card);
    });
    let browser_for_bind = Rc::downgrade(&browser);
    let source_for_bind = model.clone();
    let filtered_for_bind = view_model.clone().upcast::<gio::ListModel>();
    let cuts_for_bind = cut_locations.clone();
    let thumbnail_size_for_bind = thumbnail_size.clone();
    let entry_kind_for_bind = new_entry_is_directory.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(card) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(centered) = card.first_child().and_downcast::<gtk::CenterBox>() else {
            return;
        };
        let Some(item_content) = centered.center_widget().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = item_content.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(label) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(field) = label.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let source_position =
            source_position_for_view(&source_for_bind, Some(&filtered_for_bind), item.position());
        let entry = browser_for_bind.upgrade().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let Some(entry) = entry {
            label.set_visible(true);
            field.set_visible(false);
            set_mode_cut_style(&card, cuts_for_bind.borrow().contains(&entry.location));
            label.set_label(&entry.display_name);
            label.set_tooltip_text(Some(&entry.display_name));
            super::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                super::browser::entry_icon(&entry),
                26,
                thumbnail_size_for_bind.get(),
            );
            icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
        } else {
            card.remove_css_class("cut-item");
            let icon_name = if entry_kind_for_bind.get() {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::DOCUMENTS
            };
            crate::assets::set_primary_icon(&icon, icon_name);
            icon.set_opacity(1.0);
            label.set_visible(false);
            field.set_visible(true);
        }
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("file-grid");
    view.set_min_columns(1);
    view.set_max_columns(20);
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);

    let pending_thumbnail_resize = Rc::new(RefCell::new(None::<glib::SourceId>));
    let weak_browser_for_size = Rc::downgrade(&browser);
    let model_for_size = model.clone();
    let filtered_for_size = view_model.clone().upcast::<gio::ListModel>();
    let bound_for_size = bound_items.clone();
    let thumbnail_size_for_change = thumbnail_size.clone();
    let value_for_change = controls.thumbnail_value.clone();
    controls
        .thumbnail_scale
        .connect_value_changed(move |scale| {
            let size = scale.value().round() as i32;
            value_for_change.set_label(&format!("{size} px"));
            if let Some(pending) = pending_thumbnail_resize.take() {
                pending.remove();
            }
            let pending_for_timeout = pending_thumbnail_resize.clone();
            let browser = weak_browser_for_size.clone();
            let model = model_for_size.clone();
            let filtered = filtered_for_size.clone();
            let bound = bound_for_size.clone();
            let size_state = thumbnail_size_for_change.clone();
            let source =
                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                    pending_for_timeout.take();
                    size_state.set(size);
                    refresh_grid_thumbnail_size(&browser, depth, &model, &filtered, &bound, size);
                });
            pending_thumbnail_resize.replace(Some(source));
        });

    let weak_browser = Rc::downgrade(&browser);
    let source_for_activation = model.clone();
    let filtered_for_activation = view_model.clone().upcast::<gio::ListModel>();
    view.connect_activate(move |_, position| {
        if let Some(browser) = weak_browser.upgrade()
            && let Some(position) = source_position_for_view(
                &source_for_activation,
                Some(&filtered_for_activation),
                position,
            )
        {
            browser.activate_in_place(depth, position);
        }
    });
    connect_selection(
        &selection,
        &syncing_selection,
        browser,
        depth,
        model.clone(),
        Some(view_model.clone().upcast()),
    );
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .build();
    content.append(&collection_with_marquee(
        view.upcast_ref(),
        scroll,
        &selection,
        bound_items.clone(),
        "grid-card",
    ));
    let shell = pane;
    Pane {
        depth,
        shell,
        model,
        selection,
        filtered_model: Some(view_model.upcast()),
        filter_model: Some(filtered_model),
        syncing_selection,
        stack,
        status,
        spinner,
        truncated_hint,
        view: view.upcast(),
        bound_items,
        filter_entry: Some(controls.filter_entry),
        filter_button: Some(controls.filter_button),
        empty_trash_button: controls.empty_trash_button,
        new_entry_placeholder: Some(new_entry_placeholder),
        new_entry_is_directory: Some(new_entry_is_directory),
    }
}

fn refresh_grid_thumbnail_size(
    browser: &Weak<Browser>,
    depth: usize,
    model: &gtk::StringList,
    filtered_model: &gio::ListModel,
    bound_items: &RefCell<Vec<BoundModeItem>>,
    size: i32,
) {
    let Some(browser) = browser.upgrade() else {
        return;
    };
    bound_items.borrow().iter().for_each(|bound| {
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(card) = bound.widget.upgrade().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = card
            .first_child()
            .and_downcast::<gtk::CenterBox>()
            .and_then(|centered| centered.center_widget())
            .and_downcast::<gtk::Box>()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Image>()
        else {
            return;
        };
        let Some(position) = source_position_for_view(model, Some(filtered_model), item.position())
        else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        super::thumbnail::set_thumbnail_or_icon(
            &icon,
            &entry,
            super::browser::entry_icon(&entry),
            26,
            size,
        );
        icon.set_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
    });
}

fn configure_grid_density(pane: &Pane, density: BrowserDensity) {
    let Ok(grid) = pane.view.clone().downcast::<gtk::GridView>() else {
        return;
    };
    match density {
        BrowserDensity::Compact => {
            grid.set_min_columns(1);
            grid.set_max_columns(20);
        }
        BrowserDensity::Airy => {
            grid.set_min_columns(1);
            grid.set_max_columns(16);
        }
    }
}

fn explorer_headings(
    browser: &Rc<Browser>,
    depth: usize,
    columns: ExplorerColumnLayout,
) -> gtk::Box {
    let headings = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    headings.add_css_class("explorer-headings");
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let sorting = Rc::new(Cell::new((
        preferences.sort_key,
        preferences.sort_direction,
    )));
    let arrows: Rc<RefCell<Vec<(SortKey, gtk::Image)>>> = Rc::new(RefCell::new(Vec::new()));

    for (index, (text, key, width)) in [
        ("Name", SortKey::Name, EXPLORER_COLUMN_WIDTHS[0]),
        ("Size", SortKey::Size, EXPLORER_COLUMN_WIDTHS[1]),
        ("Type", SortKey::Type, EXPLORER_COLUMN_WIDTHS[2]),
        ("Modified", SortKey::Modified, EXPLORER_COLUMN_WIDTHS[3]),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.add_css_class("explorer-heading-cell");
        register_explorer_column_cell(&columns, index, &cell);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let arrow = crate::assets::primary_icon(
            if preferences.sort_direction == SortDirection::Ascending {
                crate::assets::icons::ARROW_UP
            } else {
                crate::assets::icons::ARROW_DOWN
            },
            12,
        );
        arrow.set_visible(preferences.sort_key == key);
        row.append(&label);
        row.append(&arrow);
        let button = gtk::Button::builder().child(&row).build();
        button.add_css_class("explorer-heading-button");
        button.set_hexpand(true);
        let weak_browser = Rc::downgrade(browser);
        let sorting_for_click = sorting.clone();
        let arrows_for_click = arrows.clone();
        button.connect_clicked(move |_| {
            let (current_key, current_direction) = sorting_for_click.get();
            let direction = if current_key == key {
                match current_direction {
                    SortDirection::Ascending => SortDirection::Descending,
                    SortDirection::Descending => SortDirection::Ascending,
                }
            } else {
                SortDirection::Ascending
            };
            sorting_for_click.set((key, direction));
            for (arrow_key, arrow) in arrows_for_click.borrow().iter() {
                arrow.set_visible(*arrow_key == key);
                if *arrow_key == key {
                    crate::assets::set_primary_icon(
                        arrow,
                        if direction == SortDirection::Ascending {
                            crate::assets::icons::ARROW_UP
                        } else {
                            crate::assets::icons::ARROW_DOWN
                        },
                    );
                }
            }
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_sort(depth, key, direction);
            }
        });
        let button_overlay = gtk::Overlay::new();
        button_overlay.set_child(Some(&button));
        button_overlay.set_hexpand(true);
        button_overlay.add_overlay(&column_resize_handle(columns.clone(), index, width));
        cell.append(&button_overlay);
        headings.append(&cell);
        arrows.borrow_mut().push((key, arrow));
    }
    headings
}

fn register_explorer_column_cell(
    columns: &ExplorerColumnLayout,
    index: usize,
    widget: &impl IsA<gtk::Widget>,
) {
    widget.set_width_request(columns.widths[index].get());
    widget.set_hexpand(false);
    let weak = glib::WeakRef::new();
    weak.set(Some(widget.upcast_ref()));
    columns.cells[index].borrow_mut().push(weak);
}

fn set_explorer_column_width(columns: &ExplorerColumnLayout, index: usize, width: i32) {
    columns.widths[index].set(width);
    columns.cells[index].borrow_mut().retain(|weak| {
        let Some(widget) = weak.upgrade() else {
            return false;
        };
        widget.set_width_request(width);
        true
    });
}

fn column_resize_handle(
    columns: ExplorerColumnLayout,
    index: usize,
    initial_width: i32,
) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    handle.add_css_class("explorer-column-resize-handle");
    handle.set_width_request(7);
    handle.set_halign(gtk::Align::End);
    handle.set_valign(gtk::Align::Fill);
    handle.set_cursor_from_name(Some("col-resize"));
    let resize = gtk::GestureDrag::new();
    resize.set_button(1);
    let starting_width = Rc::new(Cell::new(initial_width));
    let pointer_start = Rc::new(Cell::new(None::<f64>));
    let last_press = Rc::new(Cell::new(0u64));
    let starting_for_begin = starting_width.clone();
    let pointer_for_begin = pointer_start.clone();
    let last_press_for_begin = last_press.clone();
    let columns_for_begin = columns.clone();
    let columns_for_autofit = columns.clone();
    resize.connect_drag_begin(move |gesture, _, _| {
        let now = glib::monotonic_time() as u64;
        let prev = last_press_for_begin.get();
        last_press_for_begin.set(now);
        if now.wrapping_sub(prev) <= 400_000 {
            let natural = columns_for_autofit.cells[index]
                .borrow()
                .iter()
                .filter_map(glib::WeakRef::upgrade)
                .map(|widget| super::browser::max_child_natural_width(&widget))
                .max()
                .unwrap_or(initial_width);
            set_explorer_column_width(&columns_for_autofit, index, natural.max(64));
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        }
        let width = columns_for_begin.cells[index]
            .borrow()
            .iter()
            .find_map(glib::WeakRef::upgrade)
            .map_or(initial_width, |widget| widget.width());
        starting_for_begin.set(width.max(64));
        pointer_for_begin.set(
            gesture
                .current_event()
                .and_then(|event| event.position())
                .map(|(pointer_x, _)| pointer_x),
        );
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let columns_for_update = columns.clone();
    resize.connect_drag_update(move |gesture, fallback_offset_x, _| {
        let pointer_x = gesture
            .current_event()
            .and_then(|event| event.position())
            .map(|(pointer_x, _)| pointer_x);
        let offset_x = pointer_start
            .get()
            .zip(pointer_x)
            .map_or(fallback_offset_x, |(start, current)| current - start);
        let width = (f64::from(starting_width.get()) + offset_x).round() as i32;
        set_explorer_column_width(&columns_for_update, index, width.max(64));
    });
    handle.add_controller(resize);
    handle
}

fn explorer_navigation(browser: &Rc<Browser>) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("explorer-navigation");
    for (icon, tooltip, action, available) in [
        (
            crate::assets::icons::ARROW_LEFT,
            "Back (Alt+Left)",
            Browser::back as fn(&Rc<Browser>),
            browser.can_go_back(),
        ),
        (
            crate::assets::icons::ARROW_RIGHT,
            "Forward (Alt+Right)",
            Browser::forward as fn(&Rc<Browser>),
            browser.can_go_forward(),
        ),
        (
            crate::assets::icons::ARROW_UP,
            "Parent folder (Alt+Up)",
            Browser::parent as fn(&Rc<Browser>),
            browser.can_go_parent(),
        ),
    ] {
        let button = gtk::Button::builder()
            .tooltip_text(tooltip)
            .sensitive(available)
            .build();
        button.set_child(Some(&crate::assets::primary_icon(icon, 16)));
        button.add_css_class("explorer-navigation-button");
        let weak_browser = Rc::downgrade(browser);
        button.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                action(&browser);
            }
        });
        actions.append(&button);
    }
    actions
}

fn build_explorer_pane(
    browser: Rc<Browser>,
    single_click_previews: Rc<Cell<bool>>,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<HashSet<Location>>>,
    active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
    depth: usize,
    title: &str,
) -> Pane {
    let navigation = explorer_navigation(&browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("grid-header-actions");
    let empty_trash = super::browser::empty_trash_button(&browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    let (filter_entry, filter_revealer, filter_button) =
        filter_controls("Filter explorer (Ctrl+F)");
    actions.append(&filter_button);
    let (shell, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "explorer-pane",
        Some(navigation.upcast()),
        Some(actions.upcast()),
    );
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&stack, destination, transfer_handler.clone());
    }
    content.append(&filter_revealer);
    let filter_query = Rc::new(RefCell::new(String::new()));
    let query = filter_query.clone();
    let filter = gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let query = query.borrow();
        query.is_empty() || item.string().to_lowercase().contains(query.as_str())
    });
    let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
    filter_entry.connect_changed(move |entry| {
        *filter_query.borrow_mut() = entry.text().to_lowercase();
        filter.changed(gtk::FilterChange::Different);
    });
    let new_entry_placeholder = gtk::StringList::new(&[]);
    let new_entry_is_directory = Rc::new(Cell::new(true));
    let flattened_models = gio::ListStore::new::<gio::ListModel>();
    flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
    flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
    let view_model = gtk::FlattenListModel::new(Some(flattened_models));
    let view_model_object = view_model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(view_model.clone()));
    let syncing_selection = Rc::new(Cell::new(false));

    let columns = ExplorerColumnLayout::new();
    let headings = explorer_headings(&browser, depth, columns.clone());

    let factory = gtk::SignalListItemFactory::new();
    let bound_items: Rc<RefCell<Vec<BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let bound_items_for_setup = bound_items.clone();
    let selection_for_setup = selection.clone();
    let selection_anchor = Rc::new(Cell::new(None::<u32>));
    let browser_for_setup = Rc::downgrade(&browser);
    let previews_for_setup = single_click_previews.clone();
    let transfers_for_setup = transfer_handler.clone();
    let active_for_setup = active_new_entry.clone();
    let source_for_setup = model.clone();
    let view_model_for_setup = view_model_object.clone();
    let folder_location = browser.location_at(depth);
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("explorer-row");
        let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        name_cell.add_css_class("explorer-name-cell");
        let icon = gtk::Image::new();
        icon.set_pixel_size(18);
        let name = gtk::Label::new(None);
        name.add_css_class("alternate-rename-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // Keep the label's natural width from widening this fixed-width table cell.
        name.set_max_width_chars(1);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_hexpand(true);
        field.set_visible(false);
        field.connect_changed(|field| {
            super::browser::update_basename_validation(field);
        });
        let active_for_submit = active_for_setup.clone();
        let browser_for_submit = browser_for_setup.clone();
        let location_for_submit = folder_location.clone();
        field.connect_activate(move |field| {
            submit_mode_new_entry(
                &active_for_submit,
                &browser_for_submit,
                &location_for_submit,
                field,
            );
        });
        let focus = gtk::EventControllerFocus::new();
        let active_for_leave = active_for_setup.clone();
        let browser_for_leave = browser_for_setup.clone();
        let location_for_leave = folder_location.clone();
        let field_for_leave = field.clone();
        focus.connect_leave(move |_| {
            submit_mode_new_entry(
                &active_for_leave,
                &browser_for_leave,
                &location_for_leave,
                &field_for_leave,
            );
        });
        field.add_controller(focus);
        name_cell.append(&icon);
        name_cell.append(&name);
        name_cell.append(&field);
        let size = explorer_metadata_label();
        let kind = explorer_metadata_label();
        let modified = explorer_metadata_label();
        for (index, widget) in [
            name_cell.clone().upcast::<gtk::Widget>(),
            size.clone().upcast(),
            kind.clone().upcast(),
            modified.clone().upcast(),
        ]
        .into_iter()
        .enumerate()
        {
            register_explorer_column_cell(&columns, index, &widget);
        }
        row.append(&name_cell);
        row.append(&size);
        row.append(&kind);
        row.append(&modified);
        install_preview_click(
            &row,
            item,
            browser_for_setup.clone(),
            previews_for_setup.clone(),
            depth,
            Some((source_for_setup.clone(), view_model_for_setup.clone())),
        );
        install_modified_selection_click(
            &row,
            item,
            selection_for_setup.clone(),
            selection_anchor.clone(),
        );
        install_explorer_drag_drop(
            &row,
            item,
            browser_for_setup.clone(),
            transfers_for_setup.clone(),
            depth,
            Some((source_for_setup.clone(), view_model_for_setup.clone())),
        );
        item.set_child(Some(&row));
        register_bound_mode_item(&bound_items_for_setup, item, &row);
    });
    let browser_for_bind = Rc::downgrade(&browser);
    let source_for_bind = model.clone();
    let view_model_for_bind = view_model_object.clone();
    let cuts_for_bind = cut_locations.clone();
    let entry_kind_for_bind = new_entry_is_directory.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(name_cell) = row.first_child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(icon) = name_cell.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(name) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(field) = name.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let Some(size) = name_cell.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(kind) = size.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(modified) = kind.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let source_position = source_position_for_view(
            &source_for_bind,
            Some(&view_model_for_bind),
            item.position(),
        );
        let entry = browser_for_bind.upgrade().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let Some(entry) = entry {
            name.set_visible(true);
            field.set_visible(false);
            set_mode_cut_style(&row, cuts_for_bind.borrow().contains(&entry.location));
            super::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                super::browser::entry_icon(&entry),
                18,
                18,
            );
            name.set_label(&entry.display_name);
            size.set_label(&entry_size(&entry));
            kind.set_label(entry_type(&entry));
            modified.set_label(&crate::util::modified_date(&entry));
        } else {
            row.remove_css_class("cut-item");
            let icon_name = if entry_kind_for_bind.get() {
                crate::assets::icons::FOLDER
            } else {
                crate::assets::icons::DOCUMENTS
            };
            crate::assets::set_primary_icon(&icon, icon_name);
            name.set_visible(false);
            field.set_visible(true);
            size.set_label("");
            kind.set_label("");
            modified.set_label("");
        }
    });
    factory.connect_unbind(|_, item| super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("explorer-list");
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    let weak_browser = Rc::downgrade(&browser);
    let source_for_activation = model.clone();
    let view_model_for_activation = view_model_object.clone();
    view.connect_activate(move |_, position| {
        if let Some(browser) = weak_browser.upgrade()
            && let Some(position) = source_position_for_view(
                &source_for_activation,
                Some(&view_model_for_activation),
                position,
            )
        {
            browser.activate_in_place(depth, position);
        }
    });
    connect_selection(
        &selection,
        &syncing_selection,
        browser,
        depth,
        model.clone(),
        Some(view_model_object.clone()),
    );
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.set_vexpand(true);
    table.append(&headings);
    table.append(&collection_with_marquee(
        view.upcast_ref(),
        scroll,
        &selection,
        bound_items.clone(),
        "explorer-row",
    ));
    let table_scroll = gtk::ScrolledWindow::builder()
        .child(&table)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&table_scroll);
    Pane {
        depth,
        shell,
        model,
        selection,
        filtered_model: Some(view_model_object),
        filter_model: Some(filtered_model),
        syncing_selection,
        stack,
        status,
        spinner,
        truncated_hint,
        view: view.upcast(),
        bound_items,
        filter_entry: Some(filter_entry),
        filter_button: Some(filter_button),
        empty_trash_button: is_trash.then_some(empty_trash),
        new_entry_placeholder: Some(new_entry_placeholder),
        new_entry_is_directory: Some(new_entry_is_directory),
    }
}

fn pane_base(
    title: &str,
    class: &str,
    header_leading: Option<gtk::Widget>,
    header_actions: Option<gtk::Widget>,
) -> (
    gtk::Box,
    gtk::Box,
    gtk::StringList,
    gtk::Stack,
    gtk::Label,
    gtk::Spinner,
    gtk::Image,
) {
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class(class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    let spinner = gtk::Spinner::new();
    spinner.start();
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    truncated_hint.set_tooltip_text(Some(
        "This directory has more entries than could be loaded; showing a partial listing.",
    ));
    truncated_hint.set_visible(false);
    heading_box.append(&heading);
    heading_box.append(&truncated_hint);
    if let Some(leading) = header_leading {
        header.append(&leading);
    }
    header.append(&heading_box);
    header.append(&spinner);
    if let Some(actions) = header_actions {
        header.append(&actions);
    }
    shell.append(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let loading = super::browser::loading_skeleton();
    let status = gtk::Label::new(Some("This directory is empty"));
    status.add_css_class("status-message");
    status.set_wrap(true);
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_named(&content, Some("content"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&status, Some("status"));
    stack.set_visible_child_name("loading");
    shell.append(&stack);

    let model = gtk::StringList::new(&[]);
    (
        shell,
        content,
        model,
        stack,
        status,
        spinner,
        truncated_hint,
    )
}

fn register_bound_mode_item(
    items: &Rc<RefCell<Vec<BoundModeItem>>>,
    item: &gtk::ListItem,
    widget: &impl IsA<gtk::Widget>,
) {
    let weak_item = glib::WeakRef::new();
    weak_item.set(Some(item));
    let weak_widget = glib::WeakRef::new();
    weak_widget.set(Some(widget.upcast_ref()));
    items.borrow_mut().push(BoundModeItem {
        item: weak_item,
        widget: weak_widget,
    });
}

fn collection_with_marquee(
    view: &gtk::Widget,
    scroll: gtk::ScrolledWindow,
    selection: &gtk::MultiSelection,
    bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    item_class: &'static str,
) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    let marquee_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    marquee_box.add_css_class("file-marquee");
    marquee_box.set_can_target(false);
    marquee_box.set_halign(gtk::Align::Start);
    marquee_box.set_valign(gtk::Align::Start);
    marquee_box.set_visible(false);
    overlay.add_overlay(&marquee_box);

    let active = Rc::new(Cell::new(false));
    let origin = Rc::new(Cell::new((0.0, 0.0)));
    let initial = Rc::new(RefCell::new(gtk::Bitset::new_empty()));
    let modifiers = Rc::new(Cell::new((false, false)));
    let marquee = gtk::GestureDrag::new();
    marquee.set_button(1);
    marquee.set_propagation_phase(gtk::PropagationPhase::Capture);
    let active_for_begin = active.clone();
    let origin_for_begin = origin.clone();
    let initial_for_begin = initial.clone();
    let modifiers_for_begin = modifiers.clone();
    let selection_for_begin = selection.clone();
    let marquee_for_begin = marquee_box.clone();
    marquee.connect_drag_begin(move |gesture, x, y| {
        let starts_on_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|widget| widget_or_ancestor_has_class(&widget, item_class));
        let force_marquee = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::ALT_MASK);
        let can_start = force_marquee || !starts_on_item;
        active_for_begin.set(can_start);
        if !can_start {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        marquee_for_begin.set_visible(true);
        origin_for_begin.set((x, y));
        initial_for_begin.replace(selection_for_begin.selection().copy());
        let state = gesture.current_event_state();
        modifiers_for_begin.set((
            state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
            state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
        ));
    });

    let active_for_update = active.clone();
    let view_for_update = view.clone();
    let overlay_for_update = overlay.clone();
    let marquee_for_update = marquee_box.clone();
    let selection_for_update = selection.clone();
    let items_for_update = bound_items.clone();
    marquee.connect_drag_update(move |_, offset_x, offset_y| {
        if !active_for_update.get() {
            return;
        }
        let (origin_x, origin_y) = origin.get();
        let current_x = origin_x + offset_x;
        let current_y = origin_y + offset_y;
        let left = origin_x.min(current_x);
        let right = origin_x.max(current_x);
        let top = origin_y.min(current_y);
        let bottom = origin_y.max(current_y);
        if let Some(view_bounds) = view_for_update.compute_bounds(&overlay_for_update) {
            marquee_for_update
                .set_margin_start((f64::from(view_bounds.x()) + left).round().max(0.0) as i32);
            marquee_for_update
                .set_margin_top((f64::from(view_bounds.y()) + top).round().max(0.0) as i32);
            marquee_for_update.set_size_request(
                (right - left).round().max(1.0) as i32,
                (bottom - top).round().max(1.0) as i32,
            );
        }
        let initial = initial.borrow();
        let (control, shift) = modifiers.get();
        let selected = if control || shift {
            initial.copy()
        } else {
            gtk::Bitset::new_empty()
        };
        items_for_update.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            let Some(bounds) = widget.compute_bounds(&view_for_update) else {
                return true;
            };
            let intersects = f64::from(bounds.x()) < right
                && f64::from(bounds.x() + bounds.width()) > left
                && f64::from(bounds.y()) < bottom
                && f64::from(bounds.y() + bounds.height()) > top;
            let position = item.position();
            if intersects && position != gtk::INVALID_LIST_POSITION {
                if control && initial.contains(position) {
                    selected.remove(position);
                } else {
                    selected.add(position);
                }
            }
            true
        });
        let mask = gtk::Bitset::new_range(0, selection_for_update.n_items());
        selection_for_update.set_selection(&selected, &mask);
    });
    let active_for_end = active;
    let marquee_for_end = marquee_box;
    marquee.connect_drag_end(move |_, _, _| {
        active_for_end.set(false);
        marquee_for_end.set_visible(false);
    });
    view.add_controller(marquee);

    let clear = gtk::GestureClick::new();
    clear.set_button(1);
    let press = Rc::new(Cell::new((0.0, 0.0)));
    let press_for_start = press.clone();
    clear.connect_pressed(move |_, _, x, y| press_for_start.set((x, y)));
    let selection_for_clear = selection.clone();
    clear.connect_released(move |gesture, _, x, y| {
        let (start_x, start_y) = press.get();
        if (x - start_x).abs() > 3.0 || (y - start_y).abs() > 3.0 {
            return;
        }
        let target = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT));
        if !target.is_some_and(|widget| widget_or_ancestor_has_class(&widget, item_class)) {
            selection_for_clear.unselect_all();
        }
    });
    view.add_controller(clear);
    overlay
}

fn descendant_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = descendant_with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn widget_or_ancestor_has_class(widget: &gtk::Widget, class: &str) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class(class) {
            return true;
        }
        current = widget.parent();
    }
    false
}

fn install_grid_peek(
    card: &gtk::Box,
    item: &gtk::ListItem,
    state: Option<Weak<super::browser::ViewState>>,
    browser: Weak<Browser>,
    source: gtk::StringList,
    filtered: gio::ListModel,
    depth: usize,
) {
    let Some(state) = state else {
        return;
    };
    let motion = gtk::EventControllerMotion::new();
    let entered_item = item.clone();
    let entered_card: gtk::Widget = card.clone().upcast();
    let state_for_enter = state.clone();
    motion.connect_enter(move |_, _, _| {
        let position = entered_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let source_position = source_position_for_view(&source, Some(&filtered), position);
        let entry = browser.upgrade().and_then(|browser| {
            source_position.and_then(|position| browser.entry_at(depth, position))
        });
        if let (Some(state), Some(entry)) = (state_for_enter.upgrade(), entry)
            && entry.is_directory()
        {
            state.schedule_peek(depth, entry.location, entered_card.clone());
        }
    });
    motion.connect_leave(move |_| {
        if let Some(state) = state.upgrade() {
            state.schedule_close_peek();
        }
    });
    card.add_controller(motion);
}

fn install_mode_directory_drop_target(
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
    transfer_handler: TransferHandlerSlot,
) {
    widget.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.connect_enter(|target, _, _| super::browser::file_drop_action(target));
    drop.connect_motion(|target, _, _| super::browser::file_drop_action(target));
    drop.connect_drop(move |target, value, _, _| {
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        handler(
            destination.clone(),
            sources,
            super::browser::file_drop_action(target) == gtk::gdk::DragAction::MOVE,
        );
        true
    });
    widget.add_controller(drop);
}

fn install_explorer_drag_drop(
    row: &gtk::Box,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    transfer_handler: TransferHandlerSlot,
    depth: usize,
    position_map: Option<(gtk::StringList, gio::ListModel)>,
) {
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let dragged_item = item.clone();
    let browser_for_drag = browser.clone();
    let map_for_drag = position_map.clone();
    drag.connect_prepare(move |source, x, y| {
        let browser = browser_for_drag.upgrade()?;
        let position = dragged_item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return None;
        }
        let position = map_for_drag
            .as_ref()
            .map_or(Some(position as usize), |(source, filtered)| {
                source_position_for_view(source, Some(filtered), position)
            })?;
        let entry = browser.entry_at(depth, position)?;
        let selected = browser.selected_entries();
        let entries = if selected
            .iter()
            .any(|selected| selected.location == entry.location)
        {
            selected
        } else {
            vec![entry]
        };
        let paintable = gtk::WidgetPaintable::new(source.widget().as_ref());
        source.set_icon(Some(&paintable), x.round() as i32, y.round() as i32);
        super::browser::file_drag_content(&entries)
    });
    let dragged_row = row.clone();
    drag.connect_drag_begin(move |_, _| dragged_row.add_css_class("dragging"));
    let dragged_row = row.clone();
    drag.connect_drag_end(move |_, _, _| dragged_row.remove_css_class("dragging"));
    row.add_controller(drag);

    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    let highlighted_row = row.clone();
    drop.connect_enter(move |target, _, _| {
        highlighted_row.add_css_class("drop-destination");
        super::browser::file_drop_action(target)
    });
    let highlighted_row = row.clone();
    drop.connect_motion(move |target, _, _| {
        highlighted_row.add_css_class("drop-destination");
        super::browser::file_drop_action(target)
    });
    let highlighted_row = row.clone();
    drop.connect_leave(move |_| highlighted_row.remove_css_class("drop-destination"));
    let accepted_item = item.clone();
    let browser_for_accept = browser.clone();
    let map_for_accept = position_map.clone();
    drop.connect_accept(move |_, offered| {
        let Some(browser) = browser_for_accept.upgrade() else {
            return false;
        };
        let position = accepted_item.position();
        let position = map_for_accept.as_ref().map_or(
            (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
            |(source, filtered)| source_position_for_view(source, Some(filtered), position),
        );
        position.is_some()
            && browser
                .entry_at(depth, position.unwrap_or_default())
                .is_some_and(|entry| entry.is_directory())
            && offered
                .formats()
                .contains_type(gtk::gdk::FileList::static_type())
    });
    let dropped_item = item.clone();
    let browser_for_drop = browser;
    let map_for_drop = position_map;
    let dropped_row = row.clone();
    drop.connect_drop(move |target, value, _, _| {
        dropped_row.remove_css_class("drop-destination");
        let Some(browser) = browser_for_drop.upgrade() else {
            return false;
        };
        let position = dropped_item.position();
        let position = map_for_drop.as_ref().map_or(
            (position != gtk::INVALID_LIST_POSITION).then_some(position as usize),
            |(source, filtered)| source_position_for_view(source, Some(filtered), position),
        );
        let Some(destination) = position
            .and_then(|position| browser.entry_at(depth, position))
            .filter(FileEntry::is_directory)
            .map(|entry| entry.location)
        else {
            return false;
        };
        let Some(sources) = super::browser::locations_from_file_list_value(value) else {
            return false;
        };
        let Some(handler) = transfer_handler.borrow().clone() else {
            return false;
        };
        handler(
            destination,
            sources,
            super::browser::file_drop_action(target) == gtk::gdk::DragAction::MOVE,
        );
        true
    });
    row.add_controller(drop);
}

fn install_modified_selection_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    selection: gtk::MultiSelection,
    anchor: Rc<Cell<Option<u32>>>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let item = item.clone();
    click.connect_pressed(move |gesture, _, _, _| {
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let modifiers = gesture.current_event_state();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if shift {
            let anchor = anchor.get().unwrap_or(position);
            let start = anchor.min(position);
            let count = anchor.max(position).saturating_sub(start) + 1;
            selection.select_range(start, count, true);
        } else if control {
            anchor.set(Some(position));
            if selection.is_selected(position) {
                selection.unselect_item(position);
            } else {
                selection.select_item(position, false);
            }
        } else {
            anchor.set(Some(position));
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    widget.add_controller(click);
}

fn source_position_for_view(
    source: &gtk::StringList,
    filtered: Option<&gio::ListModel>,
    position: u32,
) -> Option<usize> {
    let Some(filtered) = filtered else {
        return Some(position as usize);
    };
    let item = filtered.item(position)?;
    (0..source.n_items())
        .find(|candidate| source.item(*candidate).is_some_and(|value| value == item))
        .map(|position| position as usize)
}

fn view_position_for_source(
    source: &gtk::StringList,
    filtered: Option<&gio::ListModel>,
    position: usize,
) -> Option<u32> {
    let Some(filtered) = filtered else {
        return Some(position as u32);
    };
    let item = source.item(position as u32)?;
    (0..filtered.n_items())
        .find(|candidate| filtered.item(*candidate).is_some_and(|value| value == item))
}

fn install_preview_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    browser: Weak<Browser>,
    enabled: Rc<Cell<bool>>,
    depth: usize,
    position_map: Option<(gtk::StringList, gio::ListModel)>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let item = item.clone();
    click.connect_released(move |gesture, press_count, _, _| {
        if press_count != 1 || !enabled.get() {
            return;
        }
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return;
        }
        let position = item.position();
        if position == gtk::INVALID_LIST_POSITION {
            return;
        }
        let source_position = position_map
            .as_ref()
            .map_or(Some(position as usize), |(source, filtered)| {
                source_position_for_view(source, Some(filtered), position)
            });
        let Some(browser) = browser.upgrade() else {
            return;
        };
        let Some(position) = source_position else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        if !entry.is_directory() && super::browser::entry_supports_quick_preview(&entry) {
            browser.preview(depth, position);
        }
    });
    widget.add_controller(click);
}

fn connect_selection(
    selection: &gtk::MultiSelection,
    syncing: &Rc<Cell<bool>>,
    browser: Rc<Browser>,
    depth: usize,
    source: gtk::StringList,
    filtered: Option<gio::ListModel>,
) {
    let syncing = syncing.clone();
    selection.connect_selection_changed(move |selection, _, _| {
        if syncing.get() {
            return;
        }
        let positions = bitset_positions(&selection.selection())
            .into_iter()
            .filter_map(|position| {
                source_position_for_view(&source, filtered.as_ref(), position as u32)
            })
            .collect::<Vec<_>>();
        let focused = positions.last().copied();
        browser.set_selection(depth, &positions, focused);
    });
}

fn set_selections(pane: &Pane, positions: &[usize]) {
    pane.syncing_selection.set(true);
    pane.selection.unselect_all();
    for position in positions {
        if let Some(position) =
            view_position_for_source(&pane.model, pane.filtered_model.as_ref(), *position)
        {
            pane.selection.select_item(position, false);
        }
    }
    pane.syncing_selection.set(false);
}

fn set_mode_cut_style(widget: &impl IsA<gtk::Widget>, cut: bool) {
    if cut {
        widget.add_css_class("cut");
    } else {
        widget.remove_css_class("cut");
    }
}

fn refresh_cut_pane(pane: &Pane, browser: &Browser, cuts: &[Location]) {
    pane.bound_items.borrow_mut().retain(|bound| {
        let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
            return false;
        };
        let source =
            source_position_for_view(&pane.model, pane.filtered_model.as_ref(), item.position());
        let cut = source
            .and_then(|position| browser.entry_at(pane.depth, position))
            .is_some_and(|entry| cuts.contains(&entry.location));
        set_mode_cut_style(&widget, cut);
        true
    });
}

fn replace_entries(pane: &Pane, entries: &[FileEntry]) {
    let values: Vec<_> = entries
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect();
    pane.model.splice(0, pane.model.n_items(), &values);
    show_count(pane);
}

fn reconnect_pane_model(pane: &Pane) {
    if pane.selection.model().is_some() {
        return;
    }
    if let Some(filtered) = pane.filter_model.as_ref() {
        filtered.set_model(Some(&pane.model));
    }
    if let Some(filtered) = pane.filtered_model.as_ref() {
        pane.selection.set_model(Some(filtered));
    } else {
        pane.selection.set_model(Some(&pane.model));
    }
    pane.syncing_selection.set(false);
}

fn show_count(pane: &Pane) {
    let count = pane.model.n_items();
    if count == 0 {
        pane.status.remove_css_class("error");
        pane.status.set_label("This directory is empty");
        pane.stack.set_visible_child_name("status");
    } else {
        pane.stack.set_visible_child_name("content");
    }
    if let Some(button) = &pane.empty_trash_button {
        button.set_sensitive(count > 0);
    }
}

fn apply_snapshot(pane: &Pane, snapshot: &BrowserColumnSnapshot) {
    replace_entries(pane, &snapshot.entries);
    set_selections(pane, &snapshot.selected_positions);
    if snapshot.loading {
        pane.spinner.start();
        pane.stack.set_visible_child_name("loading");
    } else {
        pane.spinner.stop();
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn explorer_metadata_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("explorer-metadata-cell");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // Metadata must truncate rather than overriding a resized column's width.
    label.set_max_width_chars(1);
    label
}

fn entry_size(entry: &FileEntry) -> String {
    if entry.is_directory() {
        return String::new();
    }
    match entry.size {
        MetadataValue::Known(bytes) => super::browser::format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => String::new(),
    }
}

fn entry_type(entry: &FileEntry) -> &'static str {
    use crate::model::EntryKind;
    match entry.kind {
        EntryKind::Directory => "Folder",
        EntryKind::DirectorySymbolicLink => "Folder link",
        EntryKind::File => "File",
        EntryKind::FileSymbolicLink => "File link",
        EntryKind::SymbolicLink => "Broken link",
        EntryKind::Other => "Other",
    }
}

fn bitset_positions(bitset: &gtk::Bitset) -> Vec<usize> {
    let Some((iterator, first)) = gtk::BitsetIter::init_first(bitset) else {
        return Vec::new();
    };
    std::iter::once(first)
        .chain(iterator)
        .map(|position| position as usize)
        .collect()
}
