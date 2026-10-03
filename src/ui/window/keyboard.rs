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
    services::NavigationHistory,
    ui::{
        browser::BrowserView,
        go_completion::{FolderSource, GoCompletion},
        preview::PreviewDrawer,
        shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

use super::{SidebarState, SidebarView, TypeToSearch, visible_modal_layer};

mod chooser;
pub(super) mod chords;
mod commands;
mod escape;
mod files;
mod focus;
mod items;
mod preview;
mod prompts;
mod sidebar;

use sidebar::{SidebarChord, activate_sidebar_focus, move_sidebar_focus, sidebar_chord};

// None tries the next Strata stage; Some(Proceed) gives the event to GTK instead.
type KeyResult = Option<Propagation>;

pub(super) struct Bindings {
    pub view: BrowserView,
    pub top_bar: TopBarNavigation,
    pub preview: PreviewDrawer,
    pub type_to_search: TypeToSearch,
    pub shortcuts: ShortcutFooter,
    pub folders: Rc<dyn FolderSource>,
    pub history: Rc<NavigationHistory>,
}

pub(super) fn install(window: &impl IsA<gtk::Window>, sidebar: &SidebarView, bindings: Bindings) {
    let window = window.upcast_ref::<gtk::Window>();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(&bindings.view.browser());
    let preferences = bindings.type_to_search.preferences.clone();
    let dispatcher = Dispatcher::bind(window, sidebar, bindings, None);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(browser) = weak_browser.upgrade() else {
            return Propagation::Proceed;
        };
        dispatcher.handle_key(&browser, key, modifiers)
    });
    window.add_controller(keys.clone());

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
    window.add_controller(wheel.clone());
    release_controllers_on_close(window, &[keys.upcast(), wheel.upcast()]);
}

// Key controllers retain the dispatcher and its window; unrealize breaks the cycle.
fn release_controllers_on_close(window: &gtk::Window, controllers: &[gtk::EventController]) {
    let controllers: Vec<_> = controllers.iter().map(ObjectExt::downgrade).collect();
    window.connect_unrealize(move |window| {
        for controller in controllers.iter().filter_map(glib::WeakRef::upgrade) {
            window.remove_controller(&controller);
        }
    });
}

pub(in crate::ui) struct ChooserPolicy {
    pub(in crate::ui) multiple: bool,
    pub(in crate::ui) confirm: Rc<dyn Fn(crate::model::FileEntry)>,
    pub(in crate::ui) cancel: Rc<dyn Fn()>,
    pub(in crate::ui) save: Option<Rc<dyn Fn()>>,
    pub(in crate::ui) edit_name: Option<Rc<dyn Fn()>>,
}

pub(in crate::ui) struct ChooserKeys {
    dispatcher: Dispatcher,
    browser: std::rc::Weak<Browser>,
}

impl ChooserKeys {
    pub(super) fn new(
        window: &gtk::Window,
        sidebar: &SidebarView,
        bindings: Bindings,
        policy: ChooserPolicy,
    ) -> Self {
        let browser = Rc::downgrade(&bindings.view.browser());
        Self {
            dispatcher: Dispatcher::bind(window, sidebar, bindings, Some(policy)),
            browser,
        }
    }

    pub(in crate::ui) fn handle(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        let browser = self.browser.upgrade()?;
        self.dispatcher.handle_chooser_key(&browser, key, modifiers)
    }
}

/// Leaving 10xer mode ends preview key ownership but keeps the drawer open.
fn release_preview_keys_on_mode_exit(
    window: &gtk::Window,
    preview: &PreviewDrawer,
    browser: &std::rc::Weak<Browser>,
) {
    let preview = preview.clone();
    let browser = browser.clone();
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        window,
        crate::ui::preferences::PreferenceManager::tenxer_mode,
        move |window, enabled| {
            if !enabled
                && preview.owns_focus(window.root().and_then(|root| root.focus()).as_ref())
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        },
    );
}

/// The **f** and **s** prompts filter and search as they are typed, and the
/// footer reports the focused listing's filter or search while the mode is on.
fn bind_footer_filter(dispatcher: &Dispatcher) {
    let view = dispatcher.view.downgrade();
    dispatcher
        .shortcuts
        .connect_prompt_changed(move |kind, text| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match kind {
                crate::ui::tenxer_mode::Prompt::Filter => view.set_listing_filter(&text),
                crate::ui::tenxer_mode::Prompt::Search => view.set_listing_search(&text),
                _ => {}
            }
        });
    let view = dispatcher.view.downgrade();
    dispatcher.shortcuts.observe_filter(move || {
        if !crate::ui::tenxer_mode::chrome_suppressed() {
            return Some(None);
        }
        view.upgrade()
            .map_or(Some(None), |view| view.filter_status())
    });
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher
        .view
        .connect_filter_results_changed(Rc::new(move || shortcuts.refresh_filter()));
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher
        .view
        .connect_search_selection_changed(Rc::new(move || shortcuts.schedule_filter_refresh()));
}

