// SPDX-License-Identifier: MIT

//! Building a browser view in stages: the location bar, the column strip, the view
//! state, and the controllers wired to it.

use super::*;

/// The location bar's widgets that the view state keeps or wires.
struct LocationBar {
    location_entry: gtk::Entry,
    confirm_location: gtk::Button,
    cancel_location: gtk::Button,
    breadcrumbs: gtk::Box,
    breadcrumb_scroller: gtk::ScrolledWindow,
    breadcrumb_reveal_pending: Rc<Cell<bool>>,
    location_stack: gtk::Stack,
    global_activity_spinner: gtk::Spinner,
    location_control: gtk::Box,
}

/// Everything the view state is built from besides its defaults.
struct StateParts {
    overlay: gtk::Overlay,
    location_control: gtk::Box,
    location_stack: gtk::Stack,
    global_activity_spinner: gtk::Spinner,
    breadcrumbs: gtk::Box,
    breadcrumb_reveal_pending: Rc<Cell<bool>>,
    location_entry: gtk::Entry,
    path_completion: Rc<location::completion::PathCompletion>,
    columns_widget: gtk::Box,
    scroller: gtk::ScrolledWindow,
    mode_views: ModeViews,
    source_generation: Rc<Cell<u64>>,
    peek_behavior: PeekBehavior,
    multiple_selection: Rc<Cell<bool>>,
    interactive: bool,
    browser: Rc<Browser>,
}

fn column_strip() -> (gtk::Box, gtk::ScrolledWindow) {
    let columns_widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    columns_widget.add_css_class("columns");
    columns_widget.set_halign(gtk::Align::Start);
    columns_widget.set_vexpand(true);

    let scroller = gtk::ScrolledWindow::builder()
        .child(&columns_widget)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroller.add_css_class("mode-scroll");
    scroller.add_css_class("columns-scroll");
    if let Some(viewport) = scroller.child().and_downcast::<gtk::Viewport>() {
        viewport.set_scroll_to_focus(false);
    }
    (columns_widget, scroller)
}

fn location_action(icon: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&crate::assets::primary_icon(icon, 16)));
    button.add_css_class("location-action");
    button
}

/// The typed-location entry with its confirm and cancel buttons.
fn location_entry_control() -> (gtk::Entry, gtk::Button, gtk::Button, gtk::Box) {
    let location_entry = gtk::Entry::builder()
        .hexpand(true)
        .width_chars(36)
        .placeholder_text(crate::i18n::tr("Enter a path or URI…"))
        .build();
    location_entry.add_css_class("location-entry");
    let confirm_location = location_action(crate::assets::icons::CHECK);
    let cancel_location = location_action(crate::assets::icons::X);
    crate::ui::accessibility::describe_location_controls(
        &location_entry,
        &confirm_location,
        &cancel_location,
    );
    let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    entry_row.append(&location_entry);
    entry_row.append(&confirm_location);
    entry_row.append(&cancel_location);
    let entry_control = gtk::Box::new(gtk::Orientation::Vertical, 0);
    entry_control.set_hexpand(true);
    entry_control.append(&entry_row);
    (
        location_entry,
        confirm_location,
        cancel_location,
        entry_control,
    )
}

fn breadcrumb_fade(side: &str, halign: gtk::Align) -> gtk::Box {
    let fade = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    fade.add_css_class("breadcrumb-fade");
    fade.add_css_class(side);
    fade.set_halign(halign);
    fade.set_valign(gtk::Align::Fill);
    fade.set_can_target(false);
    fade.set_opacity(0.0);
    fade
}

/// Scrolls the breadcrumbs to their end on the first layout after a request.
fn install_breadcrumb_reveal(breadcrumb_scroller: &gtk::ScrolledWindow, pending: Rc<Cell<bool>>) {
    let reveal = RefCell::new(None::<crate::ui::frame::FrameTask>);
    crate::ui::frame::after_layout(breadcrumb_scroller, move |scroller| {
        if scroller.is_mapped() && pending.replace(false) {
            let scroller = scroller.downgrade();
            // GTK can still update adjustment bounds during this frame's layout.
            reveal.replace(Some(crate::ui::frame::FrameTask::new(None, move || {
                if let Some(scroller) = scroller.upgrade() {
                    let adjustment = scroller.hadjustment();
                    adjustment.set_value(adjustment.upper() - adjustment.page_size());
                }
            })));
        }
    });
}

