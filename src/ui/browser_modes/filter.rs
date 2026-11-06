// SPDX-License-Identifier: MIT

//! The per-pane filter field: narrowing the listing in place, the results that
//! replace it, and the entry's keyboard and pointer activation rules.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    pane::{Pane, PaneBuild, PaneSection},
    search_options::search_collection_options,
    source_index::{PanePositions, SourceIndexMap},
};
use crate::{
    app::Browser,
    ui::{
        browser::{self, FilterFocus, ViewState, paths::is_trash_location},
        enter_target::EnterTarget,
        inline_search::{self, InlineSearch, SearchPresentation},
    },
};

/// `tooltip` names the filter shortcut with `%{keys}`.
pub(crate) fn filter_controls(
    tooltip: &'static str,
) -> (gtk::Entry, gtk::Revealer, gtk::ToggleButton) {
    let entry = gtk::Entry::builder()
        .placeholder_text(browser::filter_placeholder(0))
        .has_frame(false)
        .hexpand(true)
        .build();
    entry.add_css_class("column-filter-entry");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.add_css_class("column-filter");
    row.append(&crate::assets::chrome_icon(crate::assets::icons::FUNNEL));
    row.append(&entry);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&row)
        .build();
    let button = gtk::ToggleButton::new();
    let labelled_entry = entry.clone();
    // 10xer hides the button, so it names the default map's key.
    crate::ui::shortcut_reference::follow_keys(&button, move |button| {
        let keys = crate::ui::keys::label_in_map(
            &rust_i18n::locale(),
            &crate::ui::keys::Keys::One(crate::ui::keys::bindings::FILTER.shown()),
            crate::ui::keys::Map::Default,
        );
        let label = rust_i18n::t!(tooltip, keys = keys);
        button.set_tooltip_text(Some(&label));
        crate::ui::accessibility::set_label(button, &label);
        crate::ui::accessibility::set_label(&labelled_entry, &label);
    });
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::FUNNEL,
    )));
    crate::ui::controls::pane_header_action(&button);
    let shown_filter = revealer.clone();
    let focused_filter = entry.clone();
    button.connect_toggled(move |button| {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
        shown_filter.set_reveal_child(button.is_active());
        if button.is_active() {
            focused_filter.grab_focus();
        } else {
            focused_filter.set_text("");
        }
    });
    crate::ui::tenxer_mode::hide_filter_while_enabled(&button, &revealer);
    (entry, revealer, button)
}

/// A pane's filter field bound to the model that narrows its listing.
pub(super) struct PaneFilter {
    pub(super) entry: gtk::Entry,
    pub(super) button: gtk::ToggleButton,
    pub(super) query: Rc<RefCell<String>>,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) model: gtk::FilterListModel,
    /// Recursive-filter results replace a native listing; hiding its rows too
    /// would drop their selection out of sight.
    results_replace_listing: Rc<Cell<bool>>,
    pub(super) enter_target: Option<EnterTarget>,
}

impl PaneFilter {
    pub(super) fn bind(
        entry: gtk::Entry,
        button: gtk::ToggleButton,
        inputs: &PaneBuild,
        model: &gtk::StringList,
    ) -> Self {
        let query = Rc::new(RefCell::new(String::new()));
        let browser = &inputs.browser;
        let initial_show_hidden = browser
            .column_preferences(inputs.depth)
            .map_or_else(|| browser.preferences().show_hidden, |p| p.show_hidden);
        let show_hidden = Rc::new(Cell::new(initial_show_hidden));
        let filter = browser::entry_filter(show_hidden.clone(), query.clone());
        let filtered = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
        let results_replace_listing = Rc::new(Cell::new(false));
        let enter_target = in_place_enter_target(&entry, browser, inputs.depth);
        if let Some(state) = &inputs.state {
            browser::bind_filter_descent(&entry, state.clone());
        }
        browser::bind_listing_filter(
            &entry,
            &filter,
            &query,
            results_replace_listing.clone(),
            inputs.state.clone(),
            enter_target.clone(),
        );
        Self {
            entry,
            button,
            query,
            show_hidden,
            filter,
            model: filtered,
            results_replace_listing,
            enter_target,
        }
    }

    /// Enter opens the top result or the selection; Down lands on the top row.
    pub(super) fn connect_entry(
        &self,
        section: &PaneSection,
        inputs: &PaneBuild,
        source_index: &SourceIndexMap,
    ) {
        let activation = FilterActivation {
            browser: inputs.weak_browser(),
            depth: inputs.depth,
            positions: PanePositions {
                index: source_index.clone(),
                view: section.view_model.clone(),
            },
            selection: section.selection.clone(),
            query: self.query.clone(),
            enter_target: self.enter_target.clone(),
        };
        self.entry.connect_activate(move |_| activation.activate());
        if let Some(enter_target) = &self.enter_target {
            connect_in_place_down(
                &self.entry,
                enter_target,
                &section.view,
                inputs.state.clone(),
            );
        }
    }

