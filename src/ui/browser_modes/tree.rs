// SPDX-License-Identifier: GPL-3.0-or-later

//! The tree presentation: one vertical pane whose folders expand in place.
//!
//! Root rows mirror the browser's active column, so they behave exactly like the other
//! single-pane modes. Rows below them belong to directories the navigation state does not
//! own, so this module loads and sorts those entries itself.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use gtk::{gio, glib, prelude::*};

use super::{
    ActiveModeNewEntry, ClickActivation, ModeClickOptions, Pane, PaneSection, TransferHandlerSlot,
    bound_item_visitor, collection_with_marquee, connect_tree_selection, explorer_navigation,
    filter_controls, install_mode_directory_drop_target, pane_base, refresh_marquee_targets,
    register_bound_mode_item, should_activate_pointer_click, source_position_for_item,
    submit_mode_new_entry,
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::{DirectoryEvent, LoadHandle},
    ui::tree_entry::TreeEntry,
};

const TREE_ICON_SIZE: i32 = 18;

pub(super) struct TreeOptions {
    pub(super) state: Option<Weak<super::super::browser::ViewState>>,
    pub(super) active_new_entry: Rc<RefCell<Option<ActiveModeNewEntry>>>,
}

/// One expanded directory below the root level, with the entries the view loaded for it.
struct TreeBranch {
    store: gio::ListStore,
    load: RefCell<Option<LoadHandle>>,
    requested: Cell<bool>,
}

/// The tree's own view of the filesystem, shared by the row factory, the expander
/// toggles and the keyboard navigation.
pub(super) struct TreeContext {
    browser: Rc<Browser>,
    depth: usize,
    source: gtk::StringList,
    filter: gtk::CustomFilter,
    branches: RefCell<HashMap<Location, Rc<TreeBranch>>>,
    model: RefCell<Option<gtk::TreeListModel>>,
}

impl TreeContext {
    /// The entry a row stands for. Root rows resolve through the browser's active
    /// column; deeper rows carry their entry with them.
    pub(super) fn entry_for(&self, item: &glib::Object) -> Option<FileEntry> {
        super::super::tree_entry::row_entry(item, |value| {
            let position = source_position_for_item(&self.source, value.upcast_ref())?;
            self.browser.entry_at(self.depth, position)
        })
    }

    fn branch(&self, location: &Location) -> Rc<TreeBranch> {
        if let Some(branch) = self.branches.borrow().get(location) {
            return branch.clone();
        }
        let branch = Rc::new(TreeBranch {
            store: gio::ListStore::new::<TreeEntry>(),
            load: RefCell::new(None),
            requested: Cell::new(false),
        });
        self.branches
            .borrow_mut()
            .insert(location.clone(), branch.clone());
        branch
    }

    fn children(self: &Rc<Self>, item: &glib::Object) -> Option<gio::ListModel> {
        let entry = self.entry_for(item)?;
        if !entry.is_directory() {
            return None;
        }
        let branch = self.branch(&entry.location);
        Some(
            gtk::FilterListModel::new(Some(branch.store.clone()), Some(self.filter.clone()))
                .upcast(),
        )
    }

    /// Lists an expanded directory once. Later expansions of the same folder reuse the
    /// entries already loaded, until the pane's column reloads.
    fn load(self: &Rc<Self>, location: &Location) {
        let branch = self.branch(location);
        if branch.requested.replace(true) {
            return;
        }
        let pending: Rc<RefCell<Vec<FileEntry>>> = Rc::new(RefCell::new(Vec::new()));
        let weak_branch = Rc::downgrade(&branch);
        let weak_browser = Rc::downgrade(&self.browser);
        let handle = self.browser.list_directory(
            location.clone(),
            Rc::new(move |event| match event {
                DirectoryEvent::Batch { entries, .. } => pending.borrow_mut().extend(entries),
                DirectoryEvent::Finished { .. } => {
                    let (Some(branch), Some(browser)) =
                        (weak_branch.upgrade(), weak_browser.upgrade())
                    else {
                        return;
                    };
                    let entries = std::mem::take(&mut *pending.borrow_mut());
                    let rows: Vec<TreeEntry> = browser
                        .sorted_entries(entries)
                        .into_iter()
                        .map(TreeEntry::new)
                        .collect();
                    branch.store.splice(0, branch.store.n_items(), &rows);
                }
                _ => {}
            }),
        );
        branch.load.replace(Some(handle));
    }

