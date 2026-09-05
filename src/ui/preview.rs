// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use sourceview5::prelude::*;

use crate::{
    model::{FileEntry, MetadataValue},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        PreviewRequestId,
    },
};

const DEFAULT_WIDTH: i32 = 520;
const MIN_WIDTH: i32 = 280;
const MAX_WIDTH: i32 = 3_000;
const TEXT_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_SPINNER_DELAY: Duration = Duration::from_millis(120);
const TRANSITION: Duration = Duration::from_millis(260);
const PDF_PAGE_GAP: i32 = 6;
const PDF_MIN_ZOOM: f64 = 1.0;
const PDF_MAX_ZOOM: f64 = 4.0;
const MEDIA_PLUGIN_INSTALL_COMMAND: &str = "sudo pacman -S --needed gst-plugins-good gst-libav";

struct PreviewState {
    provider: Rc<dyn PreviewProvider>,
    revealer: gtk::Revealer,
    pane: gtk::Box,
    header_handle: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    size: gtk::Label,
    modified: gtk::Label,
    content_type: gtk::Label,
    content: gtk::Box,
    media: RefCell<Option<gtk::MediaStream>>,
    media_signals: RefCell<Vec<glib::SignalHandlerId>>,
    media_volume_slider: RefCell<Option<gtk::Scale>>,
    media_volume_icon: RefCell<Option<gtk::Image>>,
    media_volume_save: Rc<RefCell<Option<glib::SourceId>>>,
    media_toggle_mute: RefCell<Option<Rc<dyn Fn()>>>,
    split: RefCell<Option<gtk::Paned>>,
    occupied_width: RefCell<Option<Rc<dyn Fn() -> i32>>>,
    current: RefCell<Option<FileEntry>>,
    load: RefCell<Option<LoadHandle>>,
    loading_delay: RefCell<Option<glib::SourceId>>,
    pdf_loads: Rc<RefCell<HashMap<i32, LoadHandle>>>,
    current_request: Cell<Option<PreviewRequestId>>,
    next_request: Cell<u64>,
    opened: Cell<bool>,
    last_split_width: Cell<i32>,
    animating: Cell<bool>,
    animation_generation: Rc<Cell<u64>>,
}

#[derive(Clone)]
pub struct PreviewDrawer {
    state: Rc<PreviewState>,
}

