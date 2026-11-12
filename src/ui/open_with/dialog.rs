// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::{
    Choices, launch, launch_with_portal,
    navigation::{install_list_tab_navigation, install_list_typing, install_search_arrows},
    uses_portal,
};
use crate::ui::{
    blur::BlurBin,
    browser::{dismiss_modal_layer, modal_layer, show_error_dialog},
    controls::{ModalLayout, modal_layout},
    keys::{self, bindings::ESCAPE_ANY},
};

struct AppEntry {
    app: gio::AppInfo,
    row: gtk::ListBoxRow,
    section: AppSection,
    haystack: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppSection {
    Recent,
    Recommended,
    Other,
}

impl AppSection {
    fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recently Used",
            Self::Recommended => "Recommended Applications",
            Self::Other => "Other Applications",
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::ui) enum OpenWithContext {
    Explicit,
    ActivationFallback,
}

impl OpenWithContext {
    fn empty_message(self) -> &'static str {
        match self {
            Self::Explicit => "No compatible applications were found.",
            Self::ActivationFallback => "No application is registered for this file",
        }
    }
}

/// The section headings, shown only while a match is listed under them.
struct SectionHeadings {
    recent: Option<gtk::ListBoxRow>,
    recommended: Option<gtk::ListBoxRow>,
    other: Option<gtk::ListBoxRow>,
}

/// The widgets a search updates.
struct FilterTargets {
    list: glib::WeakRef<gtk::ListBox>,
    list_scroll: glib::WeakRef<gtk::ScrolledWindow>,
    empty_search: glib::WeakRef<gtk::Label>,
    confirm: glib::WeakRef<gtk::Button>,
}

/// What the Open button launches.
struct OpenRequest {
    files: Vec<gio::File>,
    content_types: Vec<String>,
    entries: Rc<Vec<AppEntry>>,
    parent: glib::WeakRef<gtk::Widget>,
    list: glib::WeakRef<gtk::ListBox>,
    always_use: gtk::CheckButton,
}

fn create_section_header(title: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-heading-row");
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_focusable(false);
    row.set_can_focus(false);

    let label = gtk::Label::new(Some(&crate::i18n::tr(title)));
    label.add_css_class("open-with-heading");
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}

fn application_icon(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::Image {
    let icon = app.icon().and_then(|icon| {
        if let Some(file_icon) = icon.downcast_ref::<gio::FileIcon>() {
            return gtk::gdk::Texture::from_file(&file_icon.file())
                .ok()
                .map(|texture| gtk::Image::from_paintable(Some(&texture)));
        }
        gtk::IconTheme::for_display(display)
            .has_gicon(&icon)
            .then(|| gtk::Image::from_gicon(&icon))
    });
    icon.unwrap_or_else(|| crate::assets::primary_icon(crate::assets::icons::FILE_CODE, 24))
}

fn create_app_row(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-row");
    crate::ui::accessibility::set_label(&row, &app.display_name());
    if let Some(description) = app.description() {
        crate::ui::accessibility::set_description(&row, Some(&description));
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = application_icon(app, display);
    icon.set_pixel_size(22);
    icon.add_css_class("open-with-icon");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
    labels.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&app.display_name()));
    name.add_css_class("open-with-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let description_text = app.description().map(|value| value.to_string());
    let description = gtk::Label::new(description_text.as_deref());
    description.add_css_class("open-with-description");
    description.set_xalign(0.0);
    description.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&name);
    if description_text
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != app.display_name())
    {
        labels.append(&description);
    }
    content.append(&icon);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

fn app_haystack(app: &gio::AppInfo) -> String {
    format!(
        "{} {} {} {} {}",
        app.display_name(),
        app.name(),
        app.description().unwrap_or_default(),
        app.executable().to_string_lossy(),
        app.id().unwrap_or_default(),
    )
    .to_lowercase()
}