fn install_scrollbar_hover(container: &gtk::Box, scrollbar: &gtk::Scrollbar) {
    let motion_controller = gtk::EventControllerMotion::new();
    let weak_scrollbar = scrollbar.downgrade();
    motion_controller.connect_enter(move |_, _, _| {
        if let Some(scrollbar) = weak_scrollbar.upgrade() {
            scrollbar.add_css_class("hovered");
        }
    });
    let weak_scrollbar = scrollbar.downgrade();
    motion_controller.connect_leave(move |_| {
        if let Some(scrollbar) = weak_scrollbar.upgrade() {
            scrollbar.remove_css_class("hovered");
        }
    });
    container.add_controller(motion_controller);
}

/// Shows the scrollbar briefly while the breadcrumbs scroll.
fn flash_scrollbar(
    breadcrumb_scrollbar: &glib::WeakRef<gtk::Scrollbar>,
    scroll_timer: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    let Some(breadcrumb_scrollbar) = breadcrumb_scrollbar.upgrade() else {
        return;
    };
    breadcrumb_scrollbar.add_css_class("scrolling");
    if let Some(source) = scroll_timer.borrow_mut().take() {
        source.remove();
    }
    let weak_scrollbar = breadcrumb_scrollbar.downgrade();
    let timer_cell = scroll_timer.clone();
    let source = glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
        timer_cell.borrow_mut().take();
        if let Some(sb) = weak_scrollbar.upgrade() {
            sb.remove_css_class("scrolling");
        }
    });
    *scroll_timer.borrow_mut() = Some(source);
}

/// The breadcrumb fades and scrollbar that show where the trail overflows.
#[derive(Clone)]
struct BreadcrumbOverflow {
    fade_left: glib::WeakRef<gtk::Box>,
    fade_right: glib::WeakRef<gtk::Box>,
    scrollbar: glib::WeakRef<gtk::Scrollbar>,
}

impl BreadcrumbOverflow {
    fn update(&self, hadjustment: &gtk::Adjustment) {
        let (Some(fade_left), Some(fade_right), Some(breadcrumb_scrollbar)) = (
            self.fade_left.upgrade(),
            self.fade_right.upgrade(),
            self.scrollbar.upgrade(),
        ) else {
            return;
        };
        let value = hadjustment.value();
        let upper = hadjustment.upper();
        let page_size = hadjustment.page_size();
        let max_scroll = (upper - page_size).max(0.0);
        let has_overflow = max_scroll > 1.0;
        breadcrumb_scrollbar.set_visible(has_overflow);
        let show_left = value > 1.0;
        let show_right = has_overflow && value < max_scroll - 1.0;
        fade_left.set_opacity(if show_left { 1.0 } else { 0.0 });
        fade_right.set_opacity(if show_right { 1.0 } else { 0.0 });
    }
}

fn track_breadcrumb_overflow(hadjustment: &gtk::Adjustment, overflow: BreadcrumbOverflow) {
    let scroll_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    hadjustment.connect_value_changed({
        let overflow = overflow.clone();
        let breadcrumb_scrollbar = overflow.scrollbar.clone();
        move |adjustment| {
            overflow.update(adjustment);
            flash_scrollbar(&breadcrumb_scrollbar, &scroll_timer);
        }
    });
    hadjustment.connect_changed(move |adjustment| overflow.update(adjustment));
}

