// SPDX-License-Identifier: MIT

mod download;
mod image_conversion;

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
        mpsc::TryRecvError,
    },
    time::Duration,
};

use ashpd::{
    PortalError, WindowIdentifierType,
    desktop::file_chooser::{Choice, FileFilter, SelectedFiles},
};
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{LocalFileSource, LocalOperationProvider, LocalPreviewProvider},
    app::BrowserEvent,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    portal::{
        ChooserKind, ChooserRequest, check_destinations, local_uri, open_selection, safe_filename,
        writable_from_read_only,
    },
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, MetadataRequest, RemoteDownload, download_remote, remote_file_url,
    },
};

use super::{
    blur::BlurBin,
    browser::{BrowserView, dismiss_modal_layer, modal_layer},
    browser_modes::BrowserMode,
    controls::{
        ModalTone, focus_button, form_check_button, form_entry, form_label, menu_option,
        message_dialog_layout,
    },
    preferences::PreferenceManager,
    preview::{PreviewDrawer, preview_target},
    shortcut_footer::ShortcutFooter,
    top_bar_navigation::TopBarNavigation,
    window::{
        ChooserKeys, ChooserPolicy, MIN_SIDEBAR_WIDTH, SIDEBAR_WIDTH, SidebarView,
        build_appearance_menu, build_sidebar, home_directory, install_modal_focus_trap,
        is_sidebar_focus_shortcut, vim_focus_direction, visible_modal_layer,
    },
};

type Completion = Box<dyn FnOnce(ashpd::backend::Result<SelectedFiles>)>;
type DestinationValidator = Rc<dyn Fn(&Path) -> Result<PathBuf, String>>;

thread_local! {
    static CHOOSERS: RefCell<HashMap<String, glib::WeakRef<gtk::Window>>> = RefCell::new(HashMap::new());
    static DESTINATION_PARENTS: RefCell<Vec<glib::WeakRef<gtk::Window>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    static DESTINATION_STATES: RefCell<Vec<std::rc::Weak<ChooserState>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn destination_chooser_for(parent: &gtk::Window) -> Option<(gtk::Window, BrowserView)> {
    DESTINATION_STATES.with(|states| {
        states
            .borrow()
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .find(|state| {
                state.window.is_visible() && state.window.transient_for().as_ref() == Some(parent)
            })
            .map(|state| (state.window.clone(), state.view.clone()))
    })
}

struct ChooserFileSource {
    source: Rc<dyn FileSource>,
    filter: Rc<RefCell<Option<gtk::FileFilter>>>,
    directory_only: Rc<Cell<bool>>,
    root_limit: Option<PathBuf>,
}

impl ChooserFileSource {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            source: Rc::new(LocalFileSource),
            filter: Rc::new(RefCell::new(None)),
            directory_only: Rc::new(Cell::new(false)),
            root_limit: None,
        })
    }

    fn set_filter(&self, filter: Option<gtk::FileFilter>) {
        self.filter.replace(filter);
    }
}

impl FileSource for ChooserFileSource {
    fn allows_navigation(&self, location: &Location) -> bool {
        chooser_location_allowed(self.root_limit.as_deref(), location)
    }

