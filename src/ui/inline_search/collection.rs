// SPDX-License-Identifier: MIT

//! Stable identity and displayed-order lookup. The rank map is only a sorter input; all
//! behavior resolves entries through the sorted GTK model, never a parallel result vector.

use super::{SEARCH_RESULTS_LABEL, SearchPresentation};
use crate::{
    model::{FileEntry, Location},
    services::SearchItem,
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

#[derive(Clone)]
pub(super) struct CollectionBehavior {
    pub(super) multiple_selection: Rc<Cell<bool>>,
    pub(super) activate: Rc<dyn Fn(FileEntry)>,
    pub(super) single_click: Rc<dyn Fn(FileEntry)>,
    pub(super) focus_items: Rc<dyn Fn()>,
}

#[derive(Clone)]
struct CollectionInteractions {
    selection: gtk::MultiSelection,
    anchor: Rc<Cell<Option<u32>>>,
    model: gtk::SortListModel,
    gesture_selection: Rc<RefCell<Option<gtk::Bitset>>>,
    pointer_activation: crate::ui::collection_interaction::PointerSequence,
    behavior: CollectionBehavior,
}

#[derive(Clone)]
pub(super) enum ResultKind {
    Rows,
    Icons { thumbnail_size: Rc<Cell<i32>> },
}

pub(super) struct BoundResult {
    pub(super) item: glib::WeakRef<gtk::ListItem>,
    pub(super) widget: glib::WeakRef<gtk::Widget>,
    pub(super) rename_label: glib::WeakRef<gtk::Widget>,
    pub(super) presentation: crate::ui::inline_search::presentation::ResultWidgets,
}

pub(super) struct ResultCollection {
    pub(super) view: gtk::Widget,
    pub(super) model: gio::ListStore,
    pub(super) sorted: gtk::SortListModel,
    pub(super) sorter: gtk::CustomSorter,
    pub(super) positions: Rc<RefCell<HashMap<PathBuf, usize>>>,
    pub(super) selection: gtk::MultiSelection,
    pub(super) bound: Rc<RefCell<Vec<BoundResult>>>,
    pub(super) anchor: Rc<Cell<Option<u32>>>,
    // GTK selects the pointer target before item gestures run; retain the intended
    // modified-click group so drag and context-menu focus can preserve it.
    pub(super) gesture_selection: Rc<RefCell<Option<gtk::Bitset>>>,
    pub(super) kind: ResultKind,
    pub(super) root: PathBuf,
    pub(super) reconciling: Rc<Cell<bool>>,
}

impl ResultCollection {
    pub(super) fn item(&self, position: u32) -> Option<SearchItem> {
        let object = self
            .sorted
            .item(position)?
            .downcast::<glib::BoxedAnyObject>()
            .ok()?;
        Some(object.borrow::<SearchItem>().clone())
    }

    pub(super) fn items(&self) -> Vec<SearchItem> {
        (0..self.sorted.n_items())
            .filter_map(|position| self.item(position))
            .collect()
    }

    pub(super) fn position_at(&self, picked: &gtk::Widget) -> Option<u32> {
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

    pub(super) fn current_position(&self) -> Option<u32> {
        self.view
            .root()
            .and_then(|root| root.focus())
            .and_then(|focus| self.position_at(&focus))
            .or_else(|| {
                (0..self.selection.n_items()).find(|position| self.selection.is_selected(*position))
            })
    }

    pub(super) fn selected_positions(&self) -> Vec<usize> {
        (0..self.selection.n_items())
            .filter(|position| self.selection.is_selected(*position))
            .map(|position| position as usize)
            .collect()
    }

    pub(super) fn bound_at(&self, position: u32) -> Option<(gtk::ListItem, gtk::Widget)> {
        self.bound.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position)
                .then(|| Some((item, bound.widget.upgrade()?)))
                .flatten()
        })
    }

    pub(super) fn focus(&self, position: u32, exclusive: bool) -> bool {
        if position >= self.selection.n_items() {
            return false;
        }
        if exclusive {
            self.gesture_selection.borrow_mut().take();
        }
        self.selection.select_item(position, exclusive);
        self.gesture_selection
            .replace(Some(self.selection.selection().copy()));
        self.restore_focus(position);
        true
    }

    fn restore_focus(&self, position: u32) {
        self.view.grab_focus();
        if let Some(list) = self.view.downcast_ref::<gtk::ListView>() {
            list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        } else if let Some(grid) = self.view.downcast_ref::<gtk::GridView>() {
            grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    pub(super) fn first_visual_row(&self, position: u32) -> bool {
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

    pub(super) fn edit_widgets(
        &self,
        position: u32,
    ) -> Option<crate::ui::collection_edit::EditWidgets> {
        self.bound.borrow().iter().find_map(|bound| {
            (bound.item.upgrade()?.position() == position).then(|| bound.presentation.edit.clone())
        })
    }

    pub(super) fn rename_label(&self, position: u32) -> Option<gtk::Widget> {
        self.bound.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position)
                .then(|| bound.rename_label.upgrade())
                .flatten()
        })
    }

    pub(super) fn update(&self, items: &[SearchItem], recursive: bool) {
        self.reconciling.set(true);
        self.gesture_selection.borrow_mut().take();
        let selected_paths: Vec<_> = self
            .selected_positions()
            .into_iter()
            .filter_map(|position| self.item(position as u32).map(|item| item.path))
            .collect();
        let selected_slot = self.selected_positions().into_iter().next();
        let anchor_path = self
            .anchor
            .get()
            .and_then(|position| self.item(position))
            .map(|item| item.path);
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
        self.anchor.set(anchor_path.and_then(|path| {
            (0..self.selection.n_items())
                .find(|position| self.item(*position).is_some_and(|item| item.path == path))
        }));
        if let Some(position) = focused_path.as_ref().and_then(|path| {
            (0..self.selection.n_items())
                .find(|position| self.item(*position).is_some_and(|item| &item.path == path))
        }) {
            self.restore_focus(position);
        }
        self.refresh_bindings(recursive);
        self.reconciling.set(false);
    }

    pub(super) fn refresh_bindings(&self, recursive: bool) {
        self.bound.borrow_mut().retain(|bound| {
            let (Some(item), Some(_widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            if let Some(result) = self.item(item.position()) {
                bound
                    .presentation
                    .bind(&self.kind, &item, &result, &self.root, recursive);
            }
            true
        });
    }

    pub(super) fn clear(&self) {
        crate::ui::thumbnail::cancel_thumbnails_in(&self.view);
        self.model.remove_all();
        self.positions.borrow_mut().clear();
        self.anchor.set(None);
        self.gesture_selection.borrow_mut().take();
    }

    pub(super) fn refresh_cut_rows(&self) {
        self.bound.borrow_mut().retain(|bound| {
            let (Some(item), Some(widget)) = (bound.item.upgrade(), bound.widget.upgrade()) else {
                return false;
            };
            if let (Some(result), Some(row)) = (
                self.item(item.position()),
                widget.downcast_ref::<gtk::Box>(),
            ) {
                crate::ui::browser::set_cut_result_style(row, &Location::local(&result.path));
            }
            true
        });
    }

    pub(super) fn set_max_columns(&self, max_columns: u32) {
        if let Some(grid) = self.view.downcast_ref::<gtk::GridView>() {
            grid.set_max_columns(max_columns);
        }
    }
}

pub(super) fn collection_entry(
    model: &impl IsA<gio::ListModel>,
    position: u32,
) -> Option<FileEntry> {
    let object = model
        .item(position)?
        .downcast::<glib::BoxedAnyObject>()
        .ok()?;
    Some(crate::ui::browser::search_result_entry(
        &object.borrow::<SearchItem>(),
    ))
}

fn install_result_interactions(
    widget: &gtk::Widget,
    item: &gtk::ListItem,
    interactions: &CollectionInteractions,
) {
    let selection = &interactions.selection;
    let anchor = &interactions.anchor;
    let model = &interactions.model;
    let multiple_selection = &interactions.behavior.multiple_selection;
    let gesture_selection = &interactions.gesture_selection;
    let pointer_activation = &interactions.pointer_activation;
    let focus_items = &interactions.behavior.focus_items;
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    pointer_activation.install(&click);
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
        pointer_for_press.press(modifiers);
        let previous = gesture_selection_for_press
            .borrow()
            .clone()
            .unwrap_or_else(|| selection_for_press.selection());
        let change = crate::ui::collection_interaction::pointer_selection(
            &previous,
            position,
            anchor_for_press.get(),
            multiple_for_press.get(),
            modifiers,
        );
        anchor_for_press.set(change.anchor);
        let selected = change.selected;
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

    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak_item = item.downgrade();
    let weak_widget = widget.downgrade();
    let selection_for_drag = selection.clone();
    let model_for_drag = model.clone();
    drag.connect_prepare(move |source, _, _| {
        let item = weak_item.upgrade()?;
        let widget = weak_widget.upgrade()?;
        let position = item.position();
        let positions: Vec<_> = if selection_for_drag.is_selected(position) {
            (0..selection_for_drag.n_items())
                .filter(|position| selection_for_drag.is_selected(*position))
                .collect()
        } else {
            vec![position]
        };
        let entries: Vec<_> = positions
            .into_iter()
            .filter_map(|position| collection_entry(&model_for_drag, position))
            .collect();
        source.set_actions(crate::ui::browser::drag_actions_for_modifiers(
            source.current_event_state(),
        ));
        source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&widget))), 0, 0);
        crate::ui::browser::file_drag_content(&entries)
    });
    widget.add_controller(drag.clone());
    widget.add_controller(click.clone());
    drag.group_with(&click);
}

