// SPDX-License-Identifier: GPL-3.0-or-later

use super::{PendingEntryRename, ViewState};
use crate::model::{FileEntry, Location};
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use std::rc::Rc;

// Hold GTK's scroll anchor only during the bounded post-completion reveal.
struct CreatedEntryViewport {
    adjustment: gtk::Adjustment,
    held: Rc<std::cell::Cell<Option<f64>>>,
    handler: Option<gtk::glib::SignalHandlerId>,
    scroll_controller: Option<(gtk::Widget, gtk::EventControllerScroll)>,
}

impl CreatedEntryViewport {
    fn new(
        adjustment: gtk::Adjustment,
        state: std::rc::Weak<ViewState>,
        pending: std::rc::Weak<PendingEntryRename>,
    ) -> Self {
        let held = Rc::new(std::cell::Cell::new(Some(adjustment.value())));
        let scroll_controller = state.upgrade().map(|state| {
            let widget: gtk::Widget = state.overlay.clone().upcast();
            let controller =
                gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
            controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            let state = Rc::downgrade(&state);
            let pending = pending.clone();
            let held = held.clone();
            controller.connect_scroll(move |_, _, _| {
                held.set(None);
                if let Some(state) = state.upgrade()
                    && let Some(pending) = pending.upgrade()
                {
                    let active = state
                        .pending_new_entry
                        .borrow()
                        .as_ref()
                        .is_some_and(|current| Rc::ptr_eq(current, &pending));
                    if active {
                        state.pending_new_entry.take();
                    }
                }
                gtk::glib::Propagation::Proceed
            });
            widget.add_controller(controller.clone());
            (widget, controller)
        });
        let target = held.clone();
        let handler = adjustment.connect_value_changed(move |adjustment| {
            let active = state
                .upgrade()
                .zip(pending.upgrade())
                .is_some_and(|(state, pending)| {
                    state
                        .pending_new_entry
                        .borrow()
                        .as_ref()
                        .is_some_and(|current| Rc::ptr_eq(current, &pending))
                });
            if active && let Some(value) = target.get() {
                let value = value.clamp(
                    adjustment.lower(),
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                );
                if adjustment.value() != value {
                    adjustment.set_value(value);
                }
            }
        });
        Self {
            adjustment,
            held,
            handler: Some(handler),
            scroll_controller,
        }
    }
}

impl Drop for CreatedEntryViewport {
    fn drop(&mut self) {
        if let Some((widget, controller)) = self.scroll_controller.take() {
            widget.remove_controller(&controller);
        }
        if let Some(handler) = self.handler.take() {
            self.adjustment.disconnect(handler);
        }
    }
}

pub(in crate::ui) struct CreatedEntryTarget {
    pub(in crate::ui) view: gtk::Widget,
    pub(in crate::ui) selection: gtk::MultiSelection,
    pub(in crate::ui) syncing: Rc<std::cell::Cell<bool>>,
    pub(in crate::ui) position: u32,
    pub(in crate::ui) widget: Option<gtk::Widget>,
}

impl CreatedEntryTarget {
    fn reveal(&self, focus: bool, baseline: Option<f64>) -> bool {
        let scroll = self
            .view
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>();
        let Some(scroll) = scroll else { return false };
        let bounds = self
            .widget
            .as_ref()
            .filter(|widget| widget.is_mapped() && widget.height() > 0)
            .and_then(|widget| {
                widget
                    .parent()
                    .unwrap_or_else(|| widget.clone())
                    .compute_bounds(&scroll)
            });
        let Some(bounds) = bounds else {
            super::super::prepare_collection_inline_edit(&self.view, self.position);
            return false;
        };
        let adjustment = scroll.vadjustment();
        let baseline = baseline.unwrap_or(adjustment.value());
        let top = f64::from(bounds.y()) + adjustment.value() - baseline;
        let desired = (baseline
            + reveal_delta(top, f64::from(bounds.height()), adjustment.page_size()))
        .clamp(
            adjustment.lower(),
            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
        );
        if desired != adjustment.value() {
            adjustment.set_value(desired);
            return false;
        }
        if focus {
            // GTK's default focus scroll can move an already visible item upwards.
            let info = gtk::ScrollInfo::new();
            info.set_enable_vertical(false);
            info.set_enable_horizontal(false);
            if let Some(list) = self.view.downcast_ref::<gtk::ListView>() {
                list.scroll_to(self.position, gtk::ListScrollFlags::FOCUS, Some(info));
            } else if let Some(grid) = self.view.downcast_ref::<gtk::GridView>() {
                grid.scroll_to(self.position, gtk::ListScrollFlags::FOCUS, Some(info));
            }
        }
        true
    }

