// SPDX-License-Identifier: MIT

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::prelude::*;

use crate::ui::keys::{
    self,
    bindings::{DOWN, OPEN_CONTAINING_FOLDER, UP},
};

/// The search palette's widgets, before the dialog state takes them over.
pub(super) struct SearchWidgets {
    pub(super) layer: gtk::Box,
    pub(super) panel: gtk::Box,
    pub(super) field: gtk::Entry,
    pub(super) indexing_spinner: gtk::Spinner,
    pub(super) list: gtk::ListBox,
    pub(super) positions: Rc<RefCell<HashMap<gtk::ListBoxRow, usize>>>,
    pub(super) scroller: gtk::ScrolledWindow,
    pub(super) results: gtk::Stack,
    pub(super) status: gtk::Label,
    pub(super) truncated_hint: gtk::Box,
}

impl SearchWidgets {
    pub(super) fn build() -> Self {
        let layer = search_layer();
        let panel = search_panel();
        let (search_bar, field, indexing_spinner) = search_bar();
        panel.append(&search_bar);
        let status = status_label();
        let (list, positions) = results_list();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.add_css_class("search-results-scroll");
        let results = results_stack(&status, &scroller);
        panel.append(&results);
        let (footer, truncated_hint) = search_footer();
        panel.append(&footer);
        crate::ui::modal::layout::install(&layer, &panel);
        Self {
            layer,
            panel,
            field,
            indexing_spinner,
            list,
            positions,
            scroller,
            results,
            status,
            truncated_hint,
        }
    }
}

fn search_layer() -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("search-backdrop");
    layer.add_css_class("app-modal-layer");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);
    layer
}

fn search_panel() -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("search-dialog");
    panel.set_halign(gtk::Align::Center);
    panel.set_valign(gtk::Align::Center);
    panel.set_size_request(760, 452);
    panel.set_vexpand(false);
    panel.set_overflow(gtk::Overflow::Hidden);
    panel
}

fn search_bar() -> (gtk::Box, gtk::Entry, gtk::Spinner) {
    let search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    search_bar.add_css_class("search-bar");
    search_bar.append(&crate::assets::primary_icon(
        crate::assets::icons::SEARCH,
        20,
    ));
    let field = gtk::Entry::builder()
        .placeholder_text(crate::i18n::tr("Search files and folders…"))
        .hexpand(true)
        .build();
    field.add_css_class("search-field");
    crate::ui::accessibility::set_label(&field, &crate::i18n::tr("Search files and folders…"));
    search_bar.append(&field);
    let indexing_spinner = gtk::Spinner::new();
    indexing_spinner.add_css_class("search-indexing-spinner");
    crate::ui::accessibility::set_description(
        &indexing_spinner,
        Some(&crate::i18n::tr("Indexing files…")),
    );
    indexing_spinner.set_valign(gtk::Align::Center);
    indexing_spinner.set_visible(false);
    search_bar.append(&indexing_spinner);
    (search_bar, field, indexing_spinner)
}

fn status_label() -> gtk::Label {
    let status = gtk::Label::new(Some(&crate::i18n::tr(
        "Type to search Home and mounted local drives",
    )));
    status.add_css_class("search-status");
    status.set_wrap(true);
    status.set_justify(gtk::Justification::Center);
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);
    status
}

fn results_list() -> (gtk::ListBox, Rc<RefCell<HashMap<gtk::ListBoxRow, usize>>>) {
    let list = gtk::ListBox::new();
    list.add_css_class("search-results");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    let positions = Rc::new(RefCell::new(HashMap::new()));
    let sorted_positions = positions.clone();
    list.set_sort_func(move |left, right| {
        let positions = sorted_positions.borrow();
        positions.get(left).cmp(&positions.get(right)).into()
    });
    (list, positions)
}

fn results_stack(status: &gtk::Label, scroller: &gtk::ScrolledWindow) -> gtk::Stack {
    let results = gtk::Stack::new();
    results.add_css_class("search-results-stack");
    results.set_size_request(-1, 360);
    results.add_named(status, Some("status"));
    results.add_named(scroller, Some("results"));
    results.set_visible_child_name("status");
    results
}

/// Key hints shorten in narrow windows so "Partial results" stays visible.
fn hint_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn followed_hint(message: &'static str, keys: keys::Keys) -> gtk::Label {
    let label = hint_label("");
    crate::ui::shortcut_reference::follow_keys(&label, move |label| {
        label.set_text(&crate::ui::shortcut_reference::with_keys(message, keys));
    });
    label
}

fn icon_hint(icon: &str, label: &gtk::Label) -> gtk::Box {
    let hint = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    hint.set_valign(gtk::Align::Center);
    hint.append(&crate::assets::primary_icon(icon, 13));
    hint.append(label);
    hint
}

fn search_footer() -> (gtk::Box, gtk::Box) {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    footer.add_css_class("search-footer");
    let navigation = followed_hint(
        "%{keys}  navigate",
        keys::Keys::Any(crate::ui::keys::shown![UP.alternate(0), DOWN.alternate(0)]),
    );
    let open = icon_hint(
        crate::assets::icons::CORNER_DOWN_LEFT,
        &hint_label(&crate::i18n::tr("open")),
    );
    navigation.add_css_class("search-hint");
    open.add_css_class("search-hint");
    footer.append(&navigation);
    footer.append(&open);
    let reveal_hint = icon_hint(
        crate::assets::icons::FOLDER_OPEN,
        &followed_hint(
            "%{keys}  open containing folder",
            keys::Keys::One(OPEN_CONTAINING_FOLDER.shown()),
        ),
    );
    reveal_hint.add_css_class("search-hint");
    footer.append(&reveal_hint);
    let truncated_hint = truncated_hint();
    footer.append(&truncated_hint);
    (footer, truncated_hint)
}

fn truncated_hint() -> gtk::Box {
    let truncated_hint = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    truncated_hint.append(&crate::assets::primary_icon(
        crate::assets::icons::TRIANGLE_ALERT,
        14,
    ));
    truncated_hint.append(&gtk::Label::new(Some(&crate::i18n::tr("Partial results"))));
    truncated_hint.add_css_class("search-hint");
    truncated_hint.add_css_class("search-hint-warning");
    truncated_hint.set_hexpand(true);
    truncated_hint.set_halign(gtk::Align::End);
    truncated_hint.set_visible(false);
    truncated_hint
}
