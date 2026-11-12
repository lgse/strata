// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use sourceview5::prelude::*;

use crate::{
    app::{Browser, BrowserEvent},
    model::{EntryKind, FileEntry, MetadataValue},
    services::{
        ArchivePreviewTree, DocumentLayout, LoadHandle, MediaPreviewSize, PdfTextLayer, Preview,
        PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
        SecretString,
        file_activity::{self, Notice},
        normalize_preview_text,
    },
};

use super::keys::{
    self, KeyPress,
    bindings::{
        COPY, MEDIA_MUTE, MEDIA_NEXT, MEDIA_PLAY_PAUSE, MEDIA_PREVIOUS, MEDIA_SEEK_BACK,
        MEDIA_SEEK_FORWARD, MEDIA_VOLUME_DOWN, MEDIA_VOLUME_UP, PREVIEW, SELECT_ALL,
    },
};
use super::{browser::format_file_size, controls::form_password_entry};

mod archive;
mod archive_view;
pub(super) mod audio;
mod components;
mod document;
mod drawer;
mod ease_in;
mod feedback;
mod find;
mod keyboard;
mod layout;
mod lifecycle;
mod loading;
mod media_content;
mod media_controls;
mod media_render;
mod media_session;
mod pdf;
mod render;
mod source_view;
pub(super) use document::document_notice;
use document::{DocumentPreview, DocumentView};
pub(in crate::ui) use layout::{separator, separator_width};
use media_controls::{MediaControls, VolumeControls, media_last_frame, snapped_volume};
use media_render::SandboxedPreview;
pub(super) use media_session::ListingPosition;
use media_session::{
    AudioPreview, MediaFamily, MediaTrack, PlaybackContinuation, VideoPreview, entry_family,
    media_family,
};
#[cfg(test)]
use pdf::{pdf_apply_ranges, pdf_desired_ranges, pdf_drop_unselected_layer, pdf_selected_text};
use source_view::{PendingSourcePreview, SourcePreviewView};
pub(in crate::ui) mod media_layout;
mod model;
#[cfg(test)]
mod pdf_ranges_tests;
mod pdf_text;
mod player_layout;
mod route;
mod session;
pub(super) mod video;
mod waveform;
mod zoom;

pub(in crate::ui) const DEFAULT_WIDTH: i32 = 520;
pub(in crate::ui) const MIN_WIDTH: i32 = 240;
const MAX_WIDTH: i32 = 3_000;
const TEXT_BYTE_LIMIT: usize = 1024 * 1024;
const SOURCE_INSERT_CHUNK_BYTES: usize = 4 * 1024;
const SOURCE_INSERT_CHUNK_LINES: usize = 64;
pub(crate) const FOCUS_PREVIEW_DELAY: Duration = Duration::from_millis(75);
const PREVIEW_SPINNER_DELAY: Duration = Duration::from_millis(120);
const TRANSITION: Duration = Duration::from_millis(260);
const PDF_PAGE_GAP: i32 = 6;
const HANDOFF_MARGIN_US: u64 = 1_000_000;
const MEDIA_PLUGIN_INSTALL_COMMAND: &str =
    "sudo pacman -S --needed gst-plugins-base gst-plugins-good";
const MEDIA_SEEK_STEP_US: i64 = 5_000_000;
const MEDIA_SEEK_PAGE_US: i64 = 30_000_000;
const MEDIA_VOLUME_STEP: f64 = 0.1;
const MEDIA_VOLUME_PAGE: f64 = 0.2;

pub(crate) fn preview_target(entry: Option<FileEntry>) -> Option<FileEntry> {
    entry.filter(entry_supports_quick_preview)
}

