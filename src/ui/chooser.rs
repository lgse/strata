// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ashpd::{
    PortalError, WindowIdentifierType,
    desktop::file_chooser::{Choice, FileFilter, SelectedFiles},
};
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{LocalFileSource, LocalOperationProvider, LocalPreviewProvider},
    app::BrowserEvent,
    model::{FileEntry, Location},
    portal::{
        ChooserKind, ChooserRequest, check_destinations, local_uri, open_selection, safe_filename,
        writable_from_read_only,
    },
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataRequest,
    },
};

use super::{
    blur::BlurBin,
    browser::{BrowserView, dismiss_modal_layer, modal_layer},
    browser_modes::BrowserMode,
    controls::{
        ModalTone, form_check_button, form_entry, form_label, menu_option, message_dialog_layout,
    },
    preview::{PreviewDrawer, preview_target},
    theme::ThemeManager,
    window::{
        MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, SidebarView, build_appearance_menu, build_sidebar,
        home_directory, install_modal_focus_trap, is_sidebar_focus_shortcut, vim_focus_direction,
        visible_modal_layer,
    },
};

type Completion = Box<dyn FnOnce(ashpd::backend::Result<SelectedFiles>)>;

thread_local! {
    static CHOOSERS: RefCell<HashMap<String, glib::WeakRef<gtk::Window>>> = RefCell::new(HashMap::new());
}

struct ChooserFileSource {
    filter: Rc<RefCell<Option<gtk::FileFilter>>>,
}

impl ChooserFileSource {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            filter: Rc::new(RefCell::new(None)),
        })
    }

    fn set_filter(&self, filter: Option<gtk::FileFilter>) {
        self.filter.replace(filter);
    }
}

impl FileSource for ChooserFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location.native_path().is_none() {
            return Err(LocationValidationError::UnsupportedScheme(
                "The system file chooser supports local files and folders only.".into(),
            ));
        }
        LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if location.native_path().is_none() {
            emit(self.validate_location(&location));
            return LoadHandle::new(|| {});
        }
        LocalFileSource.validate_location_async(location, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        location.native_path().is_some() && LocalFileSource.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LocalFileSource.fill_metadata(request, emit)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let filter = self.filter.clone();
        LocalFileSource.enumerate(
            request,
            Rc::new(move |event| {
                let event = match event {
                    DirectoryEvent::Batch {
                        request_id,
                        mut entries,
                    } => {
                        if let Some(filter) = filter.borrow().as_ref() {
                            entries.retain(|entry| file_filter_matches(filter, entry));
                        }
                        DirectoryEvent::Batch {
                            request_id,
                            entries,
                        }
                    }
                    event => event,
                };
                emit(event);
            }),
        )
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        let filter = self.filter.clone();
        LocalFileSource.watch(
            location,
            include_hidden,
            Rc::new(move |change| {
                notify(filter_directory_change(filter.borrow().as_ref(), change));
            }),
        )
    }
}

fn file_filter_matches(filter: &gtk::FileFilter, entry: &FileEntry) -> bool {
    if entry.is_directory() {
        return true;
    }
    let info = gio::FileInfo::new();
    info.set_name(Path::new(&entry.native_name));
    info.set_display_name(&entry.display_name);
    info.set_file_type(gio::FileType::Regular);
    let (content_type, _) =
        gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
    info.set_content_type(&content_type);
    filter.match_(&info)
}

fn filter_directory_change(
    filter: Option<&gtk::FileFilter>,
    change: DirectoryChange,
) -> DirectoryChange {
    match change {
        DirectoryChange::Upsert(entry)
            if filter.is_some_and(|filter| !file_filter_matches(filter, &entry)) =>
        {
            DirectoryChange::Remove(entry.location)
        }
        DirectoryChange::Move { from, entry }
            if filter.is_some_and(|filter| !file_filter_matches(filter, &entry)) =>
        {
            DirectoryChange::Remove(from)
        }
        change => change,
    }
}

#[derive(Clone)]
struct PortalFilter {
    portal: FileFilter,
    native: gtk::FileFilter,
}

enum ChoiceControl {
    Boolean {
        id: String,
        check: gtk::CheckButton,
    },
    Select {
        id: String,
        values: Vec<String>,
        dropdown: ChooserDropdown,
    },
}

type SelectionChanged = Box<dyn Fn(usize)>;

struct ChooserDropdown {
    button: gtk::MenuButton,
    popover: gtk::Popover,
    selected: Rc<Cell<usize>>,
    changed: Rc<RefCell<Option<SelectionChanged>>>,
}