/// Completion replacements are not edits; user edits invalidate pending lookups.
fn bind_go_completion(dispatcher: &Dispatcher) {
    use crate::ui::tenxer_mode::Prompt;
    let go = dispatcher.go.clone();
    let revision = dispatcher.destination_revision.clone();
    dispatcher.shortcuts.connect_prompt_reset(move || {
        go.invalidate();
        revision.set(revision.get().wrapping_add(1));
    });
    let go = dispatcher.go.clone();
    let hints: Vec<_> = [
        Prompt::Go,
        Prompt::Create,
        Prompt::Rename,
        Prompt::MoveTo,
        Prompt::CopyTo,
        Prompt::ExtractTo,
    ]
    .into_iter()
    .map(|kind| (kind, dispatcher.shortcuts.prompt_sink(kind)))
    .collect();
    let revision = dispatcher.destination_revision.clone();
    dispatcher.shortcuts.connect_prompt_changed(move |kind, _| {
        revision.set(revision.get().wrapping_add(1));
        if kind.completes_folders() {
            go.invalidate();
        }
        if let Some((_, hint)) = hints.iter().find(|(hinted, _)| *hinted == kind) {
            hint.show(None, None);
        }
    });
}

fn bind_history_prompts(dispatcher: &Dispatcher) {
    let shortcuts = dispatcher.shortcuts.clone();
    let history = dispatcher.history.clone();
    let browser = Rc::downgrade(&dispatcher.view.browser());
    dispatcher.shortcuts.connect_prompt_changed(move |kind, _| {
        if let Some(browser) = browser.upgrade() {
            prompts::show_history_candidates(&shortcuts, &history, &browser, kind);
        }
    });
    let shortcuts = dispatcher.shortcuts.clone();
    let view = dispatcher.view.clone();
    dispatcher
        .shortcuts
        .connect_candidate_activated(move |path| {
            if !shortcuts
                .open_prompt_kind()
                .is_some_and(crate::ui::tenxer_mode::Prompt::picks_history)
            {
                return;
            }
            shortcuts.dismiss_prompt();
            if !view.focus_visible_results() {
                view.browser().focus_active();
            }
            view.keyboard_navigation();
            view.browser()
                .navigate_with_selection(crate::model::Location::local(path), true);
        });
}