impl PreviewDrawer {
    pub fn new(provider: Rc<dyn PreviewProvider>) -> Self {
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pane.add_css_class("preview-pane");
        pane.set_size_request(MIN_WIDTH, -1);
        pane.set_hexpand(true);
        pane.set_vexpand(true);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("preview-header");
        let icon = crate::assets::primary_icon(crate::assets::icons::DOCUMENTS, 18);
        let title = gtk::Label::new(None);
        title.add_css_class("preview-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        let open = gtk::Button::builder()
            .tooltip_text("Open in default application")
            .valign(gtk::Align::Center)
            .build();
        open.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::EXTERNAL_LINK,
            16,
        )));
        open.add_css_class("preview-header-action");
        let close = gtk::Button::builder()
            .tooltip_text("Close preview (Space)")
            .valign(gtk::Align::Center)
            .build();
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        close.add_css_class("preview-close");
        close.add_css_class("preview-header-action");
        let header_handle = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header_handle.add_css_class("preview-header-handle");
        header_handle.set_hexpand(true);
        header_handle.set_cursor_from_name(Some("grab"));
        header_handle.append(&icon);
        header_handle.append(&title);
        header.append(&header_handle);
        header.append(&open);
        header.append(&close);
        pane.append(&header);

        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        metadata.add_css_class("preview-metadata");
        let (size_group, size) = metadata_value("SIZE");
        let (modified_group, modified) = metadata_value("MODIFIED");
        let (type_group, content_type) = metadata_value("TYPE");
        metadata.append(&size_group);
        metadata.append(&modified_group);
        metadata.append(&type_group);
        pane.append(&metadata);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("preview-content");
        content.set_vexpand(true);
        pane.append(&content);

        let revealer = gtk::Revealer::builder()
            .child(&pane)
            .transition_duration(0)
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .reveal_child(false)
            .build();

        let state = Rc::new(PreviewState {
            provider,
            revealer,
            pane,
            header_handle: header_handle.clone(),
            icon,
            title,
            size,
            modified,
            content_type,
            content,
            media: RefCell::new(None),
            media_signals: RefCell::new(Vec::new()),
            media_volume_slider: RefCell::new(None),
            media_volume_icon: RefCell::new(None),
            media_volume_save: Rc::new(RefCell::new(None)),
            media_toggle_mute: RefCell::new(None),
            split: RefCell::new(None),
            occupied_width: RefCell::new(None),
            current: RefCell::new(None),
            load: RefCell::new(None),
            loading_delay: RefCell::new(None),
            pdf_loads: Rc::new(RefCell::new(HashMap::new())),
            current_request: Cell::new(None),
            next_request: Cell::new(1),
            opened: Cell::new(false),
            last_split_width: Cell::new(0),
            animating: Cell::new(false),
            animation_generation: Rc::new(Cell::new(0)),
        });
        install_preview_drag(&header_handle, &state);
        let weak = Rc::downgrade(&state);
        open.connect_clicked(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let location = state
                .current
                .borrow()
                .as_ref()
                .map(|entry| entry.location.clone());
            if let Some(location) = location {
                if let Some(stream) = state.media.borrow().as_ref() {
                    stream.set_playing(false);
                }
                super::browser::open_location(&location, &state.pane);
            }
        });
        let weak = Rc::downgrade(&state);
        close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close();
            }
        });

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.revealer.clone().upcast()
    }

    pub fn attach_split(&self, split: &gtk::Paned, occupied_width: Rc<dyn Fn() -> i32>) {
        self.state.split.replace(Some(split.clone()));
        self.state.occupied_width.replace(Some(occupied_width));
        if !self.state.opened.get() {
            split.set_end_child(None::<&gtk::Widget>);
        }
        let weak = Rc::downgrade(&self.state);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let available = split.width();
            if available > 0
                && available != state.last_split_width.replace(available)
                && state.opened.get()
                && !state.animating.get()
            {
                let opening_width = state.opening_width(available);
                split.set_position(available.saturating_sub(opening_width));
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn is_open(&self) -> bool {
        self.state.opened.get()
    }

    pub fn has_video(&self) -> bool {
        self.is_open() && self.state.media.borrow().is_some()
    }

    pub fn handle_video_key(&self, key: gtk::gdk::Key) -> bool {
        let media = match self.state.media.borrow().as_ref() {
            Some(m) => m.clone(),
            None => return false,
        };
        let preferences = super::theme::ThemeManager::shared();
        let slider = self.state.media_volume_slider.borrow().clone();
        let icon = self.state.media_volume_icon.borrow().clone();
        let fallback = gtk::Image::new();
        let icon = icon.as_ref().unwrap_or(&fallback);
        match key {
            gtk::gdk::Key::space => {
                if media.is_playing() {
                    media.pause();
                } else {
                    media.play();
                }
                true
            }
            gtk::gdk::Key::Up | gtk::gdk::Key::Down => {
                let delta = if matches!(key, gtk::gdk::Key::Up) {
                    0.1
                } else {
                    -0.1
                };
                let current_vol = if preferences.preview_muted() {
                    0.0
                } else {
                    preferences.preview_volume()
                };
                let volume = (current_vol + delta).clamp(0.0, 1.0);
                set_preview_volume(&media, &preferences, &slider, icon, volume);
                true
            }
            gtk::gdk::Key::m | gtk::gdk::Key::M => {
                if let Some(toggle_volume) = self.state.media_toggle_mute.borrow().as_ref() {
                    toggle_volume();
                    true
                } else {
                    false
                }
            }
            gtk::gdk::Key::Left | gtk::gdk::Key::Right if media.is_seekable() => {
                let delta: i64 = if matches!(key, gtk::gdk::Key::Right) {
                    5_000_000
                } else {
                    -5_000_000
                };
                let target = (media.timestamp() + delta).max(0);
                media.seek(target);
                true
            }
            _ => false,
        }
    }

    pub fn show(&self, entry: FileEntry) {
        self.state.show(entry);
    }

    pub fn close(&self) {
        self.state.close();
    }

    pub fn toggle(&self, entry: Option<FileEntry>) {
        if self.is_open() {
            self.close();
        } else if let Some(entry) = entry {
            self.show(entry);
        }
    }
}

impl PreviewState {
    fn show(self: &Rc<Self>, entry: FileEntry) {
        let was_open = self.opened.replace(true);
        let already_showing = self.current.borrow().as_ref() == Some(&entry);
        if !was_open {
            self.revealer.set_transition_duration(0);
            self.pane.set_size_request(0, -1);
            self.revealer.set_reveal_child(true);
            if let Some(split) = self.split.borrow().as_ref() {
                split.set_end_child(Some(&self.revealer));
                self.animate_open(split);
            }
        }
        if !was_open || !already_showing {
            self.load(entry, 0);
        }
    }

    fn animate_open(self: &Rc<Self>, split: &gtk::Paned) {
        let available = split.width();
        if available <= MIN_WIDTH {
            return;
        }
        self.last_split_width.set(available);
        let target = available.saturating_sub(self.opening_width(available));
        let start = available;
        split.set_position(start);
        let animation_id = self.animation_generation.get().saturating_add(1);
        self.animation_generation.set(animation_id);
        self.animating.set(true);

        if !super::motion::animations_enabled() {
            split.set_position(target);
            self.pane.set_size_request(MIN_WIDTH, -1);
            self.animating.set(false);
            return;
        }

        let started = Instant::now();
        let split = split.clone();
        let pane = self.pane.clone();
        let generation = self.animation_generation.clone();
        let weak = Rc::downgrade(self);
        let _tick = split.clone().add_tick_callback(move |_, _| {
            if generation.get() != animation_id {
                return glib::ControlFlow::Break;
            }
            let progress =
                (started.elapsed().as_secs_f64() / TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
            let eased = super::motion::emphasized_deceleration(progress);
            let position = f64::from(start) + f64::from(target - start) * eased;
            split.set_position(position.round() as i32);
            if progress >= 1.0 {
                split.set_position(target);
                pane.set_size_request(MIN_WIDTH, -1);
                if let Some(state) = weak.upgrade() {
                    state.animating.set(false);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn opening_width(&self, available: i32) -> i32 {
        let occupied_width = self
            .occupied_width
            .borrow()
            .as_ref()
            .map_or(available.saturating_sub(DEFAULT_WIDTH), |width| width())
            .clamp(0, available);
        let desired_width = preview_width_for_empty_space(available, occupied_width);
        let maximum_width = MAX_WIDTH.min(available.saturating_sub(MIN_WIDTH).max(MIN_WIDTH));
        desired_width.clamp(MIN_WIDTH, maximum_width)
    }

    fn close(self: &Rc<Self>) {
        self.opened.set(false);
        self.animating.set(false);
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.current_request.set(None);
        self.load.borrow_mut().take();
        self.cancel_loading();
        self.pdf_loads.borrow_mut().clear();
        self.clear_content();
        self.revealer.set_transition_duration(0);
        self.revealer.set_reveal_child(false);
        if let Some(split) = self.split.borrow().as_ref() {
            split.set_position(split.width());
            split.set_end_child(None::<&gtk::Widget>);
        }
        self.pane.set_size_request(MIN_WIDTH, -1);
    }

    fn load(self: &Rc<Self>, entry: FileEntry, pdf_page: i32) {
        self.current.replace(Some(entry.clone()));
        crate::assets::set_primary_icon(&self.icon, super::browser::entry_icon(&entry));
        self.title.set_text(&entry.display_name);
        self.title
            .set_tooltip_text(Some(&entry.location.display_path()));
        self.size.set_text(&metadata_size(&entry));
        crate::util::set_modified_date(&self.modified, Some(&entry), "—");
        self.content_type.set_text(file_extension(&entry));
        self.load.borrow_mut().take();
        self.pdf_loads.borrow_mut().clear();

        let request_id = PreviewRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        self.current_request.set(Some(request_id));
        self.show_loading(request_id);
        let weak = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.handle_event(request_id, event);
        });
        let load = self.provider.load(
            PreviewRequest {
                id: request_id,
                entry,
                text_byte_limit: TEXT_BYTE_LIMIT,
                pdf_page,
            },
            emit,
        );
        self.load.replace(Some(load));
    }

    fn handle_event(self: &Rc<Self>, expected: PreviewRequestId, event: PreviewEvent) {
        if self.current_request.get() != Some(expected) {
            return;
        }
        match event {
            PreviewEvent::Ready(preview) if preview.request_id == expected => {
                self.cancel_loading();
                self.render(preview);
            }
            PreviewEvent::Failed {
                request_id,
                entry,
                message,
            } if request_id == expected => {
                self.current_request.set(None);
                self.load.borrow_mut().take();
                self.cancel_loading();
                self.title.set_text(&entry.display_name);
                self.show_message("Preview unavailable", &message);
            }
            PreviewEvent::Ready(_) | PreviewEvent::Failed { .. } => {}
        }
    }

    fn render(self: &Rc<Self>, preview: Preview) {
        self.content_type.set_text(&preview.content_type);
        self.clear_content();
        match preview.content {
            PreviewContent::Text { content, truncated } => {
                let buffer = sourceview5::Buffer::new(None);
                let languages = sourceview5::LanguageManager::default();
                let language = languages.guess_language(
                    preview.entry.location.native_path(),
                    Some(&preview.content_type),
                );
                buffer.set_language(language.as_ref());
                super::theme::register_source_buffer(&buffer);
                buffer.set_highlight_syntax(true);
                buffer.set_text(&content);
                let view = sourceview5::View::builder()
                    .buffer(&buffer)
                    .cursor_visible(false)
                    .editable(false)
                    .highlight_current_line(false)
                    .left_margin(14)
                    .right_margin(14)
                    .top_margin(12)
                    .bottom_margin(12)
                    .monospace(true)
                    .show_line_numbers(true)
                    .wrap_mode(gtk::WrapMode::None)
                    .build();
                view.add_css_class("preview-text");
                let scroll = gtk::ScrolledWindow::builder()
                    .child(&view)
                    .hscrollbar_policy(gtk::PolicyType::Automatic)
                    .vscrollbar_policy(gtk::PolicyType::Automatic)
                    .hexpand(true)
                    .vexpand(true)
                    .build();
                self.content.append(&scroll);
                if truncated {
                    let notice = gtk::Label::new(Some("Preview limited to the first 1 MB"));
                    notice.add_css_class("preview-note");
                    self.content.append(&notice);
                }
            }
            PreviewContent::Rasterized { png } => {
                let bytes = glib::Bytes::from_owned(png);
                match gtk::gdk::Texture::from_bytes(&bytes) {
                    Ok(texture) => {
                        let picture = gtk::Picture::for_paintable(&texture);
                        picture.add_css_class("preview-image");
                        picture.set_can_shrink(true);
                        picture.set_content_fit(gtk::ContentFit::Contain);
                        picture.set_hexpand(true);
                        picture.set_vexpand(true);
                        picture.set_cursor_from_name(Some("grab"));
                        install_preview_drag(&picture, self);
                        self.content.append(&picture);
                    }
                    Err(error) => self.show_message("Preview unavailable", &error.to_string()),
                }
            }
            PreviewContent::SandboxedMedia { data } => {
                let bytes = glib::Bytes::from_owned(data);
                let stream = gio::MemoryInputStream::from_bytes(&bytes);
                let media = gtk::MediaFile::for_input_stream(&stream);
                let is_gif = preview.content_type == "image/gif";
                self.media.replace(Some(media.clone().upcast()));
                let weak = Rc::downgrade(self);
                media.connect_error_notify(move |media| {
                    let Some(error) = media.error() else {
                        return;
                    };
                    if let Some(state) = weak.upgrade() {
                        state.show_media_error(&error);
                    }
                });

                let picture = gtk::Picture::for_paintable(&media);
                picture.add_css_class("preview-media");
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_hexpand(true);
                picture.set_vexpand(true);
                picture.set_cursor_from_name(Some("grab"));
                install_preview_drag(&picture, self);

                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&picture));
                overlay.set_focusable(true);
                overlay.set_can_target(true);

                let center_play = gtk::Button::new();
                center_play.add_css_class("preview-media-center");
                center_play.set_halign(gtk::Align::Center);
                center_play.set_valign(gtk::Align::Center);
                center_play.set_visible(false);
                let center_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 48);
                center_play.set_child(Some(&center_icon));
                overlay.add_overlay(&center_play);

                let media_for_center = media.clone();
                center_play.connect_clicked(move |_| {
                    if media_for_center.is_playing() {
                        media_for_center.pause();
                    } else {
                        media_for_center.play();
                    }
                });

                let media_for_click = media.clone();
                let overlay_for_focus = overlay.clone();
                let click = gtk::GestureClick::new();
                click.connect_pressed(move |_, _, _, _| {
                    overlay_for_focus.grab_focus();
                    if media_for_click.is_playing() {
                        media_for_click.pause();
                    } else {
                        media_for_click.play();
                    }
                });
                picture.add_controller(click);

                self.content.append(&overlay);

                if is_gif {
                    media.set_loop(true);
                    media.play();
                    self.append_media_controls(
                        &media,
                        &super::theme::ThemeManager::shared(),
                        &overlay.upcast(),
                        &center_play,
                        true,
                    );
                } else {
                    let preferences = super::theme::ThemeManager::shared();
                    let muted = preferences.preview_muted();
                    let volume = if muted {
                        0.0
                    } else {
                        preferences.preview_volume()
                    };
                    media.set_volume(volume);
                    media.set_muted(muted);
                    self.append_media_controls(
                        &media,
                        &preferences,
                        &overlay.upcast(),
                        &center_play,
                        false,
                    );
                    media.play();
                }

                if let Some(error) = media.error() {
                    self.show_media_error(&error);
                }
                if !is_gif {
                    let notice = gtk::Label::new(Some(
                        "Preview limited to the first 30 seconds. Open the file to play the full video.",
                    ));
                    notice.add_css_class("preview-note");
                    notice.set_justify(gtk::Justification::Center);
                    notice.set_wrap(true);
                    notice.set_xalign(0.5);
                    self.content.append(&notice);
                }
            }
            PreviewContent::Image | PreviewContent::Media => {
                self.show_message(
                    "Preview unavailable",
                    "The sandboxed renderer returned no preview",
                );
            }
            PreviewContent::Pdf { png, page, pages } => {
                self.render_pdf_viewer(preview.entry, png, page, pages);
            }
            PreviewContent::Unsupported => {
                self.show_message(
                    "No visual preview",
                    "Metadata is available for this file type.",
                );
            }
        }
    }

    fn render_pdf_viewer(
        self: &Rc<Self>,
        entry: FileEntry,
        initial_png: Vec<u8>,
        initial_page: i32,
        pages: i32,
    ) {
        let page_count = pages.clamp(0, 10_000);
        let labels: Vec<_> = (1..=page_count).map(|page| page.to_string()).collect();
        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&labels);
        let selection = gtk::NoSelection::new(Some(model));
        let factory = gtk::SignalListItemFactory::new();
        let zoom = Rc::new(Cell::new(PDF_MIN_ZOOM));
        let page_width = Rc::new(Cell::new(0));
        let visible_pages = Rc::new(RefCell::new(
            HashMap::<i32, (gtk::Overlay, gtk::Picture)>::new(),
        ));

        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let overlay = gtk::Overlay::new();
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            let spinner = gtk::Spinner::new();
            spinner.set_halign(gtk::Align::Center);
            spinner.set_valign(gtk::Align::Center);
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&spinner);
            overlay.set_hexpand(true);
            overlay.set_size_request(-1, 560);
            item.set_child(Some(&overlay));
        });

        let provider = self.provider.clone();
        let loads = self.pdf_loads.clone();
        let initial_page = Rc::new(RefCell::new(Some((initial_page, initial_png))));
        let next_request = Rc::new(Cell::new(self.next_request.get().saturating_add(10_000)));
        let entry_for_bind = entry.clone();
        let page_width_for_bind = page_width.clone();
        let visible_pages_for_bind = visible_pages.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let page_index = item.position() as i32;
            let Some(overlay) = item.child().and_downcast::<gtk::Overlay>() else {
                return;
            };
            let Some(picture) = overlay.child().and_downcast::<gtk::Picture>() else {
                return;
            };
            let Some(spinner) = overlay.last_child().and_downcast::<gtk::Spinner>() else {
                return;
            };
            let binding_name = format!("pdf-page-{page_index}");
            overlay.set_widget_name(&binding_name);
            overlay.set_tooltip_text(None);
            let target_width = page_width_for_bind.get();
            overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
            picture.set_paintable(gtk::gdk::Paintable::NONE);
            spinner.start();
            spinner.set_visible(true);
            visible_pages_for_bind
                .borrow_mut()
                .insert(page_index, (overlay.clone(), picture.clone()));

            let is_initial_page = initial_page
                .borrow()
                .as_ref()
                .is_some_and(|(page, _)| *page == page_index);
            let cached = if is_initial_page {
                initial_page.borrow_mut().take()
            } else {
                None
            };
            if let Some((_, png)) = cached {
                set_pdf_page_texture(&overlay, &picture, png, page_width_for_bind.get());
                spinner.stop();
                spinner.set_visible(false);
                return;
            }

            let request_id = PreviewRequestId(next_request.get());
            next_request.set(next_request.get().saturating_add(1));
            let weak_overlay = overlay.downgrade();
            let weak_picture = picture.downgrade();
            let weak_spinner = spinner.downgrade();
            let loads_for_event = loads.clone();
            let page_width_for_event = page_width_for_bind.clone();
            let emit = Rc::new(move |event| {
                loads_for_event.borrow_mut().remove(&page_index);
                let Some(overlay) = weak_overlay
                    .upgrade()
                    .filter(|overlay| overlay.widget_name() == binding_name)
                else {
                    return;
                };
                match event {
                    PreviewEvent::Ready(Preview {
                        request_id: response_id,
                        content: PreviewContent::Pdf { png, page, .. },
                        ..
                    }) if response_id == request_id && page == page_index => {
                        if let Some(picture) = weak_picture.upgrade() {
                            set_pdf_page_texture(
                                &overlay,
                                &picture,
                                png,
                                page_width_for_event.get(),
                            );
                        }
                    }
                    PreviewEvent::Failed {
                        request_id: response_id,
                        ..
                    } if response_id == request_id => {
                        overlay.set_tooltip_text(Some("Unable to render this PDF page"));
                    }
                    PreviewEvent::Ready(_) | PreviewEvent::Failed { .. } => return,
                }
                if let Some(spinner) = weak_spinner.upgrade() {
                    spinner.stop();
                    spinner.set_visible(false);
                }
            });
            let load = provider.load(
                PreviewRequest {
                    id: request_id,
                    entry: entry_for_bind.clone(),
                    text_byte_limit: TEXT_BYTE_LIMIT,
                    pdf_page: page_index,
                },
                emit,
            );
            loads.borrow_mut().insert(page_index, load);
        });

        let loads = self.pdf_loads.clone();
        let visible_pages_for_unbind = visible_pages.clone();
        factory.connect_unbind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                let page = item.position() as i32;
                loads.borrow_mut().remove(&page);
                visible_pages_for_unbind.borrow_mut().remove(&page);
            }
        });

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("preview-pdf-list");
        list.set_hexpand(true);
        list.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        let zoom_scroll =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        zoom_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let zoom_for_scroll = zoom.clone();
        let page_width_for_scroll = page_width.clone();
        let visible_pages_for_scroll = visible_pages.clone();
        zoom_scroll.connect_scroll(move |controller, _, dy| {
            if !controller
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Stop;
            };
            let previous = zoom_for_scroll.get();
            let next = pdf_zoom_after_scroll(previous, dy);
            if (next - previous).abs() < f64::EPSILON {
                return glib::Propagation::Stop;
            }
            zoom_for_scroll.set(next);
            let width = pdf_page_width(&scroll, next);
            page_width_for_scroll.set(width);
            resize_pdf_pages(&visible_pages_for_scroll.borrow(), width);
            preserve_pdf_view_center(&scroll, next / previous);
            glib::Propagation::Stop
        });
        scroll.add_controller(zoom_scroll);

        let reset_zoom = gtk::EventControllerKey::new();
        reset_zoom.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let zoom_for_reset = zoom.clone();
        let page_width_for_reset = page_width.clone();
        let visible_pages_for_reset = visible_pages.clone();
        reset_zoom.connect_key_pressed(move |_, key, _, modifiers| {
            if key.to_unicode() != Some('0')
                || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Stop;
            };
            zoom_for_reset.set(PDF_MIN_ZOOM);
            let width = pdf_page_width(&scroll, PDF_MIN_ZOOM);
            page_width_for_reset.set(width);
            resize_pdf_pages(&visible_pages_for_reset.borrow(), width);
            set_adjustment_value(&scroll.hadjustment(), 0.0);
            glib::Propagation::Stop
        });
        list.add_controller(reset_zoom);

        scroll.set_cursor_from_name(Some("grab"));
        let drag_origin = Rc::new(Cell::new((0.0, 0.0)));
        let pan = gtk::GestureDrag::new();
        pan.set_button(1);
        pan.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let drag_origin_for_begin = drag_origin.clone();
        pan.connect_drag_begin(move |_, _, _| {
            if let Some(scroll) = weak_scroll.upgrade() {
                scroll.set_cursor_from_name(Some("grabbing"));
                drag_origin_for_begin
                    .set((scroll.hadjustment().value(), scroll.vadjustment().value()));
            }
        });
        let weak_scroll = scroll.downgrade();
        pan.connect_drag_update(move |_, offset_x, offset_y| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            let (horizontal, vertical) = drag_origin.get();
            set_adjustment_value(&scroll.hadjustment(), horizontal - offset_x);
            set_adjustment_value(&scroll.vadjustment(), vertical - offset_y);
        });
        let weak_scroll = scroll.downgrade();
        pan.connect_drag_end(move |_, _, _| {
            if let Some(scroll) = weak_scroll.upgrade() {
                scroll.set_cursor_from_name(Some("grab"));
            }
        });
        scroll.add_controller(pan);

        let zoom_for_tick = zoom.clone();
        let page_width_for_tick = page_width.clone();
        let visible_pages_for_tick = visible_pages.clone();
        scroll.add_tick_callback(move |scroll, _| {
            if scroll.width() > PDF_PAGE_GAP * 2 + 1 {
                let width = pdf_page_width(scroll, zoom_for_tick.get());
                if width != page_width_for_tick.replace(width) {
                    resize_pdf_pages(&visible_pages_for_tick.borrow(), width);
                }
            }
            glib::ControlFlow::Continue
        });
        self.content.append(&scroll);
    }

    fn append_media_controls(
        self: &Rc<Self>,
        media: &gtk::MediaFile,
        preferences: &Rc<super::theme::ThemeManager>,
        _video_area: &gtk::Widget,
        center_play: &gtk::Button,
        is_gif: bool,
    ) {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("preview-media-bar");

        let play_icon = crate::assets::primary_icon(crate::assets::icons::PLAY, 18);
        let pause_icon = crate::assets::primary_icon(crate::assets::icons::PAUSE, 18);
        let play_button = gtk::Button::new();
        play_button.add_css_class("preview-media-button");
        play_button.set_tooltip_text(Some("Play/Pause (Space)"));
        play_button.set_child(Some(if media.is_playing() {
            &pause_icon
        } else {
            &play_icon
        }));
        let media_for_play = media.clone();
        play_button.connect_clicked(move |_| {
            if media_for_play.is_playing() {
                media_for_play.pause();
            } else {
                media_for_play.play();
            }
        });

        bar.append(&play_button);

        let play_button_for_notify = play_button.clone();
        let play_icon_for_notify = play_icon.clone();
        let pause_icon_for_notify = pause_icon.clone();
        let center_for_notify = center_play.clone();
        let media_for_playing = media.clone();
        let handler = media.connect_notify_local(Some("playing"), move |_, _| {
            let playing = media_for_playing.is_playing();
            play_button_for_notify.set_child(Some(if playing {
                &pause_icon_for_notify
            } else {
                &play_icon_for_notify
            }));
            center_for_notify.set_visible(!playing);
        });
        self.media_signals.borrow_mut().push(handler);

        if is_gif {
            self.content.append(&bar);
            return;
        }

        let time_label = gtk::Label::new(Some("0:00 / 0:00"));
        time_label.add_css_class("preview-media-time");
        time_label.set_xalign(0.5);

        let seek = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .draw_value(false)
            .build();
        seek.add_css_class("preview-media-seek");
        seek.set_range(0.0, 0.0);
        seek.set_sensitive(false);

        let volume_toggle = gtk::Button::new();
        volume_toggle.add_css_class("preview-media-button");
        volume_toggle.set_tooltip_text(Some("Mute/unmute (M)"));
        let muted = preferences.preview_muted();
        let volume_icon = crate::assets::primary_icon(
            if muted {
                crate::assets::icons::VOLUME_X
            } else {
                crate::assets::icons::VOLUME_2
            },
            16,
        );
        volume_toggle.set_child(Some(&volume_icon));

        let volume_slider = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .draw_value(false)
            .width_request(72)
            .build();
        volume_slider.add_css_class("preview-media-volume");
        volume_slider.set_range(0.0, 1.0);
        let initial_slider = if preferences.preview_muted() {
            0.0
        } else {
            preferences.preview_volume()
        };
        volume_slider.set_value(initial_slider);

        bar.append(&time_label);
        bar.append(&seek);
        bar.append(&volume_toggle);
        bar.append(&volume_slider);
        self.content.append(&bar);

        self.media_volume_slider
            .replace(Some(volume_slider.clone()));
        self.media_volume_icon.replace(Some(volume_icon.clone()));

        let seeking = Rc::new(Cell::new(false));
        let update_time = {
            let media = media.clone();
            let time_label = time_label.clone();
            let seek = seek.clone();
            let seeking = seeking.clone();
            move || sync_media_time(&media, &time_label, &seek, &seeking)
        };
        update_time();
        let update_for_timestamp = update_time.clone();
        let handler = media.connect_notify_local(Some("timestamp"), move |_, _| {
            update_for_timestamp();
        });
        self.media_signals.borrow_mut().push(handler);
        let update_for_duration = update_time.clone();
        let handler = media.connect_notify_local(Some("duration"), move |_, _| {
            update_for_duration();
        });
        self.media_signals.borrow_mut().push(handler);

        let drag = gtk::GestureDrag::new();
        let seeking_for_begin = seeking.clone();
        drag.connect_drag_begin(move |_, _, _| {
            seeking_for_begin.set(true);
        });
        let seeking_for_end = seeking.clone();
        let media_for_drag_end = media.clone();
        let seek_for_drag_end = seek.clone();
        drag.connect_drag_end(move |_, _, _| {
            seeking_for_end.set(false);
            media_for_drag_end.seek(seek_for_drag_end.value() as i64);
        });
        seek.add_controller(drag);

        let last_volume = Rc::new(Cell::new(preferences.preview_volume().max(0.1)));
        let updating_slider = Rc::new(Cell::new(false));

        let toggle_volume = Rc::new({
            let media = media.clone();
            let icon = volume_icon.clone();
            let preferences = preferences.clone();
            let slider = volume_slider.clone();
            let last_volume = last_volume.clone();
            let updating = updating_slider.clone();
            move || {
                let muted = !preferences.preview_muted();
                updating.set(true);
                if muted {
                    if slider.value() > 0.0 {
                        last_volume.set(slider.value());
                    }
                    media.set_volume(0.0);
                    slider.set_value(0.0);
                    set_preview_mute(&media, &icon, &preferences, true);
                } else {
                    let restored = last_volume.get().max(0.1);
                    media.set_volume(restored);
                    slider.set_value(restored);
                    set_preview_mute(&media, &icon, &preferences, false);
                }
                updating.set(false);
            }
        });
        let toggle_volume_for_click = toggle_volume.clone();
        self.media_toggle_mute.replace(Some(toggle_volume));
        volume_toggle.connect_clicked(move |_| {
            toggle_volume_for_click();
        });

        let media_for_volume = media.clone();
        let icon_for_volume = volume_icon.clone();
        let preferences_for_volume = preferences.clone();
        let last_volume_for_volume = last_volume.clone();
        let updating_for_volume = updating_slider.clone();
        let save_slot = self.media_volume_save.clone();
        let preferences_for_save = preferences.clone();
        volume_slider.connect_value_changed(move |scale| {
            if updating_for_volume.get() {
                return;
            }
            let volume = scale.value();
            media_for_volume.set_volume(volume);
            let muted = volume == 0.0;
            if preferences_for_volume.preview_muted() != muted {
                set_preview_mute(
                    &media_for_volume,
                    &icon_for_volume,
                    &preferences_for_volume,
                    muted,
                );
            }
            if let Some(prev) = save_slot.borrow_mut().take() {
                prev.remove();
            }
            if volume > 0.0 {
                last_volume_for_volume.set(volume);
                let save_slot_for_timeout = save_slot.clone();
                let prefs = preferences_for_save.clone();
                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(400),
                    move || {
                        save_slot_for_timeout.borrow_mut().take();
                        prefs.set_preview_volume(volume);
                    },
                );
                save_slot.borrow_mut().replace(id);
            }
        });
    }

    fn clear_content(&self) {
        if let Some(stream) = self.media.borrow_mut().take() {
            for handler in self.media_signals.borrow_mut().drain(..) {
                stream.disconnect(handler);
            }
            stream.set_playing(false);
        }
        if let Some(id) = self.media_volume_save.borrow_mut().take() {
            id.remove();
        }
        self.media_toggle_mute.replace(None);
        self.media_volume_slider.replace(None);
        self.media_volume_icon.replace(None);
        clear_box(&self.content);
    }

    fn show_loading(self: &Rc<Self>, request_id: PreviewRequestId) {
        self.clear_content();
        self.cancel_loading();
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(PREVIEW_SPINNER_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.loading_delay.borrow_mut().take();
            if state.current_request.get() != Some(request_id) {
                return;
            }
            let spinner = gtk::Spinner::new();
            spinner.add_css_class("preview-spinner");
            spinner.set_halign(gtk::Align::Center);
            spinner.set_valign(gtk::Align::Center);
            spinner.set_vexpand(true);
            spinner.start();
            state.content.append(&spinner);
        });
        self.loading_delay.replace(Some(source));
    }

    fn cancel_loading(&self) {
        if let Some(source) = self.loading_delay.borrow_mut().take() {
            source.remove();
        }
    }

    fn show_media_error(&self, error: &glib::Error) {
        let message = error.message();
        let (title, detail, command) = media_error_feedback(message);
        self.show_message_with_icon(
            title,
            &detail,
            Some(crate::assets::icons::TRIANGLE_ALERT),
            command,
        );
    }

    fn show_message(&self, title: &str, detail: &str) {
        self.show_message_with_icon(title, detail, None, None);
    }

    fn show_message_with_icon(
        &self,
        title: &str,
        detail: &str,
        icon: Option<&str>,
        command: Option<&str>,
    ) {
        self.clear_content();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 7);
        box_.add_css_class("preview-feedback");
        box_.set_halign(gtk::Align::Center);
        box_.set_valign(gtk::Align::Center);
        box_.set_vexpand(true);
        if let Some(icon) = icon {
            let icon = crate::assets::primary_icon(icon, 34);
            icon.add_css_class("preview-feedback-icon");
            icon.set_halign(gtk::Align::Center);
            box_.append(&icon);
        }
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("preview-feedback-title");
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("preview-feedback-detail");
        detail.set_justify(gtk::Justification::Center);
        detail.set_wrap(true);
        box_.append(&heading);
        box_.append(&detail);
        if let Some(command) = command {
            box_.append(&copyable_command(command));
        }
        self.content.append(&box_);
    }
}

