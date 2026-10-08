// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use super::*;

const HISTORY_LIMIT: usize = 128;

#[derive(Clone, Copy)]
struct Viewport {
    mode: BrowserMode,
    vertical: f64,
    horizontal: f64,
    /// A grid of another width wraps into other rows, so its offsets do not carry over.
    page_width: f64,
}

impl Viewport {
    fn capture(pane: &Pane, mode: BrowserMode) -> Option<Self> {
        let (vertical, horizontal) = adjustments(pane, mode)?;
        Some(Self {
            mode,
            vertical: vertical.value(),
            horizontal: horizontal.value(),
            page_width: horizontal.page_size(),
        })
    }

    fn same_view(&self, mode: BrowserMode, page_width: f64) -> bool {
        self.mode == mode
            && (mode != BrowserMode::Icons || (page_width - self.page_width).abs() < 1.0)
    }
}

#[derive(Clone)]
struct PanePosition {
    selected: Vec<Location>,
    focused: Option<Location>,
    anchor: Option<Location>,
    viewport: Viewport,
}

#[derive(Clone)]
enum PendingRestore {
    History {
        position: PanePosition,
        /// An explicit navigation, or the listing being left held focus or nothing did.
        takes_focus: bool,
    },
}

#[derive(Default)]
pub(super) struct PaneNavigation {
    history: VecDeque<(Location, PanePosition)>,
    pending: Option<PendingRestore>,
    restoring: Rc<Cell<bool>>,
    input: Option<(glib::WeakRef<gtk::Widget>, gtk::EventControllerLegacy)>,
    leaving_with_focus: bool,
}

impl PaneNavigation {
    pub(super) fn is_restoring(&self) -> bool {
        self.restoring.get()
    }

    pub(super) fn cancel(&mut self) {
        self.restoring.set(false);
        self.pending = None;
    }

    /// The directory loaded and the restored viewport is settling.
    pub(super) fn is_settling(&self) -> bool {
        self.is_restoring() && self.pending.is_none()
    }

    /// Whether a pending history restore may move focus into the listing it rebuilds.
    pub(super) fn history_takes_focus(&self) -> bool {
        matches!(
            self.pending,
            Some(PendingRestore::History {
                takes_focus: true,
                ..
            })
        )
    }

    /// `takes_focus`: the restored listing may take focus. A restore leaves focus
    /// alone after Back from a header button or with the sidebar focused.
    pub(super) fn leave(&mut self, takes_focus: bool) {
        self.leaving_with_focus = takes_focus;
    }

    pub(super) fn capture(&mut self, pane: &Pane, browser: &Browser, mode: BrowserMode) {
        // Leaving during a load/layout must not replace a complete saved visit.
        if self.is_restoring() {
            return;
        }
        let Some((location, position)) = capture_position(pane, browser, mode) else {
            return;
        };
        self.history.retain(|(saved, _)| *saved != location);
        self.history.push_back((location, position));
        if self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
    }

    pub(super) fn prepare(&mut self, pane: &Pane, snapshot: &BrowserColumnSnapshot) {
        if !snapshot.loading {
            return;
        }
        // An explicit reveal target wins over the remembered position; the saved
        // visit stays for a later Back, Forward or Up.
        if snapshot.reveal_pending {
            self.cancel();
            return;
        }
        self.pending = self
            .history
            .iter()
            .find(|(location, _)| *location == snapshot.location)
            .map(|(_, position)| PendingRestore::History {
                position: position.clone(),
                takes_focus: self.leaving_with_focus,
            });
        if self.pending.is_some() {
            self.arm_input_cancel(pane);
        }
    }