fn clear_find_on_mode_exit(
    window: &gtk::Window,
    dispatcher: &Dispatcher,
    browser: &std::rc::Weak<Browser>,
) {
    let shortcuts = dispatcher.shortcuts.clone();
    let open_with = dispatcher.open_with.clone();
    let go = dispatcher.go.clone();
    let armed_actions = dispatcher.armed_actions.clone();
    let browser = browser.clone();
    let primed = Cell::new(false);
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        window,
        crate::ui::preferences::PreferenceManager::tenxer_mode,
        move |window, enabled| {
            let starting = !primed.replace(true);
            if enabled || starting {
                shortcuts.refresh_filter();
                return;
            }
            open_with.invalidate();
            go.invalidate();
            armed_actions.take();
            shortcuts.refresh_filter();
            let focus = window.root().and_then(|root| root.focus());
            let prompt_focused = shortcuts.prompt_has_focus();
            shortcuts.dismiss_prompt();
            if (prompt_focused || focus.is_none())
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        },
    );
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
    window: gtk::Window,
    view: BrowserView,
    sidebar: SidebarFocus,
    top_bar: TopBarNavigation,
    preview: PreviewDrawer,
    type_to_search: TypeToSearch,
    shortcuts: ShortcutFooter,
    go: GoCompletion,
    history: Rc<NavigationHistory>,
    rename_target: Rc<RefCell<Option<crate::model::FileEntry>>>,
    /// Cursor and fill changes must not retarget an open prompt.
    destination_targets: Rc<RefCell<Vec<crate::model::FileEntry>>>,
    destination_revision: Rc<Cell<u64>>,
    armed_actions: Rc<RefCell<Option<files::ArmedActions>>>,
    open_with: files::OpenWithLookup,
    chooser: Option<ChooserPolicy>,
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
    fn bind(
        window: &gtk::Window,
        sidebar: &SidebarView,
        bindings: Bindings,
        chooser: Option<ChooserPolicy>,
    ) -> Self {
        let weak_browser = Rc::downgrade(&bindings.view.browser());
        let dispatcher = Self {
            window: window.clone(),
            view: bindings.view,
            top_bar: bindings.top_bar,
            preview: bindings.preview,
            type_to_search: bindings.type_to_search,
            shortcuts: bindings.shortcuts,
            go: GoCompletion::new(bindings.folders),
            history: bindings.history,
            rename_target: Rc::default(),
            destination_targets: Rc::default(),
            destination_revision: Rc::default(),
            armed_actions: Rc::default(),
            open_with: files::OpenWithLookup::default(),
            sidebar: SidebarFocus {
                state: sidebar.state.clone(),
                widget: sidebar.widget.clone(),
                previous: RefCell::new(None),
            },
            chooser,
        };
        dispatcher.preview.bind_keyboard_view(&dispatcher.view);
        let keycaps = Rc::downgrade(&sidebar.state);
        dispatcher.shortcuts.connect_chord_changed(move |chord| {
            if let Some(sidebar) = keycaps.upgrade() {
                sidebar.show_place_keycaps(chord == Some(crate::ui::tenxer_mode::Chord::Go));
            }
        });
        let cancel_on_destroy = dispatcher.shortcuts.clone();
        let go_on_destroy = dispatcher.go.clone();
        let open_with_on_destroy = dispatcher.open_with.clone();
        window.connect_unrealize(move |_| {
            cancel_on_destroy.cancel_chord();
            go_on_destroy.invalidate();
            open_with_on_destroy.invalidate();
        });
        let rename_target = dispatcher.rename_target.clone();
        let destination_targets = dispatcher.destination_targets.clone();
        dispatcher.shortcuts.connect_prompt_reset(move || {
            drop(rename_target.take());
            drop(destination_targets.take());
        });
        bind_go_completion(&dispatcher);
        bind_history_prompts(&dispatcher);
        release_preview_keys_on_mode_exit(window, &dispatcher.preview, &weak_browser);
        clear_find_on_mode_exit(window, &dispatcher, &weak_browser);
        bind_footer_filter(&dispatcher);
        dispatcher
    }

    fn handle_chooser_key(
        &self,
        browser: &Rc<Browser>,
        key: Key,
        modifiers: Modifiers,
    ) -> KeyResult {
        if !self.type_to_search.preferences.tenxer_mode() {
            return self.tenxer_keys(browser, key, modifiers);
        }
        if !items::is_modifier_key(key) {
            self.open_with.invalidate();
        }
        if self.text_focused()
            && !self.shortcuts.prompt_has_focus()
            && !self.preview_document_focused()
        {
            // The open reference's search entry counts as focused text; its
            // keys, Escape included, must reach the reference, not the request.
            if key == Key::F1 || self.shortcuts.reference_is_open() {
                return self.shortcuts.handle_key(key, modifiers);
            }
            return self.tenxer_keys(browser, key, modifiers);
        }
        self.input_owner(browser, key, modifiers)
            .or_else(|| self.tenxer_keys(browser, key, modifiers))
    }

    fn handle_key(&self, browser: &Rc<Browser>, key: Key, modifiers: Modifiers) -> Propagation {
        if !items::is_modifier_key(key) {
            self.open_with.invalidate();
        }
        let preferences = &self.type_to_search.preferences;
        if let Some(size) = preferences.text_size().for_shortcut(key, modifiers) {
            // Text-size shortcuts run before the chord consumer. Drop the mark
            // first, then resize, matching Ctrl+, opening Settings.
            self.shortcuts.cancel_chord();
            preferences.set_text_size(size);
            return Propagation::Stop;
        }
        if let Some(result) = self.input_owner(browser, key, modifiers) {
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
            .or_else(|| self.preview_navigation(&event))
            .or_else(|| self.focus_navigation(browser, &mut event))
            .or_else(|| self.dismissal(browser, &event))
            .or_else(|| self.item_navigation(browser, &event))
            .unwrap_or(Propagation::Proceed)
    }

    fn input_owner(&self, browser: &Browser, key: Key, modifiers: Modifiers) -> KeyResult {
        if let Some(layer) = visible_modal_layer(&self.window) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            self.shortcuts.cancel_chord();
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
            if let Some(result) = self.footer_key(key, modifiers) {
                return Some(result);
            }
            return Some(self.prompt_key(browser, key, modifiers));
        }
        if let Some(result) = self.tenxer_preview_text(browser, key, modifiers) {
            return Some(result);
        }
        if !self.inline_editing_active()
            && let Some(result) = self.footer_key(key, modifiers)
        {
            return Some(result);
        }
        if key == Key::Escape && crate::ui::scrolling::stop_autoscroll() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        None
    }

    /// Shortcut-reference keys run before the chord consumer. A visible prompt
    /// keeps its armed chord; every other claimed footer key cancels first.
    fn footer_key(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        let prompted = self.shortcuts.prompt_is_visible();
        let result = self.shortcuts.handle_key(key, modifiers)?;
        if !prompted {
            self.shortcuts.cancel_chord();
        }
        Some(result)
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
        if (self.text_focused() && !self.preview_document_focused()) || self.focus_in_popover() {
            self.shortcuts.cancel_chord();
            return None;
        }
        if let Some(result) = self.tenxer_chord(browser, key, modifiers) {
            return Some(result);
        }
        let command = modifiers
            .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK);
        if let Some(result) = self
            .chooser_refusal(key, modifiers)
            .or_else(|| self.chooser_save(key, modifiers))
        {
            return Some(result);
        }
        if key == Key::Q && modifiers.contains(Modifiers::SHIFT_MASK) && !command {
            self.window.close();
            return Some(Propagation::Stop);
        }
        if let Some(result) = self.tenxer_preview(browser, key, modifiers) {
            return Some(result);
        }
        let focus = gtk::prelude::RootExt::focus(&self.window);
        if self.sidebar.contains(&focus)
            && let Some(result) = self.tenxer_sidebar(browser, key, modifiers)
        {
            return Some(result);
        }
        if self.tenxer_header_focused(&focus)
            && let Some(result) = self.tenxer_header(browser, key, modifiers)
        {
            return Some(result);
        }
        if key == Key::Escape && !command && !self.inline_editing_active() {
            return self.tenxer_escape(browser);
        }
        // Unclaimed typing belongs to the listing in 10xer mode, even when
        // compositor focus restoration selected a header or footer control.
        // Text, menus, sidebar/header actions, and preview ownership run first.
        if key != Key::space
            && claims_file_list_typing(key, modifiers)
            && !self.view.item_view_has_focus()
            && !self.preview.owns_focus(focus.as_ref())
        {
            browser.focus_active();
        }
        if let Some(result) = self
            .tenxer_prompt_keys(key, modifiers)
            .or_else(|| self.tenxer_file_keys(key, modifiers))
        {
            return Some(result);
        }
        let icons = self.view.view_mode() == crate::ui::browser_modes::BrowserMode::Icons;
        let claimed = if icons {
            self.tenxer_icons(browser, key, modifiers)
        } else {
            self.tenxer_listing(browser, key, modifiers)
        };
        if claimed {
            return Some(Propagation::Stop);
        }
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

    fn tenxer_sidebar(&self, browser: &Browser, key: Key, modifiers: Modifiers) -> KeyResult {
        if super::is_context_menu_shortcut(key, modifiers)
            && sidebar::open_sidebar_context_menu(&self.sidebar.widget)
        {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        let chord = sidebar_chord(key, modifiers)?;
        match chord {
            SidebarChord::Move(delta) => {
                move_sidebar_focus(&self.sidebar.widget, delta);
            }
            SidebarChord::Activate => self.activate_sidebar(browser),
            SidebarChord::Leave => self.sidebar.restore(browser, true),
            SidebarChord::Swallow => {}
        }
        Some(Propagation::Stop)
    }

    fn activate_sidebar(&self, browser: &Browser) {
        let before = browser.active_location();
        if !activate_sidebar_focus(&self.sidebar.widget) {
            return;
        }
        if browser.active_location() != before {
            self.sidebar.previous.replace(None);
            if !self.view.item_view_has_focus() {
                browser.focus_active();
            }
        }
    }

    fn tenxer_header_focused(&self, focus: &Option<gtk::Widget>) -> bool {
        if self.top_bar.has_focus() || self.view.header_actions_have_focus() {
            return true;
        }
        let panes = self.view.widget();
        crate::ui::focus_navigation::contains_widget(&panes, focus.as_ref())
            && !self.view.item_view_has_focus()
    }

    fn tenxer_header(&self, browser: &Browser, key: Key, modifiers: Modifiers) -> KeyResult {
        if modifiers
            .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return None;
        }
        if modifiers.contains(Modifiers::SHIFT_MASK) && !matches!(key, Key::Tab | Key::ISO_Left_Tab)
        {
            return Some(Propagation::Stop);
        }
        match key {
            Key::h | Key::j => {
                self.return_from_header(browser);
                Some(Propagation::Stop)
            }
            Key::Return | Key::KP_Enter | Key::space => {
                crate::ui::focus_navigation::activate(self.window.upcast_ref());
                Some(Propagation::Stop)
            }
            Key::Delete => Some(Propagation::Stop),
            _ => None,
        }
    }

    fn return_from_header(&self, browser: &Browser) {
        if self.view.header_actions_have_focus() && self.view.focus_items_from_header() {
            return;
        }
        browser.focus_active();
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