pub(crate) fn entry_supports_quick_preview(entry: &FileEntry) -> bool {
    if !matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink) {
        return false;
    }
    if crate::services::is_model(entry)
        || crate::services::route_preview::RouteFormat::for_name(&entry.native_name).is_some()
    {
        return entry.location.native_path().is_some();
    }
    if crate::sandbox::CoverFormat::for_name(&entry.native_name).is_some() {
        return entry.location.native_path().is_some();
    }

    let native_path = entry.location.native_path();
    let (content_type, uncertain) =
        crate::services::guess_content_type(Path::new(&entry.native_name));
    let content = crate::services::content_family(&content_type);
    if crate::services::table::is_workbook(&content_type, &entry.native_name)
        || crate::sandbox::DocumentFormat::for_file(&content_type, &entry.native_name).is_some()
    {
        return native_path.is_some();
    }
    if native_path.is_none()
        && (matches!(content, PreviewContent::Pdf { .. })
            || (matches!(content, PreviewContent::Media)
                && !crate::services::supports_remote_video(&entry.native_name)))
    {
        return false;
    }
    if native_path.is_some()
        && crate::services::archive_preview_format(&entry.native_name).is_some()
    {
        return true;
    }
    // An uncertain name guess defers to the loader, which resolves the file's content type.
    !matches!(content, PreviewContent::Unsupported)
        || uncertain
        || gio::content_type_is_a(&content_type, "text/plain")
        || crate::services::has_plain_text_extension(&entry.native_name)
        || crate::services::is_extensionless_dotfile(&entry.native_name)
}

struct PreviewState {
    provider: Rc<dyn PreviewProvider>,
    revealer: gtk::Revealer,
    slot: gtk::Box,
    resize_grip: gtk::Box,
    reserve_columns: Cell<bool>,
    // Dismissing content stops selection-following without reclaiming its column slot.
    enabled: Cell<bool>,
    dismissed: Cell<bool>,
    child_pane: Cell<bool>,
    pane: gtk::Box,
    header_handle: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    size: gtk::Label,
    modified: gtk::Label,
    content_type: gtk::Label,
    content: gtk::Box,
    zoom: Rc<zoom::PreviewZoom>,
    document_view_button: gtk::Button,
    document_view_icon: gtk::Image,
    document_view: Cell<DocumentView>,
    document_preview: RefCell<Option<DocumentPreview>>,
    source_preview: SourcePreviewView,
    rendered_text: RefCell<Option<Rc<super::virtual_preview::VirtualPreviewState>>>,
    find: find::PreviewFind,
    find_button: gtk::Button,
    metadata: gtk::Box,
    raw_details: super::raw_details::RawDetails,
    raw_details_scroll: gtk::ScrolledWindow,
    raw_metadata_load: RefCell<Option<super::raw_details::MetadataLoad>>,
    open: gtk::Button,
    allow_external_open: bool,
    wrap: gtk::ToggleButton,
    text_view: RefCell<Option<sourceview5::View>>,
    text_scroll: RefCell<Option<gtk::ScrolledWindow>>,
    archive_browser: RefCell<Option<archive::ArchiveBrowser>>,
    password_entry: RefCell<Option<gtk::PasswordEntry>>,
    media: RefCell<Option<gtk::MediaStream>>,
    media_signals: RefCell<Vec<glib::SignalHandlerId>>,
    media_volume_slider: RefCell<Option<gtk::Scale>>,
    media_toggle_mute: RefCell<Option<Rc<dyn Fn()>>>,
    audio: RefCell<Option<AudioPreview>>,
    video: RefCell<Option<VideoPreview>>,
    model: RefCell<Option<model::ModelPreview>>,
    route: RefCell<Option<Rc<route::RoutePlayer>>>,
    model_spinner: gtk::Spinner,
    model_view: Cell<crate::services::ModelView>,
    handed_off: RefCell<Option<crate::model::Location>>,
    continue_playback: RefCell<Option<PlaybackContinuation>>,
    media_checks: RefCell<media_content::MediaChecks>,
    split: RefCell<Option<gtk::Paned>>,
    sizing: layout::SplitSizing,
    current: RefCell<Option<FileEntry>>,
    current_depth: Cell<Option<usize>>,
    pending_show: RefCell<Option<glib::SourceId>>,
    load: RefCell<Option<LoadHandle>>,
    loading_delay: RefCell<Option<glib::SourceId>>,
    pdf_loads: Rc<RefCell<HashMap<i32, LoadHandle>>>,
    current_request: Cell<Option<PreviewRequestId>>,
    next_request: Cell<u64>,
    // Only explicit opens claim keyboard focus; implicit preview updates must not steal it.
    focus_archive_on_ready: Cell<bool>,
    focus_archive_request: Cell<Option<PreviewRequestId>>,
    enabled_action: gio::SimpleAction,
    animating: Cell<bool>,
    /// A closed preview's drawer is still sliding out.
    closing: Cell<bool>,
    animation_generation: Rc<Cell<u64>>,
    keyboard_view: RefCell<Option<super::browser::WeakBrowserView>>,
    claim_on_resume: Cell<bool>,
    released_from: Cell<Option<Option<usize>>>,
    operations: RefCell<Option<file_activity::Subscription>>,
}

