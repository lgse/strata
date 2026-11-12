// SPDX-License-Identifier: MIT

mod commands;
mod editing;
mod focus;
mod movement;

use super::*;
use crate::{app::Browser, ui::window::SidebarState};

const STOP: Option<glib::Propagation> = Some(glib::Propagation::Stop);
const PROCEED: Option<glib::Propagation> = Some(glib::Propagation::Proceed);

/// One step of the chooser's key handling: `Some` ends handling with that result.
type Stage = fn(&ChooserShortcuts, &KeyContext<'_>) -> Option<glib::Propagation>;

/// Stages that read the key as pressed, before type-to-search untypes it.
const PRESSED_STAGES: &[Stage] = &[
    ChooserShortcuts::tenxer_text_keeps_key,
    ChooserShortcuts::reset_preview_zoom,
    ChooserShortcuts::tenxer_key,
    ChooserShortcuts::text_size,
    ChooserShortcuts::resize_thumbnails,
    ChooserShortcuts::reference_keeps_key,
    ChooserShortcuts::folder_menu_keeps_key,
    ChooserShortcuts::back_history_keeps_key,
    ChooserShortcuts::context_menu,
    ChooserShortcuts::video_key,
    ChooserShortcuts::filtered_rows_navigate,
];

/// Stages that read the key as type-to-search leaves it.
const NAVIGATION_STAGES: &[Stage] = &[
    ChooserShortcuts::mark_keyboard_navigation,
    ChooserShortcuts::recover_lost_focus,
    ChooserShortcuts::escape,
    ChooserShortcuts::rename,
    ChooserShortcuts::properties,
    ChooserShortcuts::editing_keeps_key,
    ChooserShortcuts::preview_search_result,
    ChooserShortcuts::filter,
    ChooserShortcuts::location,
    ChooserShortcuts::sidebar_focus,
    ChooserShortcuts::sidebar_toggle,
    ChooserShortcuts::location_keeps_key,
    ChooserShortcuts::choose_folder,
    ChooserShortcuts::new_folder,
    ChooserShortcuts::select_all,
    ChooserShortcuts::refresh,
    ChooserShortcuts::hidden_files,
    ChooserShortcuts::delete,
    ChooserShortcuts::browser_mode,
    ChooserShortcuts::control_keeps_key,
    ChooserShortcuts::column_popover_focus,
    ChooserShortcuts::default_tab,
    ChooserShortcuts::tenxer_name_tab,
    ChooserShortcuts::tenxer_options_tab,
    ChooserShortcuts::focused_widget_key,
    ChooserShortcuts::tenxer_tab_to_header,
    ChooserShortcuts::tenxer_tab,
    ChooserShortcuts::header_actions,
    ChooserShortcuts::sidebar_arrows,
    ChooserShortcuts::dismiss_empty_filter,
    ChooserShortcuts::outside_item_view,
    ChooserShortcuts::left_to_sidebar,
    ChooserShortcuts::toggle_preview,
    ChooserShortcuts::navigate_up,
    ChooserShortcuts::item_arrows,
    ChooserShortcuts::extend_selection,
    ChooserShortcuts::type_to_search,
    ChooserShortcuts::up_to_header,
    ChooserShortcuts::jump,
    ChooserShortcuts::move_or_open,
];

pub(super) struct ShortcutParts<'a> {
    pub(super) sidebar: &'a SidebarView,
    pub(super) sidebar_toggle: &'a gtk::ToggleButton,
    pub(super) header: &'a gtk::Box,
    pub(super) options: &'a gtk::FlowBox,
    pub(super) preview: &'a PreviewDrawer,
    pub(super) footer: &'a ShortcutFooter,
}