    /// Pointer and key input in the pane abandons the restore.
    fn arm_input_cancel(&mut self, pane: &Pane) {
        if let Some((widget, input)) = self.input.take()
            && let Some(widget) = widget.upgrade()
        {
            widget.remove_controller(&input);
        }
        // Stops the previous restore's settle loop.
        self.restoring.set(false);
        self.restoring = Rc::new(Cell::new(true));
        let restoring = self.restoring.clone();
        let input = gtk::EventControllerLegacy::new();
        input.set_propagation_phase(gtk::PropagationPhase::Capture);
        input.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::KeyPress
                    | gtk::gdk::EventType::ButtonPress
                    | gtk::gdk::EventType::Scroll
                    | gtk::gdk::EventType::TouchBegin
            ) {
                restoring.set(false);
            }
            glib::Propagation::Proceed
        });
        pane.shell.add_controller(input.clone());
        self.input = Some((pane.shell.upcast_ref::<gtk::Widget>().downgrade(), input));
    }

    /// Restores a pending position after its directory loaded and returns whether one
    /// was pending. Focus moves only when `may_take_focus`; selection, cursor and
    /// viewport come back regardless.
    pub(super) fn restore(
        &mut self,
        pane: &Pane,
        browser: &Browser,
        mode: BrowserMode,
        may_take_focus: bool,
    ) -> bool {
        let Some(PendingRestore::History {
            position,
            takes_focus,
        }) = self.pending.take().filter(|_| self.is_restoring())
        else {
            return false;
        };
        let Some((vertical, horizontal)) = adjustments(pane, mode) else {
            self.cancel();
            return false;
        };
        let saved = position.viewport;
        let focused = restore_selection(pane, browser, &position);
        let view = &pane.section.view;
        let cursor = focused.and_then(|source| {
            view_position_for_source(&pane.model, Some(&pane.section.view_model), source)
        });
        let grabs_items = may_take_focus && takes_focus;
        // Bind the cursor before restoring the viewport; focusing it afterwards
        // would otherwise reveal it at a different vertical offset.
        if grabs_items {
            super::focus_pane_surface(pane);
            if let Some(cursor) = cursor {
                focus_collection_item(view, cursor);
            }
        }
        let restoring = self.restoring.clone();
        let items = pane.section.bound_items.clone();
        let frames = Cell::new(0u8);
        let settled = Cell::new(0u8);
        let last_upper = Cell::new(-1.0);
        let exact = Cell::new(None::<bool>);
        view.add_tick_callback(move |view, _| {
            if !restoring.get()
                || !view.is_mapped()
                || (grabs_items && !collection_keeps_cursor(view))
            {
                restoring.set(false);
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get() + 1);
            if grabs_items && let Some(cursor) = cursor {
                focus_bound_cursor(&items, cursor);
            }
            if vertical.page_size() > 0.0 {
                let same = exact.get().unwrap_or_else(|| {
                    let same = saved.same_view(mode, horizontal.page_size());
                    exact.set(Some(same));
                    // Focusing the cursor already reveals it.
                    if !same
                        && !grabs_items
                        && let Some(cursor) = cursor
                    {
                        scroll_collection_to(view, cursor);
                    }
                    same
                });
                if same {
                    vertical.set_value(saved.vertical);
                    horizontal.set_value(saved.horizontal);
                }
            }
            if vertical.page_size() > 0.0
                && vertical.upper() == last_upper.replace(vertical.upper())
            {
                settled.set(settled.get() + 1);
            } else {
                settled.set(0);
            }
            if settled.get() >= 3 || frames.get() >= 20 {
                restoring.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        true
    }
}

/// The pane's directory, once it loaded without an error; a load still in progress
/// has no position worth saving.
fn loaded_location(pane: &Pane, browser: &Browser) -> Option<Location> {
    let snapshot = browser.column_snapshot(pane.depth)?;
    (!snapshot.loading && snapshot.error.is_none()).then_some(snapshot.location)
}

fn capture_position(
    pane: &Pane,
    browser: &Browser,
    mode: BrowserMode,
) -> Option<(Location, PanePosition)> {
    let location = loaded_location(pane, browser)?;
    let viewport = Viewport::capture(pane, mode)?;
    let position = PanePosition {
        selected: browser
            .selected_entries()
            .into_iter()
            .map(|entry| entry.location)
            .collect(),
        focused: browser.focused_item().map(|(_, _, entry)| entry.location),
        anchor: browser
            .selection_anchor_position(pane.depth)
            .and_then(|position| browser.entry_at(pane.depth, position))
            .map(|entry| entry.location),
        viewport,
    };
    Some((location, position))
}

/// Selects the saved locations that are still listed and visible, and returns the
/// cursor's source position.
fn restore_selection(pane: &Pane, browser: &Browser, saved: &PanePosition) -> Option<usize> {
    let selected: HashSet<_> = saved.selected.iter().collect();
    let (positions, focused, anchor) = browser
        .with_entries(pane.depth, 0..pane.model.n_items() as usize, |entries| {
            let visible = |position: usize| {
                view_position_for_source(&pane.model, Some(&pane.section.view_model), position)
                    .is_some()
            };
            let find = |location: &Option<Location>| {
                location.as_ref().and_then(|location| {
                    entries
                        .iter()
                        .position(|entry| entry.location == *location)
                        .filter(|&position| visible(position))
                })
            };
            let positions = entries
                .iter()
                .enumerate()
                .filter_map(|(position, entry)| {
                    (selected.contains(&entry.location) && visible(position)).then_some(position)
                })
                .collect::<Vec<_>>();
            (positions, find(&saved.focused), find(&saved.anchor))
        })
        .unwrap_or_default();
    let focused = focused.or_else(|| positions.first().copied());
    if let Some(anchor) = anchor.or(focused) {
        reset_native_range_origin(pane, anchor);
    }
    browser.set_selection(pane.depth, &positions, focused);
    if let Some(anchor) = anchor.or(focused) {
        browser.set_selection_anchor(pane.depth, anchor);
    }
    set_selections(pane, &positions);
    focused
}

/// Icons scroll the grid's own window both ways; List scrolls rows vertically inside
/// a window that scrolls its columns horizontally.
fn adjustments(pane: &Pane, mode: BrowserMode) -> Option<(gtk::Adjustment, gtk::Adjustment)> {
    let vertical = pane
        .section
        .view
        .ancestor(gtk::ScrolledWindow::static_type())?
        .downcast::<gtk::ScrolledWindow>()
        .ok()?;
    match mode {
        BrowserMode::Columns => None,
        BrowserMode::Icons => Some((vertical.vadjustment(), vertical.hadjustment())),
        BrowserMode::List => {
            let horizontal = vertical
                .parent()?
                .ancestor(gtk::ScrolledWindow::static_type())?
                .downcast::<gtk::ScrolledWindow>()
                .ok()?;
            Some((vertical.vadjustment(), horizontal.hadjustment()))
        }
    }
}
