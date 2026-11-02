// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct ChooserState {
    pub(super) request: ChooserRequest,
    pub(super) window: gtk::Window,
    pub(super) destination_host: Option<DestinationHost>,
    pub(super) view: BrowserView,
    pub(super) filename: Option<gtk::Entry>,
    pub(super) filename_selection: RefCell<Vec<Location>>,
    pub(super) filename_edited: Cell<bool>,
    /// Exact bytes of the filesystem name last copied into `filename`; the entry
    /// itself holds only the lossy UTF-8 rendering.
    pub(super) filename_source: RefCell<Option<OsString>>,
    pub(super) filter_dropdown: Option<ChooserDropdown>,
    pub(super) filters: Vec<PortalFilter>,
    pub(super) choices: Vec<ChoiceControl>,
    pub(super) read_only: Option<gtk::CheckButton>,
    pub(super) error: gtk::Label,
    pub(super) action_status: gtk::Stack,
    pub(super) destination_check: Cell<bool>,
    pub(super) accept_generation: Cell<u64>,
    pub(super) accept_button: gtk::Button,
    pub(super) completion: RefCell<Option<Completion>>,
    pub(super) download_cancel: RefCell<Option<Arc<AtomicBool>>>,
    pub(super) downloaded_file: RefCell<Option<(String, PathBuf)>>,
    pub(super) download_holder: gtk::Box,
    pub(super) download_progress: RefCell<Option<download::DownloadProgress>>,
}

impl ChooserState {
    pub(super) fn cancel(&self) {
        self.finish(Err(PortalError::Cancelled("file chooser dismissed".into())));
    }

    pub(super) fn download_in_progress(&self) -> bool {
        self.download_cancel.borrow().is_some()
    }

    pub(super) fn cancel_download(&self) -> bool {
        let Some(cancelled) = self.download_cancel.borrow_mut().take() else {
            return false;
        };
        cancelled.store(true, Ordering::SeqCst);
        self.dismiss_download_progress();
        self.accept_button.set_sensitive(true);
        true
    }

