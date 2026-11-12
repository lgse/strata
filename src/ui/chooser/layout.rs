// SPDX-License-Identifier: MIT

use super::*;
use super::{
    options::{append_option, build_choices, chooser_options, labeled_row, portal_filters},
    shortcuts::{ShortcutParts, install_shortcuts},
    state::select_name_stem,
};

pub(super) fn build_chooser_hosted(
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

    let (filters, selected_filter) = configure_source(&source, &request);
    let view = build_view(&source, &request, host.as_ref());
    let theme = PreferenceManager::shared();
    let browser = view.browser();
    let preview = build_preview(&theme);
    let window = build_window(&request, &view);
    if let Some(host) = &host {
        bind_host_parent(&window, host);
    }
    let header = ChooserHeader::build(&view, &theme, &preview, &window);
    let panes = ChooserPanes::build(
        &view,
        &theme,
        &window,
        &header.sidebar_toggle,
        &preview,
        source.root_limit.is_some(),
    );
    let (footer, footer_holder) = chooser_footer(&view, &theme);
    footer.set_chooser(chooser_reference(&request.kind));

    let details = ChooserDetails::build(&request, &filters, selected_filter, &source, &view);
    let actions = ChooserActions::build(&request, &theme);
    let details_scroll = details_scroll(&details, &request, &filters);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header.bar);
    root.append(&panes.preview_split);
    root.append(&footer_holder);
    root.append(&details_scroll);
    root.append(&actions.bar);
    attach_root(&window, &root, &view, &actions.accept);

    let filename_source = match &request.kind {
        ChooserKind::SaveFile { current_name } => current_name.clone(),
        _ => None,
    };
    let state = Rc::new(ChooserState {
        request,
        window: window.clone(),
        destination_host: host,
        view: view.clone(),
        filename: details.filename.clone(),
        filename_selection: RefCell::new(Vec::new()),
        filename_edited: Cell::new(false),
        filename_source: RefCell::new(filename_source),
        filter_dropdown: details.filter_dropdown,
        filters,
        choices: details.choices,
        read_only: details.read_only,
        error: actions.error,
        action_status: actions.action_status,
        destination_check: Cell::new(false),
        accept_generation: Cell::new(0),
        accept_button: actions.accept.clone(),
        completion: RefCell::new(Some(Box::new(completion))),
        download_cancel: RefCell::new(None),
        downloaded_file: RefCell::new(None),
        download_holder: actions.download_holder,
        download_progress: RefCell::new(None),
    });

    connect_actions(
        &state,
        &actions.accept,
        &actions.cancel,
        details.filename.as_ref(),
    );
    observe_browser(&state, &preview, &browser);
    #[cfg(test)]
    remember_destination_state(&state);
    let weak = Rc::downgrade(&state);
    window.connect_close_request(move |_| {
        if let Some(state) = weak.upgrade() {
            state.cancel();
        }
        glib::Propagation::Proceed
    });
    install_shortcuts(
        &state,
        ShortcutParts {
            sidebar: &panes.sidebar,
            sidebar_toggle: header.sidebar_toggle.upcast_ref(),
            header: &header.content,
            options: &details.options,
            preview: &preview,
            footer: &footer,
        },
    );
    release_on_unrealize(&window, &state, &browser, panes.sidebar);
    register_chooser_window(&state, &window);
    if cancelled.load(Ordering::SeqCst) {
        state.cancel();
        return None;
    }

    present_window(&state, &window, &browser);
    focus_initial_widget(&state, &window, &header.sidebar_toggle);
    Some(state)
}

fn configure_source(
    source: &ChooserFileSource,
    request: &ChooserRequest,
) -> (Vec<PortalFilter>, Option<usize>) {
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
    (filters, selected_filter)
}

fn build_view(
    source: &Rc<ChooserFileSource>,
    request: &ChooserRequest,
    host: Option<&DestinationHost>,
) -> BrowserView {
    let multiple = matches!(&request.kind, ChooserKind::Open { multiple: true, .. });
    let view = BrowserView::new_chooser(source.clone(), multiple);
    view.set_operation_provider(Rc::new(LocalOperationProvider));
    if let Some(host) = host {
        view.set_chooser_allows_create(host.allow_create);
    }
    view
}

fn build_preview(theme: &Rc<PreferenceManager>) -> PreviewDrawer {
    let preview_preferences = theme.clone();
    PreviewDrawer::new(
        Rc::new(LocalPreviewProvider::new(Rc::new(move || {
            preview_preferences.media_preview_backend()
        }))),
        false,
    )
}