impl ChooserDropdown {
    fn new(labels: &[&str], selected: usize) -> Self {
        let selected = selected.min(labels.len().saturating_sub(1));
        let current = labels.get(selected).copied().unwrap_or_default();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.add_css_class("column-menu");
        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("column-popover");
        let current_label = gtk::Label::new(Some(current));
        current_label.set_xalign(0.0);
        current_label.set_hexpand(true);
        current_label.set_max_width_chars(24);
        current_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let button = gtk::MenuButton::builder()
            .child(&current_label)
            .always_show_arrow(true)
            .popover(&popover)
            .build();
        button.set_tooltip_text(Some(current));
        button.add_css_class("form-control");
        button.add_css_class("chooser-dropdown");
        button.set_halign(gtk::Align::Start);

        let selected = Rc::new(Cell::new(selected));
        let changed = Rc::new(RefCell::new(None::<SelectionChanged>));
        let checks = Rc::new(RefCell::new(Vec::<gtk::Image>::new()));
        for (index, label) in labels.iter().enumerate() {
            let (option, check) = menu_option(label, index == selected.get());
            if let Some(label_widget) = option
                .child()
                .and_then(|row| row.first_child())
                .and_downcast::<gtk::Label>()
            {
                label_widget.set_max_width_chars(48);
                label_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label_widget.set_tooltip_text(Some(label));
            }
            checks.borrow_mut().push(check);
            let selected = selected.clone();
            let changed = changed.clone();
            let checks = checks.clone();
            let button = button.downgrade();
            let current_label = current_label.downgrade();
            let popover = popover.downgrade();
            let label = (*label).to_owned();
            option.connect_clicked(move |_| {
                selected.set(index);
                if let Some(current_label) = current_label.upgrade() {
                    current_label.set_label(&label);
                }
                if let Some(button) = button.upgrade() {
                    button.set_tooltip_text(Some(&label));
                }
                for (check_index, check) in checks.borrow().iter().enumerate() {
                    check.set_visible(check_index == index);
                }
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(changed) = changed.borrow().as_ref() {
                    changed(index);
                }
            });
            content.append(&option);
        }

        Self {
            button,
            popover,
            selected,
            changed,
        }
    }

    fn selected(&self) -> usize {
        self.selected.get()
    }

    fn connect_selected(&self, callback: impl Fn(usize) + 'static) {
        self.changed.replace(Some(Box::new(callback)));
    }

    fn dismiss(&self) -> bool {
        if !self.popover.is_mapped() {
            return false;
        }
        self.popover.popdown();
        true
    }
}

impl ChoiceControl {
    fn value(&self) -> (String, String) {
        match self {
            Self::Boolean { id, check } => (id.clone(), check.is_active().to_string()),
            Self::Select {
                id,
                values,
                dropdown,
            } => (
                id.clone(),
                values.get(dropdown.selected()).cloned().unwrap_or_default(),
            ),
        }
    }

    fn dismiss_dropdown(&self) -> bool {
        match self {
            Self::Boolean { .. } => false,
            Self::Select { dropdown, .. } => dropdown.dismiss(),
        }
    }
}

struct ChooserState {
    request: ChooserRequest,
    window: gtk::Window,
    view: BrowserView,
    filename: Option<gtk::Entry>,
    filter_dropdown: Option<ChooserDropdown>,
    filters: Vec<PortalFilter>,
    choices: Vec<ChoiceControl>,
    read_only: Option<gtk::CheckButton>,
    error: gtk::Label,
    destination_check: Cell<bool>,
    accept_button: gtk::Button,
    completion: RefCell<Option<Completion>>,
}

impl ChooserState {
    fn cancel(&self) {
        self.finish(Err(PortalError::Cancelled("file chooser dismissed".into())));
    }

    fn finish(&self, result: ashpd::backend::Result<SelectedFiles>) {
        let Some(completion) = self.completion.take() else {
            return;
        };
        CHOOSERS.with(|choosers| {
            let mut choosers = choosers.borrow_mut();
            if choosers
                .get(&self.request.token)
                .and_then(glib::WeakRef::upgrade)
                .as_ref()
                .is_some_and(|window| window == &self.window)
            {
                choosers.remove(&self.request.token);
            }
        });
        self.window.close();
        completion(result);
    }

    fn show_error(&self, message: &str) {
        self.error.set_label(message);
        self.error.set_visible(true);
        if let Some(details) = self.error.ancestor(gtk::ScrolledWindow::static_type()) {
            details.set_visible(true);
        }
    }