    /// Drops every loaded branch, so the next expansion lists the directory again with
    /// the preferences the pane now holds.
    pub(super) fn reset(&self) {
        for (_, branch) in self.branches.borrow_mut().drain() {
            branch.load.replace(None);
            branch.store.remove_all();
        }
    }

    fn row(&self, position: u32) -> Option<gtk::TreeListRow> {
        self.model.borrow().as_ref()?.row(position)
    }

    fn set_expanded(self: &Rc<Self>, row: &gtk::TreeListRow, expanded: bool) {
        if expanded && let Some(entry) = row.item().and_then(|item| self.entry_for(&item)) {
            self.load(&entry.location);
        }
        row.set_expanded(expanded);
    }
}

/// Which way a keystroke moves through the tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeStep {
    Previous,
    Next,
    Expand,
    Collapse,
}

/// What a keystroke does to the row it lands on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TreeMove {
    Expand,
    Collapse,
    To(u32),
}

/// The row a step acts on, as far as the decision is concerned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TreeRowState {
    pub(super) position: u32,
    pub(super) count: u32,
    pub(super) expandable: bool,
    pub(super) expanded: bool,
    pub(super) parent: Option<u32>,
}

/// Resolves a keystroke against the focused row. Steps with nowhere to go resolve to
/// nothing, so the window falls back to its usual navigation: expanding a leaf activates
/// it, and collapsing a root row leaves the directory.
pub(super) fn resolve_step(step: TreeStep, row: TreeRowState) -> Option<TreeMove> {
    match step {
        TreeStep::Previous => row.position.checked_sub(1).map(TreeMove::To),
        TreeStep::Next => {
            let next = row.position.saturating_add(1);
            (next < row.count).then_some(TreeMove::To(next))
        }
        TreeStep::Expand => (row.expandable && !row.expanded).then_some(TreeMove::Expand),
        TreeStep::Collapse if row.expanded => Some(TreeMove::Collapse),
        TreeStep::Collapse => row.parent.map(TreeMove::To),
    }
}

/// Moves, expands or collapses within the tree, reporting whether the tree consumed the
/// keystroke.
pub(super) fn step(pane: &Pane, step: TreeStep) -> bool {
    let Some(context) = pane.tree.clone() else {
        return false;
    };
    let section = &pane.section;
    // With nothing focused the window's own selection handling still applies, and it
    // picks the first entry of the directory.
    let Some(position) = super::bitset_positions(&section.selection.selection())
        .last()
        .copied()
        .map(|position| position as u32)
    else {
        return false;
    };
    let Some(row) = context.row(position) else {
        return false;
    };
    let state = TreeRowState {
        position,
        count: section.selection.n_items(),
        expandable: row.is_expandable(),
        expanded: row.is_expanded(),
        parent: row.parent().map(|parent| parent.position()),
    };
    match resolve_step(step, state) {
        Some(TreeMove::Expand) => context.set_expanded(&row, true),
        Some(TreeMove::Collapse) => context.set_expanded(&row, false),
        Some(TreeMove::To(target)) => return focus_position(section, target),
        None => return false,
    }
    refresh_expanders(&context, section);
    true
}

/// Activates the tree's focused row, opening files and entering directories in place.
pub(super) fn activate_focused(pane: &Pane) -> bool {
    let Some(context) = pane.tree.clone() else {
        return false;
    };
    let Some(position) = super::bitset_positions(&pane.section.selection.selection())
        .last()
        .copied()
    else {
        return false;
    };
    activate_position(&context, position as u32);
    true
}

fn focus_position(section: &PaneSection, position: u32) -> bool {
    let Ok(view) = section.view.clone().downcast::<gtk::ListView>() else {
        return false;
    };
    section.selection.select_item(position, true);
    view.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    true
}

fn refresh_expanders(context: &Rc<TreeContext>, section: &PaneSection) {
    section.bound_items.borrow_mut().retain(|bound| {
        let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
            return false;
        };
        if let Some(row) = context.row(item.position())
            && let Some(toggle) = super::descendant_with_class(&widget, "tree-expander-toggle")
                .and_downcast::<gtk::Button>()
        {
            apply_expander(&toggle, row.is_expandable(), row.is_expanded());
        }
        true
    });
}