/// Appends a heading and one row per app, returning the heading when shown.
fn append_section(
    list: &gtk::ListBox,
    entries: &mut Vec<AppEntry>,
    apps: &[gio::AppInfo],
    section: AppSection,
) -> Option<gtk::ListBoxRow> {
    if apps.is_empty() {
        return None;
    }
    let heading = create_section_header(section.title());
    list.append(&heading);
    for app in apps {
        let row = create_app_row(app, &list.display());
        let haystack = app_haystack(app);
        list.append(&row);
        entries.push(AppEntry {
            app: app.clone(),
            row,
            section,
            haystack,
        });
    }
    Some(heading)
}

fn populate_list(list: &gtk::ListBox, choices: Choices) -> (Vec<AppEntry>, SectionHeadings) {
    let mut entries = Vec::new();
    let Choices {
        recent,
        recommended,
        other,
    } = choices;
    let headings = SectionHeadings {
        recent: append_section(list, &mut entries, &recent, AppSection::Recent),
        recommended: append_section(list, &mut entries, &recommended, AppSection::Recommended),
        other: append_section(list, &mut entries, &other, AppSection::Other),
    };
    if let Some(first_entry) = entries.first() {
        list.select_row(Some(&first_entry.row));
    }
    (entries, headings)
}

fn dialog_layout(file_count: usize) -> ModalLayout {
    let subtitle = crate::i18n::tr(if file_count == 1 {
        "Choose an application to open this item"
    } else {
        "Choose an application to open these items"
    });
    let layout = modal_layout(
        crate::assets::icons::EXTERNAL_LINK,
        &crate::i18n::tr("Open With"),
        &subtitle,
        &crate::i18n::tr("Open"),
    );
    layout.content.add_css_class("open-with-dialog");
    layout.content.set_size_request(480, 460);
    layout.body.set_vexpand(true);
    layout
}

fn search_field() -> gtk::SearchEntry {
    let search_entry = gtk::SearchEntry::new();
    search_entry.add_css_class("open-with-search");
    search_entry.set_placeholder_text(Some(&crate::i18n::tr("Search applications…")));
    crate::ui::accessibility::set_label(&search_entry, &crate::i18n::tr("Search applications…"));
    search_entry
}

fn app_list() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("open-with-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    crate::ui::accessibility::set_label(&list, &crate::i18n::tr("Applications"));
    list
}

fn always_use_check(type_count: usize) -> gtk::CheckButton {
    let always_use = crate::ui::controls::form_check_button(&crate::i18n::tr(if type_count > 1 {
        "Always use for these file types"
    } else {
        "Always use for this file type"
    }));
    always_use.set_visible(false);
    always_use
}

fn list_scroll(list: &gtk::ListBox) -> gtk::ScrolledWindow {
    let list_scroll = gtk::ScrolledWindow::new();
    list_scroll.add_css_class("open-with-scroll");
    list_scroll.set_child(Some(list));
    list_scroll.set_propagate_natural_height(false);
    list_scroll.set_vexpand(true);
    list_scroll.set_hexpand(true);
    list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    list_scroll
}

fn empty_label(message: &str) -> gtk::Label {
    let empty = gtk::Label::new(Some(&crate::i18n::tr(message)));
    empty.add_css_class("open-with-empty");
    empty.set_wrap(true);
    empty.set_xalign(0.5);
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_vexpand(true);
    empty
}

fn show_no_applications(
    layout: &ModalLayout,
    search_entry: &gtk::SearchEntry,
    list_scroll: &gtk::ScrolledWindow,
    context: OpenWithContext,
) {
    search_entry.set_visible(false);
    list_scroll.set_visible(false);
    layout.body.append(&empty_label(context.empty_message()));
    layout.confirm.set_sensitive(false);
}