    fn active_folder(&self) -> Result<PathBuf, &'static str> {
        self.view
            .browser()
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .ok_or("Choose an accessible local folder")
    }

    fn selected_filter(&self) -> Option<FileFilter> {
        self.filter_dropdown
            .as_ref()
            .and_then(|dropdown| self.filters.get(dropdown.selected()))
            .map(|filter| filter.portal.clone())
    }

    fn selected_choices(&self) -> Vec<(String, String)> {
        self.choices.iter().map(ChoiceControl::value).collect()
    }

    fn dismiss_dropdown(&self) -> bool {
        self.filter_dropdown
            .as_ref()
            .is_some_and(ChooserDropdown::dismiss)
            || self.choices.iter().any(ChoiceControl::dismiss_dropdown)
    }

    fn complete_paths(&self, paths: Vec<PathBuf>, writable: Option<bool>) {
        let mut result = SelectedFiles::default();
        for path in paths {
            let uri = match local_uri(&path) {
                Ok(uri) => uri,
                Err(error) => {
                    self.finish(Err(error));
                    return;
                }
            };
            result = result.uri(uri);
        }
        for (id, value) in self.selected_choices() {
            result = result.choice(&id, &value);
        }
        result = result
            .current_filter(self.selected_filter())
            .writable(writable);
        self.finish(Ok(result));
    }

    fn accept(self: &Rc<Self>) {
        if self.completion.borrow().is_none()
            || self.destination_check.get()
            || visible_modal_layer(&self.window).is_some()
        {
            return;
        }
        self.error.set_visible(false);
        match &self.request.kind {
            ChooserKind::Open {
                directory,
                multiple,
            } => {
                let browser = self.view.browser();
                let Some(current) = browser.active_location() else {
                    self.show_error("Choose an accessible local folder");
                    return;
                };
                let entries = eligible_open_entries(browser.selected_entries(), *directory);
                match open_selection(&entries, &current, *directory, *multiple) {
                    Ok(paths) => self.complete_paths(
                        paths,
                        self.read_only
                            .as_ref()
                            .map(|read_only| writable_from_read_only(read_only.is_active())),
                    ),
                    Err(message) => self.show_error(message),
                }
            }
            ChooserKind::SaveFile { .. } => self.accept_save_file(),
            ChooserKind::SaveFiles { names } => {
                let folder = match self.active_folder() {
                    Ok(folder) => folder,
                    Err(message) => {
                        self.show_error(message);
                        return;
                    }
                };
                self.accept_destinations(folder, names.clone());
            }
        }
    }

    fn accept_save_file(self: &Rc<Self>) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };
        let name = filename.text().to_string();
        if let Err(message) = crate::services::validate_basename(&name) {
            filename.add_css_class("error");
            filename.set_tooltip_text(Some(message));
            filename.grab_focus();
            self.show_error(message);
            return;
        }
        filename.remove_css_class("error");
        filename.set_tooltip_text(None);
        let folder = match self.active_folder() {
            Ok(folder) => folder,
            Err(message) => {
                self.show_error(message);
                return;
            }
        };
        let name = match &self.request.kind {
            ChooserKind::SaveFile {
                current_name: Some(current),
            } if current.to_string_lossy() == name => current.clone(),
            _ => OsString::from(name),
        };
        self.accept_destinations(folder, vec![name]);
    }

    fn accept_destinations(self: &Rc<Self>, folder: PathBuf, names: Vec<OsString>) {
        if self.destination_check.replace(true) {
            return;
        }
        self.accept_button.set_sensitive(false);
        let weak = Rc::downgrade(self);
        let _task = glib::MainContext::default().spawn_local(async move {
            let result = check_destinations(&folder, &names).await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.destination_check.set(false);
            state.accept_button.set_sensitive(true);
            if state.completion.borrow().is_none() {
                return;
            }
            let destinations = match result {
                Ok(destinations) => destinations,
                Err(message) => {
                    state.show_error(&message);
                    return;
                }
            };
            if !destinations.existing_files {
                state.complete_paths(destinations.paths, None);
                return;
            }

            state.confirm_overwrite(destinations.paths);
        });
    }

    fn confirm_overwrite(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let Some(overlay) = self.window.child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let root = overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = root.as_ref() {
            root.set_blurred(true);
        }
        let names = paths
            .iter()
            .take(3)
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().chars().take(80).collect::<String>())
            .collect::<Vec<_>>()
            .join(", ");
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            if paths.len() > 1 {
                "Replace existing files?"
            } else {
                "Replace existing file?"
            },
            &names,
            "Replace",
            ModalTone::Danger,
        );
        layout
            .body
            .append(&super::controls::message_dialog_description(
                if paths.len() > 1 {
                    "One or more destination files already exist. Continuing may overwrite them."
                } else {
                    "The destination file already exists. Continuing may overwrite it."
                },
            ));
        let layer = modal_layer(&layout.content, &overlay, root.clone(), None);
        overlay.add_overlay(&layer);
        for button in [&layout.cancel, &layout.close] {
            let layer = layer.clone();
            let overlay = overlay.clone();
            let root = root.clone();
            button.connect_clicked(move |_| dismiss_modal_layer(&layer, &overlay, root.as_ref()));
        }
        let weak = Rc::downgrade(self);
        let confirmed_layer = layer.clone();
        layout.confirm.connect_clicked(move |_| {
            dismiss_modal_layer(&confirmed_layer, &overlay, root.as_ref());
            if let Some(state) = weak.upgrade() {
                state.complete_paths(paths.clone(), None);
            }
        });
        let escape = gtk::EventControllerKey::new();
        let cancel = layout.cancel.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                cancel.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        layout.cancel.grab_focus();
    }

    fn activate_file(self: &Rc<Self>, location: &Location) {
        match &self.request.kind {
            ChooserKind::Open {
                directory: false, ..
            } => self.accept(),
            ChooserKind::SaveFile { .. } => {
                let Some((folder, name)) = location
                    .native_path()
                    .and_then(|path| Some((path.parent()?, path.file_name()?)))
                    .filter(|(_, name)| safe_filename(name))
                    .map(|(folder, name)| (folder.to_owned(), name.to_owned()))
                else {
                    return;
                };
                if let Some(filename) = self.filename.as_ref() {
                    filename.set_text(&name.to_string_lossy());
                }
                self.accept_destinations(folder, vec![name]);
            }
            _ => {}
        }
    }
}