use keyboard::MediaSeek;
pub(in crate::ui) use keyboard::{
    ArchiveMove, DocumentScroll, MediaCommand, PreviewSurface, media_key_command,
};

#[cfg(test)]
mod tests;

pub(crate) const PREVIEW_LABEL: &str = "Preview";

#[derive(Clone)]
pub struct PreviewDrawer {
    state: Rc<PreviewState>,
}

impl PreviewDrawer {
    #[cfg(test)]
    pub(in crate::ui) fn media_for_test(&self) -> Option<gtk::MediaStream> {
        self.state.media.borrow().clone()
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.slot.clone().upcast()
    }

    /// Drawn only while the preview shows content or a placeholder.
    pub(in crate::ui) fn pane_widget(&self) -> gtk::Widget {
        self.state.pane.clone().upcast()
    }

    pub fn is_open(&self) -> bool {
        self.state.revealer.reveals_child()
    }

    pub fn has_video(&self) -> bool {
        self.is_open() && !self.state.sizing.is_suspended() && self.state.has_media_view()
    }

    pub fn show(&self, entry: FileEntry, depth: Option<usize>) {
        self.state.reserve_columns.set(true);
        self.state.set_enabled(true);
        if let Some(entry) = preview_target(Some(entry)) {
            self.state.show(entry, depth);
        } else {
            self.state.clear_target();
        }
    }

    pub fn show_after_focus_change(&self, entry: FileEntry, depth: Option<usize>) {
        self.state.show_after_focus_change(entry, depth);
    }

    pub fn close(&self) {
        self.state.close();
    }

    pub(in crate::ui) fn password_has_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.state
            .password_entry
            .borrow()
            .as_ref()
            .is_some_and(|entry| {
                focused.is_some_and(|focus| {
                    focus == entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(entry)
                })
            })
    }

    pub(in crate::ui) fn archive_key(&self, motion: ArchiveMove) -> bool {
        self.state.archive_key(motion)
    }

    pub fn close_archive(&self) -> bool {
        self.state.close_archive()
    }

    pub fn archive_list_has_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        self.state.archive_list_has_focus(focused)
    }

    pub fn toggle(&self, entry: Option<FileEntry>, depth: Option<usize>) {
        self.state.toggle(entry, depth);
    }
}

impl Drop for PreviewState {
    fn drop(&mut self) {
        self.stop();
    }
}

fn metadata_value(description: &str) -> gtk::Label {
    let value = gtk::Label::new(Some("—"));
    value.set_ellipsize(gtk::pango::EllipsizeMode::End);
    value.set_xalign(0.0);
    super::accessibility::set_description(&value, Some(&crate::i18n::tr(description)));
    value
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

fn preview_drag_entries(entry: Option<&FileEntry>) -> Option<Vec<FileEntry>> {
    entry.cloned().map(|entry| vec![entry])
}

fn install_preview_drag(widget: &impl IsA<gtk::Widget>, state: &Rc<PreviewState>) {
    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
        .build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    drag.connect_prepare(move |source, _, _| {
        let state = weak.upgrade()?;
        let entries = preview_drag_entries(state.current.borrow().as_ref())?;
        if let Some((texture, hot_x, hot_y)) =
            super::browser::drag_preview_icon(&state.header_handle, &entries)
        {
            source.set_icon(Some(&texture), hot_x, hot_y);
        }
        super::browser::file_drag_content(source, &entries)
    });
    let weak = Rc::downgrade(state);
    drag.connect_drag_begin(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.content.add_css_class("dragging");
        }
    });
    let weak = Rc::downgrade(state);
    drag.connect_drag_end(move |_, _, _| {
        super::browser::clear_active_drag();
        if let Some(state) = weak.upgrade() {
            state.content.remove_css_class("dragging");
            super::browser::slide_out(&state.content);
        }
    });
    widget.add_controller(drag);
}
