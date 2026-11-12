// SPDX-License-Identifier: MIT

use super::*;

mod header;

use header::Header;

impl PreviewDrawer {
    pub fn new(provider: Rc<dyn PreviewProvider>, allow_external_open: bool) -> Self {
        let pane = super::super::accessibility::pane_box();
        pane.add_css_class("preview-pane");
        super::super::accessibility::set_label(&pane, &crate::i18n::tr(PREVIEW_LABEL));
        pane.set_size_request(MIN_WIDTH, -1);
        pane.set_hexpand(true);
        pane.set_vexpand(true);

        let header = Header::new(allow_external_open);
        pane.append(&header.row);
        let find = find::PreviewFind::new();
        pane.append(&find.widget);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("preview-content");
        content.set_vexpand(true);
        pane.append(&content);

        let (raw_details, raw_details_scroll) = raw_details_section();
        pane.append(&raw_details_scroll);
        let (revealer, slot, resize_grip) = drawer_slot(&pane);

        let state = PreviewState::new(DrawerParts {
            provider,
            allow_external_open,
            pane,
            header: &header,
            find,
            content,
            raw_details,
            raw_details_scroll,
            revealer,
            slot,
            resize_grip,
        });
        connect_panel_action(&state);
        install_preview_drag(&header.handle, &state);
        state.install_keyboard_ownership();
        state.install_find();
        let weak = Rc::downgrade(&state);
        header.document_view_button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.toggle_document_view();
            }
        });
        let weak = Rc::downgrade(&state);
        state.pane.connect_unrealize(move |_| {
            if let Some(state) = weak.upgrade() {
                state.stop();
            }
        });
        bind_text_wrap(&header.wrap, &state);
        let weak = Rc::downgrade(&state);
        super::super::theme::ThemeManager::shared().bind_theme_preference(
            &state.pane,
            |manager| manager.active_model_palette(),
            move |_, _| {
                if let Some(state) = weak.upgrade() {
                    state.reload_for_palette();
                }
            },
        );
        let weak = Rc::downgrade(&state);
        header.close.connect_clicked(move |_| {
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
                Notice::Releasing(_) => state.release_for_operation(notice),
                Notice::Lifted(_) => state.resume_after_operation(&weak_browser),
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
            let Some(state) = weak_state.upgrade() else {
                return;
            };
            let Some(browser) = state
                .sizing
                .browser()
                .map(|view| view.browser())
                .or_else(|| weak_browser.upgrade())
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
        self.observe_events(browser);
    }

    pub(super) fn observe_events(&self, browser: &Rc<Browser>) {
        let preview = self.clone();
        let weak_browser = Rc::downgrade(browser);
        browser.observe(move |event| {
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if preview.state.follows(&browser) {
                preview.handle_browser_event(&browser, event);
            }
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
            } if self.is_enabled() => self.follow_focus(browser, *depth, *position),
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
            BrowserEvent::EntriesSpliced { depth, splices } if self.is_enabled() => {
                self.follow_removal(browser, *depth, splices);
            }
            _ => {}
        }
    }

    fn follow_focus(&self, browser: &Browser, depth: usize, position: usize) {
        let entry = browser.entry_at(depth, position);
        if let Some(entry) = entry.clone().and_then(|entry| preview_target(Some(entry))) {
            self.show_after_focus_change(entry, Some(depth));
        } else {
            self.state
                .lend_slot_to_child(browser, depth, entry.as_ref());
            self.clear_target();
        }
    }

    /// Clears the preview when its file is spliced out of the shown listing.
    fn follow_removal(&self, browser: &Browser, depth: usize, splices: &[crate::app::EntrySplice]) {
        let removes = splices.iter().any(|splice| splice.removed > 0);
        if self.state.current_depth.get() != Some(depth) || !removes {
            return;
        }
        let Some(current) = self.state.current.borrow().clone() else {
            return;
        };
        let still_present = (0..)
            .map_while(|position| browser.entry_at(depth, position))
            .any(|entry| entry.location == current.location);
        if !still_present {
            self.clear_target();
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

/// Widgets the drawer builds before the preview state exists.
struct DrawerParts<'a> {
    provider: Rc<dyn PreviewProvider>,
    allow_external_open: bool,
    pane: gtk::Box,
    header: &'a Header,
    find: find::PreviewFind,
    content: gtk::Box,
    raw_details: super::super::raw_details::RawDetails,
    raw_details_scroll: gtk::ScrolledWindow,
    revealer: gtk::Revealer,
    slot: gtk::Box,
    resize_grip: gtk::Box,
}

impl PreviewState {
    fn new(parts: DrawerParts) -> Rc<Self> {
        let header = parts.header;
        Rc::new(PreviewState {
            provider: parts.provider,
            revealer: parts.revealer,
            slot: parts.slot,
            resize_grip: parts.resize_grip,
            reserve_columns: Cell::new(true),
            enabled: Cell::new(false),
            dismissed: Cell::new(false),
            child_pane: Cell::new(false),
            pane: parts.pane,
            header_handle: header.handle.clone(),
            icon: header.icon.clone(),
            title: header.title.clone(),
            size: header.metadata.size.clone(),
            modified: header.metadata.modified.clone(),
            content_type: header.metadata.content_type.clone(),
            content: parts.content,
            document_view_button: header.document_view_button.clone(),
            document_view_icon: header.document_view_icon.clone(),
            document_view: Cell::new(DocumentView::Rendered),
            document_preview: RefCell::new(None),
            source_preview: SourcePreviewView::new(),
            rendered_text: RefCell::new(None),
            find: parts.find,
            find_button: header.find_button.clone(),
            metadata: header.metadata.row.clone(),
            raw_details: parts.raw_details,
            raw_details_scroll: parts.raw_details_scroll,
            raw_metadata_load: RefCell::new(None),
            open: header.open.clone(),
            allow_external_open: parts.allow_external_open,
            wrap: header.wrap.clone(),
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
            model_spinner: header.model_spinner.clone(),
            model_view: Cell::default(),
            handed_off: RefCell::new(None),
            continue_playback: RefCell::new(None),
            media_checks: RefCell::default(),
            claim_on_resume: Cell::new(false),
            released_from: Cell::new(None),
            operations: RefCell::new(None),
        })
    }

    fn toggle_document_view(self: &Rc<Self>) {
        let next = match self.document_view.get() {
            DocumentView::Rendered => DocumentView::Source,
            DocumentView::Source => DocumentView::Rendered,
        };
        self.show_document_view(next);
    }

    /// Models and routes render with the theme's palette, so a palette change re-renders them.
    fn reload_for_palette(self: &Rc<Self>) {
        let entry = self.current.borrow().clone();
        let Some(entry) = entry.filter(uses_model_palette) else {
            return;
        };
        if self.current_request.get().is_none() {
            return;
        }
        if self.sizing.is_suspended() {
            self.sizing.defer_load();
        } else if self.revealer.reveals_child() {
            self.load(entry, 0);
        }
    }

    /// A file an operation is about to change leaves the preview until it is done.
    fn release_for_operation(&self, notice: &Notice) {
        let changing = self.current.borrow().as_ref().is_some_and(|entry| {
            entry
                .local_thumbnail_path()
                .is_some_and(|path| notice.covers(path))
        });
        if changing {
            let depth = self.current_depth.get();
            self.clear_target();
            self.released_from.set(Some(depth));
        }
    }

    fn resume_after_operation(self: &Rc<Self>, browser: &std::rc::Weak<Browser>) {
        let Some(depth) = self.released_from.get() else {
            return;
        };
        let Some(browser) = browser.upgrade() else {
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
        self.released_from.set(None);
        if let Some(entry) = entry.filter(|_| self.is_enabled()) {
            self.show_after_focus_change(entry, depth);
        }
    }
}

fn uses_model_palette(entry: &FileEntry) -> bool {
    crate::services::is_model(entry)
        || crate::services::route_preview::RouteFormat::for_name(&entry.native_name).is_some()
}

fn connect_panel_action(state: &Rc<PreviewState>) {
    let weak = Rc::downgrade(state);
    state.enabled_action.connect_activate(move |_, _| {
        if let Some(state) = weak.upgrade() {
            let (entry, depth) = state.selected_entry();
            state.toggle_panel(entry, depth);
        }
    });
    let weak = Rc::downgrade(state);
    state.enabled_action.connect_change_state(move |_, value| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Some(enabled) = value.and_then(|value| value.get::<bool>()) else {
            return;
        };
        let current = state
            .enabled_action
            .state()
            .and_then(|value| value.get::<bool>());
        if Some(enabled) != current {
            let (entry, depth) = state.selected_entry();
            state.toggle_panel(entry, depth);
        }
    });
}

fn bind_text_wrap(wrap: &gtk::ToggleButton, state: &Rc<PreviewState>) {
    let preferences = super::super::preferences::PreferenceManager::shared();
    let weak = Rc::downgrade(state);
    preferences.bind_preference(
        wrap,
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
}

fn raw_details_section() -> (super::super::raw_details::RawDetails, gtk::ScrolledWindow) {
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
    (raw_details, raw_details_scroll)
}

/// The revealer that slides the pane, inside the clipping slot with its resize grip.
fn drawer_slot(pane: &gtk::Box) -> (gtk::Revealer, gtk::Box, gtk::Box) {
    let revealer = gtk::Revealer::builder()
        .child(pane)
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
    (revealer, slot, resize_grip)
}