fn eligible_open_entries(entries: Vec<FileEntry>, directory: bool) -> Vec<FileEntry> {
    entries
        .into_iter()
        .filter(|entry| entry.is_directory() == directory)
        .collect()
}

pub(crate) fn present_chooser(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
) {
    let _ = build_chooser(request, cancelled, completion);
}

fn build_chooser(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
) -> Option<Rc<ChooserState>> {
    if cancelled.load(Ordering::SeqCst) {
        completion(Err(PortalError::Cancelled(
            "file chooser request was cancelled".into(),
        )));
        return None;
    }

    let source = ChooserFileSource::new();
    let (filters, selected_filter) =
        portal_filters(&request.filters, request.current_filter.as_ref());
    source.set_filter(
        selected_filter
            .and_then(|index| filters.get(index))
            .map(|filter| filter.native.clone()),
    );
    let multiple = matches!(&request.kind, ChooserKind::Open { multiple: true, .. });
    let view = BrowserView::new_chooser(source.clone(), multiple);
    let theme = ThemeManager::shared();
    view.set_view_mode(theme.browser_mode());
    view.set_density(theme.browser_density());
    view.set_group_by_type(theme.group_by_type());
    view.set_auto_refresh_interval(theme.auto_refresh_interval());
    view.set_peek_enabled(false);
    view.set_single_click_previews(theme.single_click_previews());
    view.set_operation_provider(Rc::new(LocalOperationProvider));
    let browser = view.browser();
    let preview_preferences = theme.clone();
    let preview = PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(move || {
            preview_preferences.media_preview_backend()
        }))),
        false,
    );

    let window = gtk::Window::builder()
        .title(&request.title)
        .default_width(1050)
        .default_height(720)
        .modal(request.modal)
        .build();
    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    let sidebar_toggle = gtk::ToggleButton::builder()
        .active(true)
        .tooltip_text("Toggle sidebar (Ctrl+B)")
        .build();
    sidebar_toggle.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PANEL_LEFT,
        20,
    )));
    sidebar_toggle.add_css_class("sidebar-toggle");
    let location = view.location_widget();
    location.set_hexpand(true);
    let appearance = build_appearance_menu(&view, &browser, theme.clone());
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_content.set_hexpand(true);
    header_content.append(&sidebar_toggle);
    header_content.append(&location);
    let close = gtk::Button::builder()
        .tooltip_text("Cancel file selection (Esc)")
        .build();
    close.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        20,
    )));
    close.add_css_class("header-action");
    let closing_window = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = closing_window.upgrade() {
            window.close();
        }
    });
    let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_actions.add_css_class("header-actions");
    header_actions.append(&appearance);
    header_actions.append(&close);
    header_content.append(&header_actions);
    header.set_title_widget(Some(&header_content));

    let sidebar = build_sidebar(view.clone(), theme, true);
    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_wide_handle(false);
    content.set_position(SIDEBAR_WIDTH);
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    content.set_shrink_start_child(false);
    content.set_resize_start_child(false);
    content.set_start_child(Some(&sidebar.widget));
    content.set_end_child(Some(&view.widget()));
    content.set_vexpand(true);
    let toggled_sidebar = sidebar.widget.clone();
    sidebar_toggle.connect_toggled(move |toggle| {
        toggled_sidebar.set_visible(toggle.is_active());
    });

    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    preview_split.set_wide_handle(false);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    let measured_content = content.clone();
    let measured_view = view.clone();
    preview.attach_split(
        &preview_split,
        Rc::new(move || measured_content.position() + measured_view.preview_occupied_width()),
    );

    let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
    details.add_css_class("chooser-details");
    let filename = match &request.kind {
        ChooserKind::SaveFile { current_name } => {
            let row = labeled_row("Name", None::<&gtk::Widget>);
            let entry = form_entry();
            entry.set_hexpand(true);
            entry.set_placeholder_text(Some("Enter a filename"));
            if let Some(name) = current_name {
                entry.set_text(&name.to_string_lossy());
                entry.select_region(0, -1);
            }
            row.append(&entry);
            details.append(&row);
            Some(entry)
        }
        ChooserKind::SaveFiles { names } => {
            let names = names
                .iter()
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ");
            let label = gtk::Label::new(Some(&names));
            label.add_css_class("action-dialog-description");
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_tooltip_text(Some(&names));
            let row = labeled_row("Files", Some(label.upcast_ref()));
            details.append(&row);
            None
        }
        ChooserKind::Open { .. } => None,
    };

    let options = chooser_options();
    details.append(&options);
    let filter_dropdown = if filters.is_empty() {
        None
    } else {
        let labels = filters
            .iter()
            .map(|filter| filter.portal.label())
            .collect::<Vec<_>>();
        let dropdown = ChooserDropdown::new(&labels, selected_filter.unwrap_or(0));
        let row = labeled_row("Filter", Some(dropdown.button.upcast_ref()));
        append_option(&options, &row);
        let filters_for_change = filters.clone();
        let source_for_change = source.clone();
        let browser_for_change = browser.clone();
        dropdown.connect_selected(move |selected| {
            source_for_change.set_filter(
                filters_for_change
                    .get(selected)
                    .map(|filter| filter.native.clone()),
            );
            if let Some(last) = browser_for_change.active_depth() {
                for depth in 0..=last {
                    browser_for_change.retry_column(depth);
                }
            }
        });
        Some(dropdown)
    };

    let choices = build_choices(&request.choices, &options);
    let read_only = matches!(
        &request.kind,
        ChooserKind::Open {
            directory: false,
            ..
        }
    )
    .then(|| {
        let check = form_check_button("Open files read-only");
        append_option(&options, &check);
        check
    });
    options.set_visible(options.first_child().is_some());

    let error = gtk::Label::new(None);
    error.add_css_class("form-message");
    error.add_css_class("error");
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.set_visible(false);
    details.append(&error);

    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("action-dialog-cancel");
    let accept = gtk::Button::with_mnemonic(&request.accept_label);
    accept.add_css_class("action-dialog-confirm");
    if matches!(
        &request.kind,
        ChooserKind::Open {
            directory: true,
            ..
        }
    ) {
        accept.set_tooltip_text(Some("Select folder (Ctrl+Enter)"));
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("chooser-actions");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    actions.append(&cancel);
    actions.append(&accept);
    let details_scroll = gtk::ScrolledWindow::builder()
        .child(&details)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(220)
        .build();
    details_scroll.set_visible(
        filename.is_some()
            || !filters.is_empty()
            || !choices.is_empty()
            || read_only.is_some()
            || matches!(&request.kind, ChooserKind::SaveFiles { .. }),
    );

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&preview_split);
    root.append(&details_scroll);
    root.append(&actions);
    let blurred_root = BlurBin::new(&root);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&blurred_root));
    window.set_child(Some(&overlay));
    install_modal_focus_trap(&window);
    window.set_default_widget(Some(&accept));

    let state = Rc::new(ChooserState {
        request,
        window: window.clone(),
        view: view.clone(),
        filename: filename.clone(),
        filter_dropdown,
        filters,
        choices,
        read_only,
        error,
        destination_check: Cell::new(false),
        accept_button: accept.clone(),
        completion: RefCell::new(Some(Box::new(completion))),
    });

    let weak = Rc::downgrade(&state);
    accept.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.accept();
        }
    });
    let weak = Rc::downgrade(&state);
    cancel.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.cancel();
        }
    });
    if let Some(filename) = filename {
        let weak = Rc::downgrade(&state);
        filename.connect_activate(move |_| {
            if let Some(state) = weak.upgrade() {
                state.accept();
            }
        });
    }

    let state_for_observer = state.clone();
    let preview_for_browser = preview.clone();
    let weak_browser = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if let BrowserEvent::OpenRequested { location } = event {
            state_for_observer.activate_file(location);
        }
        if let Some(browser) = weak_browser.upgrade() {
            preview_for_browser.handle_browser_event(&browser, event);
        }
    });

    let weak = Rc::downgrade(&state);
    window.connect_close_request(move |_| {
        if let Some(state) = weak.upgrade() {
            state.cancel();
        }
        glib::Propagation::Proceed
    });
    install_shortcuts(&window, &state, &sidebar, &sidebar_toggle, &preview);
    let browser_for_destroy = browser.clone();
    window.connect_destroy(move |_| {
        browser_for_destroy.clear_observer();
        sidebar.disconnect();
    });

    let weak_window = glib::WeakRef::new();
    weak_window.set(Some(&window));
    CHOOSERS.with(|choosers| {
        let previous = {
            choosers
                .borrow_mut()
                .insert(state.request.token.clone(), weak_window)
                .and_then(|window| window.upgrade())
        };
        if let Some(previous) = previous {
            previous.close();
        }
    });
    if cancelled.load(Ordering::SeqCst) {
        state.cancel();
        return None;
    }

    gtk::prelude::WidgetExt::realize(&window);
    apply_external_parent(&window, state.request.parent.as_ref());
    browser.navigate(Location::local(&state.request.initial_directory));
    window.present();
    if let Some(filename) = state.filename.as_ref() {
        filename.grab_focus();
        filename.select_region(0, -1);
    } else {
        sidebar_toggle.grab_focus();
    }
    window.set_focus_visible(true);
    let initial_focus = gtk::prelude::RootExt::focus(&window).map(|widget| widget.downgrade());
    // View initialization also queues focus work; restore the chooser's target afterward.
    glib::idle_add_local_once(move || {
        if let Some(widget) = initial_focus.and_then(|widget| widget.upgrade()) {
            widget.grab_focus();
        }
    });
    Some(state)
}