    fn select(&self) {
        self.syncing.set(true);
        self.selection.select_item(self.position, true);
        self.syncing.set(false);
    }
}

pub(super) fn reveal_delta(top: f64, height: f64, viewport: f64) -> f64 {
    if top < 0.0 {
        top
    } else {
        (top + height - viewport).max(0.0)
    }
}

impl ViewState {
    pub(super) fn created_entry_target(
        &self,
        depth: usize,
        entry: &FileEntry,
    ) -> Option<CreatedEntryTarget> {
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self.mode_views.borrow().created_entry_target(depth, entry);
        }
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        if column.presentation.stack.is_transition_running() {
            return None;
        }
        let value = super::super::entry_model_value(entry);
        let position = (0..column.selection.n_items()).find(|position| {
            column
                .selection
                .item(*position)
                .and_downcast::<gtk::StringObject>()
                .is_some_and(|item| item.string() == value)
        })?;
        let widget = column.bound_rows.borrow().iter().find_map(|bound| {
            let item = bound.item.upgrade()?;
            (item.position() == position).then(|| bound.row.upgrade().map(|row| row.upcast()))?
        });
        Some(CreatedEntryTarget {
            view: column.list.clone().upcast(),
            selection: column.selection.clone(),
            syncing: column.syncing_selection.clone(),
            position,
            widget,
        })
    }

    fn reveal_named_entry(
        self: &Rc<Self>,
        pending: Rc<PendingEntryRename>,
        name: String,
        adjustment: gtk::Adjustment,
    ) {
        let baseline = adjustment.value();
        let pending = Rc::new(PendingEntryRename {
            depth: pending.depth,
            parent: pending.parent.clone(),
        });
        let Some(location) = pending.parent.child(std::ffi::OsStr::new(&name)) else {
            return;
        };
        self.pending_new_entry.replace(Some(pending.clone()));
        let viewport =
            CreatedEntryViewport::new(adjustment, Rc::downgrade(self), Rc::downgrade(&pending));
        let weak = Rc::downgrade(self);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let settled = std::cell::Cell::new(None);
        let revealed_frames = std::cell::Cell::new(0);
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state
                .pending_new_entry
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &pending))
            {
                return gtk::glib::ControlFlow::Break;
            }
            if std::time::Instant::now() >= deadline
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            let candidate = state
                .browser
                .column_snapshot(pending.depth)
                .filter(|snapshot| !snapshot.loading)
                .and_then(|snapshot| {
                    state
                        .browser
                        .with_entries(pending.depth, 0..snapshot.count, |entries| {
                            entries
                                .iter()
                                .enumerate()
                                .find(|(_, entry)| entry.location == location)
                                .map(|(position, entry)| (position, entry.clone()))
                        })
                        .flatten()
                });
            let Some((position, entry)) = candidate else {
                return gtk::glib::ControlFlow::Continue;
            };
            let Some(target) = state.created_entry_target(pending.depth, &entry) else {
                return gtk::glib::ControlFlow::Continue;
            };
            let current = (position, target.position);
            if settled.replace(Some(current)) != Some(current) {
                return gtk::glib::ControlFlow::Continue;
            }
            state
                .browser
                .set_selection(pending.depth, &[position], Some(position));
            target.select();
            viewport.held.set(None);
            let revealed = target.reveal(true, Some(baseline));
            if target
                .widget
                .as_ref()
                .is_some_and(|widget| widget.is_mapped() && widget.height() > 0)
            {
                viewport.held.set(Some(viewport.adjustment.value()));
            }
            revealed_frames.set(if revealed {
                revealed_frames.get() + 1
            } else {
                0
            });
            if revealed_frames.get() >= 2 {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    pub(in crate::ui::browser) fn rename_created_entry(self: &Rc<Self>, location: &Location) {
        let Some(pending) = self
            .pending_new_entry
            .borrow()
            .clone()
            .filter(|pending| Some(pending.parent.clone()) == location.parent())
        else {
            return;
        };
        let location = location.clone();
        let weak = Rc::downgrade(self);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let settled = std::cell::Cell::new(None);
        let editor_started = std::cell::Cell::new(false);
        let editor_frames = std::cell::Cell::new(0);
        // Source indices and GTK's displayed order can settle on different frames.
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state
                .pending_new_entry
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &pending))
            {
                return gtk::glib::ControlFlow::Break;
            }
            if std::time::Instant::now() >= deadline
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            let Some(snapshot) = state
                .browser
                .column_snapshot(pending.depth)
                .filter(|snapshot| !snapshot.loading)
            else {
                return gtk::glib::ControlFlow::Continue;
            };
            let position = state
                .browser
                .with_entries(pending.depth, 0..snapshot.count, |entries| {
                    entries.iter().position(|entry| entry.location == location)
                })
                .flatten();
            if let Some(position) = position {
                let Some(entry) = state.browser.entry_at(pending.depth, position) else {
                    return gtk::glib::ControlFlow::Continue;
                };
                let Some(target) = state.created_entry_target(pending.depth, &entry) else {
                    return gtk::glib::ControlFlow::Continue;
                };
                let current = (position, target.position);
                if settled.replace(Some(current)) != Some(current) {
                    return gtk::glib::ControlFlow::Continue;
                }
                state
                    .browser
                    .set_selection(pending.depth, &[position], Some(position));
                target.select();
                if editor_started.get() {
                    if state.active_rename.borrow().is_none()
                        && !state.mode_views.borrow().rename_is_active()
                    {
                        state.pending_new_entry.take();
                        return gtk::glib::ControlFlow::Break;
                    }
                    if target.reveal(false, None) {
                        editor_frames.set(editor_frames.get() + 1);
                        if editor_frames.get() >= 2 {
                            state.pending_new_entry.take();
                            return gtk::glib::ControlFlow::Break;
                        }
                    } else {
                        editor_frames.set(0);
                    }
                    return gtk::glib::ControlFlow::Continue;
                }
                if target.reveal(false, None)
                    && state.begin_rename_item(pending.depth, position, entry.clone(), true)
                {
                    editor_started.set(true);
                    let weak = Rc::downgrade(&state);
                    let pending = pending.clone();
                    let scroll = target
                        .view
                        .ancestor(gtk::ScrolledWindow::static_type())
                        .and_downcast::<gtk::ScrolledWindow>()
                        .map(|scroll| scroll.downgrade());
                    let finished: Rc<dyn Fn(String)> = Rc::new(move |name| {
                        if let Some(state) = weak.upgrade()
                            && let Some(scroll) =
                                scroll.as_ref().and_then(|scroll| scroll.upgrade())
                        {
                            let waiting = Rc::new(PendingEntryRename {
                                depth: pending.depth,
                                parent: pending.parent.clone(),
                            });
                            state.pending_new_entry.replace(Some(waiting.clone()));
                            let adjustment = scroll.vadjustment();
                            let weak = Rc::downgrade(&state);
                            let final_name = name.clone();
                            let completed: Rc<dyn Fn(bool)> = Rc::new(move |success| {
                                if let Some(state) = weak.upgrade()
                                    && state
                                        .pending_new_entry
                                        .borrow()
                                        .as_ref()
                                        .is_some_and(|current| Rc::ptr_eq(current, &waiting))
                                {
                                    if success {
                                        state.reveal_named_entry(
                                            waiting.clone(),
                                            final_name.clone(),
                                            adjustment.clone(),
                                        );
                                    } else {
                                        state.pending_new_entry.take();
                                    }
                                }
                            });
                            if name == entry.display_name {
                                completed(true);
                            } else {
                                let browser = state.browser.clone();
                                let entry = entry.clone();
                                gtk::glib::idle_add_local_once(move || {
                                    browser.rename_with_completion(entry, name, completed);
                                });
                            }
                        }
                    });
                    if let Some(active) = state.active_rename.borrow_mut().as_mut() {
                        active.created = Some(finished);
                    } else {
                        state
                            .mode_views
                            .borrow()
                            .set_created_rename_handler(finished);
                    }
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }
}