/// The breadcrumb trail: its scroller, overflow fades and the container holding them.
fn breadcrumb_bar() -> (gtk::Box, gtk::ScrolledWindow, Rc<Cell<bool>>, gtk::Box) {
    let breadcrumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    breadcrumbs.add_css_class("breadcrumbs");
    let breadcrumb_scroller = gtk::ScrolledWindow::builder()
        .child(&breadcrumbs)
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .build();
    breadcrumb_scroller.add_css_class("breadcrumb-scroller");

    let fade_left = breadcrumb_fade("breadcrumb-fade-left", gtk::Align::Start);
    let fade_right = breadcrumb_fade("breadcrumb-fade-right", gtk::Align::End);

    let breadcrumb_overlay = gtk::Overlay::new();
    breadcrumb_overlay.set_child(Some(&breadcrumb_scroller));
    breadcrumb_overlay.add_overlay(&fade_left);
    breadcrumb_overlay.add_overlay(&fade_right);
    breadcrumb_overlay.set_hexpand(true);

    let hadjustment = breadcrumb_scroller.hadjustment();
    let breadcrumb_reveal_pending = Rc::new(Cell::new(false));
    install_breadcrumb_reveal(&breadcrumb_scroller, breadcrumb_reveal_pending.clone());
    let breadcrumb_scrollbar =
        gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&hadjustment));
    breadcrumb_scrollbar.add_css_class("breadcrumb-scrollbar");
    breadcrumb_scrollbar.set_visible(false);

    let breadcrumb_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    breadcrumb_container.add_css_class("breadcrumb-container");
    breadcrumb_container.append(&breadcrumb_overlay);
    breadcrumb_container.append(&breadcrumb_scrollbar);
    breadcrumb_container.set_hexpand(true);
    install_scrollbar_hover(&breadcrumb_container, &breadcrumb_scrollbar);
    track_breadcrumb_overflow(
        &hadjustment,
        BreadcrumbOverflow {
            fade_left: fade_left.downgrade(),
            fade_right: fade_right.downgrade(),
            scrollbar: breadcrumb_scrollbar.downgrade(),
        },
    );
    (
        breadcrumbs,
        breadcrumb_scroller,
        breadcrumb_reveal_pending,
        breadcrumb_container,
    )
}

fn location_bar() -> LocationBar {
    let (location_entry, confirm_location, cancel_location, entry_control) =
        location_entry_control();
    let (breadcrumbs, breadcrumb_scroller, breadcrumb_reveal_pending, breadcrumb_container) =
        breadcrumb_bar();
    let location_stack = gtk::Stack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(100)
        .build();
    location_stack.add_named(&breadcrumb_container, Some("breadcrumbs"));
    location_stack.add_named(&entry_control, Some("entry"));
    location_stack.set_visible_child_name("breadcrumbs");
    location_stack.set_hexpand(true);
    location_stack.set_valign(gtk::Align::Center);

    let global_activity_spinner = gtk::Spinner::new();
    global_activity_spinner.add_css_class("global-activity-spinner");
    crate::ui::accessibility::set_description(
        &global_activity_spinner,
        Some(&crate::i18n::tr("Working…")),
    );
    global_activity_spinner.set_visible(false);
    let location_control = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    location_control.add_css_class("location-control");
    location_control.set_hexpand(true);
    location_control.set_valign(gtk::Align::Center);
    location_control.append(&location_stack);
    location_control.append(&global_activity_spinner);
    LocationBar {
        location_entry,
        confirm_location,
        cancel_location,
        breadcrumbs,
        breadcrumb_scroller,
        breadcrumb_reveal_pending,
        location_stack,
        global_activity_spinner,
        location_control,
    }
}

type PendingSubmit = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// Completion submits through `pending_submit`, which is filled once the view state exists.
fn attach_path_completion(
    location_entry: &gtk::Entry,
    location_stack: &gtk::Stack,
    browser: &Rc<Browser>,
    pending_submit: &PendingSubmit,
) -> Rc<location::completion::PathCompletion> {
    let pending_submit_cb = pending_submit.clone();
    let location_stack_for_completion = location_stack.clone();
    location::completion::PathCompletion::attach(
        location_entry,
        browser.clone(),
        move || {
            location_stack_for_completion
                .visible_child_name()
                .as_deref()
                == Some("entry")
        },
        move || {
            if let Some(submit) = pending_submit_cb.borrow().as_ref() {
                submit();
            }
        },
    )
}