pub(crate) fn cancel_chooser(token: &str) {
    CHOOSERS.with(|choosers| {
        let window = {
            choosers
                .borrow()
                .get(token)
                .and_then(glib::WeakRef::upgrade)
        };
        if let Some(window) = window {
            window.close();
        }
    });
}

fn portal_filters(
    filters: &[FileFilter],
    current: Option<&FileFilter>,
) -> (Vec<PortalFilter>, Option<usize>) {
    let (filters, selected) = normalize_portal_filters(filters, current);
    (
        filters
            .into_iter()
            .map(|portal| {
                let native = gtk::FileFilter::new();
                native.set_name(Some(portal.label()));
                for pattern in portal.pattern_filters() {
                    native.add_pattern(pattern);
                }
                for mime in portal.mimetype_filters() {
                    native.add_mime_type(mime);
                }
                PortalFilter { portal, native }
            })
            .collect(),
        selected,
    )
}

fn normalize_portal_filters(
    filters: &[FileFilter],
    current: Option<&FileFilter>,
) -> (Vec<FileFilter>, Option<usize>) {
    let mut filters = filters.to_vec();
    if let Some(current) = current
        && !filters.contains(current)
    {
        filters.push(current.clone());
    }
    let selected = current
        .and_then(|current| filters.iter().position(|filter| filter == current))
        .or_else(|| (!filters.is_empty()).then_some(0));
    (filters, selected)
}