fn tenxer_keys(state: &Rc<ChooserState>, parts: &ShortcutParts<'_>) -> ChooserKeys {
    let confirming = Rc::downgrade(state);
    let cancelling = Rc::downgrade(state);
    let saving = Rc::downgrade(state);
    let naming = Rc::downgrade(state);
    let save_request = matches!(
        &state.request.kind,
        ChooserKind::SaveFile { .. } | ChooserKind::SaveFiles { .. }
    );
    crate::ui::window::chooser_keys(
        &state.window,
        &state.view,
        parts.sidebar,
        TopBarNavigation::new(parts.header, &parts.sidebar.widget, parts.sidebar_toggle),
        parts.preview,
        parts.footer,
        ChooserPolicy {
            multiple: matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            ),
            confirm: Rc::new(move |entry| {
                if let Some(state) = confirming.upgrade() {
                    state.confirm_file(&entry);
                }
            }),
            cancel: Rc::new(move || {
                if let Some(state) = cancelling.upgrade() {
                    state.cancel();
                }
            }),
            save: save_request.then(|| {
                Rc::new(move || {
                    if let Some(state) = saving.upgrade() {
                        state.accept();
                    }
                }) as Rc<dyn Fn()>
            }),
            edit_name: (save_request && state.filename.is_some()).then(|| {
                Rc::new(move || {
                    if let Some(state) = naming.upgrade() {
                        state.edit_name();
                    }
                }) as Rc<dyn Fn()>
            }),
        },
    )
}

pub(super) fn install_shortcuts(state: &Rc<ChooserState>, parts: ShortcutParts<'_>) {
    let tenxer = tenxer_keys(state, &parts);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let shortcuts = ChooserShortcuts::new(&parts, tenxer);
    keys::on_press(&keys, move |_, press| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match shortcuts.handle(&state, press) {
            glib::Propagation::Proceed => edit_focused_text(&state.window, press),
            handled => handled,
        }
    });
    state.window.add_controller(keys.clone());
    let keys = keys.downgrade();
    state.window.connect_unrealize(move |window| {
        if let Some(keys) = keys.upgrade() {
            window.remove_controller(&keys);
        }
    });
}

/// The widgets the chooser's key handler reaches, kept for the window's life.
struct ChooserShortcuts {
    sidebar_state: Rc<SidebarState>,
    sidebar_widget: gtk::Widget,
    sidebar_toggle: gtk::ToggleButton,
    header: gtk::Widget,
    /// The request's filter, choices and read-only option, in Tab order.
    options: gtk::Widget,
    preview: PreviewDrawer,
    focus_before_sidebar: RefCell<Option<gtk::Widget>>,
    tenxer: ChooserKeys,
}

/// One key press, with the focus and browser it was read against.
struct KeyContext<'a> {
    state: &'a Rc<ChooserState>,
    press: KeyPress,
    preferences: Rc<PreferenceManager>,
    browser: Rc<Browser>,
    focused: Option<gtk::Widget>,
    held: ModifierType,
    /// Left reached the first header action, so focus may leave the header.
    header_left_boundary: Cell<bool>,
}

impl ChooserShortcuts {
    fn new(parts: &ShortcutParts<'_>, tenxer: ChooserKeys) -> Self {
        Self {
            sidebar_state: parts.sidebar.state.clone(),
            sidebar_widget: parts.sidebar.widget.clone(),
            sidebar_toggle: parts.sidebar_toggle.clone(),
            header: parts.header.clone().upcast(),
            options: parts.options.clone().upcast(),
            preview: parts.preview.clone(),
            focus_before_sidebar: RefCell::new(None),
            tenxer,
        }
    }

    fn handle(&self, state: &Rc<ChooserState>, press: &KeyPress) -> glib::Propagation {
        if let Some(result) = modal_or_drag(state, press) {
            return result;
        }
        let mut context = KeyContext::new(state, *press);
        if let Some(result) = self.run(&context, PRESSED_STAGES) {
            return result;
        }
        // Unlike the window, the chooser types h j k l whenever type-to-search
        // is on, 10xer mode included.
        if context.preferences.type_to_search() {
            context.press = context.press.untyped();
        }
        self.run(&context, NAVIGATION_STAGES)
            .unwrap_or(glib::Propagation::Proceed)
    }

