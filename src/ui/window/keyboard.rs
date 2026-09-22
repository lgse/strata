// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
    prelude::*,
};

use crate::{
    app::Browser,
    ui::{
        browser::BrowserView,
        minimal_mode::{MinimalPrompt, MinimalState},
        preferences::PreferenceManager,
        preview::PreviewDrawer,
        shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

use super::{SidebarState, SidebarView, TypeToSearch, visible_modal_layer};

mod commands;
mod focus;
mod items;
mod minimal;

// None tries the next Strata stage; Some(Proceed) gives the event to GTK instead.
type KeyResult = Option<Propagation>;

pub(in crate::ui) struct Bindings {
    pub view: BrowserView,
    pub top_bar: TopBarNavigation,
    pub preview: PreviewDrawer,
    pub type_to_search: TypeToSearch,
    pub shortcuts: ShortcutFooter,
    pub open_settings: Rc<dyn Fn()>,
}

pub(in crate::ui) fn install(
    window: &impl IsA<gtk::Window>,
    sidebar: &SidebarView,
    bindings: Bindings,
) {
    let window = window.as_ref();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(&bindings.view.browser());
    let preferences = bindings.type_to_search.preferences.clone();
    let apply_view = bindings.view.clone();
    let apply_footer = bindings.shortcuts.clone();
    let apply_sidebar = sidebar.state.clone();
    let minimal = Rc::new(RefCell::new(MinimalState::new()));
    let prompt_focus_lock = Rc::new(Cell::new(false));
    let sidebar_focus = SidebarFocus {
        state: sidebar.state.clone(),
        widget: sidebar.widget.clone(),
        previous: RefCell::new(None),
    };
    let prompt_focus = gtk::EventControllerFocus::new();
    let focus_window = window.downgrade();
    let focus_minimal = Rc::downgrade(&minimal);
    let focus_footer = bindings.shortcuts.downgrade();
    let focus_lock = prompt_focus_lock.clone();
    let focus_view = apply_view.downgrade();
    prompt_focus.connect_leave(move |_| {
        let (Some(window), Some(minimal), Some(footer), Some(view)) = (
            focus_window.upgrade(),
            focus_minimal.upgrade(),
            focus_footer.upgrade(),
            focus_view.upgrade(),
        ) else {
            return;
        };
        dismiss_prompt_after_focus_loss(&window, &minimal, &footer, &view, &focus_lock);
    });
    bindings
        .shortcuts
        .prompt_entry_widget()
        .add_controller(prompt_focus);
    let chord_teardown = {
        let weak_minimal = Rc::downgrade(&minimal);
        let weak_sidebar = Rc::downgrade(&sidebar.state);
        let footer = bindings.shortcuts.downgrade();
        Rc::new(move || {
            if let Some(minimal) = weak_minimal.upgrade() {
                minimal.borrow_mut().cancel_chord();
            }
            if let Some(sidebar) = weak_sidebar.upgrade() {
                sidebar.clear_minimal_chord_hints();
            }
            if let Some(footer) = footer.upgrade() {
                footer.clear_chord_mark();
            }
        }) as Rc<dyn Fn()>
    };
    sidebar.state.set_minimal_chord_teardown(chord_teardown);
    apply_minimal_mode(
        window,
        &preferences,
        &apply_view,
        &apply_footer,
        &apply_sidebar,
        &bindings.preview,
        &minimal,
    );
    let weak_window = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let (Some(window), Some(browser)) = (weak_window.upgrade(), weak_browser.upgrade()) else {
            return Propagation::Proceed;
        };
        bindings.view.cancel_pending_open_with();
        Dispatcher {
            window: &window,
            view: &bindings.view,
            top_bar: &bindings.top_bar,
            preview: &bindings.preview,
            type_to_search: &bindings.type_to_search,
            shortcuts: &bindings.shortcuts,
            open_settings: &bindings.open_settings,
            sidebar: &sidebar_focus,
            minimal: &minimal,
            prompt_focus_lock: &prompt_focus_lock,
        }
        .handle_key(&browser, key, modifiers)
    });
    window.add_controller(keys);

    // Ctrl+wheel mirrors the Ctrl +/- text-size shortcut. Capture phase so
    // scrolled windows cannot consume it first; the PDF preview's own zoom is
    // left alone by passing through scrolls inside its scroll container.
    // DISCRETE is avoided: it swallows sub-step smooth deltas before they
    // reach descendant scrolled windows, killing touchpad scrolling.
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let window_for_wheel = window.downgrade();
    let zoom = Rc::new(TextZoomScroll::default());
    wheel.connect_scroll(move |controller, _, dy| {
        let Some(window) = window_for_wheel.upgrade() else {
            return Propagation::Proceed;
        };
        let target = controller
            .current_event()
            .and_then(|event| event.position())
            .and_then(|(x, y)| window.pick(x, y, gtk::PickFlags::DEFAULT));
        zoom.handle(
            &preferences,
            controller.current_event_state(),
            controller.unit(),
            target,
            dy,
        )
    });
    window.add_controller(wheel);
}

