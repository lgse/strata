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
mod footer_bindings;
mod input_owner;
mod items;
mod preview;
mod prompts;
mod sidebar;
mod tenxer;
mod wheel_zoom;

use footer_bindings::{
    bind_candidate_prompts, bind_footer_filter, bind_prompt_hints, clear_find_on_mode_exit,
    release_preview_keys_on_mode_exit, submit_prompt_on_activate,
};
pub(super) use wheel_zoom::WheelZoom;

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
        if let Some(result) = self.rename_tab(press) {
            return Some(result);
        }
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
        if let Some(result) = self.rename_tab(press) {
            return result;
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

    fn inline_editing_active(&self) -> bool {
        self.view.inline_edit_is_open()
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