fn build_window(request: &ChooserRequest, view: &BrowserView) -> gtk::Window {
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
    crate::ui::decoration::claim_titlebar(&window);
    let presence_view = view.downgrade();
    crate::ui::presence::follow(&window, move |presence, power_saver| {
        if let Some(view) = presence_view.upgrade() {
            view.set_presence(presence, power_saver);
        }
    });
    window
}

/// The chooser closes with its parent window.
fn bind_host_parent(window: &gtk::Window, host: &DestinationHost) {
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

struct ChooserHeader {
    bar: gtk::HeaderBar,
    content: gtk::Box,
    sidebar_toggle: crate::ui::sidebar_toggle::SidebarToggle,
}

impl ChooserHeader {
    fn build(
        view: &BrowserView,
        theme: &Rc<PreferenceManager>,
        preview: &PreviewDrawer,
        window: &gtk::Window,
    ) -> Self {
        let bar = gtk::HeaderBar::new();
        bar.add_css_class("browser-header");
        bar.set_show_title_buttons(false);
        let sidebar_toggle = crate::ui::sidebar_toggle::SidebarToggle::new();
        let location = view.location_widget();
        location.set_hexpand(true);
        let appearance = build_appearance_menu(view, theme.clone(), preview);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_hexpand(true);
        content.set_valign(gtk::Align::Center);
        content.append(&sidebar_toggle);
        content.append(&location);
        let close = close_button(window);
        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_actions.add_css_class("header-actions");
        header_actions.append(&appearance);
        header_actions.append(&close);
        content.append(&header_actions);
        bar.set_title_widget(Some(&content));
        Self {
            bar,
            content,
            sidebar_toggle,
        }
    }
}

fn close_button(window: &gtk::Window) -> gtk::Button {
    let close = gtk::Button::new();
    let keys = crate::ui::keys::Keys::One(crate::ui::keys::bindings::ESCAPE.shown());
    crate::ui::shortcut_reference::follow_tooltip(&close, "Cancel file selection (%{keys})", keys);
    crate::ui::shortcut_reference::follow_label(&close, "Cancel file selection (%{keys})", keys);
    close.set_child(Some(&crate::assets::chrome_icon(crate::assets::icons::X)));
    close.add_css_class("header-action");
    let closing_window = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = closing_window.upgrade() {
            window.close();
        }
    });
    close
}

struct ChooserPanes {
    sidebar: SidebarView,
    preview_split: gtk::Paned,
}

impl ChooserPanes {
    fn build(
        view: &BrowserView,
        theme: &Rc<PreferenceManager>,
        window: &gtk::Window,
        sidebar_toggle: &crate::ui::sidebar_toggle::SidebarToggle,
        preview: &PreviewDrawer,
        confined: bool,
    ) -> Self {
        let sidebar = build_sidebar(view.clone(), theme.clone(), true);
        sidebar.schedule_after_first_paint(window);
        bind_chooser_sidebar_modes(&sidebar, sidebar_toggle, theme);
        let content = crate::ui::window::sidebar_split();
        sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
        crate::ui::window::bind_sidebar_text_size(&content, &sidebar);
        content.set_start_child(Some(&sidebar.widget));
        if confined {
            sidebar.widget.set_visible(false);
            sidebar_toggle.set_visible(false);
        }
        content.set_end_child(Some(&view.widget()));
        let toggled_sidebar = sidebar.widget.clone();
        sidebar_toggle.connect_toggled(move |toggle| {
            toggled_sidebar.set_visible(toggle.is_active());
        });

        let preview_split = preview_split(&content, preview);
        preview.attach_split(&preview_split, &content, view, Some(&sidebar));
        view.add_marquee_origin(
            &sidebar.widget,
            gtk::PackType::Start,
            OriginClick::ClearsSelection,
        );
        view.add_marquee_origin(
            &preview.widget(),
            gtk::PackType::End,
            OriginClick::keeps_selection_within(&preview.pane_widget()),
        );
        Self {
            sidebar,
            preview_split,
        }
    }
}

fn preview_split(content: &gtk::Paned, preview: &PreviewDrawer) -> gtk::Paned {
    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    // The column beside the divider keeps its resize edge; the preview grip moves it.
    preview_split.set_wide_handle(true);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    preview_split
}

struct ChooserDetails {
    details: gtk::Box,
    filename: Option<gtk::Entry>,
    options: gtk::FlowBox,
    filter_dropdown: Option<ChooserDropdown>,
    choices: Vec<ChoiceControl>,
    read_only: Option<gtk::CheckButton>,
}

