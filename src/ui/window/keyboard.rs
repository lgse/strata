// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk::ModifierType as Modifiers, glib::Propagation, prelude::*};

use crate::{
    app::Browser,
    services::NavigationHistory,
    ui::{
        browser::BrowserView,
        browser_modes::ThumbnailResize,
        keys::{self, Command, KeyPress, Map, SHORTCUT_MODIFIERS, TEXT_EDITING, bindings::*},
        modal::is_closing_layer,
        preview::PreviewDrawer,
        shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

use super::{SidebarState, SidebarView, TypeToSearch, topmost_modal_layer, visible_modal_layer};

mod chooser;
pub(super) mod chords;
mod commands;
mod direct;
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

/// Window actions that run once every other stage yields the key, wherever
/// the active table binds them. 10xer runs its terminal key only from the file
/// list and sidebar, so it is left out here in that map.
const WINDOW_ACTIONS: [(Command, &str); 5] = [
    (SEARCH, "search"),
    (JUMP_TO_FOLDER, "jump-folder"),
    (TERMINAL, "open-terminal"),
    (REFRESH, "refresh"),
    (ARROW_SCOPE, "toggle-arrow-scope"),
];

pub(super) struct DispatcherParts {
    pub view: BrowserView,
    pub top_bar: TopBarNavigation,
    pub preview: PreviewDrawer,
    pub type_to_search: TypeToSearch,
    pub shortcuts: ShortcutFooter,
    pub history: Rc<NavigationHistory>,
}

#[cfg(test)]
pub(super) fn install(
    window: &impl IsA<gtk::Window>,
    sidebar: &SidebarView,
    parts: DispatcherParts,
) {
    let _ = install_on(window, window.upcast_ref::<gtk::Window>(), sidebar, parts);
}

pub(super) fn install_on(
    window: &impl IsA<gtk::Window>,
    root: &impl IsA<gtk::Widget>,
    sidebar: &SidebarView,
    parts: DispatcherParts,
) -> Vec<gtk::EventController> {
    let window = window.upcast_ref::<gtk::Window>();
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(&parts.view.browser());
    let preferences = parts.type_to_search.preferences.clone();
    let wheel_view = parts.view.downgrade();
    let dispatcher = Rc::new(Dispatcher::bind(window, sidebar, parts, None));
    submit_prompt_on_activate(&dispatcher, weak_browser.clone());
    let key_root = root.as_ref().downgrade();
    let tab_scoped = !root.as_ref().is::<gtk::Window>();
    keys::on_press(&keys, move |_, press| {
        if !key_root
            .upgrade()
            .is_some_and(|root| root.is_mapped() && root.can_target())
            || (tab_scoped && super::composition::is_tab_shortcut(press))
        {
            return Propagation::Proceed;
        }
        let Some(browser) = weak_browser.upgrade() else {
            return Propagation::Proceed;
        };
        match dispatcher.handle_key(&browser, press) {
            Propagation::Proceed => dispatcher.after_yield(press),
            handled => handled,
        }
    });
    window.add_controller(keys.clone());

    // Ctrl+wheel mirrors the Ctrl +/- text-size shortcut, and Ctrl+Shift+wheel
    // the Ctrl+Alt +/- thumbnail one. Capture phase so scrolled windows cannot
    // consume it first; the PDF preview's own zoom is left alone by passing
    // through scrolls inside its scroll container. DISCRETE is avoided: it
    // swallows sub-step smooth deltas before they reach descendant scrolled
    // windows, killing touchpad scrolling.
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let zoom = Rc::new(WheelZoom::default());
    let wheel_root = root.as_ref().downgrade();
    wheel.connect_scroll(move |controller, dx, dy| {
        if !wheel_root
            .upgrade()
            .is_some_and(|root| root.is_mapped() && root.can_target())
        {
            return Propagation::Proceed;
        }
        let Some(view) = wheel_view.upgrade() else {
            return Propagation::Proceed;
        };
        let modifiers = controller.current_event_state();
        // Scroll events carry no position, so ask the seat where the pointer is.
        let target = controller
            .current_event()
            .filter(|_| modifiers.contains(Modifiers::CONTROL_MASK))
            .and_then(|event| {
                let surface = event.surface()?;
                let device = event.seat()?.pointer()?;
                let (x, y, _) = surface.device_position(&device)?;
                let native = gtk::Native::for_surface(&surface)?;
                let (dx, dy) = native.surface_transform();
                native.pick(x - dx, y - dy, gtk::PickFlags::DEFAULT)
            });
        zoom.handle(
            &preferences,
            &view,
            modifiers,
            controller.unit(),
            target,
            (dx, dy),
        )
    });
    window.add_controller(wheel.clone());
    let controllers = vec![keys.upcast(), wheel.upcast()];
    release_controllers_on_close(window, root, &controllers);
    controllers
}

/// GTK's own text bindings miss a key that only the layout fallback reads as an
/// editing shortcut, so the focused text field runs the matching action itself.
pub(in crate::ui) fn edit_focused_text(window: &gtk::Window, press: &KeyPress) -> Propagation {
    if !press.used_fallback() {
        return Propagation::Proceed;
    }
    let focused = gtk::prelude::RootExt::focus(window)
        .filter(|focused| focused.is::<gtk::Text>() || focused.is::<gtk::TextView>());
    let edit = TEXT_EDITING.iter().find(|(binding, _)| press.is(*binding));
    let (Some(focused), Some((_, action))) = (focused, edit) else {
        return Propagation::Proceed;
    };
    if let Err(error) = focused.activate_action(action, None) {
        tracing::warn!(%error, action, "text field action unavailable");
    }
    Propagation::Stop
}

// Key controllers retain the dispatcher and its window; unrealize breaks the cycle.
pub(super) fn release_controllers_on_close(
    window: &gtk::Window,
    root: &impl IsA<gtk::Widget>,
    controllers: &[gtk::EventController],
) {
    let controllers: Vec<_> = controllers.iter().map(ObjectExt::downgrade).collect();
    let window = window.downgrade();
    root.connect_unrealize(move |_| {
        if let Some(window) = window.upgrade() {
            for controller in controllers.iter().filter_map(glib::WeakRef::upgrade) {
                if controller.widget().is_some() {
                    window.remove_controller(&controller);
                }
            }
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
    dispatcher: Rc<Dispatcher>,
    browser: std::rc::Weak<Browser>,
}

impl ChooserKeys {
    pub(super) fn new(
        window: &gtk::Window,
        sidebar: &SidebarView,
        parts: DispatcherParts,
        policy: ChooserPolicy,
    ) -> Self {
        let browser = Rc::downgrade(&parts.view.browser());
        let dispatcher = Rc::new(Dispatcher::bind(window, sidebar, parts, Some(policy)));
        submit_prompt_on_activate(&dispatcher, browser.clone());
        Self {
            dispatcher,
            browser,
        }
    }

    pub(in crate::ui) fn handle(&self, press: &KeyPress) -> KeyResult {
        let browser = self.browser.upgrade()?;
        self.dispatcher.handle_chooser_key(&browser, press)
    }

    pub(in crate::ui) fn reference_is_open(&self) -> bool {
        self.dispatcher.shortcuts.reference_is_open()
    }

    /// Returns false for a folder or no entry so the caller can navigate instead.
    pub(in crate::ui) fn confirm_focused(&self) -> bool {
        self.browser
            .upgrade()
            .is_some_and(|browser| self.dispatcher.chooser_confirm(&browser))
    }
}

/// Leaving 10xer mode ends preview key ownership but keeps the drawer open.
fn release_preview_keys_on_mode_exit(
    window: &gtk::Widget,
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
/// footer reports the focused listing's filter or search in every key map.
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
        view.upgrade()
            .map_or(Some(None), |view| view.filter_status())
    });
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher.view.set_footer_filter_focus(Rc::new(move || {
        shortcuts.prompt_has_focus()
            && matches!(
                shortcuts.open_prompt_kind(),
                Some(
                    crate::ui::tenxer_mode::Prompt::Filter | crate::ui::tenxer_mode::Prompt::Search
                )
            )
    }));
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher
        .view
        .connect_filter_results_changed(Rc::new(move || shortcuts.refresh_filter()));
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher
        .view
        .connect_search_selection_changed(Rc::new(move || shortcuts.schedule_filter_refresh()));
    // Over a fill, motion and Space move the result cursor by focus alone, and
    // GTK may land that focus after the selection change has been reported. In
    // Columns the footer also follows the focused column.
    let shortcuts = dispatcher.shortcuts.clone();
    let handler = dispatcher
        .window
        .connect_focus_widget_notify(move |_| shortcuts.schedule_filter_refresh());
    // A closed tab's view unrealizes while its window lives on.
    let window = dispatcher.window.downgrade();
    let handler = Cell::new(Some(handler));
    dispatcher.view.widget().connect_unrealize(move |_| {
        if let (Some(window), Some(handler)) = (window.upgrade(), handler.take()) {
            window.disconnect(handler);
        }
    });
}

/// Enter reaches the prompt only after an input method commits and passes it
/// back; otherwise the key handler submits the prompt.
fn submit_prompt_on_activate(dispatcher: &Rc<Dispatcher>, browser: std::rc::Weak<Browser>) {
    let entered = Rc::downgrade(dispatcher);
    dispatcher.shortcuts.connect_prompt_activated(move || {
        if let (Some(dispatcher), Some(browser)) = (entered.upgrade(), browser.upgrade()) {
            dispatcher.submit_prompt(&browser);
        }
    });
}

fn bind_prompt_hints(dispatcher: &Dispatcher) {
    use crate::ui::tenxer_mode::Prompt;
    let revision = dispatcher.destination_revision.clone();
    dispatcher.shortcuts.connect_prompt_reset(move || {
        revision.set(revision.get().wrapping_add(1));
    });
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
        if let Some((_, hint)) = hints.iter().find(|(hinted, _)| *hinted == kind) {
            hint.show(None, None);
        }
    });
}

fn bind_candidate_prompts(dispatcher: &Dispatcher) {
    let shortcuts = dispatcher.shortcuts.clone();
    let history = dispatcher.history.clone();
    let browser = Rc::downgrade(&dispatcher.view.browser());
    let destinations = dispatcher.destinations.clone();
    let targets = dispatcher.destination_targets.clone();
    dispatcher.shortcuts.connect_prompt_changed(move |kind, _| {
        shortcuts.forget_candidate_step();
        if let Some(browser) = browser.upgrade() {
            prompts::show_history_candidates(&shortcuts, &history, &browser, kind);
            prompts::show_folder_candidates(
                &shortcuts,
                &destinations,
                &browser,
                &targets.borrow(),
                kind,
            );
        }
    });
    let shortcuts = dispatcher.shortcuts.clone();
    let view = dispatcher.view.downgrade();
    let targets = dispatcher.destination_targets.clone();
    let revision = dispatcher.destination_revision.clone();
    dispatcher
        .shortcuts
        .connect_candidate_activated(move |path| {
            let Some(view) = view.upgrade() else {
                return;
            };
            let Some(kind) = shortcuts.open_prompt_kind() else {
                return;
            };
            if kind.holds_targets() {
                prompts::send_to_destination(&view, &shortcuts, &targets, &revision, kind, path);
                return;
            }
            if !kind.picks_history() && kind != crate::ui::tenxer_mode::Prompt::Go {
                return;
            }
            prompts::return_to_listing(&shortcuts, &view, &view.browser());
            view.keyboard_navigation();
            view.browser()
                .navigate_with_selection(crate::model::Location::local(path), true);
        });
}

fn clear_find_on_mode_exit(
    window: &gtk::Widget,
    dispatcher: &Dispatcher,
    browser: &std::rc::Weak<Browser>,
) {
    let shortcuts = dispatcher.shortcuts.clone();
    let open_with = dispatcher.open_with.clone();
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
            armed_actions.take();
            shortcuts.refresh_filter();
            let focus = window.root().and_then(|root| root.focus());
            let prompt_focused = shortcuts.prompt_has_focus();
            shortcuts.dismiss_prompt();
            if window.is_mapped()
                && (prompt_focused || focus.is_none())
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        },
    );
}