    fn allows_entry(&self, entry: &FileEntry) -> bool {
        chooser_entry_allowed(
            self.filter.borrow().as_ref(),
            self.directory_only.get(),
            self.root_limit.as_deref(),
            entry,
        )
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if !self.allows_navigation(location) {
            return Err(LocationValidationError::UnsupportedScheme(
                "Choose an existing folder inside this removable device.".into(),
            ));
        }
        if location.native_path().is_none() && !location.is_recent_root() {
            return Err(LocationValidationError::UnsupportedScheme(
                "The system file chooser supports local files and folders only.".into(),
            ));
        }
        self.source.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if self.root_limit.is_some()
            || (location.native_path().is_none() && !location.is_recent_root())
        {
            emit(self.validate_location(&location));
            return LoadHandle::new(|| {});
        }
        self.source.validate_location_async(location, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        (location.native_path().is_some() || location.is_recent_root())
            && self.source.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.source.fill_metadata(request, emit)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let filter = self.filter.clone();
        let directory_only = self.directory_only.clone();
        let root_limit = self.root_limit.clone();
        self.source.enumerate(
            request,
            Rc::new(move |event| {
                let event = match event {
                    DirectoryEvent::Batch {
                        request_id,
                        mut entries,
                    } => {
                        entries.retain(|entry| {
                            chooser_entry_allowed(
                                filter.borrow().as_ref(),
                                directory_only.get(),
                                root_limit.as_deref(),
                                entry,
                            )
                        });
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
        let directory_only = self.directory_only.clone();
        let root_limit = self.root_limit.clone();
        self.source.watch(
            location,
            include_hidden,
            Rc::new(move |change| {
                notify(filter_directory_change(
                    filter.borrow().as_ref(),
                    directory_only.get(),
                    root_limit.as_deref(),
                    change,
                ));
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

fn chooser_location_allowed(root: Option<&Path>, location: &Location) -> bool {
    root.is_none_or(|root| {
        location.native_path().is_some_and(|path| {
            path.canonicalize()
                .is_ok_and(|path| path.is_dir() && path.starts_with(root))
        })
    })
}

fn chooser_entry_allowed(
    filter: Option<&gtk::FileFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    entry: &FileEntry,
) -> bool {
    chooser_location_allowed(root_limit, &entry.location)
        && entry.location.native_path().is_some()
        && (!directory_only || entry.is_directory())
        && filter.is_none_or(|filter| file_filter_matches(filter, entry))
}

fn filter_directory_change(
    filter: Option<&gtk::FileFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    change: DirectoryChange,
) -> DirectoryChange {
    match change {
        DirectoryChange::Upsert(entry)
            if !chooser_entry_allowed(filter, directory_only, root_limit, &entry) =>
        {
            DirectoryChange::Remove(entry.location)
        }
        DirectoryChange::Move { from, entry }
            if !chooser_entry_allowed(filter, directory_only, root_limit, &entry) =>
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

const DROPDOWN_EDGE_MARGIN: i32 = 24;
const MIN_DROPDOWN_CONTENT_HEIGHT: i32 = 120;

// Popovers use separate surfaces, so the window does not constrain their content height.
fn dropdown_placement(
    available_height: i32,
    anchor_top: i32,
    anchor_bottom: i32,
) -> (gtk::PositionType, i32) {
    let below = available_height.saturating_sub(anchor_bottom).max(0);
    let above = anchor_top.max(0);
    let (position, room) = if below >= above {
        (gtk::PositionType::Bottom, below)
    } else {
        (gtk::PositionType::Top, above)
    };
    (
        position,
        room.saturating_sub(DROPDOWN_EDGE_MARGIN)
            .max(MIN_DROPDOWN_CONTENT_HEIGHT),
    )
}

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
        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .build();
        scroll.add_css_class("context-menu-scroll");
        let popover = gtk::Popover::builder()
            .child(&scroll)
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
        crate::ui::accessibility::set_description(&button, Some(current));
        button.add_css_class("form-control");
        button.add_css_class("chooser-dropdown");
        button.set_halign(gtk::Align::Start);

        let scroll_for_show = scroll.clone();
        let button_for_show = button.downgrade();
        popover.connect_show(move |popover| {
            let (Some(root), Some(button)) = (popover.root(), button_for_show.upgrade()) else {
                return;
            };
            let anchor_top = button
                .compute_point(&root, &gtk::graphene::Point::new(0.0, 0.0))
                .map_or(0, |point| point.y().round() as i32);
            let (position, max_content_height) = dropdown_placement(
                root.height(),
                anchor_top,
                anchor_top.saturating_add(button.height()),
            );
            popover.set_position(position);
            scroll_for_show.set_max_content_height(max_content_height);
        });

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
                    crate::ui::accessibility::set_description(&button, Some(&label));
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
    destination_host: Option<DestinationHost>,
    view: BrowserView,
    filename: Option<gtk::Entry>,
    filename_selection: RefCell<Vec<Location>>,
    filename_edited: Cell<bool>,
    filter_dropdown: Option<ChooserDropdown>,
    filters: Vec<PortalFilter>,
    choices: Vec<ChoiceControl>,
    read_only: Option<gtk::CheckButton>,
    error: gtk::Label,
    action_status: gtk::Stack,
    destination_check: Cell<bool>,
    accept_generation: Cell<u64>,
    accept_button: gtk::Button,
    completion: RefCell<Option<Completion>>,
    download_cancel: RefCell<Option<Arc<AtomicBool>>>,
    downloaded_file: RefCell<Option<(String, PathBuf)>>,
    download_holder: gtk::Box,
    download_progress: RefCell<Option<download::DownloadProgress>>,
}

impl ChooserState {
    fn cancel(&self) {
        self.finish(Err(PortalError::Cancelled("file chooser dismissed".into())));
    }

    fn download_in_progress(&self) -> bool {
        self.download_cancel.borrow().is_some()
    }

    fn cancel_download(&self) -> bool {
        let Some(cancelled) = self.download_cancel.borrow_mut().take() else {
            return false;
        };
        cancelled.store(true, Ordering::SeqCst);
        self.dismiss_download_progress();
        self.accept_button.set_sensitive(true);
        true
    }

    fn open_remote(self: &Rc<Self>, url: &str) {
        if !matches!(
            self.request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        ) {
            self.show_error("Remote links are only supported when opening files");
            return;
        }
        if self.completion.borrow().is_none() || visible_modal_layer(&self.window).is_some() {
            return;
        }
        // Supersede pending local accepts even if this download later fails or is cancelled.
        self.accept_generation
            .set(self.accept_generation.get().wrapping_add(1));
        self.cancel_download();
        self.clear_error();
        let cached = self
            .downloaded_file
            .borrow()
            .as_ref()
            .filter(|(previous, path)| previous == url && path.is_file())
            .map(|(_, path)| path.clone());
        if let Some(path) = cached {
            self.complete_download(path);
            return;
        }
        self.downloaded_file.take();
        let url = url.to_owned();
        let cancelled = Arc::new(AtomicBool::new(false));
        *self.download_cancel.borrow_mut() = Some(cancelled.clone());
        let state = Rc::downgrade(self);
        self.show_download_progress(
            &url,
            Rc::new(move || {
                if let Some(state) = state.upgrade() {
                    state.cancel_download();
                }
            }),
        );
        self.accept_button.set_sensitive(false);
        let receiver = download_remote(url.to_owned(), cancelled.clone());
        let state = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let Some(state) = state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // A retired attempt must not complete or clear a replacement download.
            let current = || {
                state
                    .download_cancel
                    .borrow()
                    .as_ref()
                    .is_some_and(|flag| Arc::ptr_eq(flag, &cancelled))
            };
            if !current() {
                return glib::ControlFlow::Break;
            }
            // Coalesce progress so terminal events cannot wait behind a backlog.
            let mut latest_progress = None;
            let flow = loop {
                match receiver.try_recv() {
                    Ok(RemoteDownload::Named(name)) => {
                        if let Some(progress) = state.download_progress.borrow().as_ref() {
                            progress.set_name(&name);
                        }
                    }
                    Ok(RemoteDownload::Progress { downloaded, total }) => {
                        latest_progress = Some((downloaded, total));
                    }
                    Ok(RemoteDownload::Finished(path)) => {
                        state.dismiss_download_progress();
                        state.download_cancel.borrow_mut().take();
                        state
                            .downloaded_file
                            .replace(Some((url.clone(), path.clone())));
                        state.complete_download(path);
                        break glib::ControlFlow::Break;
                    }
                    Ok(RemoteDownload::Failed(message)) => {
                        state.dismiss_download_progress();
                        state.accept_button.set_sensitive(true);
                        if state.download_cancel.borrow_mut().take().is_some() {
                            state.show_error(&message);
                        }
                        break glib::ControlFlow::Break;
                    }
                    Err(TryRecvError::Empty) => break glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        state.dismiss_download_progress();
                        state.accept_button.set_sensitive(true);
                        state.download_cancel.borrow_mut().take();
                        state.show_error("The download stopped unexpectedly. Try again.");
                        break glib::ControlFlow::Break;
                    }
                }
            };
            if current()
                && let Some((downloaded, total)) = latest_progress
                && let Some(progress) = state.download_progress.borrow().as_ref()
            {
                progress.update(downloaded, total);
            }
            flow
        });
    }

    fn show_download_progress(&self, url: &str, on_cancel: Rc<dyn Fn()>) {
        self.dismiss_download_progress();
        let progress = download::DownloadProgress::new(url, on_cancel);
        self.download_holder.append(&progress.root);
        self.download_holder.set_visible(true);
        self.download_progress.replace(Some(progress));
    }

    fn dismiss_download_progress(&self) {
        if let Some(progress) = self.download_progress.take() {
            self.download_holder.remove(&progress.root);
        }
        self.download_holder.set_visible(false);
    }

    fn finish_remote(&self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        let entry = FileEntry {
            location: Location::local(&path),
            thumbnail_path: None,
            native_name: name.clone(),
            display_name: name.to_string_lossy().into_owned(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            is_hidden: false,
            recent_uri: None,
        };
        if !self.view.browser().allows_entry(&entry) {
            self.accept_button.set_sensitive(true);
            self.show_error("The file does not match the selected filter");
            return;
        }
        self.complete_paths(
            vec![path],
            self.read_only
                .as_ref()
                .map(|read_only| writable_from_read_only(read_only.is_active())),
        );
    }

    fn finish(&self, result: ashpd::backend::Result<SelectedFiles>) {
        self.cancel_download();
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
        self.action_status.set_visible_child_name("error");
    }

    fn clear_error(&self) {
        self.error.set_visible(false);
        self.action_status.set_visible_child_name("normal");
    }

    /// The selected results or the fill; in 10xer mode an unfilled cursor
    /// row counts too. The automatic first-row selection counts only when
    /// `load_cursor` allows it.
    fn chosen_entries(&self, load_cursor: bool) -> Vec<FileEntry> {
        let browser = self.view.browser();
        let entries = match self.view.selected_search_results() {
            Some(results) => results,
            None if browser.selection_is_load_cursor() => {
                return if load_cursor {
                    browser.selected_entries()
                } else {
                    Vec::new()
                };
            }
            None => browser.selected_entries(),
        };
        if entries.is_empty() && PreferenceManager::shared().tenxer_mode() {
            return self.view.focused_target().into_iter().collect();
        }
        entries
    }

    fn has_fill(&self) -> bool {
        let browser = self.view.browser();
        match self.view.selected_search_results() {
            Some(results) => !results.is_empty(),
            None => !browser.selection_is_load_cursor() && !browser.selected_entries().is_empty(),
        }
    }

    fn selected_folder(&self) -> Option<PathBuf> {
        if PreferenceManager::shared().tenxer_mode() {
            return None;
        }
        let entries = self.chosen_entries(false);
        if entries.len() == 1 && entries[0].is_directory() {
            entries[0].location.native_path().map(Path::to_path_buf)
        } else {
            None
        }
    }

    fn name_selection_entries(&self) -> Vec<FileEntry> {
        if self.view.selected_search_results().is_some() {
            return self.chosen_entries(true);
        }
        let browser = self.view.browser();
        if browser.selection_is_load_cursor() && browser.selected_entries().len() <= 1 {
            return Vec::new();
        }
        self.chosen_entries(true)
    }

    fn update_selected_filename(&self) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };

        if PreferenceManager::shared().tenxer_mode() {
            return;
        }
        let entries = self.name_selection_entries();
        let selected = entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>();
        if self.filename_selection.replace(selected.clone()) == selected {
            return;
        }
        if let [entry] = entries.as_slice()
            && !entry.is_directory()
            && let Some(name) = entry.location.native_path().and_then(Path::file_name)
            && safe_filename(name)
        {
            filename.set_text(&name.to_string_lossy());
            self.filename_edited.set(false);
            filename.remove_css_class("error");
            crate::ui::accessibility::set_description(filename, None);
        } else if matches!(
            self.request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        ) {
            filename.set_text("");
            self.filename_edited.set(false);
        }
    }

    fn active_folder(&self) -> Result<PathBuf, &'static str> {
        if let Some(folder) = self.selected_folder() {
            return Ok(folder);
        }
        let browser = self.view.browser();
        if browser
            .active_location()
            .is_some_and(|location| location.is_recent_root())
            && let [entry] = self.chosen_entries(false).as_slice()
            && let Some(parent) = entry.location.native_path().and_then(Path::parent)
        {
            return Ok(parent.to_path_buf());
        }
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
        if self
            .destination_host
            .as_ref()
            .is_some_and(|host| !host.parent.is_visible())
        {
            self.cancel();
            return;
        }
        if self.completion.borrow().is_none()
            || self.destination_check.get()
            || self.download_in_progress()
            || visible_modal_layer(&self.window).is_some()
        {
            return;
        }
        self.clear_error();
        match &self.request.kind {
            ChooserKind::Open {
                directory,
                multiple,
            } => {
                self.update_selected_filename();
                if !directory
                    && self.filename_edited.get()
                    && let Some(filename) = self.filename.as_ref()
                    && !filename.text().is_empty()
                {
                    self.accept_open_name(&filename.text());
                    return;
                }
                let browser = self.view.browser();
                let Some(current) = browser.active_location() else {
                    self.show_error("Choose an accessible local folder");
                    return;
                };
                let entries = self.chosen_entries(!*directory);
                let entries = eligible_open_entries(entries, *directory)
                    .into_iter()
                    .filter(|entry| browser.allows_entry(entry))
                    .collect::<Vec<_>>();
                match (
                    open_selection(&entries, &current, *directory, *multiple),
                    self.destination_host.as_ref(),
                ) {
                    (Ok(paths), Some(host)) => match (host.validate)(&paths[0]) {
                        Ok(path) => self.complete_paths(vec![path], None),
                        Err(message) => self.show_error(&message),
                    },
                    (Ok(paths), None) => self.complete_paths(
                        paths,
                        self.read_only
                            .as_ref()
                            .map(|read_only| writable_from_read_only(read_only.is_active())),
                    ),
                    (Err(message), _) => self.show_error(message),
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

    fn accept_open_name(self: &Rc<Self>, name: &str) {
        if let Some(url) = remote_file_url(name) {
            self.open_remote(&url);
            return;
        }
        if let Err(message) = crate::services::validate_basename(name) {
            self.show_error(message);
            return;
        }
        let folder = self
            .view
            .browser()
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .or_else(|| self.active_folder().ok());
        let Some(folder) = folder else {
            self.show_error("Choose an accessible local folder");
            return;
        };
        let location = Location::local(folder.join(name));
        self.destination_check.set(true);
        self.accept_button.set_sensitive(false);
        let weak = Rc::downgrade(self);
        let generation = self.accept_generation.get();
        let _task = glib::MainContext::default().spawn_local(async move {
            let result = crate::adapters::query_file_entry(location).await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.destination_check.set(false);
            if state.completion.borrow().is_none() || state.accept_generation.get() != generation {
                return;
            }
            state.accept_button.set_sensitive(true);
            match result {
                Ok(entry) if entry.is_directory() => {
                    state.view.browser().navigate(entry.location);
                    if let Some(filename) = state.filename.as_ref() {
                        filename.set_text("");
                        state.filename_edited.set(false);
                    }
                }
                Ok(entry) if state.view.browser().allows_entry(&entry) => {
                    match entry.location.native_path() {
                        Some(path) => state.finish_remote(path.to_path_buf()),
                        None => state.show_error("Choose an existing, accessible file"),
                    }
                }
                Ok(_) => state.show_error("The file does not match the selected filter"),
                Err(_) => state.show_error("Choose an existing, accessible file"),
            }
        });
    }

    fn accept_save_file(self: &Rc<Self>) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };
        let name = filename.text().to_string();
        if let Err(message) = crate::services::validate_basename(&name) {
            filename.add_css_class("error");
            crate::ui::accessibility::set_description(filename, Some(message));
            self.show_error(message);
            filename.grab_focus();
            return;
        }
        filename.remove_css_class("error");
        crate::ui::accessibility::set_description(filename, None);
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
        let generation = self.accept_generation.get();
        let weak = Rc::downgrade(self);
        let _task = glib::MainContext::default().spawn_local(async move {
            let result = check_destinations(&folder, &names).await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.destination_check.set(false);
            if state.accept_generation.get() != generation {
                return;
            }
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
        focus_button(&layout.cancel);
    }

    fn confirm_file(self: &Rc<Self>, entry: &FileEntry) {
        if self.completion.borrow().is_none() || !self.view.browser().allows_entry(entry) {
            return;
        }
        if matches!(
            &self.request.kind,
            ChooserKind::Open {
                directory: true,
                ..
            }
        ) {
            self.show_error("Choose folders only");
            return;
        }
        if matches!(
            &self.request.kind,
            ChooserKind::Open {
                directory: false,
                multiple: true,
            }
        ) && self.has_fill()
        {
            self.accept();
            return;
        }
        self.activate_file(&entry.location);
    }

    fn name_has_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.filename.as_ref().is_some_and(|filename| {
            focused.is_some_and(|focused| focused == filename || focused.is_ancestor(filename))
        })
    }

    fn edit_name(&self) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };
        filename.grab_focus();
        filename.select_region(0, super::collection_edit::rename_stem_end(&filename.text()));
    }

    fn activate_file(self: &Rc<Self>, location: &Location) {
        if self.download_in_progress() {
            return;
        }
        match &self.request.kind {
            ChooserKind::Open {
                directory: false, ..
            } => {
                let Some(path) = location.native_path() else {
                    self.show_error("Choose a local file");
                    return;
                };
                self.complete_paths(
                    vec![path.to_path_buf()],
                    self.read_only
                        .as_ref()
                        .map(|read_only| writable_from_read_only(read_only.is_active())),
                );
            }
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

const MIN_CHOOSER_WIDTH: i32 = 640;
const MIN_CHOOSER_HEIGHT: i32 = 460;
const MAX_CHOOSER_WIDTH: i32 = 1000;
const MAX_CHOOSER_HEIGHT: i32 = 680;
const FALLBACK_CHOOSER_WIDTH: i32 = 920;
const FALLBACK_CHOOSER_HEIGHT: i32 = 580;

fn chooser_default_dimensions_for_monitor(monitor_width: i32, monitor_height: i32) -> (i32, i32) {
    if monitor_width <= 0 || monitor_height <= 0 {
        return (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT);
    }
    let target_width = (monitor_width.saturating_mul(80) / 100)
        .min(monitor_width.saturating_sub(120))
        .clamp(MIN_CHOOSER_WIDTH.min(monitor_width), MAX_CHOOSER_WIDTH);
    let target_height = (monitor_height.saturating_mul(78) / 100)
        .min(monitor_height.saturating_sub(100))
        .clamp(MIN_CHOOSER_HEIGHT.min(monitor_height), MAX_CHOOSER_HEIGHT);

    (target_width, target_height)
}

fn chooser_initial_dimensions(
    monitor: Option<(i32, i32)>,
    parent_size_hint: Option<(i32, i32)>,
) -> (i32, i32) {
    let monitor = monitor.filter(|(width, height)| *width > 0 && *height > 0);
    let parent = parent_size_hint.filter(|(width, height)| *width > 0 && *height > 0);
    let bounds = match (monitor, parent) {
        (Some((mw, mh)), Some((pw, ph))) => Some((mw.min(pw), mh.min(ph))),
        (monitor, parent) => parent.or(monitor),
    };
    bounds.map_or(
        (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT),
        |(width, height)| chooser_default_dimensions_for_monitor(width, height),
    )
}

fn detect_monitor_geometry(
    display: Option<&gtk::gdk::Display>,
    window: Option<&gtk::Window>,
) -> Option<(i32, i32)> {
    let window_display = window.map(gtk::prelude::WidgetExt::display);
    let default_display = gtk::gdk::Display::default();
    let display = display
        .or(window_display.as_ref())
        .or(default_display.as_ref())?;

    if let Some(geom) = window
        .and_then(|w| w.surface())
        .and_then(|surface| display.monitor_at_surface(&surface))
        .map(|monitor| monitor.geometry())
        .filter(|geom| geom.width() > 0 && geom.height() > 0)
    {
        return Some((geom.width(), geom.height()));
    }

    let monitors = display.monitors();
    for index in 0..monitors.n_items() {
        if let Some(monitor) = monitors
            .item(index)
            .and_then(|item| item.downcast::<gtk::gdk::Monitor>().ok())
        {
            let geom = monitor.geometry();
            if geom.width() > 0 && geom.height() > 0 {
                return Some((geom.width(), geom.height()));
            }
        }
    }

    None
}

struct DestinationLease(glib::WeakRef<gtk::Window>);

impl DestinationLease {
    fn acquire(parent: &gtk::Window) -> Option<Self> {
        let acquired = DESTINATION_PARENTS.with(|parents| {
            let mut parents = parents.borrow_mut();
            parents.retain(|parent| parent.upgrade().is_some());
            if parents
                .iter()
                .any(|candidate| candidate.upgrade().as_ref() == Some(parent))
            {
                return false;
            }
            parents.push(parent.downgrade());
            true
        });
        if acquired {
            Some(Self(parent.downgrade()))
        } else {
            let active = CHOOSERS.with(|choosers| {
                choosers
                    .borrow()
                    .values()
                    .filter_map(glib::WeakRef::upgrade)
                    .find(|window| window.transient_for().as_ref() == Some(parent))
            });
            if let Some(window) = active {
                window.present();
            }
            None
        }
    }
}

impl Drop for DestinationLease {
    fn drop(&mut self) {
        DESTINATION_PARENTS.with(|parents| {
            parents.borrow_mut().retain(|parent| {
                parent.upgrade() != self.0.upgrade() && parent.upgrade().is_some()
            });
        });
    }
}

pub(crate) struct DestinationRequest {
    pub parent: gtk::Window,
    pub title: String,
    pub accept_label: String,
    pub initial_directory: PathBuf,
    pub root_limit: Option<PathBuf>,
    pub allow_create: bool,
    pub validate: DestinationValidator,
}

pub(crate) fn present_destination_chooser(
    destination: DestinationRequest,
    completion: impl FnOnce(PathBuf) + 'static,
) {
    let Some(lease) = DestinationLease::acquire(&destination.parent) else {
        return;
    };
    let request = ChooserRequest {
        token: glib::uuid_string_random().to_string(),
        title: destination.title,
        accept_label: destination.accept_label,
        modal: true,
        parent: None,
        parent_size_hint: Some((destination.parent.width(), destination.parent.height())),
        initial_directory: destination.initial_directory,
        kind: ChooserKind::Open {
            directory: true,
            multiple: false,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    };
    let source = Rc::new(ChooserFileSource {
        root_limit: destination.root_limit.clone(),
        source: Rc::new(LocalFileSource),
        filter: Rc::new(RefCell::new(None)),
        directory_only: Rc::new(Cell::new(true)),
    });
    let host = DestinationHost {
        parent: destination.parent,
        validate: destination.validate,
        allow_create: destination.allow_create,
        confined: destination.root_limit.is_some(),
    };
    glib::MainContext::default().spawn_local(async move {
        crate::portal::prepare_chooser_placement().await;
        if !host.parent.is_visible() {
            return;
        }
        build_chooser_hosted(
            request,
            Arc::new(AtomicBool::new(false)),
            move |result| {
                drop(lease);
                if let Ok(result) = result
                    && let Some(path) = result
                        .uris()
                        .first()
                        .and_then(|uri| gio::File::for_uri(uri.as_str()).path())
                {
                    completion(path);
                }
            },
            source,
            Some(host),
        );
    });
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
    build_chooser_with_source(request, cancelled, completion, ChooserFileSource::new())
}

fn build_chooser_with_source(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
    source: Rc<ChooserFileSource>,
) -> Option<Rc<ChooserState>> {
    build_chooser_hosted(request, cancelled, completion, source, None)
}

struct DestinationHost {
    parent: gtk::Window,
    validate: DestinationValidator,
    allow_create: bool,
    confined: bool,
}

fn build_chooser_hosted(
    request: ChooserRequest,
    cancelled: Arc<AtomicBool>,
    completion: impl FnOnce(ashpd::backend::Result<SelectedFiles>) + 'static,
    source: Rc<ChooserFileSource>,
    host: Option<DestinationHost>,
) -> Option<Rc<ChooserState>> {
    if cancelled.load(Ordering::SeqCst) {
        completion(Err(PortalError::Cancelled(
            "file chooser request was cancelled".into(),
        )));
        return None;
    }

    let (filters, selected_filter) =
        portal_filters(&request.filters, request.current_filter.as_ref());
    source.set_filter(
        selected_filter
            .and_then(|index| filters.get(index))
            .map(|filter| filter.native.clone()),
    );
    source.directory_only.set(matches!(
        &request.kind,
        ChooserKind::Open {
            directory: true,
            ..
        }
    ));
    let multiple = matches!(&request.kind, ChooserKind::Open { multiple: true, .. });
    let view = BrowserView::new_chooser(source.clone(), multiple);
    let theme = PreferenceManager::shared();
    view.set_operation_provider(Rc::new(LocalOperationProvider));
    if let Some(host) = &host {
        view.set_chooser_allows_create(host.allow_create);
    }
    let browser = view.browser();
    let preview_preferences = theme.clone();
    let preview = PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(move || {
            preview_preferences.media_preview_backend()
        }))),
        false,
    );

    let (initial_width, initial_height) = chooser_initial_dimensions(
        detect_monitor_geometry(None, None),
        request.parent_size_hint,
    );

    let window = gtk::Window::builder()
        .title(&request.title)
        .default_width(initial_width)
        .default_height(initial_height)
        .modal(request.modal)
        .build();
    if let Some(host) = &host {
        window.set_transient_for(Some(&host.parent));
        window.set_destroy_with_parent(true);
        window.set_application(host.parent.application().as_ref());
        let child = window.downgrade();
        let handler = host.parent.connect_unrealize(move |_| {
            if let Some(window) = child.upgrade() {
                window.close();
            }
        });
        let parent = host.parent.downgrade();
        let handler = RefCell::new(Some(handler));
        window.connect_unrealize(move |_| {
            if let Some(handler) = handler.take()
                && let Some(parent) = parent.upgrade()
            {
                parent.disconnect(handler);
            }
        });
    }
    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    let sidebar_toggle = gtk::ToggleButton::builder()
        .active(true)
        .tooltip_text("Toggle sidebar (Ctrl+B)")
        .build();
    sidebar_toggle.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PANEL_LEFT,
        17,
    )));
    sidebar_toggle.add_css_class("sidebar-toggle");
    sidebar_toggle.set_cursor_from_name(Some("pointer"));
    let location = view.location_widget();
    location.set_hexpand(true);
    let appearance = build_appearance_menu(&view, &browser, theme.clone(), &preview);
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_content.set_hexpand(true);
    header_content.set_valign(gtk::Align::Center);
    header_content.append(&sidebar_toggle);
    header_content.append(&location);
    let close = gtk::Button::builder()
        .tooltip_text("Cancel file selection (Esc)")
        .build();
    close.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::X)));
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

    let confined = source.root_limit.is_some();
    let sidebar = build_sidebar(view.clone(), theme.clone(), true);
    sidebar.schedule_after_first_paint(&window);
    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_wide_handle(false);
    content.set_position(SIDEBAR_WIDTH);
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    super::window::bind_sidebar_text_size(&content, &sidebar);
    content.set_shrink_start_child(false);
    content.set_resize_start_child(false);
    content.set_start_child(Some(&sidebar.widget));
    if confined {
        sidebar.widget.set_visible(false);
        sidebar_toggle.set_visible(false);
    }
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
    preview.attach_split(&preview_split, &content, &view, Some(&sidebar));
    view.add_marquee_origin(&sidebar.widget, gtk::PackType::Start);
    view.add_marquee_origin(&preview.widget(), gtk::PackType::End);
    let (footer, footer_holder) = chooser_footer(&view, &theme);
    footer.set_chooser(chooser_reference(&request.kind));

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
            crate::ui::accessibility::set_description(&label, Some(&names));
            let row = labeled_row("Files", Some(label.upcast_ref()));
            details.append(&row);
            None
        }
        ChooserKind::Open {
            directory: false, ..
        } => {
            let row = labeled_row("Name", None::<&gtk::Widget>);
            let entry = form_entry();
            entry.set_hexpand(true);
            entry.set_placeholder_text(Some("Enter a filename or https:// URL"));
            row.append(&entry);
            details.append(&row);
            Some(entry)
        }
        ChooserKind::Open {
            directory: true, ..
        } => None,
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
        let view_for_change = view.clone();
        dropdown.connect_selected(move |selected| {
            source_for_change.set_filter(
                filters_for_change
                    .get(selected)
                    .map(|filter| filter.native.clone()),
            );
            view_for_change.refresh_source_filter();
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

    let error = gtk::Label::builder()
        .accessible_role(gtk::AccessibleRole::Alert)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    error.add_css_class("form-field-error");
    error.set_visible(false);

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
        crate::ui::accessibility::set_description(&accept, Some("Select folder (Ctrl+Enter)"));
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("chooser-actions");
    let download_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    download_holder.set_visible(false);
    let normal_status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    normal_status.append(&download_holder);
    if let Some(hints) = save_hints(&request.kind, &theme) {
        normal_status.append(&hints);
    }
    let action_status = gtk::Stack::builder()
        .hexpand(true)
        .hhomogeneous(false)
        .vhomogeneous(true)
        .build();
    action_status.add_named(&normal_status, Some("normal"));
    action_status.add_named(&error, Some("error"));
    action_status.set_visible_child_name("normal");
    actions.append(&action_status);
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
    root.append(&footer_holder);
    root.append(&details_scroll);
    root.append(&actions);
    let blurred_root = BlurBin::new(&root);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&blurred_root));
    window.set_child(Some(&overlay));
    view.install_inline_edit_dismissal(&window);
    install_modal_focus_trap(&window);
    window.set_default_widget(Some(&accept));

    let state = Rc::new(ChooserState {
        request,
        window: window.clone(),
        destination_host: host,
        view: view.clone(),
        filename: filename.clone(),
        filename_selection: RefCell::new(Vec::new()),
        filename_edited: Cell::new(false),
        filter_dropdown,
        filters,
        choices,
        read_only,
        error,
        action_status,
        destination_check: Cell::new(false),
        accept_generation: Cell::new(0),
        accept_button: accept.clone(),
        completion: RefCell::new(Some(Box::new(completion))),
        download_cancel: RefCell::new(None),
        downloaded_file: RefCell::new(None),
        download_holder,
        download_progress: RefCell::new(None),
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
        filename.connect_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.filename_selection.replace(
                    state
                        .name_selection_entries()
                        .iter()
                        .map(|entry| entry.location.clone())
                        .collect(),
                );
                state.filename_edited.set(true);
            }
        });
        let weak = Rc::downgrade(&state);
        filename.connect_activate(move |_| {
            if let Some(state) = weak.upgrade() {
                state.accept();
            }
        });
    }