fn new_view_state(parts: StateParts) -> Rc<ViewState> {
    let StateParts {
        overlay,
        location_control,
        location_stack,
        global_activity_spinner,
        breadcrumbs,
        breadcrumb_reveal_pending,
        location_entry,
        path_completion,
        columns_widget,
        scroller,
        mode_views,
        source_generation,
        peek_behavior,
        multiple_selection,
        interactive,
        browser,
    } = parts;
    Rc::new(ViewState {
        overlay: overlay.clone(),
        location_control,
        location_stack,
        global_activity_spinner,
        global_activity: RefCell::new(GlobalActivityState::default()),
        breadcrumbs,
        breadcrumb_reveal_pending,
        location_entry,
        path_completion,
        columns_widget,
        scroller,
        mode: Cell::new(mode_views.mode()),
        mode_views: RefCell::new(mode_views),
        columns: RefCell::new(Vec::new()),
        hovered_column: Cell::new(None),
        preview_owns_keys: Cell::new(false),
        playback_handoff: RefCell::new(None),
        context_menu_column: Cell::new(None),
        context_menu_generation: Cell::new(0),
        context_menu_focus: RefCell::new(None),
        input_ownership: RefCell::new(crate::ui::input_ownership::InputOwnership::default()),
        pointer_owns_selection: Cell::new(false),
        folder_press: RefCell::new(None),
        swap_slot: Cell::new(None),
        tab_location: RefCell::new(tab_location::TabLocation::default()),
        column_resizing: Cell::new(false),
        column_width_limit: Cell::new(None),
        clamp_to_next_column_limit: Cell::new(true),
        horizontal_scroll_generation: Rc::new(Cell::new(0)),
        suppress_focus_scroll: Cell::new(false),
        cursor_keeps_focus: Cell::new(false),
        tab_crossing: Cell::new(tab_stops::TabCrossing::None),
        pending_mirror: RefCell::new(None),
        source_generation,
        peek: RefCell::new(None),
        pending_peek: RefCell::new(None),
        pending_close: RefCell::new(None),
        peek_anchor: RefCell::new(None),
        peek_behavior,
        peek_enabled: Cell::new(true),
        single_click_previews: Cell::new(true),
        columns_mirror_selection: Cell::new(true),
        multiple_selection,
        interactive,
        chooser_allows_create: Cell::new(true),
        columns_click_activation: Cell::new(ClickActivation::default()),
        active_rename: Rc::new(RefCell::new(None)),
        pending_rename: RefCell::new(None),
        typed_placement: RefCell::new(None),
        rename_generation: Cell::new(0),
        rename_reveal_generation: Cell::new(0),
        keyboard_refocus: Cell::new(None),
        typed_rename: RefCell::new(None),
        pending_click_rename: RefCell::new(None),
        click_rename_generation: Cell::new(0),
        pending_new_entry: RefCell::new(None),
        progress_state: RefCell::new(Rc::new(FileProgressState::new(&overlay))),
        background_file_progress: RefCell::new(HashMap::new()),
        pending_send_to_completion: RefCell::new(None),
        finished_send_to_completion: RefCell::new(None),

        pin_handler: RefCell::new(None),
        unpin_handler: RefCell::new(None),
        pin_status_handler: RefCell::new(None),
        search_selection_handlers: RefCell::new(Vec::new()),
        pending_select: RefCell::new(Vec::new()),
        pending_properties: RefCell::new(None),
        pending_location_selection: RefCell::new(None),
        pending_extract_retry: RefCell::new(None),
        extract_destination: RefCell::new(None),
        pending_archive_destination: RefCell::new(None),
        pending_delete_entries: RefCell::new(Vec::new()),
        pending_file_operation_animation: RefCell::new(None),
        pending_delete_dissolve: RefCell::new(None),
        delete_dissolve_request: Cell::new(None),
        deferred_delete_empty_depth: Cell::new(None),
        pending_navigate: RefCell::new(None),
        pending_location_credentials: RefCell::new(None),
        pending_connect: RefCell::new(None),
        pending_trash_lookup: RefCell::new(None),
        leftovers: leftovers::LeftoverState::default(),
        pending_empty_trash: RefCell::new(None),
        trash_loading: RefCell::new(None),
        unlock_slots: RefCell::new(Vec::new()),
        trash_button: glib::WeakRef::new(),
        recent_removal: crate::adapters::RecentRemovalState::new(),
        drag_autoscroll: RefCell::new(None),
        drag_source_depth: Cell::new(None),
        suppress_scroll_after_drop: Cell::new(false),
        transfer_replaces_cursor: Cell::new(false),
        transfer_preparation: RefCell::new(None),
        preparing_transfers: RefCell::new(Vec::new()),
        drop_active_depths: Cell::new(None),
        find: RefCell::new(find::FindState::default()),
        listing_filter: listing_filter::FilterState::default(),
        listing_search: listing_search::SearchState::default(),
        result_selection: result_selection::ResultSelection::default(),
        split_pane: Cell::new(false),
        other_pane: RefCell::new(None),
        #[cfg(test)]
        send_to_menu_test_override: RefCell::new(None),
        browser,
    })
}