/// Accumulates fractional Ctrl+wheel deltas into whole steps so smooth devices
/// zoom in the same increments as wheel clicks: interface text, or Icons
/// thumbnails with Shift. Resets when the input reverses direction, changes
/// units or target, or is not claimed for zooming.
#[derive(Default)]
pub(super) struct WheelZoom {
    pending: Cell<f64>,
    direction: Cell<i8>,
    unit: Cell<Option<gtk::gdk::ScrollUnit>>,
    thumbnails: Cell<bool>,
}

impl WheelZoom {
    /// Surface-unit deltas arrive in pixels; 10 px make one wheel step, the
    /// same mapping GTK's discrete conversion uses.
    const SURFACE_PIXELS_PER_STEP: f64 = 10.0;

    fn reset(&self) {
        self.pending.set(0.0);
        self.direction.set(0);
    }

    fn accumulate(&self, unit: gtk::gdk::ScrollUnit, dy: f64) -> i32 {
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

    pub(super) fn handle(
        &self,
        preferences: &crate::ui::preferences::PreferenceManager,
        view: &BrowserView,
        modifiers: Modifiers,
        unit: gtk::gdk::ScrollUnit,
        target: Option<gtk::Widget>,
        (dx, dy): (f64, f64),
    ) -> Propagation {
        let thumbnails = modifiers.contains(Modifiers::SHIFT_MASK);
        // Some systems turn Shift+wheel into horizontal scrolling.
        let delta = if thumbnails && dy == 0.0 { dx } else { dy };
        if delta == 0.0 {
            return Propagation::Proceed;
        }
        if !modifiers.contains(Modifiers::CONTROL_MASK)
            || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
            || target.as_ref().is_some_and(inside_pdf_scroll)
            || (thumbnails && view.view_mode() != crate::ui::browser_modes::BrowserMode::Icons)
        {
            self.reset();
            return Propagation::Proceed;
        }
        if self.thumbnails.replace(thumbnails) != thumbnails {
            self.reset();
        }
        let steps = self.accumulate(unit, delta);
        if steps != 0 {
            if thumbnails {
                view.resize_icons_thumbnails(ThumbnailResize::Step(-steps));
            } else {
                view.set_text_size(preferences.text_size().stepped(-steps));
            }
        }
        Propagation::Stop
    }
}

/// Default-map keys of window commands that 10xer mode leaves unbound or binds
/// to other keys, so the file list swallows them, and Ctrl+T.
fn claims_unbound_command(press: &KeyPress) -> bool {
    [
        DUPLICATE,
        FILTER,
        SIDEBAR,
        RENAME,
        ARROW_SCOPE,
        JUMP_TO_FOLDER,
    ]
    .into_iter()
    .any(|command| press.is(command.in_map(Map::Default)) && !press.is(command.in_map(Map::Tenxer)))
        || press.is(TAB_NEW)
}

fn claims_file_list_typing(press: &KeyPress) -> bool {
    press.is(SPACE.ignoring(Modifiers::SHIFT_MASK)) || super::type_to_search_query(press).is_some()
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
    history: Rc<NavigationHistory>,
    rename_target: Rc<RefCell<Option<crate::model::FileEntry>>>,
    /// Only the latest typed name submitted may act after its check.
    typed_name_revision: Rc<Cell<u64>>,
    /// Cursor and fill changes must not retarget an open prompt.
    destination_targets: Rc<RefCell<Vec<crate::model::FileEntry>>>,
    destination_revision: Rc<Cell<u64>>,
    destinations: crate::ui::folder_picker::FolderPicker,
    armed_actions: Rc<RefCell<Option<files::ArmedActions>>>,
    open_with: files::OpenWithLookup,
    chooser: Option<ChooserPolicy>,
}

struct KeyEvent {
    press: KeyPress,
    focused: Option<gtk::Widget>,
    header_left_boundary: bool,
}

impl KeyEvent {
    fn shift(&self) -> bool {
        self.press.modifiers().contains(Modifiers::SHIFT_MASK)
    }

