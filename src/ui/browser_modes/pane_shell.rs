// SPDX-License-Identifier: MIT

//! What a mode pane is built from, the shell around its listing, and the pane
//! surface that stands in for an empty or loading listing.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::prelude::*;

use super::{BrowserMode, ClickActivation, TransferHandlerSlot, source_index::{PanePositions, SourceIndexMap}};
use crate::{
    app::Browser,
    model::Location,
    ui::{
        browser::{ClipboardMarks, ViewState},
        inline_search::{InlineSearch, SearchSelectionHandlers},
        loading_skeleton, marquee,
    },
};

use super::pane::{Pane, PaneSection, STATUS_PAGE};

#[derive(Clone)]
pub(super) struct ModeClickOptions {
    pub(super) previews: Rc<Cell<bool>>,
    pub(super) activation: Rc<Cell<ClickActivation>>,
    pub(super) multiple_selection: Rc<Cell<bool>>,
}

/// What every mode pane is built from.
pub(super) struct PaneBuild {
    pub(super) browser: Rc<Browser>,
    pub(super) depth: usize,
    pub(super) title: String,
    pub(super) click: ModeClickOptions,
    pub(super) transfer: TransferHandlerSlot,
    pub(super) marks: Rc<RefCell<ClipboardMarks>>,
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) new_folder_state: Option<Weak<ViewState>>,
    pub(super) search_selection_handlers: SearchSelectionHandlers,
    pub(super) search_context_state: Rc<RefCell<Option<Weak<ViewState>>>>,
}

impl PaneBuild {
    pub(super) fn location(&self) -> Option<Location> {
        self.browser.location_at(self.depth)
    }

    pub(super) fn weak_browser(&self) -> Weak<Browser> {
        Rc::downgrade(&self.browser)
    }
}

/// The parts of a mode pane's shell that differ between Icons and List.
pub(super) struct PaneChrome<'a> {
    pub(super) depth: usize,
    pub(super) location: Option<Location>,
    pub(super) title: &'a str,
    pub(super) mode: BrowserMode,
    pub(super) class: &'static str,
    pub(super) loading: gtk::Box,
    pub(super) leading: gtk::Widget,
    pub(super) actions: gtk::Widget,
}

/// The shell widgets and directory model a mode pane is assembled around.
pub(super) struct PaneBase {
    pub(super) depth: usize,
    pub(super) location: Option<Location>,
    pub(super) shell: gtk::Box,
    pub(super) header: gtk::Box,
    pub(super) content: gtk::Box,
    pub(super) model: gtk::StringList,
    pub(super) source_index: SourceIndexMap,
    pub(super) stack: gtk::Stack,
    pub(super) status: gtk::Label,
    pub(super) administrator: gtk::Button,
    pub(super) spinner: gtk::Spinner,
    pub(super) truncated_hint: gtk::Image,
    pub(super) not_responding: gtk::Box,
}

impl PaneBase {
    /// Adds the chrome around the listing and wires the pane surface as a drop zone.
    pub(super) fn prepare_listing(&self, inputs: &PaneBuild, filter_revealer: &gtk::Revealer) {
        if self.location.as_ref().is_some_and(Location::is_administrator) {
            self.shell.insert_child_after(
                &crate::ui::browser::administrator_notice(),
                Some(&self.not_responding),
            );
        }
        if let Some(destination) = inputs.location() {
            super::drag_drop::install_mode_directory_drop_target(
                &self.stack,
                destination,
                inputs.transfer.clone(),
            );
        }
        self.content.append(filter_revealer);
    }
}

/// The listing a pane shows and the chrome actions tied to its contents.
pub(super) struct PaneListing {
    pub(super) section: PaneSection,
    pub(super) sections: Rc<RefCell<Vec<PaneSection>>>,
    pub(super) targets: marquee::MarqueeTargets,
    pub(super) marquee: marquee::Marquee,
    pub(super) search: InlineSearch,
    pub(super) leftovers_button: gtk::Button,
    pub(super) empty_trash_button: Option<gtk::Button>,
}

pub(super) fn pane_base(chrome: PaneChrome) -> PaneBase {
    let shell = crate::ui::accessibility::pane_box();
    shell.add_css_class(chrome.class);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    crate::ui::accessibility::describe_pane(&shell, chrome.title, chrome.mode);
    let (header, spinner, truncated_hint) = pane_header(&chrome);
    shell.append(&header);
    let not_responding = crate::ui::browser::not_responding_notice();
    shell.append(&not_responding);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let (status_page, status, administrator) = pane_status_page();
    // Focusable only off the content page (see `DirectorySurface`); a group role keeps its name.
    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    stack.add_named(&content, Some(loading_skeleton::CONTENT_PAGE));
    stack.add_named(&chrome.loading, Some(loading_skeleton::LOADING_PAGE));
    stack.add_named(&status_page, Some(STATUS_PAGE));
    shell.append(&stack);

    let model = gtk::StringList::new(&[]);
    let source_index = SourceIndexMap::watch(&model);
    PaneBase {
        depth: chrome.depth,
        location: chrome.location,
        shell,
        header,
        content,
        model,
        source_index,
        stack,
        status,
        administrator,
        spinner,
        truncated_hint,
        not_responding,
    }
}