pub(in crate::ui) fn show(
    parent: &impl IsA<gtk::Widget>,
    files: Vec<gio::File>,
    content_types: Vec<String>,
    choices: Choices,
    context: OpenWithContext,
    on_close: Rc<dyn Fn()>,
) {
    if uses_portal() {
        launch_with_portal(parent, files, matches!(context, OpenWithContext::Explicit));
        on_close();
        return;
    }
    let Some(window_overlay) = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let layout = dialog_layout(files.len());
    let search_entry = search_field();
    layout.body.append(&search_entry);
    let list = app_list();
    let always_use = always_use_check(content_types.len());
    layout.actions.prepend(&always_use);
    install_list_tab_navigation(&layout, &search_entry, &list, &always_use);

    let (entries, headings) = populate_list(&list, choices);
    let list_scroll = list_scroll(&list);
    layout.body.append(&list_scroll);
    let empty_search = empty_label("No matching applications were found.");
    empty_search.set_visible(false);
    layout.body.append(&empty_search);

    let has_apps = !entries.is_empty();
    always_use.set_visible(has_apps && !content_types.is_empty());
    if !has_apps {
        show_no_applications(&layout, &search_entry, &list_scroll, context);
    }
    install_search_arrows(&search_entry, &list, &list_scroll);
    install_list_typing(&list, &search_entry);
    confirm_on_search_activate(&search_entry, &layout.confirm);

    let entries = Rc::new(entries);
    let targets = FilterTargets {
        list: list.downgrade(),
        list_scroll: list_scroll.downgrade(),
        empty_search: empty_search.downgrade(),
        confirm: layout.confirm.downgrade(),
    };
    install_filter(&search_entry, entries.clone(), targets, headings);

    let layer = modal_layer(&layout.content, &window_overlay, blurred_root.clone(), None);
    layer.connect_unrealize(move |_| {
        let on_close = on_close.clone();
        glib::idle_add_local_once(move || on_close());
    });
    window_overlay.add_overlay(&layer);
    let dismiss = dismiss_action(&layer, &window_overlay, blurred_root.as_ref());
    let cancel_dismiss = dismiss.clone();
    layout.cancel.connect_clicked(move |_| cancel_dismiss());
    let close_dismiss = dismiss.clone();
    layout.close.connect_clicked(move |_| close_dismiss());

    let request = OpenRequest {
        files,
        content_types,
        entries,
        parent: parent.as_ref().downgrade(),
        list: list.downgrade(),
        always_use,
    };
    connect_open(&layout.confirm, request, dismiss.clone());
    let activate_confirm = layout.confirm.downgrade();
    list.connect_row_activated(move |_, _| {
        if let Some(confirm) = activate_confirm.upgrade() {
            confirm.emit_clicked();
        }
    });
    install_escape(&layer, &search_entry, dismiss);
    if !has_apps {
        layout.cancel.grab_focus();
    } else {
        search_entry.grab_focus();
    }
}

fn confirm_on_search_activate(search_entry: &gtk::SearchEntry, confirm: &gtk::Button) {
    let confirm_for_search = confirm.downgrade();
    search_entry.connect_activate(move |_| {
        if let Some(confirm) = confirm_for_search.upgrade()
            && confirm.is_sensitive()
        {
            confirm.emit_clicked();
        }
    });
}

fn install_filter(
    search_entry: &gtk::SearchEntry,
    entries: Rc<Vec<AppEntry>>,
    targets: FilterTargets,
    headings: SectionHeadings,
) {
    search_entry.connect_changed(move |search| {
        let (Some(list), Some(list_scroll), Some(empty_label), Some(confirm)) = (
            targets.list.upgrade(),
            targets.list_scroll.upgrade(),
            targets.empty_search.upgrade(),
            targets.confirm.upgrade(),
        ) else {
            return;
        };
        let query = search.text().trim().to_lowercase();
        let current_selected = list.selected_row();
        let filtered = filter_entries(&entries, &query, current_selected.as_ref());
        headings.show_for(&filtered);
        let has_matches = filtered.total() > 0;
        empty_label.set_visible(!has_matches);
        list_scroll.set_visible(has_matches);
        confirm.set_sensitive(has_matches);
        if !has_matches {
            list.select_row(None::<&gtk::ListBoxRow>);
        } else if !filtered.selected_is_visible
            && let Some(row) = &filtered.first_visible_row
        {
            list.select_row(Some(row));
        }
    });
}

/// Matches per section after a search, and where the selection can go.
#[derive(Default)]
struct FilterResult {
    recent: usize,
    recommended: usize,
    other: usize,
    first_visible_row: Option<gtk::ListBoxRow>,
    selected_is_visible: bool,
}

