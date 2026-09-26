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
        browser::BrowserView, preview::PreviewDrawer, shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

use super::{SidebarState, SidebarView, TypeToSearch, visible_modal_layer};

mod commands;
mod focus;
mod items;

// None tries the next Strata stage; Some(Proceed) gives the event to GTK instead.
type KeyResult = Option<Propagation>;

pub(super) struct Bindings {
    pub view: BrowserView,
    pub top_bar: TopBarNavigation,
    pub preview: PreviewDrawer,
    pub type_to_search: TypeToSearch,
    pub shortcuts: ShortcutFooter,
}

pub(super) fn install(window: &gtk::ApplicationWindow, sidebar: &SidebarView, bindings: Bindings) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(&bindings.view.browser());
    let dispatcher = Dispatcher {
        window: window.clone(),
        view: bindings.view,
        top_bar: bindings.top_bar,
        preview: bindings.preview,
        type_to_search: bindings.type_to_search,
        shortcuts: bindings.shortcuts,
        sidebar: SidebarFocus {
            state: sidebar.state.clone(),
            widget: sidebar.widget.clone(),
            previous: RefCell::new(None),
        },
    };
    let preferences = dispatcher.type_to_search.preferences.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(browser) = weak_browser.upgrade() else {
            return Propagation::Proceed;
        };
        dispatcher.handle_key(&browser, key, modifiers)
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

fn command_modifiers(modifiers: Modifiers) -> Modifiers {
    modifiers
        & (Modifiers::CONTROL_MASK
            | Modifiers::SHIFT_MASK
            | Modifiers::ALT_MASK
            | Modifiers::SUPER_MASK)
}