impl ChooserDetails {
    fn build(
        request: &ChooserRequest,
        filters: &[PortalFilter],
        selected_filter: Option<usize>,
        source: &Rc<ChooserFileSource>,
        view: &BrowserView,
    ) -> Self {
        let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
        details.add_css_class("chooser-details");
        let filename = name_row(&request.kind, &details);
        let options = chooser_options();
        details.append(&options);
        let filter_dropdown = filter_row(filters, selected_filter, source, view, &options);
        let choices = build_choices(&request.choices, &options);
        let read_only = matches!(
            &request.kind,
            ChooserKind::Open {
                directory: false,
                ..
            }
        )
        .then(|| {
            let check = form_check_button(&crate::i18n::tr("Open files read-only"));
            append_option(&options, &check);
            check
        });
        options.set_visible(options.first_child().is_some());
        Self {
            details,
            filename,
            options,
            filter_dropdown,
            choices,
            read_only,
        }
    }
}

fn name_row(kind: &ChooserKind, details: &gtk::Box) -> Option<gtk::Entry> {
    match kind {
        ChooserKind::SaveFile { current_name } => Some(name_entry(
            details,
            "Enter a filename",
            current_name.as_deref(),
        )),
        ChooserKind::SaveFiles { names } => {
            let names =
                crate::i18n::list(names.iter().map(|name| name.to_string_lossy().into_owned()));
            let label = gtk::Label::new(Some(&names));
            label.add_css_class("action-dialog-description");
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            crate::ui::accessibility::set_description(&label, Some(&names));
            let row = labeled_row(&crate::i18n::tr("Files"), Some(label.upcast_ref()));
            details.append(&row);
            None
        }
        ChooserKind::Open {
            directory: false, ..
        } => Some(name_entry(
            details,
            "Enter a filename or https:// URL",
            None,
        )),
        ChooserKind::Open {
            directory: true, ..
        } => None,
    }
}

fn name_entry(details: &gtk::Box, placeholder: &str, initial: Option<&OsStr>) -> gtk::Entry {
    let row = labeled_row(&crate::i18n::tr("Name"), None::<&gtk::Widget>);
    let entry = form_entry();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&crate::i18n::tr(placeholder)));
    crate::ui::accessibility::set_label(&entry, &crate::i18n::tr("Name"));
    if let Some(name) = initial {
        entry.set_text(&name.to_string_lossy());
        select_name_stem(&entry);
    }
    row.append(&entry);
    details.append(&row);
    entry
}

fn filter_row(
    filters: &[PortalFilter],
    selected_filter: Option<usize>,
    source: &Rc<ChooserFileSource>,
    view: &BrowserView,
    options: &gtk::FlowBox,
) -> Option<ChooserDropdown> {
    if filters.is_empty() {
        return None;
    }
    let labels = filters
        .iter()
        .map(|filter| filter.portal.label())
        .collect::<Vec<_>>();
    let dropdown = ChooserDropdown::new(&labels, selected_filter.unwrap_or(0));
    let row = labeled_row(
        &crate::i18n::tr("chooser.filter"),
        Some(dropdown.button.upcast_ref()),
    );
    append_option(options, &row);
    let filters_for_change = filters.to_vec();
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
}

struct ChooserActions {
    bar: gtk::Box,
    cancel: gtk::Button,
    accept: gtk::Button,
    error: gtk::Label,
    action_status: gtk::Stack,
    download_holder: gtk::Box,
}

impl ChooserActions {
    fn build(request: &ChooserRequest, theme: &Rc<PreferenceManager>) -> Self {
        let error = gtk::Label::builder()
            .accessible_role(gtk::AccessibleRole::Alert)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        error.add_css_class("form-field-error");
        error.set_visible(false);

        let cancel = gtk::Button::with_label(&crate::i18n::tr("Cancel"));
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
            crate::ui::shortcut_reference::follow_description(
                &accept,
                "Select folder (%{keys})",
                crate::ui::keys::Keys::One(crate::ui::keys::bindings::CHOOSE_FOLDER.shown()),
            );
        }
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("chooser-actions");
        let download_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        download_holder.set_visible(false);
        let normal_status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        normal_status.append(&download_holder);
        if let Some(hints) = save_hints(&request.kind, theme) {
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
        bar.append(&action_status);
        bar.append(&cancel);
        bar.append(&accept);
        Self {
            bar,
            cancel,
            accept,
            error,
            action_status,
            download_holder,
        }
    }
}