/// Accumulates fractional scroll deltas into whole text-size steps so smooth
/// devices zoom in the same increments as wheel clicks. Resets when the input
/// reverses direction, changes units, or is not claimed for zooming.
#[derive(Default)]
pub(super) struct TextZoomScroll {
    pending: Cell<f64>,
    direction: Cell<i8>,
    unit: Cell<Option<gtk::gdk::ScrollUnit>>,
}

impl TextZoomScroll {
    /// Surface-unit deltas arrive in pixels; 10 px make one wheel step, the
    /// same mapping GTK's discrete conversion uses.
    const SURFACE_PIXELS_PER_STEP: f64 = 10.0;

    fn reset(&self) {
        self.pending.set(0.0);
        self.direction.set(0);
    }

    pub(super) fn accumulate(&self, unit: gtk::gdk::ScrollUnit, dy: f64) -> i32 {
        let direction = dy.signum() as i8;
        let unit_changed = self.unit.replace(Some(unit)) != Some(unit);
        let direction_changed = direction != 0 && self.direction.replace(direction) != direction;
        if unit_changed || direction_changed {
            self.pending.set(0.0);
        }
        let delta = if unit == gtk::gdk::ScrollUnit::Surface {
            dy / Self::SURFACE_PIXELS_PER_STEP
        } else {
            dy
        };
        let accumulated = self.pending.get() + delta;
        let steps = accumulated.trunc() as i32;
        self.pending.set(accumulated - f64::from(steps));
        steps
    }

    fn handle(
        &self,
        preferences: &crate::ui::preferences::PreferenceManager,
        modifiers: Modifiers,
        unit: gtk::gdk::ScrollUnit,
        target: Option<gtk::Widget>,
        dy: f64,
    ) -> Propagation {
        if !modifiers.contains(Modifiers::CONTROL_MASK)
            || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            || dy == 0.0
            || target.as_ref().is_some_and(inside_pdf_scroll)
        {
            self.reset();
            return Propagation::Proceed;
        }
        let steps = self.accumulate(unit, dy);
        if steps == 0 {
            return Propagation::Stop;
        }
        handle_text_zoom_scroll(preferences, modifiers, target, f64::from(steps))
    }
}

pub(super) fn handle_text_zoom_scroll(
    preferences: &crate::ui::preferences::PreferenceManager,
    modifiers: Modifiers,
    target: Option<gtk::Widget>,
    dy: f64,
) -> Propagation {
    if !modifiers.contains(Modifiers::CONTROL_MASK)
        || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        || dy == 0.0
        || target.as_ref().is_some_and(inside_pdf_scroll)
    {
        return Propagation::Proceed;
    }
    preferences.set_text_size(preferences.text_size().stepped(-dy as i32));
    Propagation::Stop
}