fn apply_expander(toggle: &gtk::Button, expandable: bool, expanded: bool) {
    let Some(image) = toggle.child().and_downcast::<gtk::Image>() else {
        return;
    };
    toggle.set_sensitive(expandable);
    toggle.set_can_focus(expandable);
    image.set_opacity(if expandable { 1.0 } else { 0.0 });
    crate::assets::set_primary_icon(
        &image,
        if expanded {
            crate::assets::icons::CHEVRON_DOWN
        } else {
            crate::assets::icons::CHEVRON_RIGHT
        },
    );
}

pub(super) fn build_pane(
    browser: Rc<Browser>,
    click_options: ModeClickOptions,
    transfer_handler: TransferHandlerSlot,
    cut_locations: Rc<RefCell<std::collections::HashSet<Location>>>,
    options: TreeOptions,
    depth: usize,
    title: &str,
) -> Pane {
    let active_new_entry = options.active_new_entry.clone();
    let navigation = explorer_navigation(&browser);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("grid-header-actions");
    let empty_trash = super::super::browser::empty_trash_button(&browser);
    let is_trash = browser
        .location_at(depth)
        .is_some_and(|location| super::super::browser::is_trash_root(&location));
    empty_trash.set_visible(is_trash);
    empty_trash.set_sensitive(false);
    actions.append(&empty_trash);
    actions.append(&super::super::browser::pane_refresh_button(&browser, depth));
    let (filter_entry, filter_revealer, filter_button) = filter_controls("Filter tree (Ctrl+F)");
    actions.append(&filter_button);
    let (shell, header, content, model, stack, status, spinner, truncated_hint) = pane_base(
        title,
        "tree-pane",
        Some(navigation.upcast()),
        Some(actions.upcast()),
    );
    if let Some(destination) = browser.location_at(depth) {
        install_mode_directory_drop_target(&stack, destination, transfer_handler.clone());
    }
    content.append(&filter_revealer);
    let filter_query = Rc::new(RefCell::new(String::new()));
    let initial_show_hidden = browser
        .column_preferences(depth)
        .map_or_else(|| browser.preferences().show_hidden, |p| p.show_hidden);
    let show_hidden = Rc::new(Cell::new(initial_show_hidden));
    let filter = tree_filter(show_hidden.clone(), filter_query.clone());
    let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
    let filter_for_changes = filter.clone();
    filter_entry.connect_changed(move |entry| {
        *filter_query.borrow_mut() = entry.text().to_lowercase();
        filter_for_changes.changed(gtk::FilterChange::Different);
    });
    let new_entry_placeholder = gtk::StringList::new(&[]);
    let new_entry_is_directory = Rc::new(Cell::new(true));
    let flattened_models = gio::ListStore::new::<gio::ListModel>();
    flattened_models.append(&new_entry_placeholder.clone().upcast::<gio::ListModel>());
    flattened_models.append(&filtered_model.clone().upcast::<gio::ListModel>());
    let flattened = gtk::FlattenListModel::new(Some(flattened_models));

    let context = Rc::new(TreeContext {
        browser: browser.clone(),
        depth,
        source: model.clone(),
        filter: filter.clone(),
        branches: RefCell::new(HashMap::new()),
        model: RefCell::new(None),
    });
    let context_for_children = Rc::downgrade(&context);
    // Passthrough keeps root rows as the pane's own model values, so selection and
    // renames still resolve them back to positions in the browser's column.
    let tree_model = gtk::TreeListModel::new(flattened, true, false, move |item| {
        context_for_children
            .upgrade()
            .and_then(|context| context.children(item))
    });
    context.model.replace(Some(tree_model.clone()));

    let view_model_object = tree_model.clone().upcast::<gio::ListModel>();
    let selection = gtk::MultiSelection::new(Some(tree_model.clone()));
    let sections: Rc<RefCell<Vec<PaneSection>>> = Rc::new(RefCell::new(Vec::new()));

    let factory = gtk::SignalListItemFactory::new();
    let bound_items: Rc<RefCell<Vec<super::BoundModeItem>>> = Rc::new(RefCell::new(Vec::new()));
    let bound_items_for_setup = bound_items.clone();
    let selection_for_setup = selection.clone();
    let selection_anchor = Rc::new(Cell::new(None::<u32>));
    let context_for_setup = Rc::downgrade(&context);
    let browser_for_setup = Rc::downgrade(&browser);
    let previews_for_setup = click_options.previews;
    let activation_for_setup = click_options.activation;
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
        row.add_css_class("tree-row");
        row.add_css_class("file-appear");
        let weak_row = row.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(row) = weak_row.upgrade() {
                row.remove_css_class("file-appear");
            }
        });
        let expander = gtk::TreeExpander::new();
        expander.set_hide_expander(true);
        expander.set_indent_for_icon(false);
        expander.set_hexpand(true);
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        cell.add_css_class("tree-name-cell");
        let toggle = gtk::Button::new();
        toggle.add_css_class("tree-expander-toggle");
        toggle.add_css_class("flat");
        toggle.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::CHEVRON_RIGHT,
            12,
        )));
        toggle.set_valign(gtk::Align::Center);
        let expander_for_toggle = expander.clone();
        let context_for_toggle = context_for_setup.clone();
        toggle.connect_clicked(move |toggle| {
            let (Some(context), Some(row)) =
                (context_for_toggle.upgrade(), expander_for_toggle.list_row())
            else {
                return;
            };
            let expanded = !row.is_expanded();
            context.set_expanded(&row, expanded);
            apply_expander(toggle, row.is_expandable(), expanded);
        });
        let icon = gtk::Image::new();
        icon.set_pixel_size(TREE_ICON_SIZE);
        let name = gtk::Label::new(None);
        name.add_css_class("alternate-rename-label");
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(1);
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_hexpand(true);
        field.set_visible(false);
        field.connect_changed(|field| {
            super::super::browser::update_basename_validation(field);
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
        cell.append(&toggle);
        cell.append(&icon);
        cell.append(&name);
        cell.append(&field);
        expander.set_child(Some(&cell));
        row.append(&expander);
        install_click(
            &row,
            item,
            context_for_setup.clone(),
            previews_for_setup.clone(),
            activation_for_setup.clone(),
        );
        super::install_modified_selection_click(
            &row,
            item,
            selection_for_setup.clone(),
            selection_anchor.clone(),
        );
        super::install_explorer_drag_drop(
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
    let context_for_bind = Rc::downgrade(&context);
    let cuts_for_bind = cut_locations.clone();
    let entry_kind_for_bind = new_entry_is_directory.clone();
    let browser_for_bind = Rc::downgrade(&browser);
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(expander) = row.first_child().and_downcast::<gtk::TreeExpander>() else {
            return;
        };
        let Some(cell) = expander.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(toggle) = cell.first_child().and_downcast::<gtk::Button>() else {
            return;
        };
        let Some(icon) = toggle.next_sibling().and_downcast::<gtk::Image>() else {
            return;
        };
        let Some(name) = icon.next_sibling().and_downcast::<gtk::Label>() else {
            return;
        };
        let Some(field) = name.next_sibling().and_downcast::<gtk::Entry>() else {
            return;
        };
        let Some(context) = context_for_bind.upgrade() else {
            return;
        };
        let list_row = context.row(item.position());
        expander.set_list_row(list_row.as_ref());
        let object = item.item();
        let entry = object.as_ref().and_then(|object| context.entry_for(object));
        if let Some(entry) = entry {
            name.set_visible(true);
            field.set_visible(false);
            super::set_mode_cut_style(&row, cuts_for_bind.borrow().contains(&entry.location));
            super::super::thumbnail::set_thumbnail_or_icon(
                &icon,
                &entry,
                super::super::browser::entry_icon(&entry),
                TREE_ICON_SIZE,
                TREE_ICON_SIZE,
            );
            if let Some(browser) = browser_for_bind.upgrade()
                && super::super::browser::metadata_needs_fill(&entry)
                && let Some(object) = object.as_ref()
                && let Some(position) = source_position_for_item(&context.source, object)
            {
                browser.request_metadata_fill(depth, position, entry.location.clone());
            }
            name.set_label(&entry.display_name);
            apply_expander(
                &toggle,
                list_row
                    .as_ref()
                    .is_some_and(gtk::TreeListRow::is_expandable),
                list_row.as_ref().is_some_and(gtk::TreeListRow::is_expanded),
            );
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
            apply_expander(&toggle, false, false);
        }
    });
    factory.connect_unbind(|_, item| super::super::thumbnail::cancel_list_item_thumbnails(item));
    let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    view.add_css_class("tree-list");
    view.set_enable_rubberband(false);
    view.set_single_click_activate(false);
    let context_for_activation = Rc::downgrade(&context);
    view.connect_activate(move |_, position| {
        let Some(context) = context_for_activation.upgrade() else {
            return;
        };
        activate_position(&context, position);
    });
    let section = PaneSection {
        view: view.clone().upcast(),
        view_model: view_model_object,
        selection,
        bound_items: bound_items.clone(),
        syncing: Rc::new(Cell::new(false)),
        visit: bound_item_visitor(bound_items),
    };
    sections.borrow_mut().push(section.clone());
    connect_tree_selection(&section, &browser, depth, model.clone());
    if let Some(state) = options.state.as_ref().and_then(Weak::upgrade) {
        super::install_section_context_menu(
            &state,
            &section,
            Rc::downgrade(&sections),
            &model,
            depth,
        );
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroll.add_css_class("fixed-scrollbar");
    let targets: super::super::marquee::MarqueeTargets = Rc::new(RefCell::new(Vec::new()));
    let (collection, marquee) =
        collection_with_marquee(view.upcast_ref(), scroll, targets.clone(), "tree-row");
    marquee.add_origin_surface(&header);
    content.append(&collection);
    let pane = Pane {
        depth,
        shell,
        model,
        filter_model: Some(filtered_model),
        section,
        sections,
        groups: None,
        grid: None,
        tree: Some(context),
        targets,
        detached: Rc::new(Cell::new(false)),
        stack,
        status,
        spinner,
        truncated_hint,
        marquee,
        filter_entry: Some(filter_entry),
        filter_button: Some(filter_button),
        empty_trash_button: is_trash.then_some(empty_trash),
        new_entry_placeholder: Some(new_entry_placeholder),
        new_entry_is_directory: Some(new_entry_is_directory),
        show_hidden,
        filter,
    };
    refresh_marquee_targets(&pane);
    pane
}

fn activate_position(context: &Rc<TreeContext>, position: u32) {
    let Some(object) = context
        .model
        .borrow()
        .as_ref()
        .and_then(|model| model.item(position))
    else {
        return;
    };
    let Some(entry) = context.entry_for(&object) else {
        return;
    };
    match source_position_for_item(&context.source, &object) {
        Some(position) => context.browser.activate_in_place(context.depth, position),
        None if entry.is_directory() => context.browser.navigate(entry.location),
        None => context.browser.open_location(entry.location),
    }
}

fn install_click(
    widget: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    context: Weak<TreeContext>,
    previews: Rc<Cell<bool>>,
    activation: Rc<Cell<ClickActivation>>,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let item = item.clone();
    click.connect_released(move |gesture, press_count, _, _| {
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            return;
        }
        let (Some(context), Some(object)) = (context.upgrade(), item.item()) else {
            return;
        };
        let Some(entry) = context.entry_for(&object) else {
            return;
        };
        let source = source_position_for_item(&context.source, &object);
        if should_activate_pointer_click(press_count, entry.is_directory(), activation.get()) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            match source {
                Some(position) => context.browser.activate_in_place(context.depth, position),
                None if entry.is_directory() => context.browser.navigate(entry.location),
                None => context.browser.open_location(entry.location),
            }
        } else if press_count == 1
            && previews.get()
            && !entry.is_directory()
            && super::super::browser::entry_supports_quick_preview(&entry)
        {
            match source {
                Some(position) => context.browser.preview(context.depth, position),
                None => context.browser.preview_entry(entry),
            }
        }
    });
    widget.add_controller(click);
}

/// Hides entries the pane's preferences exclude, at every level: root rows carry the
/// pane's model values, deeper rows carry their own entry.
fn tree_filter(show_hidden: Rc<Cell<bool>>, query: Rc<RefCell<String>>) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| {
        let (hidden, name) = if let Some(value) = item.downcast_ref::<gtk::StringObject>() {
            let value = value.string();
            (
                super::super::browser::model_is_hidden(&value),
                super::super::browser::model_display_name(&value).to_owned(),
            )
        } else if let Some(entry) = item.downcast_ref::<TreeEntry>().and_then(TreeEntry::entry) {
            (entry.is_hidden, entry.display_name)
        } else {
            return false;
        };
        keeps_row(show_hidden.get(), &query.borrow(), hidden, &name)
    })
}

/// Whether a row survives the pane's hidden-file preference and filter text. The query
/// arrives lowercased from the filter entry.
pub(super) fn keeps_row(show_hidden: bool, query: &str, hidden: bool, name: &str) -> bool {
    if hidden && !show_hidden {
        return false;
    }
    query.is_empty() || name.to_lowercase().contains(query)
}