fn details_scroll(
    details: &ChooserDetails,
    request: &ChooserRequest,
    filters: &[PortalFilter],
) -> gtk::ScrolledWindow {
    let details_scroll = gtk::ScrolledWindow::builder()
        .child(&details.details)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(220)
        .build();
    let has_details = details.filename.is_some()
        || !filters.is_empty()
        || !details.choices.is_empty()
        || details.read_only.is_some()
        || matches!(&request.kind, ChooserKind::SaveFiles { .. });
    details_scroll.set_visible(has_details);
    details_scroll
}

fn attach_root(window: &gtk::Window, root: &gtk::Box, view: &BrowserView, accept: &gtk::Button) {
    let blurred_root = BlurBin::new(root);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&blurred_root));
    window.set_child(Some(&overlay));
    view.install_inline_edit_dismissal(window);
    install_modal_focus_trap(window);
    keys::install_native_aliases(window);
    view.set_as_modal_focus_fallback(window);
    window.set_default_widget(Some(accept));
}

fn connect_actions(
    state: &Rc<ChooserState>,
    accept: &gtk::Button,
    cancel: &gtk::Button,
    filename: Option<&gtk::Entry>,
) {
    state.install_filename_selection_click();
    let weak = Rc::downgrade(state);
    accept.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.accept();
        }
    });
    let weak = Rc::downgrade(state);
    cancel.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.cancel();
        }
    });
    if let Some(filename) = filename {
        connect_filename(state, filename);
    }
    let weak = Rc::downgrade(state);
    state
        .view
        .connect_search_selection_changed(Rc::new(move || {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.update_selected_filename();
                }
            });
        }));
}

fn connect_filename(state: &Rc<ChooserState>, filename: &gtk::Entry) {
    let weak = Rc::downgrade(state);
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
    let weak = Rc::downgrade(state);
    filename.connect_activate(move |_| {
        if let Some(state) = weak.upgrade() {
            state.accept();
        }
    });
}

fn observe_browser(
    state: &Rc<ChooserState>,
    preview: &PreviewDrawer,
    browser: &Rc<crate::app::Browser>,
) {
    let state_for_observer = state.clone();
    preview.follow_operations(browser);
    let preview_for_browser = preview.clone();
    let weak_browser = Rc::downgrade(browser);
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
}

#[cfg(test)]
fn remember_destination_state(state: &Rc<ChooserState>) {
    if state.destination_host.is_some() {
        DESTINATION_STATES.with(|states| {
            let mut states = states.borrow_mut();
            states.retain(|state| state.strong_count() > 0);
            states.push(Rc::downgrade(state));
        });
    }
}

/// Destroy can be delayed by the chooser's own closures; unrealize breaks their bindings.
fn release_on_unrealize(
    window: &gtk::Window,
    state: &Rc<ChooserState>,
    browser: &Rc<crate::app::Browser>,
    sidebar: SidebarView,
) {
    let browser_for_close = browser.clone();
    let closing_state = Rc::downgrade(state);
    window.connect_unrealize(move |window| {
        if let Some(state) = closing_state.upgrade() {
            state.cancel();
        }
        browser_for_close.clear_observer();
        sidebar.disconnect();
        PreferenceManager::shared().release_bindings_within(window);
    });
}

fn register_chooser_window(state: &ChooserState, window: &gtk::Window) {
    let weak_window = glib::WeakRef::new();
    weak_window.set(Some(window));
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
}

fn present_window(state: &ChooserState, window: &gtk::Window, browser: &Rc<crate::app::Browser>) {
    gtk::prelude::WidgetExt::realize(window);
    if state.destination_host.is_some()
        && let Some(surface) = window
            .surface()
            .and_downcast::<gdk4_wayland::WaylandToplevel>()
    {
        surface.set_application_id(crate::portal::CHOOSER_APPLICATION_ID);
    }
    apply_external_parent(window, state.request.parent.as_ref());
    let dimensions = chooser_initial_dimensions(
        detect_monitor_geometry(None, Some(window)),
        state.request.parent_size_hint,
    );
    window.set_default_size(dimensions.0, dimensions.1);
    browser.navigate(Location::local(&state.request.initial_directory));
    window.present();
}