fn copyable_command(command: &str) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("preview-command");
    overlay.set_hexpand(true);

    let field = gtk::Entry::new();
    field.add_css_class("form-control");
    field.add_css_class("preview-command-entry");
    field.set_text(command);
    field.set_editable(false);
    field.set_hexpand(true);
    overlay.set_child(Some(&field));

    let copy = gtk::Button::builder()
        .tooltip_text("Copy install command")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    copy.add_css_class("preview-command-copy");
    copy.set_has_frame(false);
    copy.set_cursor_from_name(Some("pointer"));
    let copy_icon = crate::assets::primary_icon(crate::assets::icons::COPY, 16);
    copy.set_child(Some(&copy_icon));
    let copied_command = command.to_owned();
    let feedback_generation = Rc::new(Cell::new(0_u64));
    copy.connect_clicked(move |button| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&copied_command);
        }
        let generation = feedback_generation.get().saturating_add(1);
        feedback_generation.set(generation);
        crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::CHECK);
        button.set_tooltip_text(Some("Install command copied"));
        let button = button.clone();
        let copy_icon = copy_icon.clone();
        let feedback_generation = feedback_generation.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if feedback_generation.get() == generation {
                crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::COPY);
                button.set_tooltip_text(Some("Copy install command"));
            }
        });
    });
    overlay.add_overlay(&copy);
    overlay
}

