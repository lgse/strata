// SPDX-License-Identifier: MIT

//! A column's listing surface: scrolling, load pages, marquee, focus and context menus.

use super::super::{BoundRow, file_row_target, is_column_background, is_file_row_target};
use super::models::ColumnModels;
use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser::clipboard::install_directory_drop_target;
use crate::ui::browser::context_menu::{install_folder_context_menu, install_item_context_menu};
use crate::ui::browser::presentation::LoadPresentation;
use crate::ui::marquee::{ItemVisitor, Marquee, MarqueeSetup, MarqueeTarget};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The widgets and models a column's surface handlers attach to.
pub(super) struct ColumnSurface<'a> {
    pub(super) depth: usize,
    pub(super) location: &'a Location,
    pub(super) header: &'a gtk::Box,
    pub(super) list: &'a gtk::ListView,
    pub(super) scroll: &'a gtk::ScrolledWindow,
    pub(super) presentation: &'a LoadPresentation,
    pub(super) models: &'a ColumnModels,
    pub(super) bound_rows: &'a Rc<RefCell<Vec<BoundRow>>>,
}

/// The triggers that open a column's folder and item context menus at a point.
pub(super) struct ContextTriggers {
    pub(super) folder: Rc<dyn Fn(f64, f64)>,
    pub(super) item: Rc<dyn Fn(f64, f64)>,
}

pub(super) fn visit_bound_rows(bound_rows: &Rc<RefCell<Vec<BoundRow>>>) -> ItemVisitor {
    let rows_for_visit = bound_rows.clone();
    Rc::new(move |visit| {
        rows_for_visit.borrow_mut().retain(|bound| {
            let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                return false;
            };
            visit(item.position(), row.upcast_ref());
            true
        });
    })
}

/// Marks the column as scrolling until its listing rests for 150 ms.
pub(super) fn track_listing_scrolling(
    scroll: &gtk::ScrolledWindow,
    column_scrolling: Rc<Cell<bool>>,
) {
    let timer_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    scroll.vadjustment().connect_value_changed(move |_| {
        column_scrolling.set(true);
        if let Some(source) = timer_source.borrow_mut().take() {
            source.remove();
        }
        let scrolling_reset = column_scrolling.clone();
        let timer_holder = timer_source.clone();
        let source_id = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            timer_holder.borrow_mut().take();
            scrolling_reset.set(false);
        });
        *timer_source.borrow_mut() = Some(source_id);
    });
}

/// A marquee press on the background clears the selection and closes deeper columns.
fn clear_marquee_selection(
    state: &ViewState,
    depth: usize,
    returning: &Cell<bool>,
    search_active: &Cell<bool>,
) {
    state.clear_column_selections();
    state.browser.close_column(depth + 1);
    if !returning.replace(false) || search_active.get() {
        return;
    }
    let first = state.columns.borrow().get(depth).and_then(|column| {
        (0..column.selection.n_items()).find_map(|position| column.map.source_position(position))
    });
    if let Some(position) = first {
        state.browser.select(depth, position);
    }
}

/// Remembers a press on the column background, away from rows and controls.
fn note_background_press(
    gesture: &gtk::GestureClick,
    x: f64,
    y: f64,
    pressed_origin: &Cell<Option<(f64, f64)>>,
) {
    pressed_origin.set(None);
    let Some(surface) = gesture.widget() else {
        return;
    };
    let Some(picked) = surface.pick(x, y, gtk::PickFlags::DEFAULT) else {
        return;
    };
    if !is_file_row_target(picked.clone()) && is_column_background(&surface, &picked) {
        pressed_origin.set(Some((x, y)));
    }
}

/// A still click on the background makes the column active and flashes its parent row.
fn focus_clicked_column(state: &Rc<ViewState>, depth: usize) {
    if state.drop_active_depths.get().is_some() {
        return;
    }
    state.suppress_focus_scroll.set(true);
    state.browser.set_active_column(depth);
    state.browser.focus_active();
    state.suppress_focus_scroll.set(false);
    let shell = state
        .columns
        .borrow()
        .get(depth)
        .map(|column| column.shell.clone());
    if let Some(shell) = shell {
        state.reveal_column(shell);
    }
    state.flash_column_parent(depth);
}