fn chooser_options() -> gtk::FlowBox {
    let options = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .min_children_per_line(1)
        .max_children_per_line(32)
        .column_spacing(16)
        .row_spacing(6)
        .halign(gtk::Align::Start)
        .focusable(false)
        .build();
    options.add_css_class("chooser-options");
    options
}

fn append_option(parent: &gtk::FlowBox, widget: &impl IsA<gtk::Widget>) {
    let child = gtk::FlowBoxChild::builder()
        .child(widget)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Center)
        .focusable(false)
        .build();
    parent.append(&child);
}

fn build_choices(choices: &[Choice], parent: &gtk::FlowBox) -> Vec<ChoiceControl> {
    choices
        .iter()
        .map(|choice| {
            let pairs = choice.pairs();
            if pairs.is_empty() {
                let check = form_check_button(choice.label());
                check.set_active(choice.initial_selection() == "true");
                append_option(parent, &check);
                ChoiceControl::Boolean {
                    id: choice.id().to_owned(),
                    check,
                }
            } else {
                let labels = pairs.iter().map(|(_, label)| *label).collect::<Vec<_>>();
                let values = pairs
                    .iter()
                    .map(|(value, _)| (*value).to_owned())
                    .collect::<Vec<_>>();
                let selected = values
                    .iter()
                    .position(|value| value == choice.initial_selection())
                    .unwrap_or(0);
                let dropdown = ChooserDropdown::new(&labels, selected);
                let row = labeled_row(choice.label(), Some(dropdown.button.upcast_ref()));
                append_option(parent, &row);
                ChoiceControl::Select {
                    id: choice.id().to_owned(),
                    values,
                    dropdown,
                }
            }
        })
        .collect()
}

fn labeled_row(label: &str, child: Option<&gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let label = form_label(label);
    label.set_max_width_chars(16);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_tooltip_text(Some(&label.text()));
    row.append(&label);
    if let Some(child) = child {
        row.append(child);
    }
    row
}

fn apply_external_parent(window: &gtk::Window, parent: Option<&WindowIdentifierType>) {
    let Some(WindowIdentifierType::Wayland(handle)) = parent else {
        return;
    };
    let Some(surface) = window.surface() else {
        return;
    };
    let Ok(toplevel) = surface.downcast::<gdk4_wayland::WaylandToplevel>() else {
        tracing::debug!("portal parent type does not match the current display backend");
        return;
    };
    if !toplevel.set_transient_for_exported(handle) {
        tracing::debug!("Wayland compositor rejected the portal parent handle");
    }
}