fn media_error_feedback(message: &str) -> (&'static str, String, Option<&'static str>) {
    let normalized = message.to_ascii_lowercase();
    if [
        "gstreamer",
        "plug-in",
        "plugin",
        "missing decoder",
        "no decoder",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return (
            "Additional media support required",
            "On Arch or Omarchy, install the required GStreamer plugins, then restart Strata."
                .to_owned(),
            Some(MEDIA_PLUGIN_INSTALL_COMMAND),
        );
    }
    (
        "Preview unavailable",
        format!("Unable to play this media preview: {message}"),
        None,
    )
}

fn metadata_value(label: &str) -> (gtk::Box, gtk::Label) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 2);
    group.set_hexpand(true);
    group.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(label));
    heading.add_css_class("preview-metadata-label");
    heading.set_xalign(0.0);
    let value = gtk::Label::new(Some("—"));
    value.add_css_class("preview-metadata-value");
    value.set_ellipsize(gtk::pango::EllipsizeMode::End);
    value.set_xalign(0.0);
    group.append(&heading);
    group.append(&value);
    (group, value)
}

fn set_pdf_page_texture(
    overlay: &gtk::Overlay,
    picture: &gtk::Picture,
    png: Vec<u8>,
    target_width: i32,
) {
    let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) else {
        return;
    };
    picture.set_paintable(Some(&texture));
    resize_pdf_page(overlay, picture, target_width);
}