    pub(super) fn after_download_step<T: Send + 'static>(
        self: &Rc<Self>,
        work: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&Rc<Self>, T) + 'static,
    ) {
        self.cancel_download();
        let step = Arc::new(AtomicBool::new(false));
        self.download_cancel.replace(Some(step.clone()));
        self.accept_button.set_sensitive(false);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let answer = gio::spawn_blocking(work)
                .await
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            let Some(state) = weak.upgrade() else {
                return;
            };
            let current = state
                .download_cancel
                .borrow()
                .as_ref()
                .is_some_and(|flag| Arc::ptr_eq(flag, &step));
            if current {
                state.download_cancel.take();
                state.accept_button.set_sensitive(true);
                then(&state, answer);
            }
        });
    }

    pub(super) fn open_remote(self: &Rc<Self>, url: &str) {
        if !matches!(
            self.request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        ) {
            self.show_error(&crate::i18n::tr(
                "Remote links are only supported when opening files",
            ));
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
            .filter(|(previous, _)| previous == url)
            .map(|(_, path)| path.clone());
        match cached {
            Some(path) => self.reuse_download(url.to_owned(), path),
            None => self.download(url.to_owned()),
        }
    }

    pub(super) fn download(self: &Rc<Self>, url: String) {
        self.downloaded_file.take();
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
            if !state.download_is_current(&cancelled) {
                return glib::ControlFlow::Break;
            }
            let (flow, latest_progress) = state.drain_download_events(&receiver, &url);
            if state.download_is_current(&cancelled)
                && let Some((downloaded, total)) = latest_progress
                && let Some(progress) = state.download_progress.borrow().as_ref()
            {
                progress.update(downloaded, total);
            }
            flow
        });
    }

    fn download_is_current(&self, cancelled: &Arc<AtomicBool>) -> bool {
        self.download_cancel
            .borrow()
            .as_ref()
            .is_some_and(|flag| Arc::ptr_eq(flag, cancelled))
    }

    /// Coalesces progress so terminal events cannot wait behind a backlog.
    fn drain_download_events(
        self: &Rc<Self>,
        receiver: &std::sync::mpsc::Receiver<RemoteDownload>,
        url: &str,
    ) -> (glib::ControlFlow, Option<(u64, Option<u64>)>) {
        let mut latest_progress = None;
        loop {
            match receiver.try_recv() {
                Ok(RemoteDownload::Named(name)) => {
                    if let Some(progress) = self.download_progress.borrow().as_ref() {
                        progress.set_name(&name);
                    }
                }
                Ok(RemoteDownload::Progress { downloaded, total }) => {
                    latest_progress = Some((downloaded, total));
                }
                Ok(RemoteDownload::Finished(path)) => {
                    self.dismiss_download_progress();
                    self.download_cancel.borrow_mut().take();
                    self.downloaded_file
                        .replace(Some((url.to_owned(), path.clone())));
                    self.complete_download(path);
                    break;
                }
                Ok(RemoteDownload::Failed(message)) => {
                    self.dismiss_download_progress();
                    self.accept_button.set_sensitive(true);
                    if self.download_cancel.borrow_mut().take().is_some() {
                        self.show_error(&message);
                    }
                    break;
                }
                Err(TryRecvError::Empty) => {
                    return (glib::ControlFlow::Continue, latest_progress);
                }
                Err(TryRecvError::Disconnected) => {
                    self.dismiss_download_progress();
                    self.accept_button.set_sensitive(true);
                    self.download_cancel.borrow_mut().take();
                    self.show_error(&crate::i18n::tr(
                        "The download stopped unexpectedly. Try again.",
                    ));
                    break;
                }
            }
        }
        (glib::ControlFlow::Break, latest_progress)
    }

    pub(super) fn show_download_progress(&self, url: &str, on_cancel: Rc<dyn Fn()>) {
        self.dismiss_download_progress();
        let progress = download::DownloadProgress::new(url, on_cancel);
        self.download_holder.append(&progress.root);
        self.download_holder.set_visible(true);
        self.download_progress.replace(Some(progress));
    }

    pub(super) fn dismiss_download_progress(&self) {
        if let Some(progress) = self.download_progress.take() {
            self.download_holder.remove(&progress.root);
        }
        self.download_holder.set_visible(false);
    }

    pub(super) fn finish_remote(&self, path: PathBuf) {
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
            listable: MetadataValue::Unknown,
            is_hidden: false,
            recent_uri: None,
        };
        if !self.view.browser().allows_entry(&entry) {
            self.accept_button.set_sensitive(true);
            self.show_error(&crate::i18n::tr(
                "The file does not match the selected filter",
            ));
            return;
        }
        self.complete_paths(vec![path], self.writable());
    }

    pub(super) fn finish(&self, result: ashpd::backend::Result<SelectedFiles>) {
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

    pub(super) fn show_error(&self, message: &str) {
        self.error.set_label(message);
        self.error.set_visible(true);
        self.action_status.set_visible_child_name("error");
    }

    pub(super) fn clear_error(&self) {
        self.error.set_visible(false);
        self.action_status.set_visible_child_name("normal");
    }

    /// The selected results or the fill; in 10xer mode an unfilled cursor
    /// row counts too. The automatic first-row selection counts only when
    /// `load_cursor` allows it.
    pub(super) fn chosen_entries(&self, load_cursor: bool) -> Vec<FileEntry> {
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

    pub(super) fn has_fill(&self) -> bool {
        let browser = self.view.browser();
        match self.view.selected_search_results() {
            Some(results) => !results.is_empty(),
            None => !browser.selection_is_load_cursor() && !browser.selected_entries().is_empty(),
        }
    }

    pub(super) fn selected_folder(&self) -> Option<PathBuf> {
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

    pub(super) fn name_selection_entries(&self) -> Vec<FileEntry> {
        if self.view.selected_search_results().is_some() {
            return self.chosen_entries(true);
        }
        let browser = self.view.browser();
        if browser.selection_is_load_cursor() && browser.selected_entries().len() <= 1 {
            return Vec::new();
        }
        self.chosen_entries(true)
    }

    pub(super) fn fill_filename(&self, filename: &gtk::Entry, name: &OsStr) {
        filename.set_text(&name.to_string_lossy());
        self.filename_source.replace(Some(name.to_owned()));
    }

    pub(super) fn clear_filename(&self, filename: &gtk::Entry) {
        filename.set_text("");
        self.filename_source.replace(None);
        self.filename_edited.set(false);
    }

    pub(super) fn save_destination_name(&self, text: &str) -> OsString {
        match self.filename_source.borrow().as_deref() {
            Some(source) if source.to_string_lossy() == text => source.to_owned(),
            _ => OsString::from(text),
        }
    }

    pub(super) fn install_filename_selection_click(self: &Rc<Self>) {
        if !matches!(self.request.kind, ChooserKind::SaveFile { .. }) {
            return;
        }
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if PreferenceManager::shared().tenxer_mode() {
                return;
            }
            let Some(surface) = gesture.widget() else {
                return;
            };
            let Some(picked) = surface.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            if !clicked_entry_row(&surface, picked) {
                return;
            }
            let location = state.view.browser().active_location();
            let weak = Rc::downgrade(&state);
            // Re-clicking a highlighted row emits no GTK selection change. Let the
            // row handlers finish before committing its cursor and refilling the name.
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.refill_name_after_click(location);
                }
            });
        });
        self.view.widget().add_controller(click);
    }

    fn refill_name_after_click(&self, location: Option<Location>) {
        let browser = self.view.browser();
        if !self.window.is_visible() || browser.active_location() != location {
            return;
        }
        if self.view.selected_search_results().is_none()
            && browser.selection_is_load_cursor()
            && let Some((depth, position, _)) = browser.focused_item()
        {
            browser.commit_selection();
            browser.set_selection(depth, &browser.selected_positions(depth), Some(position));
        }
        self.filename_selection.borrow_mut().clear();
        self.update_selected_filename();
    }

    pub(super) fn update_selected_filename(&self) {
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
        if let Some(name) = single_safe_file_name(&entries) {
            self.fill_filename(filename, name);
            self.filename_edited.set(false);
            filename.remove_css_class("error");
            crate::ui::accessibility::set_description(filename, None);
        } else if self.opens_files() {
            self.clear_filename(filename);
        }
    }

    fn opens_files(&self) -> bool {
        matches!(
            self.request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        )
    }

    fn recent_entry_parent(&self) -> Option<PathBuf> {
        let in_recent = self
            .view
            .browser()
            .active_location()
            .is_some_and(|location| location.is_recent_root());
        if !in_recent {
            return None;
        }
        let entries = self.chosen_entries(false);
        let [entry] = entries.as_slice() else {
            return None;
        };
        entry
            .location
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    pub(super) fn active_folder(&self) -> Result<PathBuf, &'static str> {
        if let Some(folder) = self.selected_folder() {
            return Ok(folder);
        }
        if let Some(parent) = self.recent_entry_parent() {
            return Ok(parent);
        }
        self.view
            .browser()
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .ok_or("Choose an accessible local folder")
    }

    pub(super) fn selected_filter(&self) -> Option<FileFilter> {
        self.filter_dropdown
            .as_ref()
            .and_then(|dropdown| self.filters.get(dropdown.selected()))
            .map(|filter| filter.portal.clone())
    }

    pub(super) fn selected_choices(&self) -> Vec<(String, String)> {
        self.choices.iter().map(ChoiceControl::value).collect()
    }

    pub(super) fn dismiss_dropdown(&self) -> bool {
        self.filter_dropdown
            .as_ref()
            .is_some_and(ChooserDropdown::dismiss)
            || self.choices.iter().any(ChoiceControl::dismiss_dropdown)
    }

    pub(super) fn complete_paths(&self, paths: Vec<PathBuf>, writable: Option<bool>) {
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

    pub(super) fn accept(self: &Rc<Self>) {
        if self
            .destination_host
            .as_ref()
            .is_some_and(|host| !host.parent.is_visible())
        {
            self.cancel();
            return;
        }
        if self.accept_is_blocked() {
            return;
        }
        self.clear_error();
        match &self.request.kind {
            ChooserKind::Open {
                directory,
                multiple,
            } => self.accept_open(*directory, *multiple),
            ChooserKind::SaveFile { .. } => self.accept_save_file(),
            ChooserKind::SaveFiles { names } => match self.active_folder() {
                Ok(folder) => self.accept_destinations(folder, names.clone()),
                Err(message) => self.show_error(&crate::i18n::tr(message)),
            },
        }
    }

    fn accept_is_blocked(&self) -> bool {
        self.completion.borrow().is_none()
            || self.destination_check.get()
            || self.download_in_progress()
            || visible_modal_layer(&self.window).is_some()
    }

    /// A typed name takes precedence over the selection when opening files.
    fn edited_open_name(&self, directory: bool) -> Option<String> {
        if directory || !self.filename_edited.get() {
            return None;
        }
        self.filename
            .as_ref()
            .map(|filename| filename.text().to_string())
            .filter(|text| !text.is_empty())
    }

    fn accept_open(self: &Rc<Self>, directory: bool, multiple: bool) {
        self.update_selected_filename();
        if let Some(name) = self.edited_open_name(directory) {
            self.accept_open_name(&name);
            return;
        }
        let browser = self.view.browser();
        let Some(current) = browser.active_location() else {
            self.show_error(&crate::i18n::tr("Choose an accessible local folder"));
            return;
        };
        let entries = self.chosen_entries(!directory);
        let entries = eligible_open_entries(entries, directory)
            .into_iter()
            .filter(|entry| browser.allows_entry(entry))
            .collect::<Vec<_>>();
        match (
            open_selection(&entries, &current, directory, multiple),
            self.destination_host.as_ref(),
        ) {
            (Ok(paths), Some(host)) => self.accept_destination((host.validate)(&paths[0])),
            (Ok(paths), None) => self.complete_paths(paths, self.writable()),
            (Err(message), _) => self.show_error(&crate::i18n::tr(message)),
        }
    }

    fn writable(&self) -> Option<bool> {
        self.read_only
            .as_ref()
            .map(|read_only| writable_from_read_only(read_only.is_active()))
    }

    pub(super) fn accept_destination(
        self: &Rc<Self>,
        validation: futures_lite::future::BoxedLocal<Result<PathBuf, String>>,
    ) {
        self.destination_check.set(true);
        self.accept_button.set_sensitive(false);
        let generation = self.accept_generation.get();
        let weak = Rc::downgrade(self);
        let _task = glib::MainContext::default().spawn_local(async move {
            let result = validation.await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.destination_check.set(false);
            if state.completion.borrow().is_none() || state.accept_generation.get() != generation {
                return;
            }
            state.accept_button.set_sensitive(true);
            match result {
                Ok(path) => state.complete_paths(vec![path], None),
                Err(message) => state.show_error(&message),
            }
        });
    }

    pub(super) fn accept_open_name(self: &Rc<Self>, name: &str) {
        if let Some(url) = remote_file_url(name) {
            self.open_remote(&url);
            return;
        }
        if let Err(message) = crate::services::validate_basename(name) {
            self.show_error(&crate::i18n::tr(message));
            return;
        }
        let folder = self
            .view
            .browser()
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf))
            .or_else(|| self.active_folder().ok());
        let Some(folder) = folder else {
            self.show_error(&crate::i18n::tr("Choose an accessible local folder"));
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
            state.open_named_entry(result);
        });
    }

    fn open_named_entry(&self, result: Result<FileEntry, glib::Error>) {
        let Ok(entry) = result else {
            self.show_error(&crate::i18n::tr("Choose an existing, accessible file"));
            return;
        };
        if entry.is_directory() {
            self.view.browser().navigate(entry.location);
            if let Some(filename) = self.filename.as_ref() {
                self.clear_filename(filename);
            }
            return;
        }
        if !self.view.browser().allows_entry(&entry) {
            self.show_error(&crate::i18n::tr(
                "The file does not match the selected filter",
            ));
            return;
        }
        match entry.location.native_path() {
            Some(path) => self.finish_remote(path.to_path_buf()),
            None => self.show_error(&crate::i18n::tr("Choose an existing, accessible file")),
        }
    }

    pub(super) fn accept_save_file(self: &Rc<Self>) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };
        let name = filename.text().to_string();
        if let Err(message) = crate::services::validate_basename(&name) {
            let message = crate::i18n::tr(message);
            filename.add_css_class("error");
            crate::ui::accessibility::set_description(filename, Some(&message));
            self.show_error(&message);
            filename.grab_focus();
            return;
        }
        filename.remove_css_class("error");
        crate::ui::accessibility::set_description(filename, None);
        let folder = match self.active_folder() {
            Ok(folder) => folder,
            Err(message) => {
                self.show_error(&crate::i18n::tr(message));
                return;
            }
        };
        let name = self.save_destination_name(&name);
        self.accept_destinations(folder, vec![name]);
    }

    pub(super) fn accept_destinations(self: &Rc<Self>, folder: PathBuf, names: Vec<OsString>) {
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
            if destinations.existing.is_empty() {
                state.complete_paths(destinations.paths, None);
                return;
            }

            state.confirm_overwrite(destinations);
        });
    }

    pub(super) fn confirm_overwrite(self: &Rc<Self>, destinations: DestinationCheck) {
        let DestinationCheck { paths, existing } = destinations;
        let Some(overlay) = self.window.child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let root = overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = root.as_ref() {
            root.set_blurred(true);
        }
        let names = crate::i18n::list(
            existing
                .iter()
                .take(3)
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().chars().take(80).collect::<String>()),
        );
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            &crate::i18n::tr(if existing.len() > 1 {
                "Replace existing files?"
            } else {
                "Replace existing file?"
            }),
            &names,
            &crate::i18n::tr("Replace"),
            ModalTone::Danger,
        );
        layout
            .body
            .append(&crate::ui::controls::message_dialog_description(
                &crate::i18n::tr(if existing.len() > 1 {
                    "One or more destination files already exist. Continuing may overwrite them."
                } else {
                    "The destination file already exists. Continuing may overwrite it."
                }),
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
        let cancel = layout.cancel.clone();
        close_on_escape(&layer, move || cancel.emit_clicked());
        focus_button(&layout.cancel);
    }

    pub(super) fn confirm_file(self: &Rc<Self>, entry: &FileEntry) {
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
            self.show_error(&crate::i18n::tr("Choose folders only"));
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

    pub(super) fn name_has_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.filename.as_ref().is_some_and(|filename| {
            focused.is_some_and(|focused| focused == filename || focused.is_ancestor(filename))
        })
    }

    pub(super) fn edit_name(&self) {
        let Some(filename) = self.filename.as_ref() else {
            return;
        };
        filename.grab_focus_without_selecting();
        select_name_stem(filename);
    }

    pub(super) fn activate_file(self: &Rc<Self>, location: &Location) {
        if self.download_in_progress() {
            return;
        }
        match &self.request.kind {
            ChooserKind::Open {
                directory: false, ..
            } => {
                let Some(path) = location.native_path() else {
                    self.show_error(&crate::i18n::tr("Choose a local file"));
                    return;
                };
                self.complete_paths(vec![path.to_path_buf()], self.writable());
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
                    self.fill_filename(filename, &name);
                }
                self.accept_destinations(folder, vec![name]);
            }
            _ => {}
        }
    }
}