fn install_shortcuts(
    window: &gtk::Window,
    state: &Rc<ChooserState>,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let sidebar_state = sidebar.state.clone();
    let sidebar_widget = sidebar.widget.clone();
    let sidebar_toggle = sidebar_toggle.clone();
    let preview = preview.clone();
    let focus_before_sidebar = Rc::new(RefCell::new(None::<gtk::Widget>));
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if let Some(layer) = visible_modal_layer(&state.window) {
            let focused = gtk::prelude::RootExt::focus(&state.window);
            if !focused.is_some_and(|focus| focus == layer || focus.is_ancestor(&layer)) {
                layer.grab_focus();
            }
            return glib::Propagation::Proceed;
        }
        let browser = state.view.browser();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let focused = gtk::prelude::RootExt::focus(&state.window);
        if !focused
            .as_ref()
            .is_some_and(super::focus_navigation::in_popover)
            && (super::window::is_sidebar_focus_shortcut(key, modifiers)
                || (!focused
                    .as_ref()
                    .is_some_and(super::focus_navigation::editable)
                    && super::window::is_browser_navigation_key(key, modifiers)))
        {
            state.view.keyboard_navigation();
        }
        if !control
            && !alt
            && !shift
            && !modifiers.contains(gtk::gdk::ModifierType::SUPER_MASK)
            && super::focus_navigation::arrow_direction(key).is_some()
        {
            state.window.set_focus_visible(true);
            if focused
                .as_ref()
                .is_none_or(|widget| !widget.is_mapped() || !widget.is_sensitive())
            {
                if let Some(filename) = state.filename.as_ref() {
                    filename.grab_focus();
                } else {
                    sidebar_toggle.grab_focus();
                }
                return glib::Propagation::Stop;
            }
        }
        let sidebar_has_focus = focused.as_ref().is_some_and(|focused| {
            focused == &sidebar_widget || focused.is_ancestor(&sidebar_widget)
        });
        if key == gtk::gdk::Key::Escape {
            if let Some(popover) = focused
                .as_ref()
                .and_then(|widget| widget.ancestor(gtk::Popover::static_type()))
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            if state.dismiss_dropdown() {
                return glib::Propagation::Stop;
            }
            if state.view.cancel_new_entry() || state.view.cancel_rename() {
                return glib::Propagation::Stop;
            }
            if state.view.dismiss_focused_filter() {
                return glib::Propagation::Stop;
            }
            if state.view.location_has_focus() {
                state.view.cancel_location_edit();
                return glib::Propagation::Stop;
            }
            if preview.is_open() {
                preview.close();
                return glib::Propagation::Stop;
            }
            state.cancel();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::F2
            && !control
            && !alt
            && !shift
            && !modifiers.contains(gtk::gdk::ModifierType::SUPER_MASK)
            && browser.selected_entries().len() == 1
            && !focused.as_ref().is_some_and(|widget| {
                super::focus_navigation::editable(widget)
                    || super::focus_navigation::in_popover(widget)
            })
            && state.view.begin_rename()
        {
            return glib::Propagation::Stop;
        }
        if alt
            && !control
            && !shift
            && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            && state.view.item_view_has_focus()
            && !state.view.new_entry_is_active()
            && !state.view.rename_is_active()
            && state.view.show_focused_properties()
        {
            return glib::Propagation::Stop;
        }
        if state.view.new_entry_is_active() || state.view.rename_is_active() {
            return glib::Propagation::Proceed;
        }
        if control
            && !shift
            && !alt
            && matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F)
            && state.view.show_filter()
        {
            return glib::Propagation::Stop;
        }
        if control && matches!(key, gtk::gdk::Key::l | gtk::gdk::Key::L) {
            state.view.begin_location_edit();
            return glib::Propagation::Stop;
        }
        if is_sidebar_focus_shortcut(key, modifiers) {
            if sidebar_has_focus {
                let restored = focus_before_sidebar
                    .borrow_mut()
                    .take()
                    .is_some_and(|widget| widget.grab_focus());
                if !restored {
                    browser.focus_active();
                }
            } else {
                focus_before_sidebar.replace(focused.clone());
                if !sidebar_toggle.is_active() {
                    sidebar_toggle.set_active(true);
                }
                let sidebar = sidebar_state.clone();
                glib::idle_add_local_once(move || {
                    sidebar.focus_active_place();
                });
            }
            return glib::Propagation::Stop;
        }
        if control && !shift && matches!(key, gtk::gdk::Key::b | gtk::gdk::Key::B) {
            sidebar_toggle.set_active(!sidebar_toggle.is_active());
            return glib::Propagation::Stop;
        }
        if state.view.location_has_focus() {
            return glib::Propagation::Proceed;
        }
        if is_folder_accept_shortcut(key, modifiers)
            && state.view.item_view_has_focus()
            && matches!(
                &state.request.kind,
                ChooserKind::Open {
                    directory: true,
                    ..
                }
            )
        {
            state.accept();
            return glib::Propagation::Stop;
        }
        if control && shift && matches!(key, gtk::gdk::Key::n | gtk::gdk::Key::N) {
            state.view.create_new_folder();
            return glib::Propagation::Stop;
        }
        if control
            && !shift
            && key == gtk::gdk::Key::a
            && state.view.item_view_has_focus()
            && matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            )
        {
            state.view.select_all();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::F5
            || (control && !alt && matches!(key, gtk::gdk::Key::r | gtk::gdk::Key::R))
        {
            if let Some(depth) = browser.active_depth() {
                browser.retry_column(depth);
            }
            return glib::Propagation::Stop;
        }
        if control
            && !shift
            && !alt
            && matches!(
                key,
                gtk::gdk::Key::h | gtk::gdk::Key::H | gtk::gdk::Key::period
            )
        {
            browser.toggle_hidden();
            return glib::Propagation::Stop;
        }
        if control {
            return glib::Propagation::Proceed;
        }
        let column_popover = focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .and_downcast::<gtk::Popover>()
            .filter(|popover| popover.has_css_class("column-popover"));
        if let Some(popover) = column_popover
            && !control
            && !alt
            && let Some(direction) = vim_focus_direction(key)
        {
            popover.child_focus(direction);
            return glib::Propagation::Stop;
        }
        if !alt && let Some(focused) = focused.as_ref() {
            if super::focus_navigation::in_popover(focused) {
                return glib::Propagation::Proceed;
            }
            if !shift
                && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
                && super::focus_navigation::activate(state.window.upcast_ref())
            {
                return glib::Propagation::Stop;
            }
            if super::focus_navigation::editable(focused) {
                return glib::Propagation::Proceed;
            }
        }
        if preview.has_video()
            && state.view.item_view_has_focus()
            && !alt
            && !control
            && !shift
            && matches!(
                key,
                gtk::gdk::Key::space
                    | gtk::gdk::Key::Up
                    | gtk::gdk::Key::Down
                    | gtk::gdk::Key::Left
                    | gtk::gdk::Key::Right
                    | gtk::gdk::Key::m
                    | gtk::gdk::Key::M
            )
        {
            preview.handle_video_key(key);
            return glib::Propagation::Stop;
        }
        let mut header_left_boundary = false;
        if state.view.header_actions_have_focus() && !control && !alt {
            match key {
                gtk::gdk::Key::h | gtk::gdk::Key::Left => {
                    if state.view.move_header_focus(gtk::DirectionType::Left) {
                        return glib::Propagation::Stop;
                    }
                    header_left_boundary = true;
                }
                gtk::gdk::Key::l | gtk::gdk::Key::Right => {
                    state.view.move_header_focus(gtk::DirectionType::Right);
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::j | gtk::gdk::Key::Down => {
                    state.view.focus_items_from_header();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if sidebar_has_focus
            && !control
            && !alt
            && let Some(direction) =
                vim_focus_direction(key).or_else(|| super::focus_navigation::arrow_direction(key))
        {
            if direction == gtk::DirectionType::Right {
                focus_before_sidebar.borrow_mut().take();
                browser.focus_active();
            } else if !sidebar_widget.child_focus(direction) && direction == gtk::DirectionType::Up
            {
                sidebar_toggle.grab_focus();
                state.window.set_focus_visible(true);
            }
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace
            && !control
            && !alt
            && state.view.dismiss_empty_focused_filter()
        {
            return glib::Propagation::Stop;
        }
        if !control && !alt && !state.view.item_view_has_focus() && !header_left_boundary {
            if !shift && let Some(direction) = super::focus_navigation::arrow_direction(key) {
                if direction == gtk::DirectionType::Up
                    && focused.as_ref().is_some_and(|focused| {
                        let mut widget = Some(focused.clone());
                        while let Some(current) = widget {
                            if current.has_css_class("chooser-options") {
                                return true;
                            }
                            widget = current.parent();
                        }
                        false
                    })
                {
                    browser.focus_active();
                    return glib::Propagation::Stop;
                }
                if super::focus_navigation::move_focus(state.window.upcast_ref(), direction) {
                    return glib::Propagation::Stop;
                }
            }
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::Left
            && !control
            && !alt
            && !shift
            && sidebar_toggle.is_active()
            && state.view.item_view_has_focus()
            && state.view.item_at_sidebar_edge()
        {
            focus_before_sidebar.replace(focused.clone());
            sidebar_state.focus_active_place();
            state.window.set_focus_visible(true);
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::space && !control && !alt {
            preview.toggle(preview_target(browser.focused_entry()));
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace && !control && !alt {
            state.view.navigate_up();
            return glib::Propagation::Stop;
        }
        if !alt
            && state.view.view_mode() != super::browser_modes::BrowserMode::Columns
            && let Some(direction) = super::focus_navigation::arrow_direction(key)
        {
            let extend = shift
                && matches!(
                    &state.request.kind,
                    ChooserKind::Open { multiple: true, .. }
                );
            if state.view.cross_type_group(direction, extend) {
                return glib::Propagation::Stop;
            }
            if !shift && key == gtk::gdk::Key::Up && state.view.focus_header_from_top_item() {
                return glib::Propagation::Stop;
            }
            // Keep GTK's spatial movement, then reconcile selection in visual order
            // across the independent collection views used for type groups.
            let weak = Rc::downgrade(&state);
            glib::idle_add_local_once(move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if !state.view.item_view_has_focus() {
                    return;
                }
                state.view.synchronize_native_selection(extend);
            });
            return glib::Propagation::Proceed;
        }
        if shift
            && matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            )
            && key == gtk::gdk::Key::Up
        {
            browser.extend_selection(-1);
            return glib::Propagation::Stop;
        }
        if shift
            && matches!(
                &state.request.kind,
                ChooserKind::Open { multiple: true, .. }
            )
            && key == gtk::gdk::Key::Down
        {
            browser.extend_selection(1);
            return glib::Propagation::Stop;
        }
        if !shift
            && matches!(key, gtk::gdk::Key::k | gtk::gdk::Key::Up)
            && state.view.focus_header_from_top_item()
        {
            return glib::Propagation::Stop;
        }

        match (key, alt) {
            (gtk::gdk::Key::Left, true) => browser.back(),
            (gtk::gdk::Key::Right, true) => browser.forward(),
            (gtk::gdk::Key::Up, true) => browser.parent(),
            (gtk::gdk::Key::Home, true) => {
                browser.navigate(Location::local(home_directory()));
            }
            (gtk::gdk::Key::j | gtk::gdk::Key::Down, false) => browser.move_selection(1),
            (gtk::gdk::Key::k | gtk::gdk::Key::Up, false) => browser.move_selection(-1),
            (gtk::gdk::Key::h | gtk::gdk::Key::Left, false)
                if !control
                    && state.view.first_column_has_focus()
                    && sidebar_toggle.is_active() =>
            {
                focus_before_sidebar.replace(focused.clone());
                sidebar_state.focus_active_place();
            }
            (gtk::gdk::Key::h | gtk::gdk::Key::Left, false) => state.view.navigate_left(),
            (gtk::gdk::Key::Right, false) if state.view.view_mode() == BrowserMode::Columns => {
                browser.enter_focused_directory();
            }
            (gtk::gdk::Key::l | gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter, false) => {
                state.view.activate_focused()
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

fn is_folder_accept_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
}