fn pane_header(chrome: &PaneChrome) -> (gtk::Box, gtk::Spinner, gtk::Image) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("mode-pane-header");
    crate::ui::tenxer_mode::hide_while_enabled(&header);
    let heading_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading_box.set_hexpand(true);
    heading_box.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(chrome.title));
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    heading.set_max_width_chars(1);
    crate::ui::accessibility::set_description(&heading, Some(chrome.title));
    let spinner = gtk::Spinner::new();
    spinner.set_valign(gtk::Align::Center);
    spinner.start();
    let truncated_hint = crate::assets::primary_icon(crate::assets::icons::TRIANGLE_ALERT, 16);
    crate::ui::accessibility::set_description(
        &truncated_hint,
        Some(&crate::i18n::tr(
            "This directory has more entries than could be loaded; showing a partial listing.",
        )),
    );
    truncated_hint.set_visible(false);
    heading_box.append(&heading);
    heading_box.append(&truncated_hint);
    chrome.leading.set_valign(gtk::Align::Center);
    header.append(&chrome.leading);
    header.append(&heading_box);
    header.append(&spinner);
    chrome.actions.set_valign(gtk::Align::Center);
    header.append(&chrome.actions);
    (header, spinner, truncated_hint)
}

fn pane_status_page() -> (gtk::Box, gtk::Label, gtk::Button) {
    let status = gtk::Label::new(Some(&crate::i18n::tr("This directory is empty")));
    status.add_css_class("status-message");
    status.set_wrap(true);
    let administrator = gtk::Button::with_label(&crate::i18n::tr("Open as Administrator"));
    administrator.add_css_class("retry-button");
    administrator.set_halign(gtk::Align::Center);
    administrator.set_visible(false);
    let status_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    status_page.set_valign(gtk::Align::Center);
    status_page.append(&status);
    status_page.append(&administrator);
    (status_page, status, administrator)
}

pub(super) fn pane_directory_name(browser: &Rc<Browser>, depth: usize) -> String {
    browser
        .location_at(depth)
        .map(|location| location.display_name())
        .unwrap_or_default()
}

pub(super) fn install_pane_surface(pane: &Pane, browser: &Rc<Browser>, directory: &str) {
    let depth = pane.depth;
    let weak = Rc::downgrade(browser);
    pane.administrator.connect_clicked(move |_| {
        if let Some(browser) = weak.upgrade()
            && let Some(location) = browser
                .column_snapshot(depth)
                .and_then(|snapshot| snapshot.administrator)
        {
            browser.navigate_location(location, true);
        }
    });
    loading_skeleton::DirectorySurface::install(
        &pane.stack,
        &pane.status,
        &pane.section.view,
        pane.search.result_collection_view().as_ref(),
        browser,
        directory,
    );
}

/// Focuses the pane's collection view, or the pane itself while it shows a status or
/// loading page.
pub(super) fn focus_pane_surface(pane: &Pane) -> bool {
    loading_skeleton::focus_surface_or(&pane.stack, &pane.focus_view())
}

pub(super) fn install_tab_landing(view: &gtk::Widget, state: Option<Weak<ViewState>>) {
    let Some(state) = state else {
        return;
    };
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter(move |_| {
        if let Some(state) = state.upgrade() {
            state.land_tab_crossing();
        }
    });
    view.add_controller(focus);
}

/// Opens the entry behind an activated view position in place.
pub(super) struct CollectionActivation {
    pub(super) browser: Weak<Browser>,
    pub(super) state: Option<Weak<ViewState>>,
    pub(super) positions: PanePositions,
    pub(super) depth: usize,
}

impl CollectionActivation {
    pub(super) fn new(inputs: &PaneBuild, positions: PanePositions) -> Self {
        Self {
            browser: inputs.weak_browser(),
            state: inputs.state.clone(),
            positions,
            depth: inputs.depth,
        }
    }

    pub(super) fn activate(&self, position: u32) {
        if let Some(state) = self.state.as_ref().and_then(Weak::upgrade) {
            state.cancel_click_rename();
        }
        if let Some(browser) = self.browser.upgrade()
            && let Some(position) = self.positions.source_position(position)
        {
            browser.activate_in_place(self.depth, position);
        }
    }
}