fn plain_control(modifiers: Modifiers) -> bool {
    let modifiers = command_modifiers(modifiers);
    modifiers.contains(Modifiers::CONTROL_MASK)
        && !modifiers
            .intersects(Modifiers::SHIFT_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
}

fn claims_unbound_command(key: Key, modifiers: Modifiers) -> bool {
    let control_shift = {
        let modifiers = command_modifiers(modifiers);
        modifiers.contains(Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK)
            && !modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
    };
    match key {
        Key::d
        | Key::D
        | Key::f
        | Key::F
        | Key::b
        | Key::B
        | Key::r
        | Key::R
        | Key::t
        | Key::T
        | Key::backslash
            if plain_control(modifiers) =>
        {
            true
        }
        Key::k | Key::K if control_shift => true,
        _ => false,
    }
}

fn claims_file_list_typing(key: Key, modifiers: Modifiers) -> bool {
    if command_modifiers(modifiers)
        .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
    {
        return false;
    }
    if key == Key::space {
        return true;
    }
    if matches!(key, Key::q | Key::Q) {
        return false;
    }
    super::type_to_search_query(key, modifiers).is_some()
}

fn visible_popover_menu(widget: &gtk::Widget) -> bool {
    if widget.is_visible() && widget.is::<gtk::PopoverMenu>() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if visible_popover_menu(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
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

struct Dispatcher {
    window: gtk::ApplicationWindow,
    view: BrowserView,
    sidebar: SidebarFocus,
    top_bar: TopBarNavigation,
    preview: PreviewDrawer,
    type_to_search: TypeToSearch,
    shortcuts: ShortcutFooter,
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

impl Dispatcher {
    fn handle_key(&self, browser: &Rc<Browser>, key: Key, modifiers: Modifiers) -> Propagation {
        let preferences = &self.type_to_search.preferences;
        if let Some(size) = preferences.text_size().for_shortcut(key, modifiers) {
            preferences.set_text_size(size);
            return Propagation::Stop;
        }
        if let Some(result) = self.input_owner(key, modifiers) {
            return result;
        }
        if let Some(result) = self.tenxer_keys(browser, key, modifiers) {
            return result;
        }
        let focused = gtk::prelude::RootExt::focus(&self.window);
        let navigation_key = crate::ui::focus_navigation::navigation_key(
            key,
            modifiers,
            self.type_to_search.preferences.type_to_search_active(),
            focused.as_ref(),
        );
        let mut event = KeyEvent {
            key: navigation_key,
            modifiers,
            focused,
            vim_navigation: navigation_key != key,
            header_left_boundary: false,
        };
        self.window_commands(&event)
            .or_else(|| self.inline_editing(&event))
            .or_else(|| self.filter_and_location_commands(&event))
            .or_else(|| self.video_controls(&event))
            .or_else(|| self.sidebar_commands(browser, &event))
            .or_else(|| self.context_menu_command(&event))
            .or_else(|| self.properties_command(&event))
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
                        crate::ui::focus_navigation::activate_native_arrow(&self.window, event.key);
                        Some(Propagation::Stop)
                    } else {
                        Some(Propagation::Proceed)
                    }
                } else {
                    None
                }
            })
            .or_else(|| {
                if event.key == Key::Escape
                    && event.without(
                        Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK,
                    )
                    && self.preview.password_has_focus(event.focused.as_ref())
                {
                    self.dismiss_preview_or_selection(browser)
                } else {
                    None
                }
            })
            .or_else(|| self.text_input(&event))
            .or_else(|| self.file_commands(browser, &event))
            .or_else(|| self.archive_navigation(&event))
            .or_else(|| self.focus_navigation(browser, &mut event))
            .or_else(|| self.dismissal(browser, &event))
            .or_else(|| self.item_navigation(browser, &event))
            .unwrap_or(Propagation::Proceed)
    }

    fn input_owner(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if let Some(layer) = visible_modal_layer(&self.window) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            if !focus_is_inside {
                layer.grab_focus();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if self.native_menu_owns_input() {
            return Some(Propagation::Proceed);
        }
        if self.shortcuts.prompt_has_focus() {
            if let Some(result) = self.shortcuts.handle_key(key, modifiers) {
                return Some(result);
            }
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

    fn native_menu_owns_input(&self) -> bool {
        if gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::PopoverMenu>()
                || focused.ancestor(gtk::PopoverMenu::static_type()).is_some()
                || focused
                    .ancestor(gtk::Popover::static_type())
                    .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
        }) {
            return true;
        }
        visible_popover_menu(self.window.upcast_ref())
    }

    fn tenxer_keys(&self, browser: &Rc<Browser>, key: Key, modifiers: Modifiers) -> KeyResult {
        if visible_modal_layer(&self.window).is_some() {
            return None;
        }
        let preferences = &self.type_to_search.preferences;
        if crate::ui::tenxer_mode::is_toggle_shortcut(key, modifiers) {
            preferences.set_tenxer_mode(!preferences.tenxer_mode());
            return Some(Propagation::Stop);
        }
        if !preferences.tenxer_mode() {
            return None;
        }
        if self.text_focused() || self.focus_in_popover() {
            return None;
        }
        // Plain q leaves the mode. Shift+Q closes the window. List and Columns
        // claim their own chords, including paging keys that otherwise filter,
        // toggle the sidebar, or duplicate.
        let command = modifiers
            .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK);
        if key == Key::q && !modifiers.contains(Modifiers::SHIFT_MASK) && !command {
            preferences.set_tenxer_mode(false);
            return Some(Propagation::Stop);
        }
        if key == Key::Q && modifiers.contains(Modifiers::SHIFT_MASK) && !command {
            self.window.close();
            return Some(Propagation::Stop);
        }
        if self.tenxer_listing(browser, key, modifiers) {
            return Some(Propagation::Stop);
        }
        // Claim the conflicting default map. Still-bound shortcuts fall through
        // to the existing commands.
        if claims_unbound_command(key, modifiers)
            || (self.view.item_view_has_focus() && claims_file_list_typing(key, modifiers))
        {
            return Some(Propagation::Stop);
        }
        None
    }

    fn text_focused(&self) -> bool {
        gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::Text>() || focused.is::<gtk::TextView>() || focused.is::<gtk::Entry>()
        })
    }

    fn focus_in_popover(&self) -> bool {
        gtk::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is::<gtk::Popover>() || focused.ancestor(gtk::Popover::static_type()).is_some()
        })
    }

    fn arrows_scoped_to_content(&self) -> bool {
        self.type_to_search
            .preferences
            .arrow_navigation_scoped_active()
    }

    fn enter_sidebar(&self, event: &KeyEvent) {
        let previous = self
            .view
            .item_view_has_focus()
            .then(|| event.focused.clone())
            .flatten();
        self.sidebar.enter(&previous);
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