pub(super) fn build_collection(
    presentation: SearchPresentation,
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
    let pointer_activation = crate::ui::collection_interaction::PointerSequence::default();
    let reconciling = Rc::new(Cell::new(false));
    let reconciling_for_selection = reconciling.clone();
    let syncing_selection = Rc::new(Cell::new(false));
    let syncing_for_selection = syncing_selection.clone();
    let multiple_for_selection = multiple_selection.clone();
    let gesture_selection_for_change = gesture_selection.clone();
    selection.connect_selection_changed(move |selection, position, count| {
        if reconciling_for_selection.get() || syncing_for_selection.replace(true) {
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
        model: sorted.clone(),
        gesture_selection: gesture_selection.clone(),
        pointer_activation: pointer_activation.clone(),
        behavior: behavior.clone(),
    };
    factory.connect_setup(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let presentation =
            crate::ui::inline_search::presentation::ResultWidgets::new(&kind_for_setup);
        let widget = presentation.widget.clone();
        let rename_label = presentation.rename_label();
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
            presentation,
        });
    });
    let kind_for_bind = kind.clone();
    let recursive_for_bind = recursive.clone();
    let root_for_bind = root.clone();
    let bound_for_bind = bound.clone();
    factory.connect_bind(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(result) = item
            .item()
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|object| object.borrow::<SearchItem>().clone())
        else {
            return;
        };
        let presentation = bound_for_bind
            .borrow()
            .iter()
            .find(|bound| bound.item.upgrade().as_ref() == Some(item))
            .map(|bound| bound.presentation.clone());
        if let Some(presentation) = presentation {
            presentation.bind(
                &kind_for_bind,
                item,
                &result,
                &root_for_bind,
                recursive_for_bind.get(),
            );
        }
    });
    let bound_for_unbind = bound.clone();
    factory.connect_unbind(move |_, object| {
        let edit = bound_for_unbind
            .borrow()
            .iter()
            .find(|bound| {
                bound
                    .item
                    .upgrade()
                    .as_ref()
                    .map(|item| item.upcast_ref::<glib::Object>())
                    == Some(object)
            })
            .map(|bound| bound.presentation.edit.clone());
        if let Some(edit) = edit {
            edit.unbind();
        }
        crate::ui::thumbnail::cancel_list_item_thumbnails(object);
    });
    let bound_for_teardown = bound.clone();
    factory.connect_teardown(move |_, object| {
        bound_for_teardown.borrow_mut().retain(|bound| {
            bound
                .item
                .upgrade()
                .as_ref()
                .is_some_and(|item| item.upcast_ref::<glib::Object>() != object)
        });
    });

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
    crate::ui::accessibility::set_label(&view, SEARCH_RESULTS_LABEL);
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
                        crate::ui::icons_cell::set_slot(card, size);
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
        match pointer_activation.activation() {
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
            reconciling,
        },
        scroll,
        overlay,
    )
}