impl FilterResult {
    fn total(&self) -> usize {
        self.recent + self.recommended + self.other
    }

    fn count(&mut self, entry: &AppEntry, selected: Option<&gtk::ListBoxRow>) {
        match entry.section {
            AppSection::Recent => self.recent += 1,
            AppSection::Recommended => self.recommended += 1,
            AppSection::Other => self.other += 1,
        }
        if self.first_visible_row.is_none() {
            self.first_visible_row = Some(entry.row.clone());
        }
        if selected == Some(&entry.row) {
            self.selected_is_visible = true;
        }
    }
}

fn filter_entries(
    entries: &[AppEntry],
    query: &str,
    selected: Option<&gtk::ListBoxRow>,
) -> FilterResult {
    let mut result = FilterResult::default();
    for entry in entries {
        let matches = query.is_empty() || entry.haystack.contains(query);
        entry.row.set_visible(matches);
        if matches {
            result.count(entry, selected);
        }
    }
    result
}

impl SectionHeadings {
    fn show_for(&self, filtered: &FilterResult) {
        for (heading, count) in [
            (&self.recent, filtered.recent),
            (&self.recommended, filtered.recommended),
            (&self.other, filtered.other),
        ] {
            if let Some(heading) = heading {
                heading.set_visible(count > 0);
            }
        }
    }
}

fn dismiss_action(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    blurred_root: Option<&BlurBin>,
) -> Rc<dyn Fn()> {
    let dismissed = Rc::new(Cell::new(false));
    let dismiss_layer = layer.downgrade();
    let dismiss_overlay = overlay.downgrade();
    let dismiss_root = blurred_root.map(|root| root.downgrade());
    Rc::new(move || {
        if dismissed.replace(true) {
            return;
        }
        let (Some(layer), Some(overlay)) = (dismiss_layer.upgrade(), dismiss_overlay.upgrade())
        else {
            return;
        };
        let root = dismiss_root.as_ref().and_then(|root| root.upgrade());
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    })
}

fn connect_open(confirm: &gtk::Button, request: OpenRequest, dismiss: Rc<dyn Fn()>) {
    confirm.connect_clicked(move |_| {
        let Some(list) = request.list.upgrade() else {
            return;
        };
        let Some(row) = list.selected_row() else {
            return;
        };
        let Some(entry) = request.entries.iter().find(|e| e.row == row) else {
            return;
        };
        if request.always_use.is_active() {
            set_as_default(&entry.app, &request.content_types);
        }
        let context = crate::ui::launch_context::for_display(&list.display());
        let launched = launch(
            &entry.app,
            &request.files,
            &request.content_types,
            Some(&context),
        );
        dismiss();
        if let Err(error) = launched {
            report_launch_error(&request.parent, error.to_string());
        }
    });
}

fn set_as_default(app: &gio::AppInfo, content_types: &[String]) {
    for content_type in content_types {
        if let Err(error) = app.set_as_default_for_type(content_type) {
            tracing::warn!(%content_type, %error, "unable to set default application");
        }
    }
}

fn report_launch_error(parent: &glib::WeakRef<gtk::Widget>, detail: String) {
    let open_parent = parent.clone();
    glib::timeout_add_local_once(Duration::from_millis(250), move || {
        if let Some(parent) = open_parent.upgrade() {
            show_error_dialog(&parent, &crate::i18n::tr("Unable to open file"), &detail);
        }
    });
}

fn install_escape(layer: &gtk::Box, search_entry: &gtk::SearchEntry, dismiss: Rc<dyn Fn()>) {
    let escape_search = search_entry.downgrade();
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys::on_press(&escape, move |_, press| {
        if !press.is(ESCAPE_ANY) {
            return glib::Propagation::Proceed;
        }
        if let Some(search) = escape_search.upgrade()
            && !search.text().is_empty()
        {
            search.set_text("");
            search.grab_focus();
            return glib::Propagation::Stop;
        }
        dismiss();
        glib::Propagation::Stop
    });
    layer.add_controller(escape);
}