fn on_state(
    state: &Rc<ViewState>,
    apply: impl Fn(&Rc<ViewState>) + 'static,
) -> impl Fn() + 'static {
    let weak_state = Rc::downgrade(state);
    move || {
        if let Some(state) = weak_state.upgrade() {
            apply(&state);
        }
    }
}

fn scroll_breadcrumbs(
    controller: &gtk::EventControllerScroll,
    hadjustment: &gtk::Adjustment,
    dx: f64,
    dy: f64,
) -> glib::Propagation {
    let delta = if dx.abs() > 0.001 { dx } else { dy };
    if delta.abs() <= 0.001 {
        return glib::Propagation::Proceed;
    }
    let unit = controller.unit();
    let step = delta
        * match unit {
            gtk::gdk::ScrollUnit::Wheel => hadjustment.page_size().powf(2.0 / 3.0).max(30.0),
            gtk::gdk::ScrollUnit::Surface => 2.5,
            _ => 1.0,
        };
    let current = hadjustment.value();
    let max = (hadjustment.upper() - hadjustment.page_size()).max(0.0);
    hadjustment.set_value((current + step).clamp(0.0, max));
    glib::Propagation::Stop
}

fn install_breadcrumb_scroll(breadcrumb_scroller: &gtk::ScrolledWindow) {
    let scroll_controller =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let hadjustment_for_scroll = breadcrumb_scroller.hadjustment();
    scroll_controller.connect_scroll(move |controller, dx, dy| {
        scroll_breadcrumbs(controller, &hadjustment_for_scroll, dx, dy)
    });
    breadcrumb_scroller.add_controller(scroll_controller);
}

impl ViewState {
    fn install_strip_controllers(self: &Rc<Self>) {
        // Columns are laid out from the start edge, so the blank strip beside the last
        // one is the natural place to begin a marquee that runs into it.
        register_cut_view(self);
        self.listing_filter.set_owner(self);
        self.install_input_ownership();
        self.install_column_peek_targets();
        self.install_drag_autoscroll();
        // Resize edges are mode-agnostic; only transfers stay interactive-only.
        columns::install_resize_edges(self);

        let weak_state = Rc::downgrade(self);
        columns::install_horizontal_scroll(self);
        crate::ui::marquee::install_shared_origin_surface(
            &self.scroller,
            crate::ui::marquee::OriginClick::ClearsSelection,
            move |surface, _, x, _| {
                let state = weak_state.upgrade()?;
                let laid_out = state.columns_widget.compute_bounds(surface)?;
                if x < f64::from(laid_out.x() + laid_out.width()) {
                    return None;
                }
                let columns = state.columns.borrow();
                columns.last().map(|column| column.marquee.clone())
            },
        );
        // Installed last so its capture press runs before the scroller's other gestures.
        columns::install_double_click_guard(self);
    }

    fn wire_mode_views(self: &Rc<Self>) {
        self.mode_views
            .borrow()
            .set_context_state(Rc::downgrade(self));
        if !self.interactive {
            self.mode_views
                .borrow()
                .set_new_folder_state(Rc::downgrade(self));
        }
        if self.interactive {
            let weak_state = Rc::downgrade(self);
            self.mode_views
                .borrow()
                .set_transfer_handler(Rc::new(move |destination, request| {
                    if let Some(state) = weak_state.upgrade() {
                        state.commit_drop(destination, request);
                    }
                }));
        }
    }

    fn observe_browser(self: &Rc<Self>) {
        // The observer owns the view state while its window is alive. The window clears
        // the observer on destruction to break this deliberate lifecycle cycle.
        let observer_state = self.clone();
        self.browser.observe(move |event| {
            observer_state.handle(event);
            observer_state.refresh_tab_location();
        });
        let refresh = on_state(self, |state| state.refresh_tab_location());
        self.browser.observe_navigation(refresh);
    }

    /// Any press or key in the view cancels a pending slow-click rename.
    fn install_click_rename_cancel(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let cancel = on_state(self, |state| state.cancel_click_rename());
        click.connect_pressed(move |_, _, _, _| cancel());
        self.overlay.add_controller(click);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let cancel = on_state(self, |state| state.cancel_click_rename());
        keys::on_press(&keys, move |_, _| {
            cancel();
            glib::Propagation::Proceed
        });
        self.overlay.add_controller(keys);
    }