fn inside_pdf_scroll(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.has_css_class("preview-pdf-scroll") {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// Initial binding applies CSS only; real transitions also tear down transient
/// input state and hidden queries. Application accelerators are bound separately
/// in `composition::install_browser_actions`.
fn apply_minimal_mode(
    window: &gtk::Window,
    preferences: &Rc<PreferenceManager>,
    view: &BrowserView,
    footer: &ShortcutFooter,
    sidebar: &Rc<SidebarState>,
    preview: &PreviewDrawer,
    minimal: &Rc<RefCell<MinimalState>>,
) {
    let apply_view = view.downgrade();
    let apply_footer = footer.downgrade();
    let apply_sidebar = Rc::downgrade(sidebar);
    let apply_preview = preview.downgrade();
    let weak_minimal = Rc::downgrade(minimal);
    let previous = Cell::new(None);
    preferences.bind_preference(
        window,
        PreferenceManager::minimal_mode,
        move |widget, enabled| {
            let was_enabled = previous.replace(Some(enabled));
            if enabled {
                widget.add_css_class("minimal-mode");
            } else {
                widget.remove_css_class("minimal-mode");
            }
            if was_enabled.is_none() {
                return;
            }
            let (Some(view), Some(footer), Some(sidebar), Some(preview), Some(minimal)) = (
                apply_view.upgrade(),
                apply_footer.upgrade(),
                apply_sidebar.upgrade(),
                apply_preview.upgrade(),
                weak_minimal.upgrade(),
            ) else {
                return;
            };
            view.cancel_pending_open_with();
            // Reset before hiding the entry: focus-leave must not retain search results.
            minimal.borrow_mut().reset();
            footer.hide_prompt();
            sidebar.clear_minimal_chord_hints();
            footer.clear_chord_mark();
            footer.set_filter_mark("");
            view.set_find_prompt_active(false);
            view.set_find_highlight("");
            view.dismiss_hidden_filter();
            preview.set_owns_keys_chrome(false);
            view.set_column_header_focus(true);
            if !enabled {
                view.browser().focus_active();
                view.restore_file_view_focus();
            }
            tracing::debug!(enabled, "minimal mode changed");
        },
    );
}

struct Dispatcher<'a> {
    window: &'a gtk::Window,
    view: &'a BrowserView,
    sidebar: &'a SidebarFocus,
    top_bar: &'a TopBarNavigation,
    preview: &'a PreviewDrawer,
    type_to_search: &'a TypeToSearch,
    shortcuts: &'a ShortcutFooter,
    open_settings: &'a Rc<dyn Fn()>,
    minimal: &'a Rc<RefCell<MinimalState>>,
    /// True while find/filter/search or prompt Up/Down re-grabs the entry
    /// after a listing focus change. Those must not look like pointer dismiss.
    prompt_focus_lock: &'a Rc<Cell<bool>>,
}

/// Closes an open footer prompt when focus leaves the entry, keeping the
/// pointer selection. Incremental find re-grabs under `prompt_focus_lock`.
fn dismiss_prompt_after_focus_loss(
    window: &gtk::Window,
    minimal: &Rc<RefCell<MinimalState>>,
    footer: &ShortcutFooter,
    view: &BrowserView,
    lock: &Cell<bool>,
) {
    if lock.get() || minimal.borrow().prompt().is_none() {
        return;
    }
    if footer.is_prompt_entry(&gtk::prelude::RootExt::focus(window)) {
        return;
    }
    let kind = minimal.borrow().prompt();
    minimal.borrow_mut().leave_prompt();
    footer.hide_prompt();
    view.set_find_prompt_active(false);
    if matches!(
        kind,
        Some(MinimalPrompt::FindNext | MinimalPrompt::FindPrev)
    ) {
        view.set_find_highlight("");
    }
    // Click-away commits the query already applied to the rows. An empty query
    // is cancelled so a later search dismiss cannot bring that filter back.
    if kind == Some(MinimalPrompt::Filter) {
        let query = view.hidden_filter_query();
        let trimmed = query.trim();
        if trimmed.is_empty() {
            minimal.borrow_mut().clear_applied_filter();
            view.dismiss_hidden_filter();
            footer.set_filter_mark("");
        } else {
            minimal
                .borrow_mut()
                .remember_applied_filter(trimmed.to_owned());
            footer.set_filter_mark(trimmed);
        }
    }
    if kind == Some(MinimalPrompt::Search) {
        let keep = view.force_recursive_search() && view.selected_search_results().is_some();
        if keep {
            if view.focus_first_search_result()
                && let Some(index) = view.search_hit_index()
            {
                minimal.borrow_mut().set_search_cursor(Some(index));
            }
        } else if view.force_recursive_search() || view.selected_search_results().is_some() {
            minimal.borrow_mut().clear_search_nav();
            restore_applied_hidden_filter(view, footer, minimal);
        }
    }
    tracing::debug!("minimal prompt dismissed on focus loss");
}

fn restore_applied_hidden_filter(
    view: &BrowserView,
    footer: &ShortcutFooter,
    minimal: &RefCell<MinimalState>,
) {
    let query = minimal
        .borrow()
        .applied_filter()
        .filter(|query| !query.trim().is_empty());
    match query {
        Some(query) => {
            view.set_filter_query_without_revealer(&query);
            footer.set_filter_mark(&query);
        }
        None => {
            view.dismiss_hidden_filter();
            footer.set_filter_mark("");
        }
    }
}

struct KeyEvent {
    key: Key,
    modifiers: Modifiers,
    focused: Option<gtk::Widget>,
    vim_navigation: bool,
    header_left_boundary: bool,
}

impl KeyEvent {
    fn control(&self) -> bool {
        self.modifiers.contains(Modifiers::CONTROL_MASK)
    }

    fn shift(&self) -> bool {
        self.modifiers.contains(Modifiers::SHIFT_MASK)
    }

    fn alt(&self) -> bool {
        self.modifiers.contains(Modifiers::ALT_MASK)
    }

    fn without(&self, modifiers: Modifiers) -> bool {
        !self.modifiers.intersects(modifiers)
    }

    fn text_has_focus(&self) -> bool {
        self.focused.as_ref().is_some_and(|widget| {
            widget.is::<gtk::Text>() || widget.is::<gtk::TextView>() || widget.is::<gtk::Entry>()
        })
    }
}

impl Dispatcher<'_> {
    fn handle_key(&self, browser: &Rc<Browser>, key: Key, modifiers: Modifiers) -> Propagation {
        let preferences = &self.type_to_search.preferences;
        if let Some(size) = preferences.text_size().for_shortcut(key, modifiers) {
            preferences.set_text_size(size);
            return Propagation::Stop;
        }
        if let Some(result) = self.input_owner(key, modifiers) {
            return result;
        }
        if matches!(key, Key::m | Key::M)
            && modifiers.contains(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && !modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            let preferences = &self.type_to_search.preferences;
            if preferences.minimal_mode() {
                self.leave_preview_keys();
            }
            preferences.set_minimal_mode(!preferences.minimal_mode());
            return Propagation::Stop;
        }
        let focused = gtk::prelude::RootExt::focus(self.window);
        let minimal = self.type_to_search.preferences.minimal_mode();
        let navigation_key = if minimal {
            key
        } else {
            crate::ui::focus_navigation::navigation_key(
                key,
                modifiers,
                self.type_to_search.preferences.type_to_search(),
                focused.as_ref(),
            )
        };
        let mut event = KeyEvent {
            key: navigation_key,
            modifiers,
            focused,
            vim_navigation: navigation_key != key,
            header_left_boundary: false,
        };
        if minimal {
            return self
                .minimal_commands(browser, &mut event)
                .unwrap_or(Propagation::Proceed);
        }
        if browser.is_chooser_mode() {
            return Propagation::Proceed;
        }
        self.window_commands(&event)
            .or_else(|| self.inline_editing(&event))
            .or_else(|| self.filter_and_location_commands(&event))
            .or_else(|| self.video_controls(&event))
            .or_else(|| self.sidebar_commands(browser, &event))
            .or_else(|| self.context_menu_command(&event))
            .or_else(|| {
                // Search rows own navigation; directory commands must not act on hidden selections.
                if self.view.selected_search_results().is_some()
                    && !event.text_has_focus()
                    && event.key != Key::Delete
                {
                    if matches!(event.key, Key::c | Key::x | Key::v | Key::a)
                        && let Some(result) = self.clipboard_command(&event)
                    {
                        return Some(result);
                    }
                    if event.vim_navigation {
                        crate::ui::focus_navigation::activate_native_arrow(self.window, event.key);
                        Some(Propagation::Stop)
                    } else {
                        Some(Propagation::Proceed)
                    }
                } else {
                    None
                }
            })
            .or_else(|| self.text_input(&event))
            .or_else(|| self.file_commands(browser, &event))
            .or_else(|| self.focus_navigation(browser, &mut event))
            .or_else(|| self.dismissal(browser, &event))
            .or_else(|| self.item_navigation(browser, &event))
            .unwrap_or(Propagation::Proceed)
    }

    fn input_owner(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if let Some(layer) = visible_modal_layer(self.window) {
            let focus_is_inside = gtk::prelude::RootExt::focus(self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            if !focus_is_inside {
                layer.grab_focus();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if gtk::prelude::RootExt::focus(self.window)
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
        {
            return Some(Propagation::Proceed);
        }
        if !self.inline_editing_active()
            && let Some(result) = self.shortcuts.handle_key(key, modifiers)
        {
            return Some(result);
        }
        if key == Key::Escape && crate::ui::scrolling::stop_autoscroll() {
            return Some(Propagation::Stop);
        }
        None
    }

    fn inline_editing_active(&self) -> bool {
        self.view.rename_is_active() || self.view.new_entry_is_active()
    }

    fn arrows_scoped_to_content(&self) -> bool {
        self.type_to_search.preferences.arrow_navigation_scoped()
    }
}

struct SidebarFocus {
    state: Rc<SidebarState>,
    widget: gtk::Widget,
    previous: RefCell<Option<gtk::Widget>>,
}

impl SidebarFocus {
    fn contains(&self, focused: &Option<gtk::Widget>) -> bool {
        focused
            .as_ref()
            .is_some_and(|widget| widget == &self.widget || widget.is_ancestor(&self.widget))
    }

    fn enter(&self, focused: &Option<gtk::Widget>) {
        self.previous.replace(focused.clone());
        self.state.focus_active_place();
    }

    fn restore(&self, browser: &Browser, require_mapped: bool) {
        let restored =
            self.previous.borrow_mut().take().is_some_and(|widget| {
                (!require_mapped || widget.is_mapped()) && widget.grab_focus()
            });
        if !restored {
            browser.focus_active();
        }
    }
}