fn preview_width_for_empty_space(available: i32, occupied: i32) -> i32 {
    available
        .saturating_sub(occupied)
        .saturating_mul(9)
        .saturating_div(10)
        .max(MIN_WIDTH)
}

fn pdf_zoom_after_scroll(current: f64, dy: f64) -> f64 {
    (current * (-dy * 0.14).exp()).clamp(PDF_MIN_ZOOM, PDF_MAX_ZOOM)
}

fn pdf_page_width(scroll: &gtk::ScrolledWindow, zoom: f64) -> i32 {
    let fit_width = scroll.width().saturating_sub(PDF_PAGE_GAP * 2).max(1);
    (f64::from(fit_width) * zoom).round() as i32
}

fn resize_pdf_pages(pages: &HashMap<i32, (gtk::Overlay, gtk::Picture)>, width: i32) {
    for (overlay, picture) in pages.values() {
        resize_pdf_page(overlay, picture, width);
    }
}

fn resize_pdf_page(overlay: &gtk::Overlay, picture: &gtk::Picture, target_width: i32) {
    let Some(paintable) = picture.paintable() else {
        overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
        return;
    };
    let texture_width = paintable.intrinsic_width();
    let texture_height = paintable.intrinsic_height();
    let width = if target_width > 0 {
        target_width
    } else if overlay.width() > 1 {
        overlay.width()
    } else {
        return;
    };
    if texture_width > 0 && texture_height > 0 {
        let ratio = f64::from(texture_width) / f64::from(texture_height);
        overlay.set_size_request(width, (f64::from(width) / ratio).round() as i32);
    }
}