fn background_release(
    gesture: &gtk::GestureClick,
    x: f64,
    y: f64,
    origin: &Cell<Option<(f64, f64)>>,
    state: &Weak<ViewState>,
) -> Option<Rc<ViewState>> {
    let (Some((start_x, start_y)), Some(surface)) = (origin.take(), gesture.widget()) else {
        return None;
    };
    if surface.drag_check_threshold(start_x as i32, start_y as i32, x as i32, y as i32) {
        return None;
    }
    state.upgrade()
}

/// Opens the folder menu from a secondary click on the header background.
fn open_header_context_menu(
    gesture: &gtk::GestureClick,
    (x, y): (f64, f64),
    anchor: &glib::WeakRef<gtk::Stack>,
    open_folder_context: &Rc<dyn Fn(f64, f64)>,
) {
    let (Some(header), Some(anchor)) = (gesture.widget(), anchor.upgrade()) else {
        return;
    };
    let Some(picked) = header.pick(x, y, gtk::PickFlags::DEFAULT) else {
        return;
    };
    if !is_column_background(&header, &picked) {
        return;
    }
    let Some(point) = header.compute_point(&anchor, &gtk::graphene::Point::new(x as f32, y as f32))
    else {
        return;
    };
    gesture.set_state(gtk::EventSequenceState::Claimed);
    open_folder_context(f64::from(point.x()), f64::from(point.y()));
}