/// Whether a pick landed on a file row rather than an editable or popover.
fn clicked_entry_row(surface: &gtk::Widget, picked: gtk::Widget) -> bool {
    if crate::ui::focus_navigation::editable(&picked)
        || crate::ui::focus_navigation::in_popover(&picked)
    {
        return false;
    }
    std::iter::successors(Some(picked), gtk::Widget::parent)
        .take_while(|widget| widget != surface)
        .any(|widget| {
            widget.has_css_class("file-row")
                || widget.has_css_class("list-row")
                || widget.has_css_class("icons-card")
        })
}

fn single_safe_file_name(entries: &[FileEntry]) -> Option<&OsStr> {
    let [entry] = entries else {
        return None;
    };
    if entry.is_directory() {
        return None;
    }
    entry
        .location
        .native_path()
        .and_then(Path::file_name)
        .filter(|name| safe_filename(name))
}

pub(super) fn select_name_stem(entry: &gtk::Entry) {
    entry.select_region(
        0,
        crate::ui::collection_edit::rename_stem_end(&entry.text()),
    );
}

pub(super) fn eligible_open_entries(entries: Vec<FileEntry>, directory: bool) -> Vec<FileEntry> {
    entries
        .into_iter()
        .filter(|entry| entry.is_directory() == directory)
        .collect()
}