fn preserve_pdf_view_center(scroll: &gtk::ScrolledWindow, factor: f64) {
    let horizontal = scroll.hadjustment();
    let vertical = scroll.vadjustment();
    let horizontal_center = horizontal.value() + horizontal.page_size() / 2.0;
    let vertical_center = vertical.value() + vertical.page_size() / 2.0;
    glib::idle_add_local_once(glib::clone!(
        #[weak]
        scroll,
        move || {
            set_adjustment_value(
                &scroll.hadjustment(),
                horizontal_center * factor - scroll.hadjustment().page_size() / 2.0,
            );
            set_adjustment_value(
                &scroll.vadjustment(),
                vertical_center * factor - scroll.vadjustment().page_size() / 2.0,
            );
        }
    ));
}

fn set_adjustment_value(adjustment: &gtk::Adjustment, value: f64) {
    let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(value.clamp(adjustment.lower(), maximum));
}

fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

fn metadata_size(entry: &FileEntry) -> String {
    match entry.size {
        MetadataValue::Known(bytes) => format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => "—".to_owned(),
    }
}

fn file_extension(entry: &FileEntry) -> &str {
    entry
        .location
        .native_path()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .unwrap_or("file")
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn set_preview_mute(
    media: &impl IsA<gtk::MediaStream>,
    icon: &gtk::Image,
    preferences: &Rc<super::theme::ThemeManager>,
    muted: bool,
) {
    media.set_muted(muted);
    crate::assets::set_primary_icon(
        icon,
        if muted {
            crate::assets::icons::VOLUME_X
        } else {
            crate::assets::icons::VOLUME_2
        },
    );
    preferences.set_preview_muted(muted);
}

fn set_preview_volume(
    media: &impl IsA<gtk::MediaStream>,
    preferences: &Rc<super::theme::ThemeManager>,
    slider: &Option<gtk::Scale>,
    icon: &gtk::Image,
    volume: f64,
) {
    media.set_volume(volume);
    if volume > 0.0 {
        preferences.set_preview_volume(volume);
    }
    if let Some(slider) = slider {
        slider.set_value(volume);
    }
    let muted = volume == 0.0;
    if preferences.preview_muted() != muted {
        set_preview_mute(media, icon, preferences, muted);
    }
}

fn sync_media_time(
    media: &impl IsA<gtk::MediaStream>,
    time_label: &gtk::Label,
    seek: &gtk::Scale,
    seeking: &Rc<Cell<bool>>,
) {
    let timestamp = media.timestamp();
    let duration = media.duration();
    let max = duration.max(timestamp) as f64;
    let adjustment = seek.adjustment();
    if adjustment.upper() != max {
        adjustment.set_upper(max);
        seek.set_sensitive(max > 0.0 && media.is_seekable());
    }
    if !seeking.get() {
        seek.set_value(timestamp as f64);
    }
    time_label.set_text(&format_media_time(timestamp, duration));
}

fn format_media_time(timestamp_us: i64, duration_us: i64) -> String {
    format!("{}/{}", fmt_time(timestamp_us), fmt_time(duration_us))
}

fn fmt_time(microseconds: i64) -> String {
    let total_seconds = microseconds.max(0) / 1_000_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn preview_drag_entries(entry: Option<&FileEntry>) -> Option<Vec<FileEntry>> {
    entry.cloned().map(|entry| vec![entry])
}

fn install_preview_drag(widget: &impl IsA<gtk::Widget>, state: &Rc<PreviewState>) {
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    drag.connect_prepare(move |source, x, y| {
        let state = weak.upgrade()?;
        let entries = preview_drag_entries(state.current.borrow().as_ref())?;
        let paintable = gtk::WidgetPaintable::new(Some(&state.header_handle));
        source.set_icon(Some(&paintable), x.round() as i32, y.round() as i32);
        super::browser::file_drag_content(&entries)
    });
    let weak = Rc::downgrade(state);
    drag.connect_drag_begin(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.content.add_css_class("dragging");
        }
    });
    let weak = Rc::downgrade(state);
    drag.connect_drag_end(move |_, _, _| {
        if let Some(state) = weak.upgrade() {
            state.content.remove_css_class("dragging");
            super::browser::slide_out(&state.content);
        }
    });
    widget.add_controller(drag);
}

#[cfg(test)]
mod tests;