    let weak = Rc::downgrade(&state);
    view.connect_search_selection_changed(Rc::new(move || {
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.update_selected_filename();
            }
        });
    }));

    let state_for_observer = state.clone();
    let preview_for_browser = preview.clone();
    let weak_browser = Rc::downgrade(&browser);
    browser.observe(move |event| {
        match event {
            BrowserEvent::OpenRequested { location } => state_for_observer.activate_file(location),
            BrowserEvent::FocusChanged { .. } | BrowserEvent::SelectionSetChanged { .. } => {
                state_for_observer.update_selected_filename()
            }
            BrowserEvent::SelectionSynced { .. } => {
                let weak = Rc::downgrade(&state_for_observer);
                glib::idle_add_local_once(move || {
                    if let Some(state) = weak.upgrade() {
                        state.update_selected_filename();
                    }
                });
            }
            _ => {}
        }
        if let Some(browser) = weak_browser.upgrade() {
            preview_for_browser.handle_browser_event(&browser, event);
        }
    });

    #[cfg(test)]
    if state.destination_host.is_some() {
        DESTINATION_STATES.with(|states| {
            let mut states = states.borrow_mut();
            states.retain(|state| state.strong_count() > 0);
            states.push(Rc::downgrade(&state));
        });
    }
    let weak = Rc::downgrade(&state);
    window.connect_close_request(move |_| {
        if let Some(state) = weak.upgrade() {
            state.cancel();
        }
        glib::Propagation::Proceed
    });
    let tenxer = tenxer_keys(
        &state,
        &sidebar,
        &sidebar_toggle,
        &header_content,
        &preview,
        &footer,
    );
    install_shortcuts(
        &window,
        &state,
        &sidebar,
        &sidebar_toggle,
        header_content.upcast_ref(),
        &preview,
        tenxer,
    );
    // Destroy can be delayed by the chooser's own closures; unrealize breaks their bindings.
    let browser_for_close = browser.clone();
    let closing_state = Rc::downgrade(&state);
    window.connect_unrealize(move |window| {
        if let Some(state) = closing_state.upgrade() {
            state.cancel();
        }
        browser_for_close.clear_observer();
        sidebar.disconnect();
        PreferenceManager::shared().release_bindings_within(window);
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
    if state.destination_host.is_some()
        && let Some(surface) = window
            .surface()
            .and_downcast::<gdk4_wayland::WaylandToplevel>()
    {
        surface.set_application_id(crate::portal::CHOOSER_APPLICATION_ID);
    }
    apply_external_parent(&window, state.request.parent.as_ref());
    let dimensions = chooser_initial_dimensions(
        detect_monitor_geometry(None, Some(&window)),
        state.request.parent_size_hint,
    );
    window.set_default_size(dimensions.0, dimensions.1);
    browser.navigate(Location::local(&state.request.initial_directory));
    window.present();
    if PreferenceManager::shared().tenxer_mode() {
        window.set_focus_visible(true);
        if let Some(filename) = state.filename.as_ref() {
            filename.select_region(0, 0);
        }
        return Some(state);
    } else if let Some(filename) = state.filename.as_ref() {
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
    crate::ui::accessibility::set_description(&label, Some(&label.text()));
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

fn save_hints(kind: &ChooserKind, preferences: &Rc<PreferenceManager>) -> Option<gtk::Box> {
    let hints: &[(&str, &str)] = match kind {
        ChooserKind::SaveFile { .. } => &[
            ("Enter", "Save here"),
            ("r", "Edit name"),
            ("o", "Replace file"),
        ],
        ChooserKind::SaveFiles { .. } => &[("Enter", "Save here")],
        ChooserKind::Open { .. } => return None,
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("chooser-save-hints");
    row.set_valign(gtk::Align::Center);
    for (index, (key, action)) in hints.iter().enumerate() {
        let keycap = gtk::Label::new(Some(key));
        keycap.add_css_class("sidebar-keycap");
        if index > 0 {
            keycap.set_margin_start(10);
        }
        let label = gtk::Label::new(Some(action));
        label.add_css_class("shortcut-footer-chord-hint");
        row.append(&keycap);
        row.append(&label);
    }
    preferences.bind_preference(&row, PreferenceManager::tenxer_mode, |row, enabled| {
        row.set_visible(enabled)
    });
    Some(row)
}

fn chooser_reference(kind: &ChooserKind) -> crate::ui::shortcut_reference::ChooserScope {
    use crate::ui::shortcut_reference::{ChooserRequest, ChooserScope};
    let (request, multiple) = match kind {
        ChooserKind::Open {
            directory: false,
            multiple,
        } => (ChooserRequest::Files, *multiple),
        ChooserKind::Open {
            directory: true,
            multiple,
        } => (ChooserRequest::Folders, *multiple),
        ChooserKind::SaveFile { .. } => (ChooserRequest::SaveFile, false),
        ChooserKind::SaveFiles { .. } => (ChooserRequest::SaveFiles, false),
    };
    ChooserScope { request, multiple }
}

fn chooser_footer(
    view: &BrowserView,
    preferences: &Rc<PreferenceManager>,
) -> (ShortcutFooter, gtk::Box) {
    let footer = ShortcutFooter::new(view.view_mode());
    footer.bind_preferences(preferences);
    footer.observe_browser(&view.browser());
    footer.observe_tree_view(view);
    let updated = footer.clone();
    view.connect_view_mode_changed(move |mode| updated.set_mode(mode));
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("chooser-footer");
    holder.append(footer.widget());
    preferences.bind_preference(
        &holder,
        PreferenceManager::tenxer_mode,
        |holder, enabled| holder.set_visible(enabled),
    );
    (footer, holder)
}

fn tenxer_keys(
    state: &Rc<ChooserState>,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    header: &gtk::Box,
    preview: &PreviewDrawer,
    footer: &ShortcutFooter,
) -> ChooserKeys {
    let confirming = Rc::downgrade(state);
    let cancelling = Rc::downgrade(state);
    let saving = Rc::downgrade(state);
    let naming = Rc::downgrade(state);
    let save_request = matches!(
        &state.request.kind,
        ChooserKind::SaveFile { .. } | ChooserKind::SaveFiles { .. }
    );
    super::window::chooser_keys(
        &state.window,
        &state.view,
        sidebar,
        TopBarNavigation::new(header, &sidebar.widget, sidebar_toggle),
        preview,
        footer,
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

fn install_shortcuts(
    window: &gtk::Window,
    state: &Rc<ChooserState>,
    sidebar: &SidebarView,
    sidebar_toggle: &gtk::ToggleButton,
    header: &gtk::Widget,
    preview: &PreviewDrawer,
    tenxer: ChooserKeys,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let sidebar_state = sidebar.state.clone();
    let sidebar_widget = sidebar.widget.clone();
    let sidebar_toggle = sidebar_toggle.clone();
    let header = header.clone();
    let preview = preview.clone();
    let focus_before_sidebar = Rc::new(RefCell::new(None::<gtk::Widget>));
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let preferences = PreferenceManager::shared();
        if let Some(layer) = visible_modal_layer(&state.window) {
            let focused = gtk::prelude::RootExt::focus(&state.window);
            if !focused.is_some_and(|focus| focus == layer || focus.is_ancestor(&layer)) {
                layer.grab_focus();
            }
            return glib::Propagation::Proceed;
        }
        let focused = gtk::prelude::RootExt::focus(&state.window);
        if preferences.tenxer_mode()
            && (state.name_has_focus(focused.as_ref()) || state.view.location_has_focus())
            && !matches!(key, gtk::gdk::Key::F1 | gtk::gdk::Key::Escape)
            && !(modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
                && matches!(key, gtk::gdk::Key::m | gtk::gdk::Key::M))
        {
            return glib::Propagation::Proceed;
        }
        if let Some(result) = tenxer.handle(key, modifiers) {
            return result;
        }
        if let Some(size) = preferences.text_size().for_shortcut(key, modifiers) {
            preferences.set_text_size(size);
            return glib::Propagation::Stop;
        }
        let browser = state.view.browser();
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if focused
            .as_ref()
            .and_then(|focused| focused.ancestor(gtk::Popover::static_type()))
            .is_some_and(|popover| popover.has_css_class("folder-context-popover"))
        {
            return glib::Propagation::Proceed;
        }
        if super::window::is_context_menu_shortcut(key, modifiers)
            && !focused.as_ref().is_some_and(|widget| {
                super::focus_navigation::editable(widget)
                    || super::focus_navigation::in_popover(widget)
            })
            && state.view.open_focused_context_menu()
        {
            return glib::Propagation::Stop;
        }
        if state.view.item_view_has_focus()
            && !state.view.new_entry_is_active()
            && !state.view.rename_is_active()
            && !focused.as_ref().is_some_and(|widget| {
                super::focus_navigation::editable(widget)
                    || super::focus_navigation::in_popover(widget)
            })
            && preview.handle_video_key(key, modifiers)
        {
            return glib::Propagation::Stop;
        }
        // Filtered rows own navigation, not the hidden directory selection.
        if matches!(key, gtk::gdk::Key::Up | gtk::gdk::Key::Down)
            && state.view.selected_search_results().is_some()
            && !focused.as_ref().is_some_and(|widget| {
                super::focus_navigation::editable(widget)
                    || super::focus_navigation::in_popover(widget)
            })
        {
            state.window.set_focus_visible(true);
            return glib::Propagation::Proceed;
        }
        let original_key = key;
        let key = super::focus_navigation::navigation_key(
            key,
            modifiers,
            PreferenceManager::shared().type_to_search(),
            focused.as_ref(),
        );
        let vim_navigation = key != original_key;
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
                if preferences.tenxer_mode() {
                    browser.focus_active();
                } else if let Some(filename) = state.filename.as_ref() {
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
            if preferences.tenxer_mode() && state.name_has_focus(focused.as_ref()) {
                browser.focus_active();
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
            if preview.is_enabled() {
                preview.close();
                return glib::Propagation::Stop;
            }
            if state.cancel_download() {
                return glib::Propagation::Stop;
            }
            state.cancel();
            return glib::Propagation::Stop;
        }
        if super::window::is_rename_shortcut(key, modifiers)
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
        if key == gtk::gdk::Key::space
            && !modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK,
            )
            && let Some(entry) = state.view.selected_search_result()
        {
            if state.view.activate_directory_on_space() {
                return glib::Propagation::Stop;
            }
            preview.toggle(
                preview_target(Some(entry)),
                state.view.browser().active_depth(),
            );
            return glib::Propagation::Stop;
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
            if preferences.tenxer_mode() {
                if !sidebar_toggle.is_active() {
                    return glib::Propagation::Stop;
                }
                if sidebar_has_focus {
                    let restored = focus_before_sidebar
                        .borrow_mut()
                        .take()
                        .is_some_and(|widget| widget.is_mapped() && widget.grab_focus());
                    if !restored {
                        browser.focus_active();
                    }
                } else {
                    focus_before_sidebar.replace(focused.clone());
                    sidebar_state.focus_active_place();
                    state.window.set_focus_visible(true);
                }
                return glib::Propagation::Stop;
            }
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
        let toggles_sidebar = if preferences.tenxer_mode() {
            matches!(key, gtk::gdk::Key::n | gtk::gdk::Key::N)
        } else {
            matches!(key, gtk::gdk::Key::b | gtk::gdk::Key::B)
        };
        if control && !shift && toggles_sidebar {
            if state
                .destination_host
                .as_ref()
                .is_none_or(|host| !host.confined)
            {
                sidebar_toggle.set_active(!sidebar_toggle.is_active());
            }
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
            if state
                .destination_host
                .as_ref()
                .is_none_or(|host| host.allow_create)
            {
                state.view.create_new_folder();
            }
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
        if key == gtk::gdk::Key::F5 {
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
        if key == gtk::gdk::Key::Delete
            && !control
            && !alt
            && state.view.item_view_has_focus()
            && !state.view.filter_has_focus()
            && state.view.confirm_delete(shift)
        {
            return glib::Propagation::Stop;
        }
        if control
            && !shift
            && !alt
            && let Some(mode) = super::window::browser_mode_for_digit(key)
        {
            super::window::apply_browser_mode(&state.view, &PreferenceManager::shared(), mode);
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
        if preferences.tenxer_mode()
            && super::focus_navigation::plain_tab_direction(key, modifiers).is_some()
            && let Some(filename) = state.filename.as_ref()
        {
            let in_header = focused
                .as_ref()
                .is_some_and(|focused| focused == &header || focused.is_ancestor(&header));
            if state.name_has_focus(focused.as_ref()) {
                browser.focus_active();
                return glib::Propagation::Stop;
            }
            if in_header {
                filename.grab_focus();
                filename.select_region(0, -1);
                return glib::Propagation::Stop;
            }
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
        if preferences.tenxer_mode()
            && state.view.item_view_has_focus()
            && super::focus_navigation::plain_tab_direction(key, modifiers)
                == Some(gtk::DirectionType::TabForward)
        {
            if header.child_focus(gtk::DirectionType::TabForward) {
                state.window.set_focus_visible(true);
            }
            return glib::Propagation::Stop;
        }
        if preferences.tenxer_mode()
            && super::focus_navigation::plain_tab_direction(key, modifiers).is_some()
        {
            if !super::focus_navigation::contains_widget(&state.view.widget(), focused.as_ref()) {
                browser.focus_active();
            }
            return glib::Propagation::Stop;
        }
        let mut header_left_boundary = false;
        if state.view.header_actions_have_focus() && !control && !alt {
            match key {
                gtk::gdk::Key::h | gtk::gdk::Key::Left => {
                    if state.view.move_header_focus(gtk::DirectionType::Left)
                        || preferences.tenxer_mode()
                    {
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
            && !preferences.tenxer_mode()
            && !control
            && !alt
            && let Some(direction) =
                vim_focus_direction(key).or_else(|| super::focus_navigation::arrow_direction(key))
        {
            if preferences.tenxer_mode() {
                browser.focus_active();
                if super::focus_navigation::arrow_direction(key).is_none() {
                    return glib::Propagation::Stop;
                }
            } else if direction == gtk::DirectionType::Right {
                focus_before_sidebar.borrow_mut().take();
                browser.focus_active();
            } else if !sidebar_widget.child_focus(direction) && direction == gtk::DirectionType::Up
            {
                sidebar_toggle.grab_focus();
                state.window.set_focus_visible(true);
            }
            if !preferences.tenxer_mode() {
                return glib::Propagation::Stop;
            }
        }
        if key == gtk::gdk::Key::BackSpace
            && !control
            && !alt
            && state.view.dismiss_empty_focused_filter()
        {
            return glib::Propagation::Stop;
        }
        if !control && !alt && !state.view.item_view_has_focus() && !header_left_boundary {
            if preferences.tenxer_mode()
                && (super::focus_navigation::arrow_direction(key).is_some()
                    || vim_focus_direction(key).is_some())
            {
                browser.focus_active();
                if super::focus_navigation::arrow_direction(key).is_none() {
                    return glib::Propagation::Stop;
                }
            } else if !shift && let Some(direction) = super::focus_navigation::arrow_direction(key)
            {
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
                return glib::Propagation::Proceed;
            } else {
                return glib::Propagation::Proceed;
            }
        }
        if key == gtk::gdk::Key::Left
            && !control
            && !alt
            && !shift
            && sidebar_toggle.is_active()
            && state.view.item_view_has_focus()
            && state.view.item_at_sidebar_edge()
            && !PreferenceManager::shared().arrow_navigation_scoped_active()
        {
            focus_before_sidebar.replace(focused.clone());
            sidebar_state.focus_active_place();
            state.window.set_focus_visible(true);
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::space && !control && !alt {
            if !modifiers
                .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::SUPER_MASK)
                && state.view.activate_directory_on_space()
            {
                return glib::Propagation::Stop;
            }
            preview.toggle(
                preview_target(browser.focused_entry()),
                browser.active_depth(),
            );
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
            if !shift
                && key == gtk::gdk::Key::Up
                && !PreferenceManager::shared().arrow_navigation_scoped_active()
                && state.view.focus_header_from_top_item()
            {
                return glib::Propagation::Stop;
            }
            // Keep GTK's spatial movement, then reconcile selection in visual order.
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
            if vim_navigation {
                super::focus_navigation::activate_native_arrow(&state.window, key);
                return glib::Propagation::Stop;
            }
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
        if state.view.item_view_has_focus()
            && let Some(query) = super::window::type_to_search_query(key, modifiers)
            && preferences.type_to_search()
            && match query {
                super::window::TypeToSearchQuery::Empty => state.view.show_filter(),
                super::window::TypeToSearchQuery::Character(character) => {
                    state.view.show_filter_with_query(&character.to_string())
                }
            }
        {
            return glib::Propagation::Stop;
        }
        if !shift
            && matches!(key, gtk::gdk::Key::k | gtk::gdk::Key::Up)
            && !PreferenceManager::shared().arrow_navigation_scoped_active()
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
                    && sidebar_toggle.is_active()
                    && !PreferenceManager::shared().arrow_navigation_scoped_active() =>
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
    window.add_controller(keys.clone());
    let keys = keys.downgrade();
    window.connect_unrealize(move |window| {
        if let Some(keys) = keys.upgrade() {
            window.remove_controller(&keys);
        }
    });
}

fn is_folder_accept_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
        && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
}