fn focus_initial_widget(
    state: &ChooserState,
    window: &gtk::Window,
    sidebar_toggle: &crate::ui::sidebar_toggle::SidebarToggle,
) {
    if PreferenceManager::shared().tenxer_mode() {
        window.set_focus_visible(true);
        if let Some(filename) = state.filename.as_ref() {
            filename.select_region(0, 0);
        }
        return;
    }
    if let Some(filename) = state.filename.as_ref() {
        filename.grab_focus_without_selecting();
        select_name_stem(filename);
    } else {
        sidebar_toggle.grab_focus();
    }
    window.set_focus_visible(true);
    let target_filename = state.filename.clone();
    let initial_focus = gtk::prelude::RootExt::focus(window).map(|widget| widget.downgrade());
    // View initialization also queues focus work; restore the chooser's target afterward.
    glib::idle_add_local_once(move || {
        if let Some(filename) = target_filename.as_ref() {
            filename.grab_focus_without_selecting();
            select_name_stem(filename);
        } else if let Some(widget) = initial_focus.and_then(|widget| widget.upgrade()) {
            widget.grab_focus();
        }
    });
}

pub(super) fn apply_external_parent(window: &gtk::Window, parent: Option<&WindowIdentifierType>) {
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

pub(super) fn save_hints(
    kind: &ChooserKind,
    preferences: &Rc<PreferenceManager>,
) -> Option<gtk::Box> {
    use crate::ui::keys::{
        self, Keys, Shown,
        bindings::{ENTER, OPEN, RENAME},
    };
    let hints: &[(Shown, &str)] = match kind {
        ChooserKind::SaveFile { .. } => &[
            (ENTER.shown(), "Save here"),
            (RENAME.alternate(0).shown(), "Edit name"),
            (OPEN.shown(), "Replace file"),
        ],
        ChooserKind::SaveFiles { .. } => &[(ENTER.shown(), "Save here")],
        ChooserKind::Open { .. } => return None,
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("chooser-save-hints");
    row.set_valign(gtk::Align::Center);
    for (index, (binding, action)) in hints.iter().enumerate() {
        // Shown only in 10xer mode, so it shows that map.
        let keycap = gtk::Label::new(None);
        let shown = *binding;
        crate::ui::shortcut_reference::follow_keys(&keycap, move |keycap| {
            keycap.set_text(&keys::label_in_map(
                &rust_i18n::locale(),
                &Keys::One(shown),
                Map::Tenxer,
            ));
        });
        keycap.add_css_class("sidebar-keycap");
        if index > 0 {
            keycap.set_margin_start(10);
        }
        let label = gtk::Label::new(Some(&crate::i18n::tr(action)));
        label.add_css_class("shortcut-footer-chord-hint");
        row.append(&keycap);
        row.append(&label);
    }
    preferences.bind_preference(&row, PreferenceManager::tenxer_mode, |row, enabled| {
        row.set_visible(enabled)
    });
    Some(row)
}

pub(super) fn chooser_reference(kind: &ChooserKind) -> crate::ui::shortcut_reference::ChooserScope {
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

pub(super) fn chooser_footer(
    view: &BrowserView,
    preferences: &Rc<PreferenceManager>,
) -> (ShortcutFooter, gtk::Box) {
    let footer = ShortcutFooter::new(view.view_mode());
    footer.bind_preferences(preferences);
    footer.observe_browser(&view.browser());
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

/// The chooser keeps its sidebar modes for the dialog only.
pub(super) fn bind_chooser_sidebar_modes(
    sidebar: &SidebarView,
    toggle: &crate::ui::sidebar_toggle::SidebarToggle,
    preferences: &Rc<PreferenceManager>,
) {
    use crate::ui::sidebar_toggle::SidebarMode;
    let state = Rc::downgrade(&sidebar.state);
    let stepping_toggle = toggle.clone();
    toggle.set_step(move || {
        let Some(state) = state.upgrade() else {
            return;
        };
        match SidebarMode::of(stepping_toggle.is_active(), state.rail.get()).next() {
            SidebarMode::Expanded => {
                state.set_compact(false);
                stepping_toggle.set_active(true);
            }
            SidebarMode::Compact => state.set_compact(true),
            SidebarMode::Hidden => stepping_toggle.set_active(false),
        }
    });
    let describe = {
        let toggle = toggle.clone();
        let state = Rc::downgrade(&sidebar.state);
        Rc::new(move || {
            if let Some(state) = state.upgrade() {
                toggle.describe(SidebarMode::of(toggle.is_active(), state.rail.get()));
            }
        })
    };
    let on_toggled = describe.clone();
    toggle.connect_toggled(move |_| on_toggled());
    let on_rail = describe.clone();
    sidebar.state.connect_rail_changed(move |_| on_rail());
    preferences.bind_preference(toggle, PreferenceManager::shown_keys, move |_, _| {
        describe();
    });
}