    /// Wraps the listing in the search that shows recursive results in its place.
    pub(super) fn wrap_search(
        &self,
        listing: &impl IsA<gtk::Widget>,
        inputs: &PaneBuild,
        presentation: SearchPresentation,
    ) -> InlineSearch {
        let browser_for_search = inputs.browser.clone();
        let depth = inputs.depth;
        let search = inline_search::wrap(
            listing,
            &self.entry,
            move || {
                browser_for_search
                    .location_at(depth)
                    .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
            },
            &inputs.browser,
            search_collection_options(presentation, inputs),
        );
        self.results_replace_listing.set(search.replaces_listing());
        search
    }
}

/// Enter in the filter field opens what it would open from the keyboard.
struct FilterActivation {
    browser: Weak<Browser>,
    depth: usize,
    positions: PanePositions,
    selection: gtk::MultiSelection,
    query: Rc<RefCell<String>>,
    enter_target: Option<EnterTarget>,
}

impl FilterActivation {
    fn activate(&self) {
        if self.query.borrow().trim().is_empty() {
            return;
        }
        activate_filtered_item(
            &self.browser,
            self.depth,
            &self.positions,
            filter_activation_position(self.enter_target.as_ref(), &self.selection),
        );
    }
}

/// The in-place filter's target, for a pane whose filter narrows the listing itself
/// rather than showing search results in its place. Its selection is the listing's
/// own: a new query only stops that selection from counting as the user's pick.
fn in_place_enter_target(
    entry: &gtk::Entry,
    browser: &Browser,
    depth: usize,
) -> Option<EnterTarget> {
    let searches = browser
        .location_at(depth)
        .is_some_and(|location| location.native_path().is_some());
    (!searches).then(|| EnterTarget::new(entry))
}

pub(super) fn attach_in_place(
    enter_target: &EnterTarget,
    section: &PaneSection,
    scroll: &gtk::ScrolledWindow,
) {
    let syncing = section.syncing.clone();
    enter_target.attach(
        &section.view,
        &section.selection,
        scroll,
        section.visit.clone(),
        move || syncing.get(),
    );
}

/// Down from the field lands on the top row Enter would open, not the listing's cursor.
fn connect_in_place_down(
    entry: &gtk::Entry,
    enter_target: &EnterTarget,
    view: &gtk::Widget,
    state: Option<Weak<ViewState>>,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let enter_target = enter_target.clone();
    let view = view.downgrade();
    crate::ui::keys::on_press(&keys, move |_, press| {
        let Some(view) = view
            .upgrade()
            .filter(|_| press.is(crate::ui::keys::bindings::DOWN) && enter_target.offers_top())
        else {
            return glib::Propagation::Proceed;
        };
        if let Some(state) = state.as_ref().and_then(Weak::upgrade) {
            browser::claim_keyboard_navigation(&state);
        } else if let Some(window) = view.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        view.grab_focus();
        browser::scroll_results_to(
            &view,
            0,
            gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
        );
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

fn filter_activation_position(
    enter_target: Option<&EnterTarget>,
    selection: &gtk::MultiSelection,
) -> Option<u32> {
    if enter_target.is_some_and(EnterTarget::offers_top) {
        return Some(0);
    }
    (0..selection.n_items()).find(|position| selection.is_selected(*position))
}

fn activate_filtered_item(
    browser: &Weak<Browser>,
    depth: usize,
    positions: &PanePositions,
    position: Option<u32>,
) {
    let Some(source_position) = position.and_then(|position| positions.source_position(position))
    else {
        return;
    };
    let Some(browser) = browser.upgrade() else {
        return;
    };
    let Some(entry) = browser.entry_at(depth, source_position) else {
        return;
    };
    open_filtered_entry(&browser, depth, source_position, entry);
}

fn open_filtered_entry(
    browser: &Rc<Browser>,
    depth: usize,
    source_position: usize,
    entry: crate::model::FileEntry,
) {
    let opens_file = browser.is_chooser_mode() && !entry.is_directory();
    // Trashed files do not open; the top-result fallback must not move the selection.
    let trashed_file = is_trash_location(&entry.location) && !entry.is_directory();
    if !opens_file && trashed_file {
        return;
    }
    browser.select(depth, source_position);
    if opens_file {
        browser.open_location(entry.location);
    } else {
        browser.activate_in_place(depth, source_position);
    }
}

/// Only a revealed funnel counts: a 10xer footer filter leaves it closed and has its own
/// Escape order. Away from native folders the filter narrows the listing in place, so its
/// narrowed rows are the results.
pub(super) fn pane_filter_focus(pane: &Pane, focused: Option<&gtk::Widget>) -> Option<FilterFocus> {
    if pane
        .filter_entry
        .as_ref()
        .is_some_and(|entry| super::focus::widget_has_focus(entry, focused))
    {
        return Some(FilterFocus::Entry);
    }
    let revealed = pane
        .filter_button
        .as_ref()
        .is_some_and(gtk::ToggleButton::is_active);
    let results = if pane.search.replaces_listing() {
        pane.search.results_view().is_some() && pane.search.has_item_focus(focused)
    } else {
        !pane.filter_query.borrow().is_empty()
            && pane
                .item_sections()
                .iter()
                .any(|section| super::focus::widget_has_focus(&section.view, focused))
    };
    (revealed && results).then_some(FilterFocus::Results)
}