    fn wire_location_controls(self: &Rc<Self>, bar: &LocationBar) {
        let submit = on_state(self, |state| state.submit_location());
        self.location_entry.connect_activate(move |_| submit());
        let submit = on_state(self, |state| state.submit_location());
        bar.confirm_location.connect_clicked(move |_| submit());
        let cancel = on_state(self, |state| state.cancel_location_edit());
        bar.cancel_location.connect_clicked(move |_| cancel());
        let breadcrumb_scroller = &bar.breadcrumb_scroller;
        breadcrumb_scroller.set_cursor_from_name(Some("text"));
        let edit_location = gtk::GestureClick::new();
        let weak_state = Rc::downgrade(self);
        edit_location.connect_released(move |gesture, _, x, y| {
            let clicked_button = gesture
                .widget()
                .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
                .is_some_and(is_breadcrumb_button_target);
            if !clicked_button && let Some(state) = weak_state.upgrade() {
                state.begin_location_edit();
            }
        });
        breadcrumb_scroller.add_controller(edit_location);

        let hierarchy_menu = gtk::GestureClick::new();
        hierarchy_menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        hierarchy_menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_state = Rc::downgrade(self);
        hierarchy_menu.connect_pressed(move |gesture, _, x, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(state) = weak_state.upgrade()
                && let Some(widget) = gesture.widget()
            {
                state.show_breadcrumb_hierarchy_menu(&widget, x);
            }
        });
        breadcrumb_scroller.add_controller(hierarchy_menu);
    }

    fn wire(self: &Rc<Self>, bar: &LocationBar, pending_submit: &PendingSubmit) {
        find::register_highlight_view(self);
        let weak_state = Rc::downgrade(self);
        self.browser.set_freshness_hold(move || {
            weak_state.upgrade().is_some_and(|state| {
                state.inline_edit_is_open() || state.rename_operation_pending()
            })
        });
        self.enter_mode(self.mode.get());
        *pending_submit.borrow_mut() = Some(Rc::new(on_state(self, |state| {
            state.submit_location();
        })));
        self.install_strip_controllers();
        self.wire_mode_views();
        self.observe_browser();
        self.install_click_rename_cancel();
        self.wire_location_controls(bar);
        install_breadcrumb_scroll(&bar.breadcrumb_scroller);
    }
}

impl BrowserView {
    pub(super) fn with_options(
        source: Rc<dyn FileSource>,
        peek_behavior: PeekBehavior,
        multiple: bool,
        interactive: bool,
    ) -> Self {
        let (columns_widget, scroller) = column_strip();
        let overlay = gtk::Overlay::new();
        let bar = location_bar();

        let preferences = crate::ui::preferences::PreferenceManager::shared();
        let browser = Browser::with_preferences(source, preferences.sort_preferences());
        browser.set_chooser_mode(!interactive);
        let preferences_for_sorting = preferences.clone();
        browser.observe_preferences(move |sorting| {
            preferences_for_sorting.set_sort_preferences(sorting);
        });
        let source_generation = Rc::new(Cell::new(0u64));
        let multiple_selection = Rc::new(Cell::new(multiple));
        let mode_views = ModeViews::new(&scroller, browser.clone(), multiple_selection.clone());
        overlay.set_child(Some(&mode_views.widget()));
        let pending_submit: PendingSubmit = Rc::new(RefCell::new(None));
        let path_completion = attach_path_completion(
            &bar.location_entry,
            &bar.location_stack,
            &browser,
            &pending_submit,
        );
        let state = new_view_state(StateParts {
            overlay,
            location_control: bar.location_control.clone(),
            location_stack: bar.location_stack.clone(),
            global_activity_spinner: bar.global_activity_spinner.clone(),
            breadcrumbs: bar.breadcrumbs.clone(),
            breadcrumb_reveal_pending: bar.breadcrumb_reveal_pending.clone(),
            location_entry: bar.location_entry.clone(),
            path_completion,
            columns_widget,
            scroller,
            mode_views,
            source_generation,
            peek_behavior,
            multiple_selection,
            interactive,
            browser,
        });
        state.wire(&bar, &pending_submit);

        let view = Self { state };
        view.bind_preferences(&preferences);
        view
    }
}
