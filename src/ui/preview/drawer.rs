// SPDX-License-Identifier: MIT

use super::*;

impl PreviewDrawer {
    pub fn new(provider: Rc<dyn PreviewProvider>, allow_external_open: bool) -> Self {
        let pane = super::super::accessibility::pane_box();
        pane.add_css_class("preview-pane");
        super::super::accessibility::set_label(&pane, &crate::i18n::tr(PREVIEW_LABEL));
        pane.set_size_request(MIN_WIDTH, -1);
        pane.set_hexpand(true);
        pane.set_vexpand(true);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.add_css_class("preview-header");
        let icon = crate::assets::chrome_icon(crate::assets::icons::DOCUMENTS);
        let title = gtk::Label::new(None);
        title.add_css_class("preview-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        let open = gtk::Button::builder()
            .tooltip_text(crate::i18n::tr("Open in default application"))
            .valign(gtk::Align::Center)
            .build();
        super::super::accessibility::set_label(
            &open,
            &crate::i18n::tr("Open in default application"),
        );
        open.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::EXTERNAL_LINK,
        )));
        open.add_css_class("preview-header-action");
        open.set_visible(allow_external_open);
        let model_spinner = gtk::Spinner::builder()
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        model_spinner.add_css_class("preview-spinner");
        model_spinner.add_css_class("preview-header-spinner");
        let document_view_icon = crate::assets::primary_icon(crate::assets::icons::FILE_CODE, 16);
        let document_view_button = gtk::Button::builder()
            .tooltip_text(crate::i18n::tr("View source"))
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        document_view_button.set_child(Some(&document_view_icon));
        document_view_button.add_css_class("preview-header-action");
        super::super::accessibility::set_label(
            &document_view_button,
            &crate::i18n::tr("View source"),
        );
        let wrap = gtk::ToggleButton::builder()
            .tooltip_text(crate::i18n::tr("Toggle word wrap"))
            .valign(gtk::Align::Center)
            .build();
        super::super::accessibility::set_label(&wrap, &crate::i18n::tr("Toggle word wrap"));
        wrap.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::WRAP_TEXT,
        )));
        wrap.add_css_class("preview-header-action");
        wrap.set_visible(false);
        let close = gtk::Button::builder().valign(gtk::Align::Center).build();
        super::super::preferences::PreferenceManager::shared().bind_preference(
            &close,
            super::super::preferences::PreferenceManager::shown_keys,
            |close, _| {
                let label = crate::ui::shortcut_reference::with_keys(
                    "Close preview (%{keys})",
                    keys::Keys::One(PREVIEW.shown()),
                );
                close.set_tooltip_text(Some(&label));
                super::super::accessibility::set_label(close, &label);
            },
        );
        close.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::PANEL_RIGHT_CLOSE,
        )));
        close.add_css_class("preview-close");
        close.add_css_class("preview-header-action");
        let header_handle = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header_handle.add_css_class("preview-header-handle");
        header_handle.set_hexpand(true);
        header_handle.set_margin_end(8);
        header_handle.set_cursor_from_name(Some("grab"));
        header_handle.append(&icon);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        heading.set_hexpand(true);
        heading.set_valign(gtk::Align::Center);
        heading.append(&title);
        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        metadata.add_css_class("preview-metadata");
        let size = metadata_value("Size");
        let modified = metadata_value("Modified");
        let content_type = metadata_value("Type");
        content_type.set_hexpand(true);
        metadata.append(&size);
        metadata.append(&gtk::Label::new(Some("·")));
        metadata.append(&modified);
        metadata.append(&gtk::Label::new(Some("·")));
        metadata.append(&content_type);
        heading.append(&metadata);
        header_handle.append(&heading);
        header.append(&header_handle);
        let find_button = find::icon_button("Find in preview", crate::assets::icons::SEARCH);
        find_button.set_visible(false);
        header.append(&document_view_button);
        header.append(&find_button);
        header.append(&model_spinner);
        header.append(&open);
        header.append(&wrap);
        header.append(&close);
        pane.append(&header);
        let find = find::PreviewFind::new();
        pane.append(&find.widget);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("preview-content");
        content.set_vexpand(true);
        pane.append(&content);

        let raw_details = super::super::raw_details::RawDetails::new();
        raw_details.section.set_margin_start(16);
        raw_details.section.set_margin_end(16);
        raw_details.section.set_margin_bottom(12);
        let raw_details_scroll = gtk::ScrolledWindow::builder()
            .child(&raw_details.section)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(240)
            .visible(false)
            .build();
        raw_details_scroll.add_css_class("media-details-scroll");
        pane.append(&raw_details_scroll);

        let revealer = gtk::Revealer::builder()
            .child(&pane)
            .transition_duration(0)
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .reveal_child(false)
            .build();

        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        // The pane keeps its resting width while the drawer slides, so clip it here.
        slot.set_overflow(gtk::Overflow::Hidden);
        revealer.set_hexpand(true);
        let resize_grip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        resize_grip.add_css_class("preview-resize-grip");
        resize_grip.set_width_request(layout::RESIZE_GRIP_WIDTH);
        resize_grip.set_halign(gtk::Align::Start);
        resize_grip.set_cursor_from_name(Some("col-resize"));
        let slot_overlay = gtk::Overlay::new();
        slot_overlay.set_hexpand(true);
        slot_overlay.set_child(Some(&revealer));
        slot_overlay.add_overlay(&resize_grip);
        slot.append(&slot_overlay);

        let state = Rc::new(PreviewState {
            provider,
            revealer,
            slot,
            resize_grip,
            reserve_columns: Cell::new(true),
            enabled: Cell::new(false),
            dismissed: Cell::new(false),
            child_pane: Cell::new(false),
            pane,
            header_handle: header_handle.clone(),
            icon,
            title,
            size,
            modified,
            content_type,
            content,
            document_view_button: document_view_button.clone(),
            document_view_icon,
            document_view: Cell::new(DocumentView::Rendered),
            document_preview: RefCell::new(None),
            source_preview: SourcePreviewView::new(),
            rendered_text: RefCell::new(None),
            find,
            find_button,
            metadata,
            raw_details,
            raw_details_scroll,
            raw_metadata_load: RefCell::new(None),
            open: open.clone(),
            allow_external_open,
            wrap: wrap.clone(),
            text_view: RefCell::new(None),
            text_scroll: RefCell::new(None),
            archive_browser: RefCell::new(None),
            password_entry: RefCell::new(None),
            media: RefCell::new(None),
            media_signals: RefCell::new(Vec::new()),
            media_volume_slider: RefCell::new(None),
            media_toggle_mute: RefCell::new(None),
            split: RefCell::new(None),
            sizing: layout::SplitSizing::default(),
            current: RefCell::new(None),
            current_depth: Cell::new(None),
            pending_show: RefCell::new(None),
            load: RefCell::new(None),
            loading_delay: RefCell::new(None),
            pdf_loads: Rc::new(RefCell::new(HashMap::new())),
            current_request: Cell::new(None),
            next_request: Cell::new(1),
            focus_archive_on_ready: Cell::new(false),
            focus_archive_request: Cell::new(None),
            enabled_action: gio::SimpleAction::new_stateful(
                "preview-panel",
                None,
                &false.to_variant(),
            ),
            animating: Cell::new(false),
            closing: Cell::new(false),
            animation_generation: Rc::new(Cell::new(0)),
            keyboard_view: RefCell::new(None),
            audio: RefCell::new(None),
            video: RefCell::new(None),
            model: RefCell::new(None),
            route: RefCell::new(None),
            model_spinner,
            model_view: Cell::default(),
            handed_off: RefCell::new(None),
            continue_playback: RefCell::new(None),
            claim_on_resume: Cell::new(false),
            released_from: Cell::new(None),
            operations: RefCell::new(None),
        });
        let weak = Rc::downgrade(&state);
        state.enabled_action.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                let (entry, depth) = state.selected_entry();
                state.toggle_panel(entry, depth);
            }
        });
        let weak = Rc::downgrade(&state);
        state.enabled_action.connect_change_state(move |_, value| {
            if let Some(state) = weak.upgrade()
                && let Some(enabled) = value.and_then(|value| value.get::<bool>())
                && Some(enabled)
                    != state
                        .enabled_action
                        .state()
                        .and_then(|value| value.get::<bool>())
            {
                let (entry, depth) = state.selected_entry();
                state.toggle_panel(entry, depth);
            }
        });
        install_preview_drag(&header_handle, &state);
        state.install_keyboard_ownership();
        state.install_find();
        let weak = Rc::downgrade(&state);
        document_view_button.connect_clicked(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let next = match state.document_view.get() {
                DocumentView::Rendered => DocumentView::Source,
                DocumentView::Source => DocumentView::Rendered,
            };
            state.show_document_view(next);
        });
        let weak = Rc::downgrade(&state);
        state.pane.connect_unrealize(move |_| {
            if let Some(state) = weak.upgrade() {
                state.stop();
            }
        });
        let preferences = super::super::preferences::PreferenceManager::shared();
        let weak = Rc::downgrade(&state);
        preferences.bind_preference(
            &wrap,
            super::super::preferences::PreferenceManager::preview_text_wrap,
            move |_, wrapped| {
                if let Some(state) = weak.upgrade() {
                    state.apply_text_wrap(wrapped);
                    state.wrap.set_active(wrapped);
                }
            },
        );
        wrap.connect_toggled(move |button| {
            if preferences.preview_text_wrap() != button.is_active() {
                preferences.set_preview_text_wrap(button.is_active());
            }
        });
        let weak = Rc::downgrade(&state);
        super::super::theme::ThemeManager::shared().bind_theme_preference(
            &state.pane,
            |manager| manager.active_model_palette(),
            move |_, _| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let entry = state.current.borrow().clone();
                if let Some(entry) = entry
                    && (crate::services::is_model(&entry)
                        || crate::services::route_preview::RouteFormat::for_name(
                            &entry.native_name,
                        )
                        .is_some())
                    && state.current_request.get().is_some()
                {
                    if state.sizing.is_suspended() {
                        state.sizing.defer_load();
                    } else if state.revealer.reveals_child() {
                        state.load(entry, 0);
                    }
                }
            },
        );
        let weak = Rc::downgrade(&state);
        close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close();
            }
        });

        Self { state }
    }

    /// Remember opens even before render: double-click activation can outrun
    /// the first click's preview load and otherwise autoplay beside the player.
    pub(in crate::ui) fn prepare_handoff(
        &self,
        location: &crate::model::Location,
    ) -> Option<Duration> {
        let state = &self.state;
        state.handed_off.replace(Some(location.clone()));
        if state
            .current
            .borrow()
            .as_ref()
            .is_none_or(|entry| entry.location != *location)
        {
            return None;
        }
        let media = state.media.borrow().clone()?;
        media.set_playing(false);
        media
            .downcast_ref::<super::super::media::DecodedMedia>()?
            .video_size()?;
        let position = media.timestamp().max(0) as u64;
        let duration = media.duration().max(0) as u64;
        let inside = position > HANDOFF_MARGIN_US
            && (duration == 0 || position.saturating_add(HANDOFF_MARGIN_US) < duration);
        inside.then(|| Duration::from_micros(position))
    }

    pub(in crate::ui) fn follow_operations(&self, browser: &Rc<Browser>) {
        let weak_state = Rc::downgrade(&self.state);
        let weak_browser = Rc::downgrade(browser);
        let subscription = file_activity::observe(move |notice| {
            let Some(state) = weak_state.upgrade() else {
                return;
            };
            match notice {
                Notice::Releasing(_) => {
                    let changing = state.current.borrow().as_ref().is_some_and(|entry| {
                        entry
                            .local_thumbnail_path()
                            .is_some_and(|path| notice.covers(path))
                    });
                    if changing {
                        let depth = state.current_depth.get();
                        state.clear_target();
                        state.released_from.set(Some(depth));
                    }
                }
                Notice::Lifted(_) => {
                    let Some(depth) = state.released_from.get() else {
                        return;
                    };
                    let Some(browser) = weak_browser.upgrade() else {
                        return;
                    };
                    let depth = depth.or_else(|| browser.active_depth());
                    let entry = depth
                        .and_then(|depth| browser.cursor_entry(depth))
                        .and_then(|entry| preview_target(Some(entry)));
                    if entry
                        .as_ref()
                        .and_then(FileEntry::local_thumbnail_path)
                        .is_some_and(file_activity::is_changing)
                    {
                        return;
                    }
                    state.released_from.set(None);
                    if let Some(entry) = entry.filter(|_| state.is_enabled()) {
                        state.show_after_focus_change(entry, depth);
                    }
                }
                Notice::Ended(_) => {}
            }
        });
        self.state.operations.replace(Some(subscription));
    }

    pub fn observe_browser(&self, browser: &Rc<Browser>) {
        self.follow_operations(browser);
        let weak_state = Rc::downgrade(&self.state);
        let weak_browser = Rc::downgrade(browser);
        self.state.open.connect_clicked(move |_| {
            let (Some(state), Some(browser)) = (weak_state.upgrade(), weak_browser.upgrade())
            else {
                return;
            };
            let Some(location) = state
                .current
                .borrow()
                .as_ref()
                .map(|entry| entry.location.clone())
            else {
                return;
            };
            let position = PreviewDrawer {
                state: state.clone(),
            }
            .prepare_handoff(&location);
            super::super::browser::open_location_at(&location, position, &state.pane, &browser);
        });
        let preview = self.clone();
        let weak_browser = Rc::downgrade(browser);
        browser.observe(move |event| {
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            preview.handle_browser_event(&browser, event);
        });
    }

    pub fn handle_browser_event(&self, browser: &Browser, event: &BrowserEvent) {
        match event {
            BrowserEvent::PreviewRequested { entry, automatic } => {
                if *automatic && self.state.dismissed.get() {
                    return;
                }
                self.show(entry.clone(), browser.active_depth());
            }
            BrowserEvent::SelectionSynced { .. }
                if super::super::marquee::is_updating_selection() => {}
            BrowserEvent::FocusChanged {
                depth,
                position: Some(position),
                ..
            }
            | BrowserEvent::SelectionSynced {
                depth,
                focused: Some(position),
            }
            | BrowserEvent::SelectionSetChanged {
                depth,
                focused: position,
                ..
            } if self.is_enabled() => {
                let entry = browser.entry_at(*depth, *position);
                if let Some(entry) = entry.clone().and_then(|entry| preview_target(Some(entry))) {
                    self.show_after_focus_change(entry, Some(*depth));
                } else {
                    self.state
                        .lend_slot_to_child(browser, *depth, entry.as_ref());
                    self.clear_target();
                }
            }
            BrowserEvent::FocusChanged {
                depth,
                position: None,
                ..
            }
            | BrowserEvent::SelectionSynced {
                depth,
                focused: None,
            } if self.is_enabled() => {
                self.state.lend_slot_to_child(browser, *depth, None);
                self.clear_target()
            }
            BrowserEvent::EntriesSpliced { depth, splices }
                if self.is_enabled()
                    && self.state.current_depth.get() == Some(*depth)
                    && splices.iter().any(|splice| splice.removed > 0) =>
            {
                let Some(current) = self.state.current.borrow().clone() else {
                    return;
                };
                let still_present = (0..)
                    .map_while(|position| browser.entry_at(*depth, position))
                    .any(|entry| entry.location == current.location);
                if !still_present {
                    self.clear_target();
                }
            }
            _ => {}
        }
    }

    pub(in crate::ui) fn handle_video_key(&self, press: &KeyPress) -> bool {
        let command = [
            (MEDIA_PLAY_PAUSE, MediaCommand::PlayPause),
            (
                MEDIA_SEEK_BACK,
                MediaCommand::Seek(MediaSeek::By(-MEDIA_SEEK_STEP_US)),
            ),
            (
                MEDIA_SEEK_FORWARD,
                MediaCommand::Seek(MediaSeek::By(MEDIA_SEEK_STEP_US)),
            ),
            (MEDIA_VOLUME_UP, MediaCommand::VolumeUp),
            (MEDIA_VOLUME_DOWN, MediaCommand::VolumeDown),
            (MEDIA_MUTE, MediaCommand::Mute),
            (MEDIA_PREVIOUS, MediaCommand::Previous),
            (MEDIA_NEXT, MediaCommand::Next),
        ]
        .into_iter()
        .find(|(binding, _)| press.is(*binding));
        command.is_some_and(|(_, command)| self.has_video() && self.state.media_command(command))
    }
}