    fn without(&self, modifiers: Modifiers) -> bool {
        !self.press.modifiers().intersects(modifiers)
    }

    /// The arrow an alias reads as, which GTK's list bindings never see.
    fn vim_arrow(&self) -> Option<gtk::gdk::Key> {
        self.press
            .aliased()
            .then(|| crate::ui::focus_navigation::spatial_arrow(&self.press))
            .flatten()
            .map(keys::Binding::key)
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
        parts: DispatcherParts,
        chooser: Option<ChooserPolicy>,
    ) -> Self {
        let weak_browser = Rc::downgrade(&parts.view.browser());
        let dispatcher = Self {
            window: window.clone(),
            view: parts.view,
            top_bar: parts.top_bar,
            preview: parts.preview,
            type_to_search: parts.type_to_search,
            shortcuts: parts.shortcuts,
            history: parts.history,
            rename_target: Rc::default(),
            typed_name_revision: Rc::default(),
            destination_targets: Rc::default(),
            destination_revision: Rc::default(),
            destinations: crate::ui::folder_picker::FolderPicker::default(),
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
        let destinations_on_destroy = dispatcher.destinations.clone();
        let open_with_on_destroy = dispatcher.open_with.clone();
        dispatcher.view.widget().connect_unrealize(move |_| {
            cancel_on_destroy.cancel_chord();
            destinations_on_destroy.cancel();
            open_with_on_destroy.invalidate();
        });
        let rename_target = dispatcher.rename_target.clone();
        let typed_name_revision = dispatcher.typed_name_revision.clone();
        let destination_targets = dispatcher.destination_targets.clone();
        let destinations = dispatcher.destinations.clone();
        dispatcher.shortcuts.connect_prompt_reset(move || {
            typed_name_revision.set(typed_name_revision.get().wrapping_add(1));
            drop(rename_target.take());
            drop(destination_targets.take());
            destinations.cancel();
        });
        bind_prompt_hints(&dispatcher);
        bind_candidate_prompts(&dispatcher);
        release_preview_keys_on_mode_exit(
            &dispatcher.view.widget(),
            &dispatcher.preview,
            &weak_browser,
        );
        clear_find_on_mode_exit(&dispatcher.view.widget(), &dispatcher, &weak_browser);
        bind_footer_filter(&dispatcher);
        dispatcher
    }

    fn handle_chooser_key(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        if !self.type_to_search.preferences.tenxer_mode() {
            return self.tenxer_keys(browser, press);
        }
        if !press.is_modifier() {
            self.open_with.invalidate();
        }
        if self.text_focused()
            && !self.shortcuts.prompt_has_focus()
            && !self.preview_document_focused()
        {
            // The open reference's search entry counts as focused text; its
            // keys, Escape included, must reach the reference, not the request.
            if press.is(REFERENCE.in_map(Map::Default).ignoring(SHORTCUT_MODIFIERS))
                || self.shortcuts.reference_is_open()
            {
                return self.shortcuts.handle_key(press);
            }
            return self.tenxer_keys(browser, press);
        }
        self.input_owner(browser, press)
            .or_else(|| self.tenxer_keys(browser, press))
    }

    fn handle_key(&self, browser: &Rc<Browser>, press: &KeyPress) -> Propagation {
        if !press.is_modifier() {
            self.open_with.invalidate();
        }
        let preferences = &self.type_to_search.preferences;
        // Text-size shortcuts and a focused document's zoom reset run before
        // the chord consumer. Drop the mark first, matching Ctrl+, opening
        // Settings.
        if self.preview.reset_pdf_zoom(press) {
            self.shortcuts.cancel_chord();
            return Propagation::Stop;
        }
        if let Some(size) = preferences.text_size().for_shortcut(press) {
            self.shortcuts.cancel_chord();
            self.view.set_text_size(size);
            return Propagation::Stop;
        }
        if let Some(change) = ThumbnailResize::for_shortcut(press)
            && self.view.resize_icons_thumbnails(change)
        {
            self.shortcuts.cancel_chord();
            return Propagation::Stop;
        }
        if let Some(result) = self.input_owner(browser, press) {
            return result;
        }
        if let Some(result) = self.tenxer_keys(browser, press) {
            return result;
        }
        let mut event = KeyEvent {
            press: *press,
            focused: gtk::prelude::RootExt::focus(&self.window),
            header_left_boundary: false,
        };
        self.window_commands(&event)
            .or_else(|| self.direct_commands(&event))
            .or_else(|| self.default_tab_navigation(&event))
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
                    && !event.press.is(MOVE_TO_TRASH
                        .in_map(Map::Default)
                        .ignoring(SHORTCUT_MODIFIERS))
                {
                    if event.press.is_any(&[COPY, CUT, PASTE, SELECT_ALL])
                        && let Some(result) = self.clipboard_command(&event)
                    {
                        return Some(result);
                    }
                    if let Some(arrow) = event.vim_arrow() {
                        crate::ui::focus_navigation::activate_native_arrow(&self.window, arrow);
                        Some(Propagation::Stop)
                    } else {
                        Some(Propagation::Proceed)
                    }
                } else {
                    None
                }
            })
            .or_else(|| {
                if event.press.is(ESCAPE_ANY_SHIFT)
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

    fn input_owner(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if let Some(layer) = topmost_modal_layer(&self.window, &is_closing_layer) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            self.shortcuts.cancel_chord();
            if !focus_is_inside {
                layer.grab_focus();
                return Some(Propagation::Stop);
            }
            return Some(Propagation::Proceed);
        }
        if visible_modal_layer(&self.window).is_some() {
            // A layer animating out has dropped focus; only window actions, whose
            // guard skips closing layers, may act until focus is restored.
            self.shortcuts.cancel_chord();
            return Some(Propagation::Proceed);
        }
        if self.native_menu_owns_input() {
            return Some(Propagation::Proceed);
        }
        if press.is(ESCAPE_ANY) && self.view.cancel_column_drag() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        let mut focused = gtk::prelude::RootExt::focus(&self.window);
        while let Some(widget) = focused {
            if widget.has_css_class("file-operation-card") {
                self.shortcuts.cancel_chord();
                return Some(Propagation::Proceed);
            }
            focused = widget.parent();
        }
        if self.shortcuts.prompt_has_focus() {
            if let Some(result) = self.footer_key(press) {
                return Some(result);
            }
            return Some(self.prompt_key(browser, press));
        }
        if self
            .preview
            .handle_find_key(press, self.type_to_search.preferences.tenxer_mode())
        {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        if let Some(result) = self.tenxer_preview_text(browser, press) {
            return Some(result);
        }
        if !self.inline_editing_active()
            && let Some(result) = self.footer_key(press)
        {
            return Some(result);
        }
        if press.is(ESCAPE_ANY) && crate::ui::scrolling::stop_autoscroll() {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        None
    }

    /// Where GTK's accelerators ran: window actions, then a focused text
    /// field's editing keys.
    fn after_yield(&self, press: &KeyPress) -> Propagation {
        if self.shortcuts.reference_is_open() {
            return edit_focused_text(&self.window, press);
        }
        match self.window_action(press) {
            Propagation::Proceed => edit_focused_text(&self.window, press),
            handled => handled,
        }
    }

    /// Like a GTK accelerator, an enabled action consumes its key even when the
    /// action's guard then declines to run.
    fn window_action(&self, press: &KeyPress) -> Propagation {
        let Some((command, name)) = WINDOW_ACTIONS
            .iter()
            .find(|(command, _)| press.is(*command))
        else {
            return Propagation::Proceed;
        };
        if *command == TERMINAL && self.type_to_search.preferences.tenxer_mode() {
            return Propagation::Proceed;
        }
        match self.window.downcast_ref::<gtk::ApplicationWindow>() {
            Some(window) if window.is_sensitive() && window.is_action_enabled(name) => {
                gtk::gio::prelude::ActionGroupExt::activate_action(window, name, None);
                Propagation::Stop
            }
            _ => Propagation::Proceed,
        }
    }

    /// Shortcut-reference keys run before the chord consumer. A visible prompt
    /// keeps its armed chord; every other claimed footer key cancels first.
    fn footer_key(&self, press: &KeyPress) -> KeyResult {
        let prompted = self.shortcuts.prompt_is_visible();
        let result = self.shortcuts.handle_key(press)?;
        if !prompted {
            self.shortcuts.cancel_chord();
        }
        Some(result)
    }

    fn inline_editing_active(&self) -> bool {
        self.view.inline_edit_is_open()
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

    fn tenxer_keys(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        if visible_modal_layer(&self.window).is_some() {
            return None;
        }
        let preferences = &self.type_to_search.preferences;
        if press.is(TENXER_MODE) {
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
        if let Some(result) = self.tenxer_chord(browser, press) {
            return Some(result);
        }
        if let Some(result) = self
            .chooser_refusal(press)
            .or_else(|| self.chooser_save(press))
        {
            return Some(result);
        }
        if press.is(CLOSE_WINDOW) {
            self.window.close();
            return Some(Propagation::Stop);
        }
        if let Some(result) = self.tenxer_preview(browser, press) {
            return Some(result);
        }
        let focus = gtk::prelude::RootExt::focus(&self.window);
        if self.sidebar.contains(&focus)
            && let Some(result) = self.tenxer_sidebar(browser, press)
        {
            return Some(result);
        }
        if self.tenxer_header_focused(&focus)
            && let Some(result) = self.tenxer_header(browser, press)
        {
            return Some(result);
        }
        if press.is(ESCAPE_ANY_SHIFT) && !self.inline_editing_active() {
            return self.tenxer_escape(browser);
        }
        // Unclaimed typing belongs to the listing in 10xer mode, even when
        // compositor focus restoration selected a header or footer control.
        // Text, menus, sidebar/header actions, and preview ownership run first.
        if super::type_to_search_query(press).is_some()
            && !self.view.item_view_has_focus()
            && !self.preview.owns_focus(focus.as_ref())
        {
            browser.focus_active();
        }
        if let Some(result) = self
            .tenxer_prompt_keys(press)
            .or_else(|| self.tenxer_file_keys(press))
        {
            return Some(result);
        }
        let icons = self.view.view_mode() == crate::ui::browser_modes::BrowserMode::Icons;
        let claimed = if icons {
            self.tenxer_icons(browser, press)
        } else {
            self.tenxer_listing(browser, press)
        };
        if claimed {
            return Some(Propagation::Stop);
        }
        if claims_unbound_command(press)
            || (self.view.item_view_has_focus()
                && claims_file_list_typing(press)
                && !self.runs_directly(press))
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

    fn tenxer_sidebar(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        if press.is(CONTEXT_MENU) && sidebar::open_sidebar_context_menu(&self.sidebar.widget) {
            self.shortcuts.cancel_chord();
            return Some(Propagation::Stop);
        }
        let chord = sidebar_chord(press)?;
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

    fn tenxer_header(&self, browser: &Browser, press: &KeyPress) -> KeyResult {
        let held = press.modifiers();
        if held.intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK) {
            return None;
        }
        // The held Shift, including one that typed a symbol, swallows the key.
        if held.contains(Modifiers::SHIFT_MASK) && !press.is(FOCUS_PREVIOUS) {
            return Some(Propagation::Stop);
        }
        if press.is_any(&[LEFT.alternates(), DOWN.alternates()]) {
            self.return_from_header(browser);
        } else if press.is(ENTER) || press.is(SPACE) {
            crate::ui::focus_navigation::activate(self.window.upcast_ref());
        } else if !press.is(MOVE_TO_TRASH.in_map(Map::Default)) {
            return None;
        }
        Some(Propagation::Stop)
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