impl ViewState {
    pub(super) fn listing_scroll(&self, list: &gtk::ListView) -> gtk::ScrolledWindow {
        let scroll = gtk::ScrolledWindow::builder()
            .child(list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        scroll.add_css_class("browser-listing-scroll");
        crate::ui::scrolling::install_autoscroll(&scroll, &self.overlay);
        scroll
    }

    pub(super) fn column_presentation(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
        scroll: &gtk::ScrolledWindow,
        list: &gtk::ListView,
    ) -> LoadPresentation {
        let retry = gtk::Button::with_label(&crate::i18n::tr("Retry"));
        retry.add_css_class("retry-button");
        let weak_browser = Rc::downgrade(&self.browser);
        retry.connect_clicked(move |_| {
            if let Some(browser) = weak_browser.upgrade() {
                browser.retry_column(depth);
            }
        });
        let administrator = gtk::Button::with_label(&crate::i18n::tr("Open as Administrator"));
        administrator.add_css_class("retry-button");
        let weak_state = Rc::downgrade(self);
        administrator.connect_clicked(move |_| {
            if let Some(state) = weak_state.upgrade()
                && let Some(location) = state
                    .browser
                    .column_snapshot(depth)
                    .and_then(|snapshot| snapshot.administrator)
            {
                state.open_as_administrator(location);
            }
        });
        LoadPresentation::new(scroll, Some(retry), Some(administrator)).with_focus_fallback(
            &location.display_name(),
            list,
            &self.browser,
        )
    }

    pub(super) fn install_column_marquee(
        self: &Rc<Self>,
        surface: &ColumnSurface<'_>,
        visit_rows: ItemVisitor,
    ) -> Marquee {
        let depth = surface.depth;
        let weak_for_clear = Rc::downgrade(self);
        let returning_to_column = Rc::new(Cell::new(false));
        let returning_for_clear = returning_to_column.clone();
        let search_active_for_clear = surface.models.recursive_search_active.clone();
        let marquee_targets: Rc<RefCell<Vec<MarqueeTarget>>> =
            Rc::new(RefCell::new(vec![MarqueeTarget {
                selection: surface.models.selection.clone(),
                visit_items: visit_rows,
            }]));
        let marquee = crate::ui::marquee::install(MarqueeSetup {
            view: surface.list.clone().upcast(),
            surface: surface.presentation.stack.clone().upcast(),
            scroll: surface.scroll.clone(),
            overlay: self.overlay.clone(),
            targets: marquee_targets.clone(),
            is_item: crate::ui::marquee::item_content_predicate(
                marquee_targets,
                Rc::new(crate::ui::pointer::hits_item_content),
                true,
            ),
            clear_selection: Rc::new(move || {
                if let Some(state) = weak_for_clear.upgrade() {
                    clear_marquee_selection(
                        &state,
                        depth,
                        &returning_for_clear,
                        &search_active_for_clear,
                    );
                }
            }),
            allow_drag: self.multiple_selection.clone(),
        });
        marquee.add_origin_surface(surface.header);
        // GTK prepends controllers. Install these last so their capture-phase
        // press marks returning_to_column before the marquee clears selection.
        let background_clicks: Vec<_> = [
            surface.presentation.stack.upcast_ref::<gtk::Widget>(),
            surface.header.upcast_ref(),
        ]
        .into_iter()
        .map(|widget| {
            self.install_column_background_focus(widget, depth, returning_to_column.clone())
        })
        .collect();
        for click in &background_clicks {
            marquee.group_background_click(click);
        }
        marquee
    }

    fn install_column_background_focus(
        self: &Rc<Self>,
        surface: &gtk::Widget,
        depth: usize,
        returning_to_column: Rc<Cell<bool>>,
    ) -> gtk::GestureClick {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let origin = Rc::new(Cell::new(None));
        let pressed_origin = origin.clone();
        let weak_for_press = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            returning_to_column.set(
                weak_for_press
                    .upgrade()
                    .is_some_and(|state| state.browser.active_depth() != Some(depth)),
            );
            note_background_press(gesture, x, y, &pressed_origin);
        });
        let stopped_origin = origin.clone();
        click.connect_stopped(move |_| stopped_origin.set(None));
        let weak = Rc::downgrade(self);
        click.connect_released(move |gesture, _, x, y| {
            if let Some(state) = background_release(gesture, x, y, &origin, &weak) {
                focus_clicked_column(&state, depth);
            }
        });
        surface.add_controller(click.clone());
        click
    }

    /// The strip is one Tab stop: Tab from outside lands on the active column.
    pub(super) fn install_column_focus(self: &Rc<Self>, column: &gtk::Box, depth: usize) {
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let owns_context_menu = state
                .context_menu_column
                .get()
                .is_none_or(|owner| owner == depth);
            let takes_focus = !state.land_tab_crossing()
                && owns_context_menu
                && state.drop_active_depths.get().is_none();
            if takes_focus {
                state.browser.set_active_column(depth);
                state.refresh_destination_style();
            }
        });
        column.add_controller(focus);
    }

    pub(super) fn install_column_drop_targets(
        self: &Rc<Self>,
        location: &Location,
        targets: &[&gtk::Widget],
    ) {
        if !self.interactive {
            return;
        }
        for target in targets {
            install_directory_drop_target(self, *target, location.clone());
        }
    }

    pub(super) fn install_column_context_menus(
        self: &Rc<Self>,
        surface: &ColumnSurface<'_>,
    ) -> ContextTriggers {
        let depth = surface.depth;
        let stack = &surface.presentation.stack;
        let folder = install_folder_context_menu(
            self,
            stack.upcast_ref(),
            {
                let entries = surface.models.selection.downgrade();
                Rc::new(move || {
                    entries
                        .upgrade()
                        .is_some_and(|entries| entries.n_items() > 0)
                })
            },
            Rc::new(|picked| is_file_row_target(picked.clone())),
            depth,
            surface.location.clone(),
        );
        let header_context_click = gtk::GestureClick::new();
        header_context_click.set_button(3);
        let open_folder_context = folder.clone();
        let context_anchor = stack.downgrade();
        header_context_click.connect_pressed(move |gesture, _, x, y| {
            open_header_context_menu(gesture, (x, y), &context_anchor, &open_folder_context);
        });
        surface.header.add_controller(header_context_click);

        let rows_for_context = surface.bound_rows.clone();
        let pick_position = Rc::new(move |picked: &gtk::Widget| {
            let picked = file_row_target(picked.clone())?;
            rows_for_context.borrow().iter().find_map(|bound| {
                let row = bound.row.upgrade()?;
                let item = bound.item.upgrade()?;
                (row == picked).then_some(item.position())
            })
        });
        let map_for_context = surface.models.map.clone();
        let source_position = Rc::new(move |position| map_for_context.source_position(position));
        let item = install_item_context_menu(
            self,
            surface.list.upcast_ref(),
            &surface.models.selection,
            pick_position,
            source_position,
            Rc::new(|| {}),
            depth,
        );
        ContextTriggers { folder, item }
    }
}