    fn run(&self, context: &KeyContext<'_>, stages: &[Stage]) -> Option<glib::Propagation> {
        stages.iter().find_map(|stage| stage(self, context))
    }

    fn sidebar_has_focus(&self, context: &KeyContext<'_>) -> bool {
        context.focus_within(&self.sidebar_widget)
    }

    /// Moves focus into the sidebar, remembering where to return.
    fn enter_sidebar(&self, context: &KeyContext<'_>) {
        self.focus_before_sidebar.replace(context.focused.clone());
        self.sidebar_state.focus_active_place();
    }
}

/// An open modal layer keeps the keys; Escape first ends a column drag.
fn modal_or_drag(state: &ChooserState, press: &KeyPress) -> Option<glib::Propagation> {
    if let Some(layer) = visible_modal_layer(&state.window) {
        let focused = gtk::prelude::RootExt::focus(&state.window);
        if !focused.is_some_and(|focus| focus == layer || focus.is_ancestor(&layer)) {
            layer.grab_focus();
        }
        return PROCEED;
    }
    (press.is(ESCAPE_ANY) && state.view.cancel_column_drag()).then_some(glib::Propagation::Stop)
}

impl<'a> KeyContext<'a> {
    fn new(state: &'a Rc<ChooserState>, press: KeyPress) -> Self {
        Self {
            state,
            press,
            preferences: PreferenceManager::shared(),
            browser: state.view.browser(),
            focused: gtk::prelude::RootExt::focus(&state.window),
            held: press.modifiers(),
            header_left_boundary: Cell::new(false),
        }
    }

    fn view(&self) -> &BrowserView {
        &self.state.view
    }

    fn tenxer(&self) -> bool {
        self.preferences.tenxer_mode()
    }

    fn alt(&self) -> bool {
        self.held.contains(ModifierType::ALT_MASK)
    }

    fn control(&self) -> bool {
        self.held.contains(ModifierType::CONTROL_MASK)
    }

    fn multiple(&self) -> bool {
        matches!(
            &self.state.request.kind,
            ChooserKind::Open { multiple: true, .. }
        )
    }

    fn tab(&self) -> Option<gtk::DirectionType> {
        plain_tab_direction(&self.press)
    }

    fn focus_in_popover(&self) -> bool {
        self.focused
            .as_ref()
            .is_some_and(crate::ui::focus_navigation::in_popover)
    }

    fn focus_editable(&self) -> bool {
        self.focused
            .as_ref()
            .is_some_and(crate::ui::focus_navigation::editable)
    }

    fn focus_in_text_or_popover(&self) -> bool {
        self.focused.as_ref().is_some_and(|widget| {
            crate::ui::focus_navigation::editable(widget)
                || crate::ui::focus_navigation::in_popover(widget)
        })
    }

    fn focus_within(&self, widget: &gtk::Widget) -> bool {
        self.focused
            .as_ref()
            .is_some_and(|focused| focused == widget || focused.is_ancestor(widget))
    }

    fn focused_popover(&self) -> Option<gtk::Popover> {
        self.focused
            .as_ref()
            .and_then(|widget| widget.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
    }

    fn name_has_focus(&self) -> bool {
        self.state.name_has_focus(self.focused.as_ref())
    }

    /// A new entry or rename is being typed in the item view.
    fn item_view_editing(&self) -> bool {
        self.view().new_entry_is_active() || self.view().rename_is_active()
    }

    fn item_view_idle(&self) -> bool {
        self.view().item_view_has_focus() && !self.item_view_editing()
    }

    fn arrow_scoped(&self) -> bool {
        self.preferences.arrow_navigation_scoped_active()
    }
}
