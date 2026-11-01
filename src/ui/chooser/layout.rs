// SPDX-License-Identifier: MIT

use super::*;
use super::{
    options::{append_option, build_choices, chooser_options, labeled_row, portal_filters},
    shortcuts::{install_shortcuts, tenxer_keys},
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
    crate::ui::decoration::claim_titlebar(&window);
    let presence_view = view.downgrade();
    crate::ui::presence::follow(&window, move |presence, power_saver| {
        if let Some(view) = presence_view.upgrade() {
            view.set_presence(presence, power_saver);
        }
    });
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
    header.add_css_class("browser-header");
    header.set_show_title_buttons(false);
    let sidebar_toggle = crate::ui::sidebar_toggle::SidebarToggle::new();
    let location = view.location_widget();
    location.set_hexpand(true);
    let appearance = build_appearance_menu(&view, &browser, theme.clone(), &preview);
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_content.set_hexpand(true);
    header_content.set_valign(gtk::Align::Center);
    header_content.append(&sidebar_toggle);
    header_content.append(&location);
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
    let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_actions.add_css_class("header-actions");
    header_actions.append(&appearance);
    header_actions.append(&close);
    header_content.append(&header_actions);
    header.set_title_widget(Some(&header_content));

    let confined = source.root_limit.is_some();
    let sidebar = build_sidebar(view.clone(), theme.clone(), true);
    sidebar.schedule_after_first_paint(&window);
    bind_chooser_sidebar_modes(&sidebar, &sidebar_toggle, &theme);
    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_wide_handle(false);
    content.set_position(SIDEBAR_WIDTH);
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    crate::ui::window::bind_sidebar_text_size(&content, &sidebar);
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
    // The column beside the divider keeps its resize edge; the preview grip moves it.
    preview_split.set_wide_handle(true);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    preview.attach_split(&preview_split, &content, &view, Some(&sidebar));
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
    let (footer, footer_holder) = chooser_footer(&view, &theme);
    footer.set_chooser(chooser_reference(&request.kind));

    let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
    details.add_css_class("chooser-details");
    let filename = match &request.kind {
        ChooserKind::SaveFile { current_name } => {
            let row = labeled_row(&crate::i18n::tr("Name"), None::<&gtk::Widget>);
            let entry = form_entry();
            entry.set_hexpand(true);
            entry.set_placeholder_text(Some(&crate::i18n::tr("Enter a filename")));
            crate::ui::accessibility::set_label(&entry, &crate::i18n::tr("Name"));
            if let Some(name) = current_name {
                entry.set_text(&name.to_string_lossy());
                select_name_stem(&entry);
            }
            row.append(&entry);
            details.append(&row);
            Some(entry)
        }
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
        } => {
            let row = labeled_row(&crate::i18n::tr("Name"), None::<&gtk::Widget>);
            let entry = form_entry();
            entry.set_hexpand(true);
            entry.set_placeholder_text(Some(&crate::i18n::tr("Enter a filename or https:// URL")));
            crate::ui::accessibility::set_label(&entry, &crate::i18n::tr("Name"));
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
        let row = labeled_row(
            &crate::i18n::tr("chooser.filter"),
            Some(dropdown.button.upcast_ref()),
        );
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
        let check = form_check_button(&crate::i18n::tr("Open files read-only"));
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
    keys::install_native_aliases(&window);
    view.set_as_modal_focus_fallback(&window);
    window.set_default_widget(Some(&accept));

    let filename_source = match &request.kind {
        ChooserKind::SaveFile { current_name } => current_name.clone(),
        _ => None,
    };
    let state = Rc::new(ChooserState {
        request,
        window: window.clone(),
        destination_host: host,
        view: view.clone(),
        filename: filename.clone(),
        filename_selection: RefCell::new(Vec::new()),
        filename_edited: Cell::new(false),
        filename_source: RefCell::new(filename_source),
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

    state.install_filename_selection_click();
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
    preview.follow_operations(&browser);
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
        sidebar_toggle.upcast_ref(),
        &header_content,
        &preview,
        &footer,
    );
    install_shortcuts(
        &window,
        &state,
        &sidebar,
        sidebar_toggle.upcast_ref(),
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
        filename.grab_focus_without_selecting();
        select_name_stem(filename);
    } else {
        sidebar_toggle.grab_focus();
    }
    window.set_focus_visible(true);
    let target_filename = state.filename.clone();
    let initial_focus = gtk::prelude::RootExt::focus(&window).map(|widget| widget.downgrade());
    // View initialization also queues focus work; restore the chooser's target afterward.
    glib::idle_add_local_once(move || {
        if let Some(filename) = target_filename.as_ref() {
            filename.grab_focus_without_selecting();
            select_name_stem(filename);
        } else if let Some(widget) = initial_focus.and_then(|widget| widget.upgrade()) {
            widget.grab_focus();
        }
    });
    Some(state)
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
